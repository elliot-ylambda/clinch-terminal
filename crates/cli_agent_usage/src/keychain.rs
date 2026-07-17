//! Read Claude Code's OAuth token from the OS secret store (macOS Keychain).

use serde::Deserialize;

pub const CLAUDE_SERVICE: &str = "Claude Code-credentials";

pub trait ReadSecret {
    /// Return the stored secret string for (service, account), or None.
    fn read(&self, service: &str, account: &str) -> Option<String>;

    /// Report whether `read` would complete without raising a user prompt.
    /// Callers gate unsanctioned (non-user-gesture) reads on `Trusted`.
    /// Defaults to `Trusted` for stores whose reads never prompt.
    fn probe_trust(&self, service: &str, account: &str) -> KeychainTrust {
        let _ = (service, account);
        KeychainTrust::Trusted
    }
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

/// Decide whether a poller should attempt to re-acquire a token this tick.
///
/// Unsanctioned reads are additionally gated on [`ReadSecret::probe_trust`]
/// (they happen only when provably silent), so this backoff bounds probe/read
/// *work* rather than prompts:
/// - If we hold an unexpired cached token, never read (`false`).
/// - Otherwise (no token, or the cached one has expired) read only if we have
///   never read, or at least `reread_backoff_ms` has elapsed since the last
///   read. The backoff matters when Claude Code's *stored* token is itself
///   expired (e.g. Claude Code hasn't run lately): without it, "expired cached
///   token" would be true every tick and we'd probe every poll. With it,
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

/// Whether the item's ACL lets `/usr/bin/security` read the secret WITHOUT a
/// user prompt. Derived from `security dump-keychain -a` output, which is
/// metadata-only and never prompts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeychainTrust {
    /// The CLI read will complete silently.
    Trusted,
    /// The CLI read would raise the macOS keychain-password prompt.
    Untrusted,
    /// No such item — nothing to read, nothing to prompt about.
    ItemMissing,
}

/// What the poller should do about acquiring a token this round.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenReadPlan {
    /// A user gesture (Turn on / Authorize click) sanctioned a prompt.
    ReadAllowingPrompt,
    /// The ACL trusts the CLI; the read cannot prompt.
    ReadSilently,
    /// No credentials item exists; skip quietly.
    SkipItemMissing,
    /// Reading would prompt without a gesture; surface the Authorize
    /// affordance instead.
    SkipNeedsAuthorization,
}

/// Decide how to acquire the token given ACL trust and whether the user just
/// asked for it. The invariant this encodes: a keychain read that can prompt
/// happens ONLY as the direct result of a user gesture.
pub fn plan_token_read(trust: KeychainTrust, gesture: bool) -> TokenReadPlan {
    match (trust, gesture) {
        (KeychainTrust::ItemMissing, _) => TokenReadPlan::SkipItemMissing,
        (_, true) => TokenReadPlan::ReadAllowingPrompt,
        (KeychainTrust::Trusted, false) => TokenReadPlan::ReadSilently,
        (KeychainTrust::Untrusted, false) => TokenReadPlan::SkipNeedsAuthorization,
    }
}

/// Parse `security dump-keychain -a` output and report whether the generic
/// password item for (service, account) can be read by `/usr/bin/security`
/// without a prompt: the decrypt ACL entry must allow it (explicit `(OK)`
/// listing or a `<null>` allow-everyone list) and the partition list, if
/// present, must include `apple-tool:`.
pub fn parse_dump_trust(dump: &str, service: &str, account: &str) -> KeychainTrust {
    let svce_marker = format!("\"svce\"<blob>=\"{service}\"");
    let acct_marker = format!("\"acct\"<blob>=\"{account}\"");
    for item in split_dump_items(dump) {
        let has = |marker: &str| item.iter().any(|line| line.trim() == marker);
        if has(&svce_marker) && has(&acct_marker) {
            return trust_from_item(&item);
        }
    }
    KeychainTrust::ItemMissing
}

/// Group dump lines into per-item blocks; each item starts with an unindented
/// `keychain: "..."` line.
fn split_dump_items(dump: &str) -> Vec<Vec<&str>> {
    let mut items: Vec<Vec<&str>> = Vec::new();
    for line in dump.lines() {
        if line.starts_with("keychain: ") {
            items.push(Vec::new());
        }
        if let Some(current) = items.last_mut() {
            current.push(line);
        }
    }
    items
}

