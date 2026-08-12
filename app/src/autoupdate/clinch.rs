use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, ensure, Context as _, Result};
use async_fs::File;
use base64::prelude::{Engine as _, BASE64_STANDARD};
use chrono::{DateTime, NaiveDate, Utc};
use command::blocking::Command as BlockingCommand;
use command::r#async::Command;
use futures::StreamExt as _;
use futures_lite::io::AsyncWriteExt as _;
use ring::signature::{UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use url::Url;

use crate::channel::ChannelState;

const MANIFEST_ASSET: &str = "Clinch.update.json";
const SIGNATURE_ASSET: &str = "Clinch.update.sig";
const ARCHIVE_ASSET: &str = "Clinch.app.zip";
const X86_64_ARCHIVE_ASSET: &str = "Clinch-x86_64.app.zip";
const EXPECTED_REPOSITORY: &str = "elliot-ylambda/clinch-terminal";
const EXPECTED_BUNDLE_ID: &str = "sh.clinch.Clinch";
const MAX_MANIFEST_SIZE: usize = 256 * 1024;
const MAX_SIGNATURE_SIZE: usize = 4096;
// Release archives are still large enough that slower connections need more than the metadata
// timeout. Keep a steadily progressing transfer from becoming a failed update.
const ARCHIVE_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const EMBEDDED_PUBLIC_KEY: &str =
    include_str!("../../../resources/update/clinch-update-public-key.json");

/// How long a successful automatic check suppresses the next one.
const AUTOMATIC_CHECK_INTERVAL_SECS: i64 = 7 * 24 * 60 * 60;
/// How long a failed automatic check suppresses the next one. Short enough to recover the same
/// day from a brief outage, long enough that a persistently failing check costs a handful of
/// requests per day rather than one per window focus.
const FAILED_CHECK_BACKOFF_SECS: i64 = 6 * 60 * 60;

fn last_check_path() -> PathBuf {
    warp_core::paths::state_dir().join("last-update-check")
}

/// Cadence state for Clinch's automatic update check.
///
/// Both fields are instants rather than calendar dates, so the cadence is timezone-independent
/// and does not depend on when the local day happens to roll over. `retry_after` is written only
/// when a check fails; without it a failing check records nothing and re-fires on every window
/// focus.
#[derive(Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub(super) struct UpdateCheckRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_success: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    retry_after: Option<DateTime<Utc>>,
}

impl UpdateCheckRecord {
    /// Reads a stored record, tolerating both unreadable content and the legacy `YYYY-MM-DD`
    /// format written when the cadence was daily. Anything unrecognized means "no check
    /// recorded", which errs toward checking rather than silently never checking again.
    pub(super) fn parse(raw: &str) -> Self {
        let raw = raw.trim();
        if let Ok(record) = serde_json::from_str::<Self>(raw) {
            return record;
        }
        if let Ok(date) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
            return Self {
                last_success: date.and_hms_opt(0, 0, 0).map(|naive| naive.and_utc()),
                retry_after: None,
            };
        }
        Self::default()
    }

    pub(super) fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_owned())
    }

    pub(super) fn automatic_check_due(&self, now: DateTime<Utc>) -> bool {
        if self
            .retry_after
            .is_some_and(|retry_after| now < retry_after)
        {
            return false;
        }
        self.last_success.is_none_or(|last_success| {
            now - last_success >= chrono::Duration::seconds(AUTOMATIC_CHECK_INTERVAL_SECS)
        })
    }

    pub(super) fn record_success(&mut self, now: DateTime<Utc>) {
        self.last_success = Some(now);
        self.retry_after = None;
    }

    pub(super) fn record_failure(&mut self, now: DateTime<Utc>) {
        self.retry_after = Some(now + chrono::Duration::seconds(FAILED_CHECK_BACKOFF_SECS));
    }
}

fn read_record() -> UpdateCheckRecord {
    std::fs::read_to_string(last_check_path())
        .map(|raw| UpdateCheckRecord::parse(&raw))
        .unwrap_or_default()
}

