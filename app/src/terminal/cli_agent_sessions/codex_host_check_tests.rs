use std::path::PathBuf;

#[cfg(unix)]
use super::{check_codex_host, CodexHostHealth};
use super::{missing_host_message, search_path, WarnOnce, CODE_MODE_HOST_BINARY};

#[cfg(unix)]
fn write_executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt as _;

    std::fs::write(path, "#!/bin/sh\n").unwrap();
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).unwrap();
}

#[cfg(unix)]
fn path_env_for(dirs: &[&std::path::Path]) -> std::ffi::OsString {
    std::env::join_paths(dirs.iter().map(|dir| dir.to_path_buf())).unwrap()
}

#[cfg(unix)]
#[test]
fn healthy_when_host_is_next_to_codex() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let bin = temp_dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    write_executable(&bin.join("codex"));
    write_executable(&bin.join(CODE_MODE_HOST_BINARY));

    assert_eq!(
        check_codex_host(&path_env_for(&[&bin])),
        CodexHostHealth::Healthy
    );
}

#[cfg(unix)]
#[test]
fn healthy_when_codex_is_a_symlink_and_host_is_next_to_its_target() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let bin = temp_dir.path().join("bin");
    let libexec = temp_dir.path().join("libexec");
    std::fs::create_dir(&bin).unwrap();
    std::fs::create_dir(&libexec).unwrap();
    write_executable(&libexec.join("codex"));
    write_executable(&libexec.join(CODE_MODE_HOST_BINARY));
    std::os::unix::fs::symlink(libexec.join("codex"), bin.join("codex")).unwrap();

    // Only `bin` is on the PATH: the host is reachable solely through the
    // canonicalized symlink target's directory, mirroring a Homebrew
    // symlink-into-bin install.
    assert_eq!(
        check_codex_host(&path_env_for(&[&bin])),
        CodexHostHealth::Healthy
    );
}

#[cfg(unix)]
#[test]
fn healthy_when_host_is_elsewhere_on_path() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let bin = temp_dir.path().join("bin");
    let other = temp_dir.path().join("other");
    std::fs::create_dir(&bin).unwrap();
    std::fs::create_dir(&other).unwrap();
    write_executable(&bin.join("codex"));
    write_executable(&other.join(CODE_MODE_HOST_BINARY));

    assert_eq!(
        check_codex_host(&path_env_for(&[&bin, &other])),
        CodexHostHealth::Healthy
    );
}

#[cfg(unix)]
#[test]
fn missing_host_is_unhealthy() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let bin = temp_dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    write_executable(&bin.join("codex"));

    assert_eq!(
        check_codex_host(&path_env_for(&[&bin])),
        CodexHostHealth::CodeModeHostMissing
    );
}

#[cfg(unix)]
#[test]
fn missing_codex_reports_codex_not_found() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let bin = temp_dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();

    // No warning is ever shown for this state: without `codex` there is
    // nothing to nag about.
    assert_eq!(
        check_codex_host(&path_env_for(&[&bin])),
        CodexHostHealth::CodexNotFound
    );
}

#[test]
fn search_path_appends_fallback_dirs_and_dedupes() {
    let interactive = std::env::join_paths([
        PathBuf::from("/somewhere/bin"),
        PathBuf::from("/opt/homebrew/bin"),
    ])
    .unwrap()
    .into_string()
    .unwrap();

    let searched: Vec<PathBuf> = std::env::split_paths(&search_path(Some(interactive))).collect();

    assert_eq!(
        searched,
        [
            PathBuf::from("/somewhere/bin"),
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
        ]
    );
}

#[test]
fn warn_once_claims_exactly_once() {
    let warn_once = WarnOnce::new();
    assert!(!warn_once.claimed());
    assert!(warn_once.claim());
    assert!(warn_once.claimed());
    assert!(!warn_once.claim());
    assert!(!warn_once.claim());
}

#[test]
fn missing_host_message_names_binary_and_brew_cause() {
    let message = missing_host_message();
    assert!(message.contains(CODE_MODE_HOST_BINARY));
    assert!(message.contains("brew upgrade --cask codex"));
}
