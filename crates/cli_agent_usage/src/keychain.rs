//! Read Claude Code's OAuth token from the OS secret store (macOS Keychain).

use serde::Deserialize;

pub const CLAUDE_SERVICE: &str = "Claude Code-credentials";

pub trait ReadSecret {
    /// Return the stored secret string for (service, account), or None.
    fn read(&self, service: &str, account: &str) -> Option<String>;
}

#[derive(Clone)]
pub struct ClaudeToken {
    pub access_token: String,
    pub expires_at_ms: Option<i64>,
}

impl std::fmt::Debug for ClaudeToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeToken")
            .field("access_token", &"<redacted>")
            .field("expires_at_ms", &self.expires_at_ms)
            .finish()
    }
}

impl ClaudeToken {
    pub fn is_expired(&self, now_ms: i64) -> bool {
        match self.expires_at_ms {
            Some(exp) => now_ms >= exp,
            None => false,
        }
    }
}

#[derive(Deserialize)]
struct Blob {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: Option<OAuth>,
}

#[derive(Deserialize)]
struct OAuth {
    #[serde(rename = "accessToken")]
    access_token: Option<String>,
    #[serde(rename = "expiresAt")]
    expires_at: Option<i64>,
}

pub fn parse_claude_token(blob: &str) -> Option<ClaudeToken> {
    let parsed: Blob = serde_json::from_str(blob).ok()?;
    let oauth = parsed.claude_ai_oauth?;
    let access_token = oauth.access_token?;
    if access_token.is_empty() {
        return None;
    }
    Some(ClaudeToken {
        access_token,
        expires_at_ms: oauth.expires_at,
    })
}

pub fn read_claude_token(reader: &dyn ReadSecret, account: &str) -> Option<ClaudeToken> {
    let blob = reader.read(CLAUDE_SERVICE, account)?;
    parse_claude_token(&blob)
}

/// Decide whether a poller should re-read the Keychain this tick.
///
/// The Keychain read is exactly what triggers the macOS "allow Clinch to read
/// Claude Code credentials" prompt, so we want to read as rarely as possible:
/// - If we hold an unexpired cached token, never read (`false`).
/// - Otherwise (no token, or the cached one has expired) read only if we have
///   never read, or at least `reread_backoff_ms` has elapsed since the last
///   read. The backoff matters when Claude Code's *stored* token is itself
///   expired (e.g. Claude Code hasn't run lately): without it, "expired cached
///   token" would be true every tick and we'd re-prompt every poll — the very
///   bug we are fixing. With it, re-reads are capped to one per backoff window.
pub fn should_read_keychain(
    cached: Option<&ClaudeToken>,
    last_read_ms: Option<i64>,
    now_ms: i64,
    reread_backoff_ms: i64,
) -> bool {
    if let Some(token) = cached {
        if !token.is_expired(now_ms) {
            return false;
        }
    }
    match last_read_ms {
        None => true,
        Some(last) => now_ms.saturating_sub(last) >= reread_backoff_ms,
    }
}

pub struct MacKeychain;

#[cfg(target_os = "macos")]
impl ReadSecret for MacKeychain {
    fn read(&self, service: &str, account: &str) -> Option<String> {
        let pw = security_framework::passwords::get_generic_password(service, account).ok()?;
        String::from_utf8(pw).ok()
    }
}

#[cfg(not(target_os = "macos"))]
impl ReadSecret for MacKeychain {
    fn read(&self, _service: &str, _account: &str) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(&'static str);
    impl ReadSecret for Fake {
        fn read(&self, _s: &str, _a: &str) -> Option<String> {
            Some(self.0.to_string())
        }
    }

    const BLOB: &str = r#"{"mcpOAuth":{},"claudeAiOauth":{"accessToken":"tok_abc","refreshToken":"r","expiresAt":1782879812921,"scopes":["user:inference"],"subscriptionType":"max"}}"#;

    #[test]
    fn parses_access_token_and_expiry() {
        let t = parse_claude_token(BLOB).unwrap();
        assert_eq!(t.access_token, "tok_abc");
        assert_eq!(t.expires_at_ms, Some(1782879812921));
        assert!(!t.is_expired(1782879812921 - 1000));
        assert!(t.is_expired(1782879812921 + 1000));
        assert!(t.is_expired(1782879812921)); // boundary: now == expiresAt -> expired (>=)
    }

    #[test]
    fn reads_via_provider() {
        let t = read_claude_token(&Fake(BLOB), "anyuser").unwrap();
        assert_eq!(t.access_token, "tok_abc");
    }

    #[test]
    fn garbage_blob_is_none() {
        assert!(parse_claude_token("not json").is_none());
    }

    fn token(expires_at_ms: i64) -> ClaudeToken {
        ClaudeToken {
            access_token: "tok".to_string(),
            expires_at_ms: Some(expires_at_ms),
        }
    }

    const BACKOFF: i64 = 300_000; // 5 min

    #[test]
    fn should_read_when_no_cached_token_and_never_read() {
        assert!(should_read_keychain(None, None, 1_000, BACKOFF));
    }

    #[test]
    fn should_not_read_while_cached_token_is_valid() {
        let t = token(10_000);
        // Cached, unexpired -> never read, regardless of last_read/backoff.
        assert!(!should_read_keychain(Some(&t), None, 5_000, BACKOFF));
        assert!(!should_read_keychain(Some(&t), Some(0), 9_999, BACKOFF));
    }

    #[test]
    fn expired_cached_token_reads_only_after_backoff() {
        let t = token(1_000); // expired at now=2_000
        let now = 2_000;
        // Just read 1s ago -> within backoff -> don't re-read (this is what stops
        // the every-60s re-prompt when the stored token is perpetually expired).
        assert!(!should_read_keychain(
            Some(&t),
            Some(now - 1_000),
            now,
            BACKOFF
        ));
        // Backoff elapsed -> allowed to re-read.
        assert!(should_read_keychain(
            Some(&t),
            Some(now - BACKOFF),
            now,
            BACKOFF
        ));
    }

    #[test]
    fn no_token_but_recent_read_respects_backoff() {
        // read returned nothing usable a moment ago -> wait out the backoff.
        assert!(!should_read_keychain(None, Some(1_000), 1_500, BACKOFF));
        assert!(should_read_keychain(
            None,
            Some(1_000),
            1_000 + BACKOFF,
            BACKOFF
        ));
    }

    #[test]
    fn debug_redacts_token() {
        let t = ClaudeToken {
            access_token: "SECRET".to_string(),
            expires_at_ms: Some(1234567890),
        };
        let debug_str = format!("{:?}", t);
        assert!(
            !debug_str.contains("SECRET"),
            "token must be redacted in debug output"
        );
        assert!(
            debug_str.contains("<redacted>"),
            "should show redaction marker"
        );
        assert!(debug_str.contains("1234567890"), "expiry should be visible");
    }
}
