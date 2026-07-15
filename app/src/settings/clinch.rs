use anyhow::Result;
use serde::{Deserialize, Serialize};
use settings::macros::define_settings_group;
use settings::{SecureSetting, Setting, SupportedPlatforms, SyncToCloud};
use warpui::{AppContext, ModelContext};
use warpui_extras::secure_storage;

const IMESSAGE_CONFIGURATION_STORAGE_KEY: &str = "ClinchIMessageConfiguration";

#[derive(
    Clone,
    Debug,
    Deserialize,
    Eq,
    PartialEq,
    schemars::JsonSchema,
    Serialize,
    settings_value::SettingsValue,
)]
pub struct IMessageConfiguration {
    pub enabled: bool,
    pub setup_complete: bool,
    #[serde(default = "default_imessage_notifications_enabled")]
    #[schemars(default = "default_imessage_notifications_enabled")]
    pub notifications_enabled_by_default: bool,
    pub recipient: String,
    pub chat_id: Option<i64>,
    pub chat_guid: Option<String>,
}

fn default_imessage_notifications_enabled() -> bool {
    true
}

impl Default for IMessageConfiguration {
    fn default() -> Self {
        Self {
            enabled: false,
            setup_complete: false,
            notifications_enabled_by_default: true,
            recipient: String::new(),
            chat_id: None,
            chat_guid: None,
        }
    }
}

pub struct IMessageConfigurationSetting {
    inner: IMessageConfiguration,
    is_explicitly_set: bool,
}

impl IMessageConfigurationSetting {
    fn emit_changed(
        ctx: &mut ModelContext<ClinchSettings>,
        change_event_reason: settings::ChangeEventReason,
    ) {
        ctx.emit(ClinchSettingsChangedEvent::IMessageConfigurationSetting {
            change_event_reason,
        });
    }
}

impl SecureSetting for IMessageConfigurationSetting {
    fn write_secure_storage_value(
        storage: &dyn secure_storage::SecureStorage,
        key: &str,
        value: &str,
    ) -> Result<(), secure_storage::Error> {
        storage.write_value_with_owner_only_fallback(key, value)
    }
}

impl Setting for IMessageConfigurationSetting {
    type Group = ClinchSettings;
    type Value = IMessageConfiguration;

    fn new(value: Option<Self::Value>) -> Self {
        match value {
            Some(value) => Self {
                inner: value,
                is_explicitly_set: true,
            },
            None => Self {
                inner: Self::default_value(),
                is_explicitly_set: false,
            },
        }
    }

    fn setting_name() -> &'static str {
        "IMessageConfigurationSetting"
    }

    fn storage_key() -> &'static str {
        IMESSAGE_CONFIGURATION_STORAGE_KEY
    }

    fn supported_platforms() -> SupportedPlatforms {
        SupportedPlatforms::MAC
    }

    fn sync_to_cloud() -> SyncToCloud {
        SyncToCloud::Never
    }

    fn is_private() -> bool {
        true
    }

    fn value(&self) -> &Self::Value {
        &self.inner
    }

    fn clear_value(&mut self, ctx: &mut ModelContext<Self::Group>) -> Result<()> {
        Self::clear_from_secure_storage(ctx)?;
        self.inner = Self::default_value();
        self.is_explicitly_set = false;
        Self::emit_changed(ctx, settings::ChangeEventReason::Clear);
        Ok(())
    }

    fn load_value(
        &mut self,
        new_value: Self::Value,
        explicitly_set: bool,
        ctx: &mut ModelContext<Self::Group>,
    ) -> Result<()> {
        if self.value() != &new_value || self.is_explicitly_set != explicitly_set {
            self.inner = new_value;
            self.is_explicitly_set = explicitly_set;
            Self::emit_changed(ctx, settings::ChangeEventReason::LocalChange);
        }
        Ok(())
    }

    fn set_value_from_cloud_sync(
        &mut self,
        _: Self::Value,
        _: &mut ModelContext<Self::Group>,
    ) -> Result<()> {
        Ok(())
    }

    fn set_value(
        &mut self,
        new_value: Self::Value,
        ctx: &mut ModelContext<Self::Group>,
    ) -> Result<()> {
        let changed_in_storage = Self::write_to_secure_storage(&new_value, ctx)?;
        if self.value() != &new_value || changed_in_storage {
            self.inner = new_value;
            self.is_explicitly_set = true;
            Self::emit_changed(ctx, settings::ChangeEventReason::LocalChange);
        }
        Ok(())
    }

    fn default_value() -> Self::Value {
        IMessageConfiguration::default()
    }

    fn new_from_storage(ctx: &mut AppContext) -> Self {
        Self::new(Self::read_from_secure_storage(ctx))
    }

    fn is_supported_on_current_platform(&self) -> bool {
        SupportedPlatforms::MAC.matches_current_platform()
    }

    fn is_value_explicitly_set(&self) -> bool {
        self.is_explicitly_set
    }
}

impl std::ops::Deref for IMessageConfigurationSetting {
    type Target = IMessageConfiguration;

    fn deref(&self) -> &Self::Target {
        self.value()
    }
}

define_settings_group!(ClinchSettings, settings: [
    imessage_configuration: IMessageConfigurationSetting,
    auto_create_worktrees_for_new_tabs: AutoCreateWorktreesForNewTabs {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "clinch.projects.auto_create_worktrees_for_new_tabs",
        description: "Create ordinary new terminal and Agent tabs in isolated Git worktrees based on main when the active project is a local Git repository.",
    }
]);

impl ClinchSettings {
    pub fn imessage(&self) -> &IMessageConfiguration {
        self.imessage_configuration.value()
    }
}

#[cfg(test)]
#[path = "clinch_tests.rs"]
mod tests;
