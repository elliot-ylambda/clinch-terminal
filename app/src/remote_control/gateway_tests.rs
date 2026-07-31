use clinch_companion_protocol::{AppInstanceId, TargetRef, UploadId};
use tokio::io::AsyncWriteExt as _;

use super::*;

fn upload_plan(directory: &Path, filename: &str, bytes: &[u8]) -> UploadPlan {
    UploadPlan {
        upload_id: UploadId::new(),
        target: TargetRef {
            app_instance_id: AppInstanceId::new(),
            project_id: "project".to_owned(),
            tab_id: "tab".to_owned(),
            pane_id: "pane".to_owned(),
        },
        destination_directory: directory.to_owned(),
        filename: filename.to_owned(),
        size: bytes.len() as u64,
        sha256: hex::encode(Sha256::digest(bytes)),
    }
}

#[test]
fn public_origin_must_be_plain_tailnet_https_origin() {
    let security = GatewaySecurity::with_loopback_host("127.0.0.1:1234".to_owned());
    assert!(security
        .set_public_origin("https://mac.tailnet.ts.net/")
        .is_ok());
    for rejected in [
        "http://mac.tailnet.ts.net/",
        "https://mac.tailnet.ts.net:444/",
        "https://user@mac.tailnet.ts.net/",
        "https://mac.tailnet.ts.net/path",
        "https://mac.tailnet.ts.net/#secret",
    ] {
        assert!(security.set_public_origin(rejected).is_err(), "{rejected}");
    }
}

#[test]
fn host_and_origin_checks_are_exact() {
    let security = GatewaySecurity::with_loopback_host("127.0.0.1:1234".to_owned());
    security
        .set_public_origin("https://mac.tailnet.ts.net/")
        .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(HOST, HeaderValue::from_static("mac.tailnet.ts.net"));
    headers.insert(
        ORIGIN,
        HeaderValue::from_static("https://mac.tailnet.ts.net"),
    );
    assert!(security.validate_host(&headers));
    assert!(security.validate_origin(&headers));
    headers.insert(ORIGIN, HeaderValue::from_static("https://evil.example"));
    assert!(!security.validate_origin(&headers));
}

#[test]
fn collision_safe_publish_never_overwrites_existing_file() {
    let directory = tempfile::tempdir().unwrap();
    let staging = directory.path().join(".stage");
    std::fs::write(&staging, b"new").unwrap();
    std::fs::write(directory.path().join("photo.png"), b"old").unwrap();
    let published = publish_without_overwrite(&staging, directory.path(), "photo.png").unwrap();
    assert_eq!(published.file_name().unwrap(), "photo (1).png");
    assert_eq!(
        std::fs::read(directory.path().join("photo.png")).unwrap(),
        b"old"
    );
    assert_eq!(std::fs::read(published).unwrap(), b"new");
    assert!(!staging.exists());
}

#[tokio::test]
async fn upload_digest_mismatch_removes_the_staging_file() {
    let directory = tempfile::tempdir().unwrap();
    let bytes = b"phone upload";
    let mut plan = upload_plan(directory.path(), "photo.png", bytes);
    plan.sha256 = "0".repeat(64);
    let mut upload = stage_upload(plan).unwrap();
    let staging_path = upload.staging_path.clone();
    upload.file.write_all(bytes).await.unwrap();
    upload.received = bytes.len() as u64;
    upload.digest.update(bytes);

    assert!(finalize_upload(upload).await.is_err());
    assert!(!staging_path.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn completed_upload_is_published_owner_only() {
    use std::os::unix::fs::PermissionsExt as _;

    let directory = tempfile::tempdir().unwrap();
    let bytes = b"phone upload";
    let mut upload = stage_upload(upload_plan(directory.path(), "photo.png", bytes)).unwrap();
    upload.file.write_all(bytes).await.unwrap();
    upload.received = bytes.len() as u64;
    upload.digest.update(bytes);

    let (_, published) = finalize_upload(upload).await.unwrap();
    assert_eq!(std::fs::read(&published).unwrap(), bytes);
    assert_eq!(
        std::fs::metadata(&published).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn client_message_rate_is_bounded_and_recovers() {
    let mut messages = VecDeque::new();
    let start = Instant::now();
    for _ in 0..MAX_CLIENT_MESSAGES_PER_SECOND {
        assert!(admit_client_message(&mut messages, start));
    }
    assert!(!admit_client_message(&mut messages, start));
    assert!(admit_client_message(
        &mut messages,
        start + Duration::from_secs(2)
    ));
}