fn write_record(record: &UpdateCheckRecord) {
    let path = last_check_path();
    let Some(parent) = path.parent() else { return };
    if let Err(error) = std::fs::create_dir_all(parent).and_then(|_| {
        let temporary = path.with_extension(format!("tmp.{}", std::process::id()));
        std::fs::write(&temporary, format!("{}\n", record.to_json()))?;
        std::fs::rename(temporary, path)
    }) {
        log::warn!("could not record the Clinch update check: {error}");
    }
}

pub(super) fn automatic_check_due(now: DateTime<Utc>) -> bool {
    read_record().automatic_check_due(now)
}

pub(super) fn record_successful_check(now: DateTime<Utc>) {
    let mut record = read_record();
    record.record_success(now);
    write_record(&record);
}

pub(super) fn record_failed_check(now: DateTime<Utc>) {
    let mut record = read_record();
    record.record_failure(now);
    write_record(&record);
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TrustedKey {
    pub key_id: String,
    pub ed25519_public_key: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Archive {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub version: String,
    pub sequence: u64,
    pub minimum_macos_version: String,
    pub bundle_id: String,
    pub signing_key_id: String,
    pub archive: Archive,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub archives: BTreeMap<String, Archive>,
    pub release_notes: String,
    pub release_url: String,
    #[serde(default)]
    pub rollback: bool,
    #[serde(default)]
    pub next_public_key: Option<TrustedKey>,
}

#[derive(Clone, Debug)]
pub struct VerifiedRelease {
    pub manifest: Manifest,
    selected_archive: Archive,
}

impl VerifiedRelease {
    pub fn version_info(&self) -> channel_versions::VersionInfo {
        let mut version = channel_versions::VersionInfo::new(self.manifest.version.clone());
        version.is_rollback = Some(self.manifest.rollback);
        version
    }

    pub fn archive_sha256(&self) -> &str {
        &self.selected_archive.sha256
    }

    pub fn archive_size(&self) -> u64 {
        self.selected_archive.size
    }
}

impl Manifest {
    fn archive_for_architecture(&self, architecture: &str) -> Result<&Archive> {
        if self.archives.is_empty() {
            return Ok(&self.archive);
        }
        self.archives
            .get(architecture)
            .with_context(|| format!("release omitted the {architecture} archive"))
    }
}

fn architecture_from_hardware_probe(
    supports_arm64: bool,
    process_architecture: &str,
) -> Result<&'static str> {
    if supports_arm64 {
        return Ok("arm64");
    }
    match process_architecture {
        "aarch64" => Ok("arm64"),
        "x86_64" => Ok("x86_64"),
        architecture => Err(anyhow!("unsupported Mac architecture {architecture}")),
    }
}

fn machine_architecture() -> Result<&'static str> {
    let arm64_probe = BlockingCommand::new("/usr/sbin/sysctl")
        .args(["-in", "hw.optional.arm64"])
        .output();
    let supports_arm64 = arm64_probe.is_ok_and(|output| {
        output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "1"
    });
    architecture_from_hardware_probe(supports_arm64, std::env::consts::ARCH)
}

#[derive(Clone, Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    assets: Vec<GithubAsset>,
}

#[derive(Clone, Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

fn asset<'a>(release: &'a GithubRelease, name: &str) -> Result<&'a GithubAsset> {
    release
        .assets
        .iter()
        .find(|asset| asset.name == name)
        .with_context(|| format!("GitHub release omitted {name}"))
}

fn validate_https_url(value: &str, expected_host: &str) -> Result<Url> {
    let url = Url::parse(value).context("update manifest contains an invalid URL")?;
    ensure!(url.scheme() == "https", "update URL must use HTTPS");
    ensure!(
        url.host_str() == Some(expected_host),
        "update URL has an unexpected host"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "update URL must not contain credentials"
    );
    Ok(url)
}

fn trusted_keys_path() -> PathBuf {
    warp_core::paths::state_dir().join("trusted-update-keys.json")
}

