//! Reads the per-pane agent-resume registry written by the claude wrapper / codex hooks.
//! See docs/superpowers/specs/2026-06-20-warp-agent-session-resume-design.md.

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Deserialize)]
struct RegistryEntry {
    command: String,
    #[serde(default)]
    cwd: Option<String>,
}

const CLINCH_RESUME_LAUNCHER: &str = "clinch_agent_resume_launch";
const LEGACY_WARP_RESUME_LAUNCHER: &str = "warp_agent_resume_launch";

/// Installs/refreshes the capture hooks shipped inside the Clinch app bundle.
///
/// This is intentionally fail-open: a hand-edited third-party config must never prevent the
/// terminal itself from launching. The bundled installer is idempotent and uses only macOS
/// system tools, so running it before every Clinch GUI session also heals deleted/stale hooks.
#[cfg(target_os = "macos")]
pub fn install_bundled_capture_layer() {
    use std::process::Stdio;

    use command::blocking::Command;

    let Some(resources_dir) = warp_core::paths::bundled_resources_dir() else {
        return;
    };
    let installer = resources_dir.join("agent-resume").join("install.sh");
    if !installer.is_file() {
        // Expected for unbundled local/test binaries.
        return;
    }

    let status = Command::new("/bin/bash")
        .arg(installer)
        .arg("--quiet")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if !status.is_ok_and(|status| status.success()) {
        eprintln!("clinch: automatic Claude/Codex resume setup did not complete");
    }
}

/// A ready-to-run "fork this session" command plus the directory it should run in.
/// Derived from the resume command the capture scripts already store.
pub struct ForkLaunch {
    pub command: String,
    pub cwd: Option<String>,
}

fn registry_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(Path::new(&home).join(".warp").join("agent-resume"))
}

fn read_entry_in(dir: &Path, uuid_hex: &str) -> Option<RegistryEntry> {
    let path = dir.join(format!("{uuid_hex}.json"));
    let contents = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
}

fn read_command_in(dir: &Path, uuid_hex: &str) -> Option<String> {
    Some(normalize_restore_command(
        read_entry_in(dir, uuid_hex)?.command,
    ))
}

/// Rebrand persisted commands written by older Clinch builds at the read boundary. This makes
/// existing panes display and execute the Clinch launcher immediately, while the shell keeps a
/// legacy alias as a fallback for commands restored by an older app build.
pub(crate) fn normalize_restore_command(command: String) -> String {
    let trimmed = command.trim();
    if trimmed == LEGACY_WARP_RESUME_LAUNCHER {
        return CLINCH_RESUME_LAUNCHER.to_string();
    }
    let Some(rest) = trimmed.strip_prefix("warp_agent_resume_launch ") else {
        return command;
    };
    format!("{CLINCH_RESUME_LAUNCHER} {rest}")
}

/// Turns a stored resume command (`clinch_agent_resume_launch <agent> <id> [flags…]`) into
/// a fork command, carrying the session's launch flags into the fork. Returns `None` for
/// commands we don't know how to fork (the only forkable agents today are Claude and
/// Codex). The legacy Warp-prefixed launcher remains readable while saved registries migrate.
fn derive_fork_command(command: &str) -> Option<String> {
    let command = command.trim();
    let rest = command
        .strip_prefix("clinch_agent_resume_launch ")
        .or_else(|| command.strip_prefix("warp_agent_resume_launch "))?;
    let mut parts = rest.split_whitespace();
    let agent = parts.next()?;
    let id = parts.next()?;
    let flags = parts.collect::<Vec<_>>().join(" ");
    let flags = if flags.is_empty() {
        String::new()
    } else {
        format!(" {flags}")
    };
    match agent {
        "claude" => Some(format!("claude --resume {id}{flags} --fork-session")),
        "codex" => Some(format!("codex fork {id}{flags}")),
        _ => None,
    }
}

fn read_fork_launch_in(dir: &Path, uuid_hex: &str) -> Option<ForkLaunch> {
    let entry = read_entry_in(dir, uuid_hex)?;
    let command = derive_fork_command(&entry.command)?;
    Some(ForkLaunch {
        command,
        cwd: entry.cwd,
    })
}

/// Returns the resume command stored for `uuid`, if any. `uuid` is the raw pane UUID bytes;
/// it is hex-encoded (lowercase) to match `$WARP_TERMINAL_SESSION_UUID`.
pub fn read_on_restore_command(uuid: &[u8]) -> Option<String> {
    let dir = registry_dir()?;
    read_command_in(&dir, &hex::encode(uuid))
}

/// Returns the fork launch (command + cwd) for `uuid`, if the pane has a forkable
/// agent session in the registry.
pub fn read_fork_launch(uuid: &[u8]) -> Option<ForkLaunch> {
    let dir = registry_dir()?;
    read_fork_launch_in(&dir, &hex::encode(uuid))
}

#[cfg(test)]
#[path = "agent_resume_tests.rs"]
mod tests;
