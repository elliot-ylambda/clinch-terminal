//! Best-effort startup provisioning of the agent skills bundled with Clinch
//! into the Claude Code and Codex personal skill directories.

use std::path::{Path, PathBuf};

use crate::channel::ChannelState;

const MANAGED_MARKER_PREFIX: &str = "<!-- managed-by: Clinch; version: ";
const MANAGED_MARKER_SUFFIX: &str = "-->";

/// Extracts the Clinch managed-marker version (e.g. `[1, 0, 0]`) from a skill
/// file, if the file carries one.
fn managed_version(contents: &str) -> Option<Vec<u64>> {
    contents.lines().find_map(|line| {
        let rest = line.trim().strip_prefix(MANAGED_MARKER_PREFIX)?;
        let version = rest.strip_suffix(MANAGED_MARKER_SUFFIX)?.trim();
        let parts: Option<Vec<u64>> = version.split('.').map(|part| part.parse().ok()).collect();
        parts.filter(|parts| !parts.is_empty())
    })
}

#[derive(Debug, PartialEq, Eq)]
enum InstallDecision {
    Install,
    SkipUpToDate,
    SkipUserOwned,
}

/// A missing target is installed. An existing target is only ever replaced
/// when both sides carry a Clinch marker and the bundled version is newer;
/// files without a marker belong to the user.
fn decide(bundled_contents: &str, existing_contents: Option<&str>) -> InstallDecision {
    let Some(existing) = existing_contents else {
        return InstallDecision::Install;
    };
    let (Some(bundled_version), Some(existing_version)) =
        (managed_version(bundled_contents), managed_version(existing))
    else {
        return InstallDecision::SkipUserOwned;
    };
    if bundled_version > existing_version {
        InstallDecision::Install
    } else {
        InstallDecision::SkipUpToDate
    }
}

fn bundled_control_wrapper(resources_root: &Path) -> PathBuf {
    resources_root
        .join("bin")
        .join(ChannelState::channel().warpctrl_command_name())
}

fn bundle_has_control_wrapper(resources_root: &Path) -> bool {
    crate::util::path::file_exists_and_is_executable(&bundled_control_wrapper(resources_root))
}

/// Installs every bundled skill under `source_root` into
/// `<install_root>/skills/`. Quietly does nothing when `presence_dir` is absent
/// (the agent is not installed on this machine); creating it would litter
/// machines that never ran that agent.
fn install_skills_for_agent(source_root: &Path, presence_dir: &Path, install_root: &Path) {
    if !presence_dir.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(source_root) else {
        return;
    };
    for entry in entries.flatten() {
        let bundled_skill = entry.path().join("SKILL.md");
        let Ok(bundled_contents) = std::fs::read_to_string(&bundled_skill) else {
            continue;
        };
        let target_dir = install_root.join("skills").join(entry.file_name());
        let target = target_dir.join("SKILL.md");
        let existing_contents = std::fs::read_to_string(&target).ok();
        if decide(&bundled_contents, existing_contents.as_deref()) != InstallDecision::Install {
            continue;
        }
        let installed = std::fs::create_dir_all(&target_dir)
            .and_then(|()| std::fs::write(&target, &bundled_contents));
        if let Err(err) = installed {
            eprintln!(
                "clinch: could not provision bundled skill {}: {err}",
                target.display()
            );
        }
    }
}

