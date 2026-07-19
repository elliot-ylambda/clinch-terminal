//! Best-effort startup provisioning of the agent skills bundled with Clinch
//! into the Claude Code and Codex personal skill directories.

use std::path::Path;

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

/// Installs every bundled skill under `source_root` into
/// `<agent_config_dir>/skills/`. Quietly does nothing when the agent's config
/// dir is absent (the agent is not installed on this machine); creating it
/// would litter machines that never ran that agent.
fn install_skills_from(source_root: &Path, agent_config_dir: &Path) {
    if !agent_config_dir.is_dir() {
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
        let target_dir = agent_config_dir.join("skills").join(entry.file_name());
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
#[path = "agent_skills_tests.rs"]
mod tests;
