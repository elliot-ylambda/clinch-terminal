//! Best-effort, non-interactive access to Claude Code's existing OAuth token.
//! Provider sessions own their login; this optional reader only supports usage gauges.

use serde::Deserialize;

pub const CLAUDE_SERVICE: &str = "Claude Code-credentials";

/// `Unavailable` includes locked stores, denied access, and timeouts. It must
/// never be treated as proof that credentials are absent or need replacement.
pub enum SecretRead {
    Secret(String),
    ItemMissing,
    Unavailable,
}

impl std::fmt::Debug for SecretRead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Secret(_) => f.debug_tuple("Secret").field(&"<redacted>").finish(),
            Self::ItemMissing => f.write_str("ItemMissing"),
            Self::Unavailable => f.write_str("Unavailable"),
        }
    }
}

pub trait ReadSecret {
    /// Read without user interaction. Return `Unavailable` when permission or
    /// an unlocked store is required; never request a computer password.
    fn read(&self, service: &str, account: &str) -> SecretRead;
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
    match reader.read(CLAUDE_SERVICE, account) {
        SecretRead::Secret(blob) => parse_claude_token(&blob),
        SecretRead::ItemMissing | SecretRead::Unavailable => None,
    }
}

/// Decide whether a poller should attempt to re-acquire a token this tick.
///
/// All reads are non-interactive, so this backoff bounds read work:
/// - If we hold an unexpired cached token, never read (`false`).
/// - Otherwise (no token, or the cached one has expired) read only if we have
///   never read, or at least `reread_backoff_ms` has elapsed since the last
///   read. The backoff matters when Claude Code's *stored* token is itself
///   expired (e.g. Claude Code hasn't run lately): without it, "expired cached
///   token" would be true every tick and we'd read every poll. With it,
///   attempts are capped to one per backoff window.
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

/// Outcome of one bounded, non-interactive attempt to use the existing login.
pub enum TokenAcquisition {
    Token(ClaudeToken),
    ItemMissing,
    Unavailable,
    RetryLater,
}

/// Background refresh and explicit Retry clicks use this identical silent
/// path. A user gesture never grants permission to display a system dialog.
pub fn acquire_claude_token(reader: &dyn ReadSecret, account: &str) -> TokenAcquisition {
    match reader.read(CLAUDE_SERVICE, account) {
        SecretRead::Secret(blob) => parse_claude_token(&blob)
            .map(TokenAcquisition::Token)
            .unwrap_or(TokenAcquisition::RetryLater),
        SecretRead::ItemMissing => TokenAcquisition::ItemMissing,
        SecretRead::Unavailable => TokenAcquisition::Unavailable,
    }
}

pub struct MacKeychain;

#[cfg(target_os = "macos")]
mod native {
    use std::ffi::c_void;
    use std::ptr;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use security_framework_sys::base::{errSecItemNotFound, errSecSuccess};
    use security_framework_sys::keychain::{
        SecKeychainFindGenericPassword, SecKeychainGetUserInteractionAllowed,
    };
    use security_framework_sys::keychain_item::SecKeychainItemFreeContent;

    use super::SecretRead;

    const READ_TIMEOUT: Duration = Duration::from_secs(10);
    static READ_IN_FLIGHT: AtomicBool = AtomicBool::new(false);
    fn interaction_is_disabled() -> bool {
        // Clinch installs a permanent no-UI policy at startup. Do not change
        // process-wide policy here: account-backed builds may allow credential
        // dialogs for other features. Modern per-query flags do not protect
        // legacy login.keychain items, so fail closed unless UI is disabled.
        let mut allowed = 1;
        unsafe {
            SecKeychainGetUserInteractionAllowed(&mut allowed) == errSecSuccess && allowed == 0
        }
    }

    struct InFlight;
    impl Drop for InFlight {
        fn drop(&mut self) {
            READ_IN_FLIGHT.store(false, Ordering::Release);
        }
    }