fn trust_from_item(lines: &[&str]) -> KeychainTrust {
    let mut security_may_decrypt = false;
    // Absent partition_id entry = pre-partition-list item, no extra gate.
    let mut partition_allows_apple_tools = true;

    for entry in split_acl_entries(lines) {
        let Some(authorizations) = entry.iter().find_map(|line| {
            line.trim()
                .strip_prefix("authorizations")
                .and_then(|rest| rest.split_once(':'))
                .map(|(_, list)| list)
        }) else {
            continue;
        };
        let grants = |auth: &str| authorizations.split_whitespace().any(|word| word == auth);

        if grants("decrypt") {
            security_may_decrypt |= entry.iter().any(|line| {
                let line = line.trim();
                // `<null>` means every application may use this entry.
                line == "applications: <null>"
                    || (line.contains("/usr/bin/security") && line.ends_with("(OK)"))
            });
        }
        if grants("partition_id") {
            partition_allows_apple_tools = entry
                .iter()
                .find_map(|line| line.trim().strip_prefix("description:"))
                .is_some_and(|ids| ids.split(',').any(|id| id.trim() == "apple-tool:"));
        }
    }

    if security_may_decrypt && partition_allows_apple_tools {
        KeychainTrust::Trusted
    } else {
        KeychainTrust::Untrusted
    }
}

/// Group the lines after `access:` into per-`entry N:` chunks.
fn split_acl_entries<'a>(lines: &[&'a str]) -> Vec<Vec<&'a str>> {
    let mut entries: Vec<Vec<&str>> = Vec::new();
    let mut in_acl = false;
    for line in lines {
        if line.starts_with("access:") {
            in_acl = true;
            continue;
        }
        if !in_acl {
            continue;
        }
        let trimmed = line.trim();
        if trimmed.starts_with("entry ") && trimmed.ends_with(':') {
            entries.push(Vec::new());
            continue;
        }
        if let Some(current) = entries.last_mut() {
            current.push(line);
        }
    }
    entries
}

pub struct MacKeychain;

/// Decode `security find-generic-password -w` stdout: the secret followed by
/// one trailing newline of CLI framing.
#[cfg(target_os = "macos")]
fn secret_from_security_stdout(stdout: Vec<u8>) -> Option<String> {
    let raw = String::from_utf8(stdout).ok()?;
    let secret = raw.strip_suffix('\n').unwrap_or(&raw);
    (!secret.is_empty()).then(|| secret.to_string())
}

