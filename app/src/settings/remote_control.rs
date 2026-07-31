//! Secure, local-only enablement for Clinch Remote Control.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use settings::macros::define_settings_group;
use settings::{SecureSetting, Setting, SupportedPlatforms, SyncToCloud};
use warp_core::channel::ChannelState;
use warpui::{AppContext, ModelContext};
use warpui_extras::secure_storage;

const REMOTE_CONTROL_MODE_STORAGE_KEY: &str = "ClinchRemoteControlMode";

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Deserialize,
    Eq,
    PartialEq,
    schemars::JsonSchema,
    Serialize,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Whether the private Clinch Remote Control companion is enabled.",
    rename_all = "snake_case"
)]
pub enum RemoteControlMode {
    #[default]
    Disabled,
    Enabled,
}

impl RemoteControlMode {
    pub fn is_enabled(self) -> bool {
        matches!(self, Self::Enabled)
    }
}

define_settings_group!(RemoteControlSettings, settings: [
    mode: RemoteControlModeSetting,
]);

pub struct RemoteControlModeSetting {
    inner: RemoteControlMode,
    is_explicitly_set: bool,
}

impl RemoteControlModeSetting {
    fn emit_changed(
        ctx: &mut ModelContext<RemoteControlSettings>,
        change_event_reason: settings::ChangeEventReason,
    ) {
        ctx.emit(
            RemoteControlSettingsChangedEvent::RemoteControlModeSetting {
                change_event_reason,
            },
        );
    }
}

impl SecureSetting for RemoteControlModeSetting {
    fn write_secure_storage_value(
        storage: &dyn secure_storage::SecureStorage,
        key: &str,
        value: &str,
    ) -> Result<(), secure_storage::Error> {
        storage.write_value_with_owner_only_fallback(key, value)
    }
}

impl Setting for RemoteControlModeSetting {
    type Group = RemoteControlSettings;
    type Value = RemoteControlMode;

    fn new(value: Option<Self::Value>) -> Self {
        Self {
            inner: value.unwrap_or_default(),
            is_explicitly_set: value.is_some(),
        }
    }

    fn setting_name() -> &'static str {
        "RemoteControlModeSetting"
    }

    fn storage_key() -> &'static str {
        REMOTE_CONTROL_MODE_STORAGE_KEY
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
        if self.inner != new_value || self.is_explicitly_set != explicitly_set {
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
        if self.inner != new_value || changed_in_storage {
            self.inner = new_value;
            self.is_explicitly_set = true;
            Self::emit_changed(ctx, settings::ChangeEventReason::LocalChange);
        }
        Ok(())
    }

    fn default_value() -> Self::Value {
        RemoteControlMode::Disabled
    }

    fn new_from_storage(ctx: &mut AppContext) -> Self {
        // Remote Control belongs only to backend-free Clinch channels. Avoid even reading its
        // local Keychain entry while an inherited account-backed channel is running.
        if ChannelState::has_backend() {
            Self::new(None)
        } else {
            Self::new(Self::read_from_secure_storage(ctx))
        }
    }

    fn is_supported_on_current_platform(&self) -> bool {
        SupportedPlatforms::MAC.matches_current_platform()
    }

    fn is_value_explicitly_set(&self) -> bool {
        self.is_explicitly_set
    }
}

impl std::ops::Deref for RemoteControlModeSetting {
    type Target = RemoteControlMode;

    fn deref(&self) -> &Self::Target {
        self.value()
    }
}

impl RemoteControlSettings {
    pub fn mode(&self) -> RemoteControlMode {
        *self.mode
    }

    pub fn is_enabled(&self) -> bool {
        self.mode().is_enabled()
    }
}