    struct PasswordData(*mut c_void);
    impl Drop for PasswordData {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { SecKeychainItemFreeContent(ptr::null_mut(), self.0) };
            }
        }
    }

    fn read(service: &str, account: &str) -> SecretRead {
        if !interaction_is_disabled() {
            return SecretRead::Unavailable;
        }
        let (Ok(service_len), Ok(account_len)) =
            (u32::try_from(service.len()), u32::try_from(account.len()))
        else {
            return SecretRead::Unavailable;
        };
        let mut length = 0;
        let mut data = PasswordData(ptr::null_mut());
        let status = unsafe {
            SecKeychainFindGenericPassword(
                ptr::null(),
                service_len,
                service.as_ptr().cast(),
                account_len,
                account.as_ptr().cast(),
                &mut length,
                &mut data.0,
                ptr::null_mut(),
            )
        };
        if status == errSecItemNotFound {
            return SecretRead::ItemMissing;
        }
        if status != errSecSuccess || data.0.is_null() || length == 0 {
            return SecretRead::Unavailable;
        }
        // Security.framework owns this buffer until PasswordData is dropped.
        let bytes = unsafe { std::slice::from_raw_parts(data.0.cast::<u8>(), length as usize) };
        match std::str::from_utf8(bytes) {
            Ok(secret) => SecretRead::Secret(secret.to_owned()),
            Err(_) => SecretRead::Unavailable,
        }
    }

    pub(super) fn bounded_read(service: &str, account: &str) -> SecretRead {
        let service = service.to_owned();
        let account = account.to_owned();
        bounded_read_with(move || read(&service, &account), READ_TIMEOUT)
    }

    fn bounded_read_with(
        read: impl FnOnce() -> SecretRead + Send + 'static,
        timeout: Duration,
    ) -> SecretRead {
        if READ_IN_FLIGHT.swap(true, Ordering::AcqRel) {
            return SecretRead::Unavailable;
        }
        // The guard stays with a timed-out worker until it actually finishes,
        // preventing future polls from accumulating stuck native threads.
        let flight = InFlight;
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        if std::thread::Builder::new()
            .name("clinch-usage-keychain".into())
            .spawn(move || {
                let result = read();
                drop(flight);
                let _ = sender.send(result);
            })
            .is_err()
        {
            return SecretRead::Unavailable;
        }
        receiver
            .recv_timeout(timeout)
            .unwrap_or(SecretRead::Unavailable)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn timed_out_reads_do_not_accumulate_workers_and_recover_after_completion() {
            let (release, blocked) = std::sync::mpsc::sync_channel(1);
            assert!(matches!(
                bounded_read_with(
                    move || {
                        blocked.recv().unwrap();
                        SecretRead::ItemMissing
                    },
                    Duration::from_millis(10)
                ),
                SecretRead::Unavailable
            ));
            assert!(matches!(
                bounded_read_with(
                    || panic!("must not start a second read"),
                    Duration::from_secs(1)
                ),
                SecretRead::Unavailable
            ));
            release.send(()).unwrap();
            let deadline = instant::Instant::now() + Duration::from_secs(2);
            while READ_IN_FLIGHT.load(Ordering::Acquire) {
                assert!(instant::Instant::now() < deadline, "worker did not finish");
                std::thread::yield_now();
            }
            assert!(matches!(
                bounded_read_with(|| SecretRead::ItemMissing, Duration::from_secs(1)),
                SecretRead::ItemMissing
            ));
        }
    }
}

#[cfg(target_os = "macos")]
impl ReadSecret for MacKeychain {
    fn read(&self, service: &str, account: &str) -> SecretRead {
        native::bounded_read(service, account)
    }
}

#[cfg(not(target_os = "macos"))]
impl ReadSecret for MacKeychain {
    fn read(&self, _service: &str, _account: &str) -> SecretRead {
        SecretRead::ItemMissing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(&'static str);
    impl ReadSecret for Fake {
        fn read(&self, _s: &str, _a: &str) -> SecretRead {
            SecretRead::Secret(self.0.to_string())
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
    fn denied_read_stays_unavailable_on_retry() {
        struct Denied(std::cell::Cell<usize>);
        impl ReadSecret for Denied {
            fn read(&self, service: &str, account: &str) -> SecretRead {
                assert_eq!(service, CLAUDE_SERVICE);
                assert_eq!(account, "u");
                self.0.set(self.0.get() + 1);
                SecretRead::Unavailable
            }
        }
        let denied = Denied(std::cell::Cell::new(0));
        for _ in 0..2 {
            assert!(matches!(
                acquire_claude_token(&denied, "u"),
                TokenAcquisition::Unavailable
            ));
        }
        assert_eq!(denied.0.get(), 2);
    }

    #[test]
    fn missing_credential_and_denied_access_are_distinct() {
        struct Missing;
        impl ReadSecret for Missing {
            fn read(&self, _: &str, _: &str) -> SecretRead {
                SecretRead::ItemMissing
            }
        }
        assert!(matches!(
            acquire_claude_token(&Missing, "u"),
            TokenAcquisition::ItemMissing
        ));
        assert!(matches!(
            acquire_claude_token(&Fake(BLOB), "u"),
            TokenAcquisition::Token(_)
        ));
        assert!(matches!(
            acquire_claude_token(&Fake("bad json"), "u"),
            TokenAcquisition::RetryLater
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

    #[test]
    fn secret_read_debug_redacts_secret() {
        let debug_str = format!("{:?}", SecretRead::Secret("SECRET".to_string()));
        assert!(!debug_str.contains("SECRET"));
        assert!(debug_str.contains("<redacted>"));
    }
}