#[cfg(target_os = "macos")]
impl ReadSecret for MacKeychain {
    /// Read via the Apple-signed `security` CLI rather than an in-process
    /// Security-framework call. Claude Code writes this item through the same
    /// CLI, so the item's ACL already trusts /usr/bin/security and the read
    /// completes without any per-app Keychain prompt. An in-process
    /// SecItemCopyMatching authorizes per app instead — and for debug bundles
    /// signed with get-task-allow, securityd refuses to persist "Always
    /// Allow", so that prompt reappeared on every read, forever.
    fn read(&self, service: &str, account: &str) -> Option<String> {
        let output = command::blocking::Command::new("/usr/bin/security")
            .args(["find-generic-password", "-s", service, "-a", account, "-w"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        secret_from_security_stdout(output.stdout)
    }

    /// Both probes are metadata-only and never raise a prompt: an
    /// attribute-only `find-generic-password` (no `-w`) proves existence, and
    /// `dump-keychain -a` exposes the ACL that decides whether the `-w` read
    /// in [`Self::read`] will be silent. The `apple-tool:`/`security` trust
    /// that Claude Code's CLI-written item starts with is NOT durable — any
    /// old native-API reader that triggers an "Always Allow" repair can
    /// rewrite the partition list out from under us — so this is checked
    /// before every unsanctioned read rather than assumed.
    fn probe_trust(&self, service: &str, account: &str) -> KeychainTrust {
        let exists = command::blocking::Command::new("/usr/bin/security")
            .args(["find-generic-password", "-s", service, "-a", account])
            .output();
        match exists {
            Ok(output) if output.status.success() => {}
            // Not found — or spawn failure (e.g. EAGAIN under load), where
            // "missing" fails quiet and self-heals on the next backoff tick.
            _ => return KeychainTrust::ItemMissing,
        }

        let dump = command::blocking::Command::new("/usr/bin/security")
            .args(["dump-keychain", "-a"])
            .output();
        let Ok(dump) = dump else {
            return KeychainTrust::Untrusted;
        };
        if !dump.status.success() {
            return KeychainTrust::Untrusted;
        }
        // An item that exists but is absent from the dumped default keychain
        // parses as ItemMissing; report Untrusted so the read stays behind an
        // explicit user gesture instead of prompting unbidden.
        match parse_dump_trust(&String::from_utf8_lossy(&dump.stdout), service, account) {
            KeychainTrust::ItemMissing => KeychainTrust::Untrusted,
            trust => trust,
        }
    }
}

#[cfg(not(target_os = "macos"))]
impl ReadSecret for MacKeychain {
    fn read(&self, _service: &str, _account: &str) -> Option<String> {
        None
    }

    fn probe_trust(&self, _service: &str, _account: &str) -> KeychainTrust {
        KeychainTrust::ItemMissing
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

    #[cfg(target_os = "macos")]
    mod security_stdout {
        use super::super::secret_from_security_stdout;

        #[test]
        fn trims_single_trailing_newline_only() {
            assert_eq!(
                secret_from_security_stdout(b"{\"k\":1}\n".to_vec()).as_deref(),
                Some("{\"k\":1}")
            );
            // No trailing newline is also valid output.
            assert_eq!(
                secret_from_security_stdout(b"{\"k\":1}".to_vec()).as_deref(),
                Some("{\"k\":1}")
            );
            // Only the final newline is CLI framing; inner ones belong to the secret.
            assert_eq!(
                secret_from_security_stdout(b"a\nb\n".to_vec()).as_deref(),
                Some("a\nb")
            );
        }

        #[test]
        fn empty_or_invalid_output_is_none() {
            assert!(secret_from_security_stdout(Vec::new()).is_none());
            assert!(secret_from_security_stdout(b"\n".to_vec()).is_none());
            assert!(secret_from_security_stdout(vec![0xff, 0xfe]).is_none());
        }
    }

    mod dump_trust {
        use super::super::{parse_dump_trust, plan_token_read, KeychainTrust, TokenReadPlan};

        const SERVICE: &str = "Claude Code-credentials";

        /// A dump block shaped exactly like real `security dump-keychain -a`
        /// output, parameterized by account, the decrypt entry's applications
        /// section, and the trailing (partition/other) entries.
        fn item(account: &str, decrypt_apps: &str, tail_entries: &str) -> String {
            format!(
                r#"keychain: "/Users/u/Library/Keychains/login.keychain-db"
version: 512
class: "genp"
attributes:
    0x00000007 <blob>="{SERVICE}"
    0x00000008 <blob>=<NULL>
    "acct"<blob>="{account}"
    "cdat"<timedate>=0x32303236303630333137343032385A00  "20260603174028Z\000"
    "svce"<blob>="{SERVICE}"
access: 5 entries
    entry 0:
        authorizations (6): decrypt derive export_clear export_wrapped mac sign
        don't-require-password
        description: {SERVICE}
{decrypt_apps}
    entry 1:
        authorizations (1): encrypt
        don't-require-password
        description: {SERVICE}
        applications: <null>
{tail_entries}"#
            )
        }

        const SECURITY_OK: &str = r#"        applications (2):
            0: /Applications/OldApp.app (status -2147415734)
                requirement: identifier "old.app" and anchor apple generic
            1: /usr/bin/security (OK)
                requirement: identifier "com.apple.security" and anchor apple"#;

        const SECURITY_STALE: &str = r#"        applications (1):
            0: /usr/bin/security (status -67068)
                requirement: identifier "com.apple.security" and anchor apple"#;

        const NO_SECURITY: &str = r#"        applications (1):
            0: /Applications/OldApp.app (OK)
                requirement: identifier "old.app" and anchor apple generic"#;

        const APPS_NULL: &str = "        applications: <null>";

        fn partition(ids: &str) -> String {
            format!(
                r#"    entry 2:
        authorizations (1): partition_id
        don't-require-password
        description: {ids}
        applications: <null>"#
            )
        }

        #[test]
        fn trusted_when_security_ok_and_apple_tool_partition() {
            let dump = item("u1", SECURITY_OK, &partition("apple-tool:, teamid:AAAA000000"));
            assert_eq!(
                parse_dump_trust(&dump, SERVICE, "u1"),
                KeychainTrust::Trusted
            );
        }

        #[test]
        fn untrusted_when_partition_lacks_apple_tool() {
            let dump = item("u1", SECURITY_OK, &partition("teamid:AAAA000000"));
            assert_eq!(
                parse_dump_trust(&dump, SERVICE, "u1"),
                KeychainTrust::Untrusted
            );
        }

        #[test]
        fn untrusted_when_security_grant_is_stale() {
            let dump = item("u1", SECURITY_STALE, &partition("apple-tool:"));
            assert_eq!(
                parse_dump_trust(&dump, SERVICE, "u1"),
                KeychainTrust::Untrusted
            );
        }

        #[test]
        fn untrusted_when_security_absent_from_decrypt_entry() {
            let dump = item("u1", NO_SECURITY, &partition("apple-tool:"));
            assert_eq!(
                parse_dump_trust(&dump, SERVICE, "u1"),
                KeychainTrust::Untrusted
            );
        }

        #[test]
        fn security_listed_only_in_non_decrypt_entry_is_untrusted() {
            // The encrypt/other entries naming the tool must not count; only
            // the decrypt entry gates reading the secret.
            let tail = format!(
                r#"    entry 2:
        authorizations (1): change_acl
        don't-require-password
        description: {SERVICE}
        applications (1):
            0: /usr/bin/security (OK)
                requirement: identifier "com.apple.security" and anchor apple
{}"#,
                partition("apple-tool:")
            );
            let dump = item("u1", NO_SECURITY, &tail);
            assert_eq!(
                parse_dump_trust(&dump, SERVICE, "u1"),
                KeychainTrust::Untrusted
            );
        }

        #[test]
        fn trusted_with_null_applications_and_no_partition_entry() {
            // `applications: <null>` = every app may read; no partition_id
            // entry = pre-Sierra item with no partition restriction.
            let dump = item("u1", APPS_NULL, "");
            assert_eq!(
                parse_dump_trust(&dump, SERVICE, "u1"),
                KeychainTrust::Trusted
            );
        }

        #[test]
        fn missing_when_service_not_in_dump() {
            let dump = item("u1", SECURITY_OK, &partition("apple-tool:"));
            assert_eq!(
                parse_dump_trust(&dump, "Other-service", "u1"),
                KeychainTrust::ItemMissing
            );
        }

        #[test]
        fn matches_account_across_multiple_items() {
            // Same service twice: u1's item is untrusted, u2's is trusted.
            let dump = format!(
                "{}\n{}",
                item("u1", NO_SECURITY, &partition("teamid:AAAA000000")),
                item("u2", SECURITY_OK, &partition("apple-tool:"))
            );
            assert_eq!(
                parse_dump_trust(&dump, SERVICE, "u2"),
                KeychainTrust::Trusted
            );
            assert_eq!(
                parse_dump_trust(&dump, SERVICE, "u1"),
                KeychainTrust::Untrusted
            );
        }

        #[test]
        fn gesture_reads_even_when_untrusted() {
            assert_eq!(
                plan_token_read(KeychainTrust::Untrusted, true),
                TokenReadPlan::ReadAllowingPrompt
            );
            assert_eq!(
                plan_token_read(KeychainTrust::Trusted, true),
                TokenReadPlan::ReadAllowingPrompt
            );
        }

        #[test]
        fn gesture_on_missing_item_skips_quietly() {
            assert_eq!(
                plan_token_read(KeychainTrust::ItemMissing, true),
                TokenReadPlan::SkipItemMissing
            );
        }

        #[test]
        fn without_gesture_only_trusted_reads() {
            assert_eq!(
                plan_token_read(KeychainTrust::Trusted, false),
                TokenReadPlan::ReadSilently
            );
            assert_eq!(
                plan_token_read(KeychainTrust::Untrusted, false),
                TokenReadPlan::SkipNeedsAuthorization
            );
            assert_eq!(
                plan_token_read(KeychainTrust::ItemMissing, false),
                TokenReadPlan::SkipItemMissing
            );
        }
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
