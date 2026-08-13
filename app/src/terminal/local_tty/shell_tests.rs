use super::*;

#[cfg(unix)]
fn write_executable(path: &Path) {
    std::fs::write(path, "#!/bin/sh\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[test]
fn test_program_invalid_bash() {
    // This test assumes there is no bash binary at /some/weird/path/bash.
    let shell_path = "/some/weird/path/bash".to_owned();
    assert!(supported_shell_path_and_type(&shell_path).is_none());
}

#[test]
fn test_program_invalid_zsh() {
    // This test assumes there is no bash zsh at /some/weird/path/bash.
    let shell_path = "/some/weird/path/zsh".to_owned();
    assert!(supported_shell_path_and_type(&shell_path).is_none());
}

#[test]
fn test_program_unknown_shell() {
    let shell_path = "/some/weird/path/wtfsh".to_owned();
    assert!(supported_shell_path_and_type(&shell_path).is_none());
}

#[test]
fn test_trim_wsl_err_from_output() {
    assert_eq!(
        take_until_utf16_crlf(b"/bin/bash\n".to_vec()),
        b"/bin/bash\n".to_vec()
    );
    assert_eq!(
        take_until_utf16_crlf(b"/bin/bash\n\r\0\n\0W\0A\0R\0N\0I\0N\0G\0".to_vec()),
        b"/bin/bash\n".to_vec()
    );
}

#[test]
#[cfg(unix)]
fn clinch_control_environment_requires_current_bundle_wrapper() {
    let bundle = tempfile::tempdir().unwrap();
    let bin = bundle.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();

    assert!(clinch_control_environment_for(
        "sh.clinch.ClinchDev",
        Some(bundle.path().to_owned()),
        "warpctrl-local",
    )
    .is_none());

    let wrapper = bin.join("warpctrl-local");
    write_executable(&wrapper);
    assert_eq!(
        clinch_control_environment_for(
            "sh.clinch.ClinchDev",
            Some(bundle.path().to_owned()),
            "warpctrl-local",
        ),
        Some(("warpctrl-local".to_owned(), wrapper))
    );
}

#[test]
#[cfg(unix)]
fn clinch_control_environment_rejects_non_clinch_apps() {
    let bundle = tempfile::tempdir().unwrap();
    let wrapper = bundle.path().join("bin/warpctrl");
    std::fs::create_dir_all(wrapper.parent().unwrap()).unwrap();
    write_executable(&wrapper);

    assert!(clinch_control_environment_for(
        "dev.warp.Warp-Stable",
        Some(bundle.path().to_owned()),
        "warpctrl",
    )
    .is_none());
}