fn trusted_keys() -> Result<Vec<TrustedKey>> {
    let embedded: TrustedKey =
        serde_json::from_str(EMBEDDED_PUBLIC_KEY).context("invalid embedded Clinch update key")?;
    let mut keys = vec![embedded];
    match std::fs::read_to_string(trusted_keys_path()) {
        Ok(contents) => {
            let persisted: Vec<TrustedKey> = serde_json::from_str(&contents)
                .context("invalid persisted Clinch update key ring")?;
            keys.extend(persisted);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("could not read Clinch update key ring"),
    }
    keys.sort_by(|left, right| left.key_id.cmp(&right.key_id));
    keys.dedup_by(|left, right| left.key_id == right.key_id);
    Ok(keys)
}

fn decode_key(key: &TrustedKey) -> Result<Vec<u8>> {
    ensure!(
        !key.key_id.is_empty()
            && key
                .key_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'),
        "invalid update signing key identifier"
    );
    let bytes = BASE64_STANDARD
        .decode(key.ed25519_public_key.trim())
        .context("invalid base64 update public key")?;
    ensure!(bytes.len() == 32, "Ed25519 public keys must be 32 bytes");
    Ok(bytes)
}

fn persist_next_key(current_keys: &[TrustedKey], next_key: Option<&TrustedKey>) -> Result<()> {
    let Some(next_key) = next_key else {
        return Ok(());
    };
    decode_key(next_key)?;
    if current_keys.iter().any(|key| key.key_id == next_key.key_id) {
        return Ok(());
    }
    let embedded: TrustedKey = serde_json::from_str(EMBEDDED_PUBLIC_KEY)?;
    let mut persisted = current_keys
        .iter()
        .filter(|key| key.key_id != embedded.key_id)
        .cloned()
        .collect::<Vec<_>>();
    persisted.push(next_key.clone());
    let path = trusted_keys_path();
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("trusted key path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let temporary = path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&temporary, serde_json::to_vec_pretty(&persisted)?)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}

fn verify_manifest_signature(
    manifest_bytes: &[u8],
    signature_bytes: &[u8],
    keys: &[TrustedKey],
) -> Result<Manifest> {
    let manifest: Manifest =
        serde_json::from_slice(manifest_bytes).context("invalid Clinch update manifest JSON")?;
    let key = keys
        .iter()
        .find(|key| key.key_id == manifest.signing_key_id)
        .context("manifest was signed by an unknown Clinch release key")?;
    let public_key = decode_key(key)?;
    let signature = BASE64_STANDARD
        .decode(signature_bytes.strip_ascii_whitespace())
        .context("invalid base64 update signature")?;
    UnparsedPublicKey::new(&ED25519, public_key)
        .verify(manifest_bytes, &signature)
        .map_err(|_| anyhow!("Clinch update manifest signature is invalid"))?;
    Ok(manifest)
}

trait StripAsciiWhitespace {
    fn strip_ascii_whitespace(&self) -> Vec<u8>;
}

impl StripAsciiWhitespace for [u8] {
    fn strip_ascii_whitespace(&self) -> Vec<u8> {
        self.iter()
            .copied()
            .filter(|byte| !byte.is_ascii_whitespace())
            .collect()
    }
}

fn validate_archive(
    archive: &Archive,
    expected_name: &str,
    manifest: &Manifest,
    release: &GithubRelease,
) -> Result<()> {
    ensure!(archive.name == expected_name, "unexpected update archive");
    ensure!(
        archive.sha256.len() == 64
            && archive
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
        "invalid update archive SHA-256"
    );
    ensure!(archive.size > 0, "update archive is empty");
    let archive_url = validate_https_url(&archive.url, "github.com")?;
    let github_archive = asset(release, expected_name)?;
    ensure!(
        archive_url.as_str() == github_archive.browser_download_url,
        "manifest archive URL does not match its GitHub release"
    );
    ensure!(
        archive.size == github_archive.size,
        "manifest archive size does not match GitHub"
    );
    let expected_path = format!(
        "/{EXPECTED_REPOSITORY}/releases/download/{}/{}",
        manifest.version, expected_name
    );
    ensure!(
        archive_url.path() == expected_path,
        "manifest archive URL points outside the pinned repository"
    );
    Ok(())
}

fn validate_manifest(manifest: &Manifest, release: &GithubRelease) -> Result<()> {
    ensure!(
        manifest.schema_version == 1,
        "unsupported update manifest schema"
    );
    ensure!(
        manifest.bundle_id == EXPECTED_BUNDLE_ID,
        "unexpected update bundle identifier"
    );
    ensure!(
        manifest.version == release.tag_name,
        "manifest version does not match its GitHub release"
    );
    ensure!(
        manifest.version.starts_with('v')
            && manifest
                .version
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte)),
        "invalid Clinch release version"
    );
    validate_archive(&manifest.archive, ARCHIVE_ASSET, manifest, release)?;
    if !manifest.archives.is_empty() {
        ensure!(
            manifest.archives.len() == 2
                && manifest.archives.contains_key("arm64")
                && manifest.archives.contains_key("x86_64"),
            "update manifest must contain exactly the supported Mac archives"
        );
        ensure!(
            manifest.archives.get("arm64") == Some(&manifest.archive),
            "legacy update archive must match the Apple Silicon archive"
        );
        validate_archive(
            manifest
                .archives
                .get("x86_64")
                .expect("checked x86_64 archive"),
            X86_64_ARCHIVE_ASSET,
            manifest,
            release,
        )?;
    }
    ensure!(
        manifest.release_notes.len() <= 64 * 1024,
        "release notes are unreasonably large"
    );
    let release_url = validate_https_url(&manifest.release_url, "github.com")?;
    ensure!(
        release_url.as_str() == release.html_url,
        "manifest release URL does not match GitHub"
    );
    let minimum_components = manifest
        .minimum_macos_version
        .split('.')
        .map(str::parse::<u64>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("invalid minimum macOS version")?;
    ensure!(
        !minimum_components.is_empty(),
        "minimum macOS version is empty"
    );
    if let Ok(info) = warp_core::operating_system_info::OperatingSystemInfo::get() {
        if let Some(current_macos) = info.version() {
            ensure!(
                compare_numeric_versions(current_macos, &manifest.minimum_macos_version)?
                    != Ordering::Less,
                "this update requires macOS {} or newer",
                manifest.minimum_macos_version
            );
        }
    }
    Ok(())
}

fn numeric_version(value: &str) -> Result<Vec<u64>> {
    let components = value
        .trim_start_matches('v')
        .split(|character: char| !character.is_ascii_digit())
        .filter(|component| !component.is_empty())
        .map(str::parse::<u64>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("release version contains an invalid numeric component")?;
    ensure!(
        !components.is_empty(),
        "release version has no numeric components"
    );
    Ok(components)
}

fn compare_numeric_versions(left: &str, right: &str) -> Result<Ordering> {
    let mut left = numeric_version(left)?;
    let mut right = numeric_version(right)?;
    let length = left.len().max(right.len());
    left.resize(length, 0);
    right.resize(length, 0);
    Ok(left.cmp(&right))
}

fn validate_release_order_against(
    manifest: &Manifest,
    current_version: &str,
    current_sequence: Option<u64>,
) -> Result<()> {
    if manifest.version == current_version {
        return Ok(());
    }
    let ordering = compare_numeric_versions(&manifest.version, current_version)?;
    ensure!(
        ordering != Ordering::Less || manifest.rollback,
        "refusing an unauthenticated downgrade policy"
    );
    if let Some(current_sequence) = current_sequence {
        ensure!(
            manifest.sequence > current_sequence,
            "update sequence is not newer than the installed release"
        );
    }
    Ok(())
}

pub fn validate_release_order(manifest: &Manifest) -> Result<()> {
    let current_version = ChannelState::app_version().context("current app has no release tag")?;
    validate_release_order_against(
        manifest,
        current_version,
        ChannelState::app_update_sequence(),
    )
}

async fn fetch_small_asset(
    client: &http_client::Client,
    url: &str,
    maximum_size: usize,
) -> Result<Vec<u8>> {
    let response = client
        .get(url)
        .timeout(Duration::from_secs(30))
        .header("Accept", "application/octet-stream")
        .header("User-Agent", "Clinch-Updater")
        .send()
        .await?
        .error_for_status()?;
    let bytes = response.bytes().await?;
    ensure!(bytes.len() <= maximum_size, "update metadata is too large");
    Ok(bytes.to_vec())
}

pub async fn fetch_latest(client: &http_client::Client) -> Result<VerifiedRelease> {
    let base_url = ChannelState::releases_base_url();
    let response = client
        .get(format!("{base_url}/latest").as_str())
        .timeout(Duration::from_secs(30))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "Clinch-Updater")
        .send()
        .await?
        .error_for_status()?;
    let release: GithubRelease = response.json().await?;
    let manifest_asset = asset(&release, MANIFEST_ASSET)?;
    let signature_asset = asset(&release, SIGNATURE_ASSET)?;
    validate_https_url(&manifest_asset.browser_download_url, "github.com")?;
    validate_https_url(&signature_asset.browser_download_url, "github.com")?;
    let (manifest_bytes, signature_bytes) = futures::try_join!(
        fetch_small_asset(
            client,
            &manifest_asset.browser_download_url,
            MAX_MANIFEST_SIZE
        ),
        fetch_small_asset(
            client,
            &signature_asset.browser_download_url,
            MAX_SIGNATURE_SIZE
        )
    )?;
    let keys = trusted_keys()?;
    let manifest = verify_manifest_signature(&manifest_bytes, &signature_bytes, &keys)?;
    validate_manifest(&manifest, &release)?;
    validate_release_order(&manifest)?;
    persist_next_key(&keys, manifest.next_public_key.as_ref())?;
    let selected_archive = manifest
        .archive_for_architecture(machine_architecture()?)?
        .clone();
    Ok(VerifiedRelease {
        manifest,
        selected_archive,
    })
}

pub(super) fn update_dir(update_id: &str) -> PathBuf {
    warp_core::paths::cache_dir()
        .join("autoupdate")
        .join(update_id)
}

pub(super) fn staged_bundle_path(update_id: &str) -> PathBuf {
    update_dir(update_id).join("extracted/Clinch.app")
}

pub(super) fn archive_path(update_id: &str) -> PathBuf {
    update_dir(update_id).join(ARCHIVE_ASSET)
}

pub(super) fn staged_bundle_sequence(update_id: &str) -> Result<u64> {
    let info = plist::Value::from_file(staged_bundle_path(update_id).join("Contents/Info.plist"))?;
    info.as_dictionary()
        .and_then(|dictionary| dictionary.get("ClinchUpdateSequence"))
        .and_then(plist::Value::as_string)
        .context("staged app update sequence is missing")?
        .parse()
        .context("staged app update sequence is invalid")
}

async fn validate_staged_bundle(path: &Path, manifest: &Manifest) -> Result<()> {
    let info = plist::Value::from_file(path.join("Contents/Info.plist"))?;
    let dictionary = info
        .as_dictionary()
        .context("staged app Info.plist is not a dictionary")?;
    ensure!(
        dictionary
            .get("CFBundleIdentifier")
            .and_then(plist::Value::as_string)
            == Some(manifest.bundle_id.as_str()),
        "staged app has an unexpected bundle identifier"
    );
    ensure!(
        dictionary
            .get("WarpVersion")
            .and_then(plist::Value::as_string)
            == Some(manifest.version.as_str()),
        "staged app version does not match the update manifest"
    );
    ensure!(
        dictionary
            .get("ClinchUpdateSequence")
            .and_then(plist::Value::as_string)
            .and_then(|value| value.parse::<u64>().ok())
            == Some(manifest.sequence),
        "staged app sequence does not match the update manifest"
    );
    let executable = dictionary
        .get("CFBundleExecutable")
        .and_then(plist::Value::as_string)
        .unwrap_or("stable");
    let executable_path = path.join("Contents/MacOS").join(executable);
    ensure!(
        executable_path.is_file(),
        "staged app executable is missing"
    );
    for relative_path in [
        "Contents/Resources/update/clinch-update-helper",
        "Contents/Resources/update/clinch-update-swap",
    ] {
        let updater_component = path.join(relative_path);
        let metadata = std::fs::metadata(&updater_component)
            .with_context(|| format!("staged app omitted {relative_path}"))?;
        ensure!(
            metadata.is_file() && metadata.permissions().mode() & 0o111 != 0,
            "staged app has an invalid {relative_path}"
        );
    }

    let signature = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(path)
        .output()
        .await?;
    ensure!(
        signature.status.success(),
        "staged app code signature is invalid"
    );
    let architecture = Command::new("/usr/bin/lipo")
        .arg("-archs")
        .arg(&executable_path)
        .output()
        .await?;
    ensure!(
        architecture.status.success(),
        "could not inspect staged executable"
    );
    let machine = machine_architecture()?;
    let architectures = String::from_utf8_lossy(&architecture.stdout);
    let architectures = architectures.split_whitespace().collect::<Vec<_>>();
    if manifest.archives.is_empty() {
        ensure!(
            architectures.contains(&machine),
            "staged app does not support this Mac"
        );
    } else {
        ensure!(
            architectures == [machine],
            "staged app is not the native build for this Mac"
        );
    }
    Ok(())
}

pub async fn download_and_stage(
    release: &VerifiedRelease,
    update_id: &str,
    client: &http_client::Client,
) -> Result<PathBuf> {
    let archive = &release.selected_archive;
    let directory = update_dir(update_id);
    if directory.exists() {
        async_fs::remove_dir_all(&directory).await?;
    }
    async_fs::create_dir_all(&directory).await?;
    let archive_path = directory.join(ARCHIVE_ASSET);
    let response = client
        .get(&archive.url)
        .timeout(ARCHIVE_DOWNLOAD_TIMEOUT)
        .header("Accept", "application/octet-stream")
        .header("User-Agent", "Clinch-Updater")
        .send()
        .await?
        .error_for_status()?;
    let mut output = File::create(&archive_path).await?;
    let mut stream = response.bytes_stream();
    let mut digest = Sha256::new();
    let mut size = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        size = size
            .checked_add(chunk.len() as u64)
            .context("update archive size overflow")?;
        ensure!(
            size <= archive.size,
            "update archive exceeded its authenticated size"
        );
        digest.update(&chunk);
        output.write_all(&chunk).await?;
    }
    output.flush().await?;
    output.sync_data().await?;
    ensure!(
        size == archive.size,
        "update archive size does not match authenticated metadata"
    );
    ensure!(
        hex::encode(digest.finalize()) == archive.sha256,
        "update archive SHA-256 does not match authenticated metadata"
    );

    let extracted = directory.join("extracted");
    async_fs::create_dir_all(&extracted).await?;
    let extraction = Command::new("/usr/bin/ditto")
        .args(["-x", "-k"])
        .arg(&archive_path)
        .arg(&extracted)
        .output()
        .await?;
    ensure!(
        extraction.status.success(),
        "could not extract Clinch update archive"
    );
    let staged = extracted.join("Clinch.app");
    validate_staged_bundle(&staged, &release.manifest).await?;
    Ok(staged)
}

pub fn release_notes(release: &VerifiedRelease) -> &str {
    &release.manifest.release_notes
}

pub fn release_url(release: &VerifiedRelease) -> &str {
    &release.manifest.release_url
}

#[cfg(test)]
#[path = "clinch_tests.rs"]
mod tests;
