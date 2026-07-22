//! Read Claude Code's OAuth token from the OS secret store (macOS Keychain).

use serde::Deserialize;

#[cfg(target_os = "macos")]
use std::io::{self, Read};
#[cfg(target_os = "macos")]
use std::process::{Output, Stdio};
#[cfg(target_os = "macos")]
use std::thread;
#[cfg(target_os = "macos")]
use std::time::Duration;

#[cfg(target_os = "macos")]
use instant::Instant;

pub const CLAUDE_SERVICE: &str = "Claude Code-credentials";

#[cfg(target_os = "macos")]
const KEYCHAIN_METADATA_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(target_os = "macos")]
const KEYCHAIN_SILENT_READ_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(target_os = "macos")]
const KEYCHAIN_PROMPT_READ_TIMEOUT: Duration = Duration::from_secs(2 * 60);

/// Result of asking the platform secret store for one credential.
///
/// `Unavailable` deliberately includes a cancelled prompt and a timed-out
/// helper. Neither case is proof that the item is missing.
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
    /// Read a credential that has already been proven not to prompt.
    fn read(&self, service: &str, account: &str) -> SecretRead;

    /// Read directly after an explicit user gesture, allowing the platform to
    /// raise its credential prompt. Stores whose reads never prompt can use
    /// the normal implementation.
    fn read_allowing_prompt(&self, service: &str, account: &str) -> SecretRead {
        self.read(service, account)
    }

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
    match reader.read(CLAUDE_SERVICE, account) {
        SecretRead::Secret(blob) => parse_claude_token(&blob),
        SecretRead::ItemMissing | SecretRead::Unavailable => None,
    }
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
/// user prompt. Derived from item-specific ACL metadata, which does not read
/// the secret or raise a credential prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeychainTrust {
    /// The CLI read will complete silently.
    Trusted,
    /// The CLI read would raise the macOS keychain-password prompt.
    Untrusted,
    /// No such item — nothing to read, nothing to prompt about.
    ItemMissing,
    /// The metadata probe failed or exceeded its deadline, so a silent read
    /// cannot be proven safe. An explicit user gesture can still try directly.
    Unavailable,
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
    /// The non-interactive trust probe failed. Surface the same explicit
    /// fallback, but let the producer retry the probe on its normal cadence.
    SkipProbeUnavailable,
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
        (KeychainTrust::Unavailable, false) => TokenReadPlan::SkipProbeUnavailable,
    }
}

/// Outcome of one bounded attempt to acquire Claude Code's token.
pub enum TokenAcquisition {
    Token(ClaudeToken),
    ItemMissing,
    NeedsAuthorization,
    RetryLater,
}

/// Acquire Claude Code's token without ever raising an unsolicited prompt.
///
/// A user gesture intentionally bypasses the ACL dump: the gesture already
/// authorizes a prompt, and waiting for a full-Keychain metadata scan first is
/// both redundant and potentially very slow. Background attempts still probe
/// trust before reading.
pub fn acquire_claude_token(
    reader: &dyn ReadSecret,
    account: &str,
    user_gesture: bool,
) -> TokenAcquisition {
    let plan = if user_gesture {
        TokenReadPlan::ReadAllowingPrompt
    } else {
        plan_token_read(reader.probe_trust(CLAUDE_SERVICE, account), false)
    };

    let read = match plan {
        TokenReadPlan::ReadAllowingPrompt => reader.read_allowing_prompt(CLAUDE_SERVICE, account),
        TokenReadPlan::ReadSilently => reader.read(CLAUDE_SERVICE, account),
        TokenReadPlan::SkipItemMissing => return TokenAcquisition::ItemMissing,
        TokenReadPlan::SkipNeedsAuthorization | TokenReadPlan::SkipProbeUnavailable => {
            return TokenAcquisition::NeedsAuthorization;
        }
    };

    match read {
        SecretRead::Secret(blob) => parse_claude_token(&blob)
            .map(TokenAcquisition::Token)
            .unwrap_or(TokenAcquisition::RetryLater),
        SecretRead::ItemMissing => TokenAcquisition::ItemMissing,
        SecretRead::Unavailable => TokenAcquisition::NeedsAuthorization,
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
fn keychain_path_from_find_stdout(stdout: &[u8]) -> Option<String> {
    let output = std::str::from_utf8(stdout).ok()?;
    output.lines().find_map(|line| {
        line.strip_prefix("keychain: \"")
            .and_then(|path| path.strip_suffix('"'))
            .filter(|path| !path.is_empty())
            .map(ToOwned::to_owned)
    })
}

#[cfg(target_os = "macos")]
fn spawn_pipe_reader<R>(mut reader: R) -> thread::JoinHandle<io::Result<Vec<u8>>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes)?;
        Ok(bytes)
    })
}

