//! Read Claude Code's OAuth token from the OS secret store (macOS Keychain).

use serde::Deserialize;

pub const CLAUDE_SERVICE: &str = "Claude Code-credentials";

pub trait ReadSecret {
    /// Return the stored secret string for (service, account), or None.
    fn read(&self, service: &str, account: &str) -> Option<String>;
}

#[derive(Debug, Clone)]
pub struct ClaudeToken {
    pub access_token: String,
    pub expires_at_ms: Option<i64>,
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
}
