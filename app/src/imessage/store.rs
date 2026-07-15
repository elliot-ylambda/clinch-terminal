use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::domain::RouteState;

#[cfg(not(test))]
const STATE_DIRECTORY: &str = "imessage";
const STATE_FILE: &str = "route-state.json";

#[derive(Clone, Debug)]
pub(crate) struct RouteStateStore {
    path: PathBuf,
}

impl Default for RouteStateStore {
    fn default() -> Self {
        #[cfg(test)]
        let path = std::env::temp_dir()
            .join(format!("clinch-imessage-test-{}", uuid::Uuid::new_v4()))
            .join(STATE_FILE);
        #[cfg(not(test))]
        let path = warp_core::paths::state_dir()
            .join(STATE_DIRECTORY)
            .join(STATE_FILE);
        Self::new(path)
    }
}

impl RouteStateStore {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn load(&self) -> Result<RouteState> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(RouteState::default())
            }
            Err(error) => {
                return Err(error).with_context(|| format!("read {}", self.path.display()))
            }
        };
        let mut state: RouteState = serde_json::from_slice(&bytes)
            .with_context(|| format!("parse {}", self.path.display()))?;
        if state.version != RouteState::default().version {
            anyhow::bail!("unsupported iMessage route state version {}", state.version);
        }
        state.migrate_legacy_notification_overrides();
        Ok(state)
    }

    pub(crate) fn save(&self, state: &RouteState) -> Result<()> {
        let parent = self
            .path
            .parent()
            .context("iMessage route state path has no parent")?;
        create_owner_only_directory(parent)?;

        let temp = parent.join(format!(
            ".{STATE_FILE}.tmp.{}.{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let bytes = serde_json::to_vec(state).context("serialize iMessage route state")?;
        let result = (|| {
            let mut file = create_owner_only_file(&temp)?;
            file.write_all(&bytes)
                .with_context(|| format!("write {}", temp.display()))?;
            file.sync_all()
                .with_context(|| format!("sync {}", temp.display()))?;
            drop(file);
            fs::rename(&temp, &self.path)
                .with_context(|| format!("replace {}", self.path.display()))?;
            set_owner_only_file(&self.path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }

    pub(crate) fn clear(&self) -> Result<()> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).with_context(|| format!("remove {}", self.path.display())),
        }
    }
}

#[cfg(unix)]
fn create_owner_only_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt};

    let mut builder = fs::DirBuilder::new();
    builder.recursive(true).mode(0o700);
    builder
        .create(path)
        .with_context(|| format!("create {}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("secure {}", path.display()))
}

#[cfg(not(unix))]
fn create_owner_only_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("create {}", path.display()))
}

fn create_owner_only_file(path: &Path) -> Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options
        .open(path)
        .with_context(|| format!("create {}", path.display()))
}

#[cfg(unix)]
fn set_owner_only_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("secure {}", path.display()))
}

#[cfg(not(unix))]
fn set_owner_only_file(_path: &Path) -> Result<()> {
    Ok(())
}