#[cfg(target_os = "macos")]
fn join_pipe_reader(
    reader: Option<thread::JoinHandle<io::Result<Vec<u8>>>>,
) -> io::Result<Vec<u8>> {
    match reader {
        Some(reader) => reader
            .join()
            .map_err(|_| io::Error::other("Keychain output reader panicked"))?,
        None => Ok(Vec::new()),
    }
}

/// Run one Keychain helper with a hard deadline while concurrently draining
/// both pipes. Draining matters for `dump-keychain`: a large ACL dump can fill
/// a pipe and otherwise prevent the child from ever exiting.
#[cfg(target_os = "macos")]
fn output_with_timeout(
    command: &mut command::blocking::Command,
    timeout: Duration,
) -> io::Result<Output> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdout = child.stdout.take().map(spawn_pipe_reader);
    let stderr = child.stderr.take().map(spawn_pipe_reader);
    let deadline = Instant::now() + timeout;

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                thread::sleep(remaining.min(Duration::from_millis(10)));
            }
            Ok(None) => {
                // Only `wait` after a successful kill. If signalling itself
                // fails, a blocking wait would defeat the deadline we are
                // enforcing. The pipe-reader handles are intentionally
                // detached on this error path.
                if child.kill().is_ok() {
                    let _ = child.wait();
                }
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("Keychain helper exceeded {timeout:?}"),
                ));
            }
            Err(error) => {
                if child.kill().is_ok() {
                    let _ = child.wait();
                }
                return Err(error);
            }
        }
    };

    Ok(Output {
        status,
        stdout: join_pipe_reader(stdout)?,
        stderr: join_pipe_reader(stderr)?,
    })
}

#[cfg(target_os = "macos")]
fn security_item_missing(output: &Output) -> bool {
    // `security` exits with the low 8 bits of errSecItemNotFound (-25300).
    output.status.code() == Some(44)
}

#[cfg(target_os = "macos")]
fn read_security_secret(service: &str, account: &str, timeout: Duration) -> SecretRead {
    let output = output_with_timeout(
        command::blocking::Command::new("/usr/bin/security").args([
            "find-generic-password",
            "-s",
            service,
            "-a",
            account,
            "-w",
        ]),
        timeout,
    );
    let Ok(output) = output else {
        return SecretRead::Unavailable;
    };
    if security_item_missing(&output) {
        return SecretRead::ItemMissing;
    }
    if !output.status.success() {
        return SecretRead::Unavailable;
    }
    secret_from_security_stdout(output.stdout)
        .map(SecretRead::Secret)
        .unwrap_or(SecretRead::Unavailable)
}

/// Item-specific ACL inspection through Security.framework. Unlike
/// `security dump-keychain -a`, these calls do not enumerate and format every
/// item in the user's Keychain. They inspect no secret data and do not prompt.
#[cfg(target_os = "macos")]
mod native_acl {
    use std::ffi::{c_char, c_void};
    use std::ptr;
    use std::slice;

    use core_foundation_sys::array::{CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef};
    use core_foundation_sys::base::{CFEqual, CFRelease, CFTypeRef, OSStatus};
    use core_foundation_sys::data::{CFDataGetBytePtr, CFDataGetLength, CFDataRef};
    use core_foundation_sys::string::{
        kCFStringEncodingUTF8, CFStringGetCString, CFStringGetLength,
        CFStringGetMaximumSizeForEncoding, CFStringRef,
    };
    use security_framework_sys::base::{
        errSecItemNotFound, errSecSuccess, SecAccessRef, SecKeychainItemRef,
    };
    use security_framework_sys::keychain::SecKeychainFindGenericPassword;

    use super::KeychainTrust;

