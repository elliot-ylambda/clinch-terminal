//! Local setting that gates local control.
//!
//! Clinch stores this non-secret preference in an owner-only file so a new app
//! signature cannot make it unreadable. Other apps retain secure storage.
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use settings::macros::define_settings_group;
use settings::{SecureSetting, Setting, SupportedPlatforms, SyncToCloud};
use warp_core::channel::{Channel, ChannelState};
use warpui::{AppContext, ModelContext};
use warpui_extras::secure_storage::{self, AppContextExt as _};

const LOCAL_CONTROL_MODE_STORAGE_KEY: &str = "LocalControlMode";

/// User-selected local-control availability.
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
    description = "Whether local control is enabled.",
    rename_all = "snake_case"
)]
pub enum LocalControlMode {
    #[default]
    Disabled,
    Enabled,
}

fn is_clinch_app_id(app_id: &str) -> bool {
    matches!(app_id, "sh.clinch.Clinch" | "sh.clinch.ClinchDev")
}

/// Local control works out of the box in Clinch. Other public Warp channels
/// retain their opt-in default, while internal dogfood builds remain enabled.
fn default_mode_for_app(channel: Channel, app_id: &str) -> LocalControlMode {
    if is_clinch_app_id(app_id) || channel.is_dogfood() {
        LocalControlMode::Enabled
    } else {
        LocalControlMode::Disabled
    }
}

impl LocalControlMode {
    pub const ALL: [Self; 2] = [Self::Disabled, Self::Enabled];

    pub fn is_enabled(self) -> bool {
        matches!(self, Self::Enabled)
    }

    pub fn as_dropdown_label(self) -> &'static str {
        match self {
            Self::Disabled => "Disabled",
            Self::Enabled => "Enabled",
        }
    }
}

define_settings_group!(LocalControlSettings, settings: [
    local_control_mode: LocalControlModeSetting,
]);

/// Setting wrapper for the authoritative local-control mode.
pub struct LocalControlModeSetting {
    inner: LocalControlMode,
    is_explicitly_set: bool,
    local_path: Option<PathBuf>,
    load_error: Option<String>,
}

impl LocalControlModeSetting {
    /// Why startup left control disabled. An explicit selection recovers it.
    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    fn from_storage(
        storage: &dyn secure_storage::SecureStorage,
        local_path: Option<PathBuf>,
    ) -> Self {
        let loaded = (|| -> Result<Option<LocalControlMode>> {
            if let Some(path) = &local_path {
                if let Some(mode) = read_local_mode(path)? {
                    return Ok(Some(mode));
                }
            }
            let legacy = (|| -> Result<Option<LocalControlMode>> {
                let value = match storage.read_value(Self::storage_key()) {
                    Ok(value) => value,
                    Err(secure_storage::Error::NotFound) => return Ok(None),
                    // Debug retains the underlying Security.framework OSStatus;
                    // Display currently reduces these errors to "unknown error".
                    Err(err) => {
                        anyhow::bail!("Could not read the saved local control setting: {err:?}")
                    }
                };
                serde_json::from_str(&value)
                    .context("The saved local control setting is invalid")
                    .map(Some)
            })();
            let mode = match legacy {
                Ok(mode) => mode,
                Err(err) if local_path.is_some() => {
                    // Clinch enables local control by default, including upgrades whose
                    // old app signature can no longer read the Keychain preference.
                    // Readable opt-outs above are preserved. Persist this default once
                    // without changing the inaccessible legacy item. Existing local
                    // preference errors have already returned, so they cannot reset an
                    // explicit choice to the default.
                    log::warn!("Using Clinch's enabled local control default: {err:#}");
                    Some(LocalControlMode::Enabled)
                }
                Err(err) => return Err(err),
            };
            if let (Some(path), Some(mode)) = (&local_path, mode) {
                write_local_mode(path, mode)
                    .context("Could not migrate the local control setting")?;
            }
            Ok(mode)
        })();
        let mut setting = match loaded {
            Ok(value) => Self::new(value),
            Err(err) => {
                log::error!("Failed to load local control setting: {err:#}");
                let mut setting = Self::new(Some(LocalControlMode::Disabled));
                setting.load_error = Some(format!(
                    "Local control is off because its saved setting could not be read. Select Enabled or Disabled to save your choice again. {err:#}"
                ));
                setting
            }
        };
        setting.local_path = local_path;
        setting
    }

    fn persist_value(&self, mode: LocalControlMode, ctx: &AppContext) -> Result<bool> {
        if let Some(path) = &self.local_path {
            // A direct user choice supersedes an inaccessible legacy value,
            // without reading, overwriting, or deleting the Keychain item.
            write_local_mode(path, mode)?;
            Ok(true)
        } else {
            Self::write_to_secure_storage(&mode, ctx)
        }
    }

    fn emit_changed(
        ctx: &mut ModelContext<LocalControlSettings>,
        change_event_reason: settings::ChangeEventReason,
    ) {
        ctx.emit(LocalControlSettingsChangedEvent::LocalControlModeSetting {
            change_event_reason,
        });
    }
}

impl SecureSetting for LocalControlModeSetting {
    fn write_secure_storage_value(
        storage: &dyn secure_storage::SecureStorage,
        key: &str,
        value: &str,
    ) -> Result<(), secure_storage::Error> {
        storage.write_value_with_owner_only_fallback(key, value)
    }
}
impl Setting for LocalControlModeSetting {
    type Group = LocalControlSettings;
    type Value = LocalControlMode;

