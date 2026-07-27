use base64::prelude::{Engine as _, BASE64_STANDARD};
use ring::signature::{Ed25519KeyPair, KeyPair as _};

use super::*;

const TEST_SEED: [u8; 32] = [7; 32];

fn fixture() -> (Manifest, GithubRelease) {
    let version = "v0.2026.07.13.1200";
    let archive_url = format!(
        "https://github.com/{EXPECTED_REPOSITORY}/releases/download/{version}/{ARCHIVE_ASSET}"
    );
    (
        Manifest {
            schema_version: 1,
            version: version.to_owned(),
            sequence: 1_800_000_000,
            minimum_macos_version: "14.0".to_owned(),
            bundle_id: EXPECTED_BUNDLE_ID.to_owned(),
            signing_key_id: "test-key".to_owned(),
            archive: Archive {
                name: ARCHIVE_ASSET.to_owned(),
                url: archive_url.clone(),
                size: 123,
                sha256: "a".repeat(64),
            },
            release_notes: "Reliable updates.".to_owned(),
            release_url: format!(
                "https://github.com/{EXPECTED_REPOSITORY}/releases/tag/{version}"
            ),
            rollback: false,
            next_public_key: None,
        },
        GithubRelease {
            tag_name: version.to_owned(),
            html_url: format!(
                "https://github.com/{EXPECTED_REPOSITORY}/releases/tag/{version}"
            ),
            assets: vec![
                GithubAsset {
                    name: MANIFEST_ASSET.to_owned(),
                    browser_download_url: format!(
                        "https://github.com/{EXPECTED_REPOSITORY}/releases/download/{version}/{MANIFEST_ASSET}"
                    ),
                    size: 1,
                },
                GithubAsset {
                    name: SIGNATURE_ASSET.to_owned(),
                    browser_download_url: format!(
                        "https://github.com/{EXPECTED_REPOSITORY}/releases/download/{version}/{SIGNATURE_ASSET}"
                    ),
                    size: 1,
                },
                GithubAsset {
                    name: ARCHIVE_ASSET.to_owned(),
                    browser_download_url: archive_url,
                    size: 123,
                },
            ],
        },
    )
}

fn signed_manifest(manifest: &Manifest) -> (Vec<u8>, Vec<u8>, TrustedKey) {
    let pair = Ed25519KeyPair::from_seed_unchecked(&TEST_SEED).expect("test seed");
    let bytes = serde_json::to_vec(manifest).expect("serialize manifest");
    let signature = BASE64_STANDARD
        .encode(pair.sign(&bytes).as_ref())
        .into_bytes();
    let key = TrustedKey {
        key_id: manifest.signing_key_id.clone(),
        ed25519_public_key: BASE64_STANDARD.encode(pair.public_key().as_ref()),
    };
    (bytes, signature, key)
}

#[test]
fn verifies_signed_manifest_and_pinned_github_assets() {
    let (manifest, release) = fixture();
    let (bytes, signature, key) = signed_manifest(&manifest);

    let verified = verify_manifest_signature(&bytes, &signature, &[key]).expect("valid signature");
    assert_eq!(verified, manifest);
    validate_manifest(&verified, &release).expect("valid release binding");
}

#[test]
fn rejects_manifest_tampering_after_signing() {
    let (manifest, _) = fixture();
    let (mut bytes, signature, key) = signed_manifest(&manifest);
    let index = bytes
        .iter()
        .position(|byte| *byte == b'R')
        .expect("release notes contain R");
    bytes[index] = b'U';

    assert!(verify_manifest_signature(&bytes, &signature, &[key]).is_err());
}

#[test]
fn rejects_archive_from_another_release_or_repository() {
    let (mut manifest, release) = fixture();
    manifest.archive.url =
        "https://github.com/example/other/releases/download/v1/Clinch.app.zip".to_owned();

    assert!(validate_manifest(&manifest, &release).is_err());
}

#[test]
fn release_order_requires_new_sequence_and_explicit_rollback() {
    let (mut manifest, _) = fixture();
    validate_release_order_against(&manifest, "v0.2026.07.12.1200", Some(manifest.sequence - 1))
        .expect("newer release");

    manifest.version = "v0.2026.07.11.1200".to_owned();
    assert!(validate_release_order_against(
        &manifest,
        "v0.2026.07.12.1200",
        Some(manifest.sequence - 1)
    )
    .is_err());
    manifest.rollback = true;
    validate_release_order_against(&manifest, "v0.2026.07.12.1200", Some(manifest.sequence - 1))
        .expect("authenticated rollback with a new sequence");
    let release = VerifiedRelease {
        manifest: manifest.clone(),
    };
    assert_eq!(release.version_info().is_rollback, Some(true));
    assert_eq!(release.archive_sha256(), "a".repeat(64));
    assert_eq!(release.archive_size(), 123);
    assert!(validate_release_order_against(
        &manifest,
        "v0.2026.07.12.1200",
        Some(manifest.sequence)
    )
    .is_err());
}