    type SecAclRef = *mut c_void;
    type SecTrustedApplicationRef = *mut c_void;
    type SecKeychainPromptSelector = u16;

    extern "C" {
        fn SecKeychainItemCopyAccess(
            item: SecKeychainItemRef,
            access: *mut SecAccessRef,
        ) -> OSStatus;
        fn SecAccessCopyACLList(access: SecAccessRef, acl_list: *mut CFArrayRef) -> OSStatus;
        fn SecACLCopyAuthorizations(acl: SecAclRef) -> CFArrayRef;
        fn SecACLCopyContents(
            acl: SecAclRef,
            application_list: *mut CFArrayRef,
            description: *mut CFStringRef,
            prompt_selector: *mut SecKeychainPromptSelector,
        ) -> OSStatus;
        fn SecTrustedApplicationCopyData(
            application: SecTrustedApplicationRef,
            data: *mut CFDataRef,
        ) -> OSStatus;
        fn SecTrustedApplicationCreateFromPath(
            path: *const c_char,
            application: *mut SecTrustedApplicationRef,
        ) -> OSStatus;

        #[link_name = "kSecACLAuthorizationDecrypt"]
        static SEC_ACL_AUTHORIZATION_DECRYPT: CFStringRef;
        #[link_name = "kSecACLAuthorizationPartitionID"]
        static SEC_ACL_AUTHORIZATION_PARTITION_ID: CFStringRef;
    }

    struct OwnedCf(CFTypeRef);

    impl OwnedCf {
        fn new(value: CFTypeRef) -> Option<Self> {
            if value.is_null() {
                None
            } else {
                Some(Self(value))
            }
        }
    }

    impl Drop for OwnedCf {
        fn drop(&mut self) {
            // SAFETY: `OwnedCf` is created only for values returned under a
            // Core Foundation create/copy ownership rule, exactly once.
            unsafe { CFRelease(self.0) };
        }
    }

    fn array_count(array: CFArrayRef) -> Result<isize, ()> {
        if array.is_null() {
            return Err(());
        }
        // SAFETY: the caller supplied a live CFArray reference.
        let count = unsafe { CFArrayGetCount(array) };
        (count >= 0).then_some(count).ok_or(())
    }

    fn array_contains_authorization(array: CFArrayRef, target: CFStringRef) -> bool {
        if target.is_null() {
            return false;
        }
        let Ok(count) = array_count(array) else {
            return false;
        };
        (0..count).any(|index| {
            // SAFETY: `index` is within the live CFArray and both values are
            // Core Foundation objects supplied by Security.framework.
            unsafe {
                let value: CFTypeRef = CFArrayGetValueAtIndex(array, index).cast();
                !value.is_null() && CFEqual(value, target.cast::<c_void>()) != 0
            }
        })
    }

    fn cf_string_to_string(value: CFStringRef) -> Option<String> {
        if value.is_null() {
            return None;
        }
        // SAFETY: `value` remains retained by the caller for this conversion.
        let length = unsafe { CFStringGetLength(value) };
        if length < 0 {
            return None;
        }
        // SAFETY: same live CFString and a documented Core Foundation encoding.
        let maximum = unsafe { CFStringGetMaximumSizeForEncoding(length, kCFStringEncodingUTF8) };
        let capacity = usize::try_from(maximum).ok()?.checked_add(1)?;
        let capacity_cf = isize::try_from(capacity).ok()?;
        let mut buffer = vec![0u8; capacity];
        // SAFETY: `buffer` is writable for `capacity` bytes, including the NUL.
        let copied = unsafe {
            CFStringGetCString(
                value,
                buffer.as_mut_ptr().cast::<c_char>(),
                capacity_cf,
                kCFStringEncodingUTF8,
            )
        };
        if copied == 0 {
            return None;
        }
        let length = buffer.iter().position(|byte| *byte == 0)?;
        String::from_utf8(buffer[..length].to_vec()).ok()
    }