    fn new(value: Option<Self::Value>) -> Self {
        Self {
            inner: value.unwrap_or_else(Self::default_value),
            is_explicitly_set: value.is_some(),
            local_path: clinch_mode_path(),
            load_error: None,
        }
    }

    fn setting_name() -> &'static str {
        "LocalControlModeSetting"
    }

    fn storage_key() -> &'static str {
        LOCAL_CONTROL_MODE_STORAGE_KEY
    }

    fn supported_platforms() -> SupportedPlatforms {
        SupportedPlatforms::DESKTOP
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
        if self.local_path.is_some() {
            // Keep a local value so clearing cannot resurrect a stale legacy
            // preference on the next launch.
            self.persist_value(Self::default_value(), ctx)?;
        } else {
            Self::clear_from_secure_storage(ctx)?;
        }
        self.inner = self.validate(Self::default_value());
        self.is_explicitly_set = false;
        self.load_error = None;
        Self::emit_changed(ctx, settings::ChangeEventReason::Clear);
        Ok(())
    }

    fn load_value(
        &mut self,
        new_value: Self::Value,
        explicitly_set: bool,
        ctx: &mut ModelContext<Self::Group>,
    ) -> Result<()> {
        let validated = self.validate(new_value);
        if self.value() != &validated || self.is_explicitly_set != explicitly_set {
            self.inner = validated;
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
        let changed_in_storage = self.persist_value(new_value, ctx)?;
        if self.value() != &new_value || changed_in_storage || self.load_error.is_some() {
            self.inner = self.validate(new_value);
            self.is_explicitly_set = true;
            self.load_error = None;
            Self::emit_changed(ctx, settings::ChangeEventReason::LocalChange);
        }
        Ok(())
    }

    fn default_value() -> Self::Value {
        default_mode_for_app(ChannelState::channel(), &ChannelState::app_id().to_string())
    }

    fn new_from_storage(ctx: &mut AppContext) -> Self {
        Self::from_storage(ctx.secure_storage(), clinch_mode_path())
    }

    fn is_supported_on_current_platform(&self) -> bool {
        SupportedPlatforms::DESKTOP.matches_current_platform()
    }

    fn is_value_explicitly_set(&self) -> bool {
        self.is_explicitly_set
    }
}

impl std::ops::Deref for LocalControlModeSetting {
    type Target = LocalControlMode;

    fn deref(&self) -> &Self::Target {
        self.value()
    }
}

impl LocalControlSettings {
    pub fn load_error(&self) -> Option<&str> {
        self.local_control_mode.load_error()
    }

    pub fn mode(&self) -> LocalControlMode {
        *self.local_control_mode
    }

    pub fn is_enabled(&self) -> bool {
        self.mode().is_enabled()
    }
}

fn clinch_mode_path() -> Option<PathBuf> {
    // Clinch currently ships on Unix, where these permissions enforce the
    // owner-only policy. Keep native secure storage on other platforms.
    (cfg!(unix) && is_clinch_app_id(&ChannelState::app_id().to_string())).then(|| {
        warp_core::paths::state_dir()
            .join("local-control-settings")
            .join("mode.json")
    })
}

fn validate_local_path(path: &Path, directory: bool) -> Result<std::fs::Metadata> {
    let metadata = std::fs::symlink_metadata(path)?;
    anyhow::ensure!(
        if directory {
            metadata.is_dir()
        } else {
            metadata.is_file()
        },
        "Local control preference path has an unexpected file type"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        anyhow::ensure!(
            metadata.uid() == nix::unistd::geteuid().as_raw() && metadata.mode() & 0o077 == 0,
            "Local control preference must be accessible only by its owner"
        );
    }
    Ok(metadata)
}

fn read_local_mode(path: &Path) -> Result<Option<LocalControlMode>> {
    let parent = path
        .parent()
        .context("Local control preference has no directory")?;
    let read = (|| -> Result<LocalControlMode> {
        validate_local_path(parent, true)?;
        validate_local_path(path, false)?;
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.custom_flags(nix::libc::O_NOFOLLOW);
        }
        let file = options.open(path)?;
        serde_json::from_reader(file.take(4096)).context("The local control preference is invalid")
    })();
    match read {
        Ok(mode) => Ok(Some(mode)),
        Err(err)
            if err
                .downcast_ref::<std::io::Error>()
                .is_some_and(|err| err.kind() == std::io::ErrorKind::NotFound) =>
        {
            Ok(None)
        }
        Err(err) => Err(err).context("Could not read the local control preference"),
    }
}

fn write_local_mode(path: &Path, mode: LocalControlMode) -> Result<()> {
    let parent = path
        .parent()
        .context("Local control preference has no directory")?;
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder
        .create(parent)
        .context("Could not create the local control preference directory")?;
    validate_local_path(parent, true)?;
    match std::fs::symlink_metadata(path) {
        Ok(_) => {
            validate_local_path(path, false)?;
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(&serde_json::to_vec(&mode)?)?;
    file.as_file().sync_all()?;
    file.persist(path)
        .map_err(|err| err.error)
        .context("Could not save the local control preference")?;
    Ok(())
}

#[cfg(test)]
#[path = "local_control_tests.rs"]
mod tests;
