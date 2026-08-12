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

fn render_bundled_skill(source_root: &Path, contents: &str) -> String {
    let command_name = ChannelState::channel().warpctrl_command_name();
    let wrapper_path = source_root
        .parent()
        .and_then(Path::parent)
        .map(|resources_root| resources_root.join("bin").join(command_name))
        .unwrap_or_else(|| PathBuf::from(command_name));

    contents
        .replace("{{clinch_control_binary_name}}", command_name)
        .replace(
            "{{clinch_control_wrapper_path}}",
            &wrapper_path.to_string_lossy(),
        )
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
        let bundled_contents = render_bundled_skill(source_root, &bundled_contents);
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
        });
    }
    if let (Some(presence_dir), Some(home)) = (codex_presence, home) {
        locations.push(AgentSkillLocation {
            presence_dir,
            install_root: home.join(".agents"),
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
    let Some(source_root) = warp_core::paths::bundled_resources_dir()
        .map(|resources| resources.join("bundled").join("agent-skills"))
        .filter(|path| path.is_dir())
    else {
        // Expected for unit tests and unbundled binaries.
        return;
    };
    for location in agent_skill_locations() {
        install_skills_for_agent(&source_root, &location.presence_dir, &location.install_root);
    }
}

#[cfg(test)]
#[path = "agent_skills_tests.rs"]
mod tests;