    fn decode_hex(value: &str) -> Option<Vec<u8>> {
        if !value.len().is_multiple_of(2) {
            return None;
        }
        value
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                let high = char::from(pair[0]).to_digit(16)?;
                let low = char::from(pair[1]).to_digit(16)?;
                Some(((high << 4) | low) as u8)
            })
            .collect()
    }

    pub(super) fn partition_allows_apple_tools(description: &str) -> bool {
        let decoded = decode_hex(description).and_then(|bytes| String::from_utf8(bytes).ok());
        let description = decoded.as_deref().unwrap_or(description);
        description.contains("<string>apple-tool:</string>")
            || description
                .split(',')
                .any(|partition| partition.trim() == "apple-tool:")
    }

    fn trusted_application_data(application: SecTrustedApplicationRef) -> Result<Vec<u8>, ()> {
        let mut data: CFDataRef = ptr::null();
        // SAFETY: `application` is borrowed from the retained ACL application
        // array; Security.framework initializes `data` under the copy rule.
        let status = unsafe { SecTrustedApplicationCopyData(application, &mut data) };
        if status != errSecSuccess {
            return Err(());
        }
        let _data = OwnedCf::new(data.cast()).ok_or(())?;
        // SAFETY: `data` remains retained by `_data` while length and bytes are read.
        let length = unsafe { CFDataGetLength(data) };
        let length = usize::try_from(length).map_err(|_| ())?;
        // SAFETY: a non-null byte pointer spans exactly `length` bytes for a
        // live CFData object. `from_raw_parts` permits a dangling pointer only
        // for zero length, so handle that case without dereferencing it.
        if length == 0 {
            return Ok(Vec::new());
        }
        let bytes = unsafe { CFDataGetBytePtr(data) };
        if bytes.is_null() {
            return Err(());
        }
        // SAFETY: validated above against the retained CFData's reported length.
        Ok(unsafe { slice::from_raw_parts(bytes, length) }.to_vec())
    }

    fn application_list_trusts_security(applications: CFArrayRef) -> Result<bool, ()> {
        // A null list means every application is trusted by this ACL entry.
        if applications.is_null() {
            return Ok(true);
        }
        let mut security_application: SecTrustedApplicationRef = ptr::null_mut();
        // SAFETY: the path is a static NUL-terminated C string and the output
        // follows the create rule on success.
        let status = unsafe {
            SecTrustedApplicationCreateFromPath(
                c"/usr/bin/security".as_ptr(),
                &mut security_application,
            )
        };
        if status != errSecSuccess {
            return Err(());
        }
        let _security_application = OwnedCf::new(security_application.cast()).ok_or(())?;
        let expected_data = trusted_application_data(security_application)?;

        let count = array_count(applications)?;
        for index in 0..count {
            // SAFETY: `index` is within the retained application array.
            let application =
                unsafe { CFArrayGetValueAtIndex(applications, index) as SecTrustedApplicationRef };
            if application.is_null() {
                return Err(());
            }
            let data = trusted_application_data(application)?;
            if data == expected_data {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn probe(service: &str, account: &str) -> KeychainTrust {
        let Ok(service_length) = u32::try_from(service.len()) else {
            return KeychainTrust::Unavailable;
        };
        let Ok(account_length) = u32::try_from(account.len()) else {
            return KeychainTrust::Unavailable;
        };

        let mut item: SecKeychainItemRef = ptr::null_mut();
        // SAFETY: lengths match the two UTF-8 byte buffers; password output is
        // intentionally null so this metadata lookup cannot decrypt or prompt.
        let status = unsafe {
            SecKeychainFindGenericPassword(
                ptr::null(),
                service_length,
                service.as_ptr().cast::<c_char>(),
                account_length,
                account.as_ptr().cast::<c_char>(),
                ptr::null_mut(),
                ptr::null_mut(),
                &mut item,
            )
        };
        if status == errSecItemNotFound {
            return KeychainTrust::ItemMissing;
        }
        if status != errSecSuccess {
            return KeychainTrust::Unavailable;
        }
        let Some(_item) = OwnedCf::new(item.cast()) else {
            return KeychainTrust::Unavailable;
        };

        let mut access: SecAccessRef = ptr::null_mut();
        // SAFETY: `item` remains retained by `_item`; `access` is initialized
        // under the copy rule on success.
        if unsafe { SecKeychainItemCopyAccess(item, &mut access) } != errSecSuccess {
            return KeychainTrust::Unavailable;
        }
        let Some(_access) = OwnedCf::new(access.cast()) else {
            return KeychainTrust::Unavailable;
        };

        let mut acl_list: CFArrayRef = ptr::null();
        // SAFETY: `access` remains retained and `acl_list` follows the copy rule.
        if unsafe { SecAccessCopyACLList(access, &mut acl_list) } != errSecSuccess {
            return KeychainTrust::Unavailable;
        }
        let Some(_acl_list) = OwnedCf::new(acl_list.cast()) else {
            return KeychainTrust::Unavailable;
        };
        let Ok(acl_count) = array_count(acl_list) else {
            return KeychainTrust::Unavailable;
        };

        let mut security_may_decrypt = false;
        let mut saw_partition = false;
        let mut partition_permits_apple_tools = false;

        for index in 0..acl_count {
            // SAFETY: `index` is within the retained ACL array.
            let acl = unsafe { CFArrayGetValueAtIndex(acl_list, index) as SecAclRef };
            if acl.is_null() {
                return KeychainTrust::Unavailable;
            }
            // SAFETY: `acl` is borrowed from the retained ACL list; the copy
            // result is owned and released at the end of this iteration.
            let authorizations = unsafe { SecACLCopyAuthorizations(acl) };
            let Some(_authorizations) = OwnedCf::new(authorizations.cast()) else {
                return KeychainTrust::Unavailable;
            };
            let grants_decrypt = array_contains_authorization(
                authorizations,
                // SAFETY: this exported Security.framework constant is a live CFString.
                unsafe { SEC_ACL_AUTHORIZATION_DECRYPT },
            );
            let grants_partition = array_contains_authorization(
                authorizations,
                // SAFETY: this exported Security.framework constant is a live CFString.
                unsafe { SEC_ACL_AUTHORIZATION_PARTITION_ID },
            );
            if !grants_decrypt && !grants_partition {
                continue;
            }

            let mut applications: CFArrayRef = ptr::null();
            let mut description: CFStringRef = ptr::null();
            let mut prompt_selector: SecKeychainPromptSelector = 0;
            // SAFETY: all output pointers are valid and `acl` remains retained
            // through `_acl_list`. Both returned objects use the copy rule.
            if unsafe {
                SecACLCopyContents(
                    acl,
                    &mut applications,
                    &mut description,
                    &mut prompt_selector,
                )
            } != errSecSuccess
            {
                return KeychainTrust::Unavailable;
            }
            let _applications = OwnedCf::new(applications.cast());
            let _description = OwnedCf::new(description.cast());

            if grants_decrypt {
                match application_list_trusts_security(applications) {
                    Ok(trusted) => security_may_decrypt |= trusted,
                    Err(()) => return KeychainTrust::Unavailable,
                }
            }
            if grants_partition {
                saw_partition = true;
                let Some(description) = cf_string_to_string(description) else {
                    return KeychainTrust::Unavailable;
                };
                partition_permits_apple_tools |= partition_allows_apple_tools(&description);
            }
        }

        if security_may_decrypt && (!saw_partition || partition_permits_apple_tools) {
            KeychainTrust::Trusted
        } else {
            KeychainTrust::Untrusted
        }
    }
}

#[cfg(target_os = "macos")]
fn bounded_native_acl_probe(service: &str, account: &str) -> Option<KeychainTrust> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let service = service.to_owned();
    let account = account.to_owned();
    let worker = thread::Builder::new()
        .name("clinch-keychain-acl".to_string())
        .spawn(move || {
            let _ = sender.send(native_acl::probe(&service, &account));
        });
    if worker.is_err() {
        return Some(KeychainTrust::Unavailable);
    }
    receiver.recv_timeout(KEYCHAIN_METADATA_TIMEOUT).ok()
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
    fn read(&self, service: &str, account: &str) -> SecretRead {
        read_security_secret(service, account, KEYCHAIN_SILENT_READ_TIMEOUT)
    }

    fn read_allowing_prompt(&self, service: &str, account: &str) -> SecretRead {
        read_security_secret(service, account, KEYCHAIN_PROMPT_READ_TIMEOUT)
    }

    /// The item-specific Security.framework probe is metadata-only and never
    /// raises a prompt. If those legacy ACL APIs are unavailable, bounded CLI
    /// metadata calls provide a compatibility fallback. The
    /// `apple-tool:`/`security` trust that Claude Code's CLI-written item starts
    /// with is NOT durable — an old native-API reader can rewrite the partition
    /// list — so this is checked before every unsanctioned read rather than
    /// assumed.
    fn probe_trust(&self, service: &str, account: &str) -> KeychainTrust {
        match bounded_native_acl_probe(service, account) {
            Some(KeychainTrust::Unavailable) => {}
            Some(trust) => return trust,
            // A stuck Security.framework call stays isolated on its worker
            // thread and cannot stall the usage producer past the deadline.
            None => return KeychainTrust::Unavailable,
        }

        let exists = output_with_timeout(
            command::blocking::Command::new("/usr/bin/security").args([
                "find-generic-password",
                "-s",
                service,
                "-a",
                account,
            ]),
            KEYCHAIN_METADATA_TIMEOUT,
        );
        let Ok(exists) = exists else {
            return KeychainTrust::Unavailable;
        };
        if security_item_missing(&exists) {
            return KeychainTrust::ItemMissing;
        }
        if !exists.status.success() {
            return KeychainTrust::Unavailable;
        }
        let Some(keychain_path) = keychain_path_from_find_stdout(&exists.stdout) else {
            return KeychainTrust::Unavailable;
        };

        // Limit the ACL scan to the exact Keychain containing this item. A
        // search-list-wide dump can take minutes on large or unhealthy stores.
        let dump = output_with_timeout(
            command::blocking::Command::new("/usr/bin/security").args([
                "dump-keychain",
                "-a",
                &keychain_path,
            ]),
            KEYCHAIN_METADATA_TIMEOUT,
        );
        let Ok(dump) = dump else {
            return KeychainTrust::Unavailable;
        };
        if !dump.status.success() {
            return KeychainTrust::Unavailable;
        }
        // An item that exists but is absent from the dumped default keychain
        // parses as ItemMissing; report Untrusted so the read stays behind an
        // explicit user gesture instead of prompting unbidden.
        match parse_dump_trust(&String::from_utf8_lossy(&dump.stdout), service, account) {
            KeychainTrust::ItemMissing => KeychainTrust::Unavailable,
            trust => trust,
        }
    }
}

#[cfg(not(target_os = "macos"))]
impl ReadSecret for MacKeychain {
    fn read(&self, _service: &str, _account: &str) -> SecretRead {
        SecretRead::ItemMissing
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

    #[cfg(target_os = "macos")]
    mod security_stdout {
        use std::io;
        use std::time::Duration;

        use instant::Instant;

        use super::super::{
            keychain_path_from_find_stdout, native_acl::partition_allows_apple_tools,
            output_with_timeout, secret_from_security_stdout,
        };

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

        #[test]
        fn extracts_owning_keychain_path() {
            let output = br#"keychain: "/Users/u/Library/Keychains/login.keychain-db"
class: "genp"
attributes:
    "acct"<blob>="u"
"#;
            assert_eq!(
                keychain_path_from_find_stdout(output).as_deref(),
                Some("/Users/u/Library/Keychains/login.keychain-db")
            );
            assert!(keychain_path_from_find_stdout(b"class: \"genp\"\n").is_none());
        }

        #[test]
        fn recognizes_plain_and_hex_encoded_apple_tool_partitions() {
            assert!(partition_allows_apple_tools(
                "apple-tool:, teamid:AAAA000000"
            ));
            assert!(partition_allows_apple_tools(
                "3c706c6973743e3c61727261793e3c737472696e673e6170706c652d746f6f6c3a3c2f737472696e673e3c2f61727261793e3c2f706c6973743e"
            ));
            assert!(!partition_allows_apple_tools("teamid:AAAA000000"));
            assert!(!partition_allows_apple_tools("not-valid-hex"));
        }

        #[test]
        fn bounded_output_captures_both_pipes() {
            let output = output_with_timeout(
                command::blocking::Command::new("/bin/sh")
                    .args(["-c", "printf stdout; printf stderr >&2"]),
                Duration::from_secs(1),
            )
            .unwrap();
            assert!(output.status.success());
            assert_eq!(output.stdout, b"stdout");
            assert_eq!(output.stderr, b"stderr");
        }

        #[test]
        fn bounded_output_concurrently_drains_large_pipes() {
            let output = output_with_timeout(
                command::blocking::Command::new("/bin/sh").args([
                    "-c",
                    "i=0; while [ $i -lt 8192 ]; do printf 0123456789abcdef; printf fedcba9876543210 >&2; i=$((i + 1)); done",
                ]),
                Duration::from_secs(5),
            )
            .unwrap();
            assert!(output.status.success());
            assert_eq!(output.stdout.len(), 131_072);
            assert_eq!(output.stderr.len(), 131_072);
        }

        #[test]
        fn bounded_output_kills_hung_helper() {
            let started = Instant::now();
            let error = output_with_timeout(
                command::blocking::Command::new("/bin/sleep").arg("30"),
                Duration::from_millis(25),
            )
            .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::TimedOut);
            assert!(started.elapsed() < Duration::from_secs(2));
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
            let dump = item(
                "u1",
                SECURITY_OK,
                &partition("apple-tool:, teamid:AAAA000000"),
            );
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
            assert_eq!(
                plan_token_read(KeychainTrust::Unavailable, true),
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
            assert_eq!(
                plan_token_read(KeychainTrust::Unavailable, false),
                TokenReadPlan::SkipProbeUnavailable
            );
        }
    }

    mod acquisition {
        use super::super::{
            acquire_claude_token, KeychainTrust, ReadSecret, SecretRead, TokenAcquisition,
        };
        use super::BLOB;

        struct GestureStore;
        impl ReadSecret for GestureStore {
            fn read(&self, _service: &str, _account: &str) -> SecretRead {
                panic!("a gesture must use the prompting read")
            }

            fn read_allowing_prompt(&self, _service: &str, _account: &str) -> SecretRead {
                SecretRead::Secret(BLOB.to_string())
            }

            fn probe_trust(&self, _service: &str, _account: &str) -> KeychainTrust {
                panic!("a gesture must bypass the slow metadata probe")
            }
        }

        struct UnavailableProbe;
        impl ReadSecret for UnavailableProbe {
            fn read(&self, _service: &str, _account: &str) -> SecretRead {
                panic!("an unproven background read must not happen")
            }

            fn probe_trust(&self, _service: &str, _account: &str) -> KeychainTrust {
                KeychainTrust::Unavailable
            }
        }

        struct MissingOnGesture;
        impl ReadSecret for MissingOnGesture {
            fn read(&self, _service: &str, _account: &str) -> SecretRead {
                SecretRead::ItemMissing
            }
        }

        struct TrustedStore;
        impl ReadSecret for TrustedStore {
            fn read(&self, _service: &str, _account: &str) -> SecretRead {
                SecretRead::Secret(BLOB.to_string())
            }

            fn read_allowing_prompt(&self, _service: &str, _account: &str) -> SecretRead {
                panic!("a background read must stay silent")
            }

            fn probe_trust(&self, _service: &str, _account: &str) -> KeychainTrust {
                KeychainTrust::Trusted
            }
        }

        #[test]
        fn gesture_bypasses_probe_and_reads_directly() {
            assert!(matches!(
                acquire_claude_token(&GestureStore, "u", true),
                TokenAcquisition::Token(_)
            ));
        }

        #[test]
        fn unavailable_background_probe_requests_explicit_authorization() {
            assert!(matches!(
                acquire_claude_token(&UnavailableProbe, "u", false),
                TokenAcquisition::NeedsAuthorization
            ));
        }

        #[test]
        fn missing_item_after_gesture_does_not_loop_authorization() {
            assert!(matches!(
                acquire_claude_token(&MissingOnGesture, "u", true),
                TokenAcquisition::ItemMissing
            ));
        }

        #[test]
        fn trusted_background_probe_uses_silent_read() {
            assert!(matches!(
                acquire_claude_token(&TrustedStore, "u", false),
                TokenAcquisition::Token(_)
            ));
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

    #[test]
    fn secret_read_debug_redacts_secret() {
        let debug_str = format!("{:?}", SecretRead::Secret("SECRET".to_string()));
        assert!(!debug_str.contains("SECRET"));
        assert!(debug_str.contains("<redacted>"));
    }
}