#[test]
fn version_comparison_handles_clinch_date_tags() {
    assert_eq!(
        compare_numeric_versions("v0.2026.07.13.0001", "v0.2026.07.12.2359").unwrap(),
        Ordering::Greater
    );
    assert_eq!(
        compare_numeric_versions("15.2", "15.2.0").unwrap(),
        Ordering::Equal
    );
}

#[test]
fn validates_rotated_public_keys() {
    let pair = Ed25519KeyPair::from_seed_unchecked(&[9; 32]).expect("test seed");
    let key = TrustedKey {
        key_id: "next-key".to_owned(),
        ed25519_public_key: BASE64_STANDARD.encode(pair.public_key().as_ref()),
    };
    assert_eq!(decode_key(&key).expect("valid key").len(), 32);

    let invalid = TrustedKey {
        ed25519_public_key: BASE64_STANDARD.encode([0; 31]),
        ..key
    };
    assert!(decode_key(&invalid).is_err());
}

fn instant(raw: &str) -> DateTime<Utc> {
    raw.parse().expect("test timestamp")
}

#[test]
fn a_record_with_no_history_is_due_for_an_automatic_check() {
    let record = UpdateCheckRecord::default();

    assert!(record.automatic_check_due(instant("2026-07-27T12:00:00Z")));
}

#[test]
fn a_successful_check_suppresses_automatic_checks_for_one_week() {
    let mut record = UpdateCheckRecord::default();
    record.record_success(instant("2026-07-27T12:00:00Z"));

    assert!(!record.automatic_check_due(instant("2026-08-02T11:59:59Z")));
    assert!(record.automatic_check_due(instant("2026-08-03T12:00:00Z")));
}

#[test]
fn a_failed_check_backs_off_instead_of_rechecking_immediately() {
    let mut record = UpdateCheckRecord::default();
    record.record_failure(instant("2026-07-27T12:00:00Z"));

    // The bug this replaces: a failed check recorded nothing, so every window
    // focus re-fired a request.
    assert!(!record.automatic_check_due(instant("2026-07-27T12:00:01Z")));
    assert!(!record.automatic_check_due(instant("2026-07-27T17:59:59Z")));
    assert!(record.automatic_check_due(instant("2026-07-27T18:00:00Z")));
}

#[test]
fn a_failure_backoff_applies_even_after_the_weekly_window_elapses() {
    let mut record = UpdateCheckRecord::default();
    record.record_success(instant("2026-07-01T12:00:00Z"));
    record.record_failure(instant("2026-07-27T12:00:00Z"));

    assert!(!record.automatic_check_due(instant("2026-07-27T13:00:00Z")));
}

#[test]
fn a_successful_check_clears_a_pending_failure_backoff() {
    let mut record = UpdateCheckRecord::default();
    record.record_failure(instant("2026-07-27T12:00:00Z"));
    record.record_success(instant("2026-07-27T13:00:00Z"));

    assert!(!record.automatic_check_due(instant("2026-07-27T14:00:00Z")));
    assert!(record.automatic_check_due(instant("2026-08-03T13:00:00Z")));
}

#[test]
fn legacy_date_only_records_are_read_as_a_successful_check() {
    let record = UpdateCheckRecord::parse("2026-07-27\n");

    assert!(!record.automatic_check_due(instant("2026-07-30T12:00:00Z")));
    assert!(record.automatic_check_due(instant("2026-08-04T00:00:00Z")));
}

#[test]
fn unreadable_records_fall_back_to_checking() {
    for raw in ["", "not-a-record", "{\"last_success\":"] {
        assert!(
            UpdateCheckRecord::parse(raw).automatic_check_due(instant("2026-07-27T12:00:00Z")),
            "expected {raw:?} to be treated as no recorded check"
        );
    }
}

#[test]
fn records_round_trip_through_serialization() {
    let mut record = UpdateCheckRecord::default();
    record.record_success(instant("2026-07-27T12:00:00Z"));
    record.record_failure(instant("2026-07-28T12:00:00Z"));

    let restored = UpdateCheckRecord::parse(&record.to_json());

    assert_eq!(record, restored);
}