/// Removes obsolete Codex-location copies only after an equal-or-newer managed
/// replacement exists in the shared Agent Skills directory. Files without the
/// Clinch marker are user-owned and are never touched.
fn remove_migrated_legacy_skills(source_root: &Path, legacy_root: &Path, install_root: &Path) {
    let Ok(entries) = std::fs::read_dir(source_root) else {
        return;
    };
    for entry in entries.flatten() {
        let skill_name = entry.file_name();
        let legacy_dir = legacy_root.join(&skill_name);
        let legacy = legacy_dir.join("SKILL.md");
        let installed = install_root
            .join("skills")
            .join(&skill_name)
            .join("SKILL.md");
        let (Ok(legacy_contents), Ok(installed_contents)) = (
            std::fs::read_to_string(&legacy),
            std::fs::read_to_string(&installed),
        ) else {
            continue;
        };
        let (Some(legacy_version), Some(installed_version)) = (
            managed_version(&legacy_contents),
            managed_version(&installed_contents),
        ) else {
            continue;
        };
        if installed_version < legacy_version {
            continue;
        }
        if let Err(err) = std::fs::remove_file(&legacy) {
            eprintln!(
                "clinch: could not remove migrated managed skill {}: {err}",
                legacy.display()
            );
            continue;
        }
        // Remove the legacy skill directory only when it contains no user
        // files. `remove_dir` safely leaves non-empty directories in place.
        let _ = std::fs::remove_dir(legacy_dir);
    }
}

#[cfg(test)]
fn install_skills_from(source_root: &Path, agent_config_dir: &Path) {
    install_skills_for_agent(source_root, agent_config_dir, agent_config_dir);
}

fn app_id_enables_bundled_skills(app_id: &str) -> bool {
    matches!(app_id, "sh.clinch.Clinch" | "sh.clinch.ClinchDev")
}

#[derive(Debug, PartialEq, Eq)]
struct AgentSkillLocation {
    presence_dir: PathBuf,
    install_root: PathBuf,
    legacy_managed_root: Option<PathBuf>,
}

/// User-scope skill locations for supported agents. Claude Code discovers
/// personal skills below its config directory. Codex uses the shared Agent
/// Skills location at `~/.agents/skills`; `CODEX_HOME` remains the presence
/// check so Clinch does not create that directory on machines without Codex.
fn agent_skill_locations() -> Vec<AgentSkillLocation> {
    let home = dirs::home_dir();
    let claude = std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.clone().map(|home| home.join(".claude")));
    let codex_presence = std::env::var_os("CODEX_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.clone().map(|home| home.join(".codex")));

    let mut locations = Vec::new();
    if let Some(claude) = claude {
        locations.push(AgentSkillLocation {
            presence_dir: claude.clone(),
            install_root: claude,
            legacy_managed_root: None,
        });
    }
    if let (Some(presence_dir), Some(home)) = (codex_presence, home) {
        let legacy_managed_root = presence_dir.join("skills");
        locations.push(AgentSkillLocation {
            presence_dir,
            install_root: home.join(".agents"),
            legacy_managed_root: Some(legacy_managed_root),
        });
    }
    locations
}

/// Installs or refreshes the agent skills shipped in the current Clinch bundle
/// into the Claude Code and Codex personal skill directories. Best-effort by
/// design: a failure must never prevent Clinch from starting, and outdated or
/// missing skills are retried on the next launch.
pub fn install_bundled_skills() {
    if !app_id_enables_bundled_skills(&ChannelState::app_id().to_string()) {
        return;
    }
    let Some(resources_root) = warp_core::paths::bundled_resources_dir() else {
        // Expected for unit tests and unbundled binaries.
        return;
    };
    // Never advertise agent control from a partial or older bundle. The
    // skills resolve the wrapper exported by the current app, so provisioning
    // them without that wrapper would create a durable but unusable user-scope
    // instruction file.
    if !bundle_has_control_wrapper(&resources_root) {
        return;
    }
    let source_root = resources_root.join("bundled").join("agent-skills");
    if !source_root.is_dir() {
        return;
    }
    for location in agent_skill_locations() {
        install_skills_for_agent(&source_root, &location.presence_dir, &location.install_root);
        if let Some(legacy_root) = location.legacy_managed_root {
            remove_migrated_legacy_skills(&source_root, &legacy_root, &location.install_root);
        }
    }
}

#[cfg(test)]
#[path = "agent_skills_tests.rs"]
mod tests;
