//! Which Claude account Claude Code is signed in to, read from its config file. No credentials are
//! involved, and it is how Clinch notices an account switch: the old account's token keeps working,
//! so nothing else would say the gauges now describe the wrong account.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Deserialize;

#[derive(Deserialize)]
struct ClaudeConfig {
    #[serde(rename = "oauthAccount")]
    oauth_account: Option<OauthAccount>,
}

#[derive(Deserialize)]
struct OauthAccount {
    #[serde(rename = "accountUuid")]
    account_uuid: Option<String>,
}

/// `None` when the file can't be read or parsed, `Some(None)` when it says no account is signed in.
fn read_account(path: &Path) -> Option<Option<String>> {
    let contents = std::fs::read(path).ok()?;
    let config: ClaudeConfig = serde_json::from_slice(&contents).ok()?;
    Some(
        config
            .oauth_account
            .and_then(|account| account.account_uuid),
    )
}

/// Re-reads the config only when it changes, since Claude Code rewrites it often and it is large.
pub struct ClaudeAccountWatcher {
    path: PathBuf,
    modified: Option<SystemTime>,
    account: Option<String>,
}

impl ClaudeAccountWatcher {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            modified: None,
            account: None,
        }
    }

    /// The signed-in account. A file that fails to read, such as one caught mid-write, keeps the
    /// last known answer rather than looking like a switch.
    pub fn current(&mut self) -> Option<&str> {
        let modified = std::fs::metadata(&self.path)
            .and_then(|metadata| metadata.modified())
            .ok();
        if modified.is_some() && modified != self.modified {
            self.modified = modified;
            if let Some(account) = read_account(&self.path) {
                self.account = account;
            }
        }
        self.account.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn write_config(path: &Path, contents: &str, age_secs: u64) {
        std::fs::write(path, contents).unwrap();
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(age_secs))
            .unwrap();
    }

    #[test]
    fn follows_the_signed_in_account_and_survives_a_bad_read() {
        let dir =
            std::env::temp_dir().join(format!("clinch-claude-account-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".claude.json");
        let mut watcher = ClaudeAccountWatcher::new(path.clone());
        assert_eq!(watcher.current(), None);

        write_config(
            &path,
            r#"{"oauthAccount":{"accountUuid":"a"},"other":1}"#,
            30,
        );
        assert_eq!(watcher.current(), Some("a"));

        write_config(&path, r#"{"oauthAccount":{"accountUuid":"b"}}"#, 20);
        assert_eq!(watcher.current(), Some("b"));

        // Caught mid-write: not a switch.
        write_config(&path, r#"{"oauthAccount":{"accou"#, 10);
        assert_eq!(watcher.current(), Some("b"));

        // Signed out.
        write_config(&path, r#"{"numStartups":3}"#, 5);
        assert_eq!(watcher.current(), None);

        std::fs::remove_dir_all(dir).unwrap();
    }
}
