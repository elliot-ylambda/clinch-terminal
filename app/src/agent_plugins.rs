//! Best-effort startup provisioning for the Warp notification plugins bundled with Clinch.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use command::blocking::Command;

use crate::channel::ChannelState;

fn app_id_enables_bundled_plugins(app_id: &str) -> bool {
    matches!(app_id, "sh.clinch.Clinch" | "sh.clinch.ClinchDev")
}

fn installer_in(resources_dir: &Path) -> PathBuf {
    resources_dir
        .join("bundled")
        .join("agent-plugins")
        .join("install.sh")
}

fn bundled_installer() -> Option<PathBuf> {
    warp_core::paths::bundled_resources_dir().map(|resources| installer_in(&resources))
}

/// Installs or refreshes the exact Claude Code and Codex notification plugins shipped in the
/// current Clinch bundle. Provider absence and provider policy failures are intentionally
/// non-fatal; the bundled script retries missing/outdated plugins on the next application launch.
pub fn install_bundled_plugins() {
    if !app_id_enables_bundled_plugins(&ChannelState::app_id().to_string()) {
        return;
    }

    let Some(installer) = bundled_installer().filter(|path| path.is_file()) else {
        // Expected for unit tests and unbundled binaries.
        return;
    };

    let status = Command::new("/bin/bash")
        .arg(installer)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if !status.is_ok_and(|status| status.success()) {
        eprintln!("clinch: bundled Claude/Codex notification plugins could not be provisioned");
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{app_id_enables_bundled_plugins, installer_in};

    #[test]
    fn enables_only_clinch_app_ids() {
        assert!(app_id_enables_bundled_plugins("sh.clinch.Clinch"));
        assert!(app_id_enables_bundled_plugins("sh.clinch.ClinchDev"));
        assert!(!app_id_enables_bundled_plugins("dev.warp.Warp-Stable"));
    }

    #[test]
    fn resolves_installer_inside_bundled_resources() {
        assert_eq!(
            installer_in(Path::new("/Applications/Clinch.app/Contents/Resources")),
            Path::new(
                "/Applications/Clinch.app/Contents/Resources/bundled/agent-plugins/install.sh"
            )
        );
    }
}
