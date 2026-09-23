use std::fs;
use std::os::unix::fs::symlink;

use anyhow::Result;

use super::*;

#[test]
fn local_control_command_uses_clinch_ctrl_only_for_clinch_apps() {
    assert_eq!(
        local_control_cli_invocation_for("sh.clinch.Clinch", Channel::Stable),
        "clinch ctrl"
    );
    assert_eq!(
        local_control_cli_invocation_for("sh.clinch.ClinchDev", Channel::Local),
        "clinch-local ctrl"
    );
    assert_eq!(
        local_control_cli_invocation_for("dev.warp.Warp-Dev", Channel::Dev),
        "warpctrl-dev"
    );
}

#[test]
fn path_resolves_to_detects_matching_symlink() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let source = temp_dir.path().join("source");
    let target = temp_dir.path().join("target");

    fs::write(&source, "wrapper")?;
    assert!(!path_resolves_to(&target, &source));

    symlink(&source, &target)?;
    assert!(path_resolves_to(&target, &source));

    Ok(())
}
