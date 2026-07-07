//! On-demand discovery of CLI-agent slash commands (Claude Code / Codex) for the
//! quick-insert-button picker. Unlike skills, there is no live watcher — the
//! popup scans when it opens.
//!
// The quick-insert modal (Task 5) is the only consumer; until it lands, the
// public entry point and the Codex path are unreferenced. Remove this allow
// when the modal calls `discover_commands`.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::{AppContext, SingletonEntity};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandScope {
    Home,
    Project,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandProvider {
    Claude,
    Codex,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredCommand {
    /// Display + insert text, e.g. "/review" or "/frontend:component".
    pub name: String,
    pub invocation: String,
    pub description: Option<String>,
    pub scope: CommandScope,
    pub provider: CommandProvider,
    pub path: PathBuf,
}

/// Discovers slash commands across user (home) and project scope.
pub fn discover_commands(working_directory: &Path, ctx: &AppContext) -> Vec<DiscoveredCommand> {
    let mut out = Vec::new();

    if let Some(home) = dirs::home_dir() {
        out.extend(scan_claude_commands_dir(
            &home.join(".claude/commands"),
            CommandScope::Home,
        ));
        out.extend(scan_codex_prompts_dir(
            &home.join(".codex/prompts"),
            CommandScope::Home,
        ));
    }

    let project_root = repo_metadata::repositories::DetectedRepositories::as_ref(ctx)
        .get_root_for_path(&LocalOrRemotePath::Local(working_directory.to_path_buf()))
        .and_then(|root| root.to_local_path().map(Path::to_path_buf));
    if let Some(root) = project_root {
        out.extend(scan_claude_commands_dir(
            &root.join(".claude/commands"),
            CommandScope::Project,
        ));
    }

    out
}

/// Recursively scans a Claude `commands` dir; names are namespaced by subdir
/// (`frontend/component.md` -> `/frontend:component`).
fn scan_claude_commands_dir(dir: &Path, scope: CommandScope) -> Vec<DiscoveredCommand> {
    let mut out = Vec::new();
    collect_markdown(dir, dir, &mut out, scope, CommandProvider::Claude);
    out
}

/// Scans a Codex `prompts` dir (flat; name = `/<file-stem>`).
fn scan_codex_prompts_dir(dir: &Path, scope: CommandScope) -> Vec<DiscoveredCommand> {
    let mut out = Vec::new();
    collect_markdown(dir, dir, &mut out, scope, CommandProvider::Codex);
    // Codex prompts are not namespaced by subdir; the ":" join is harmless for a
    // flat dir but keep provider distinct for future divergence.
    out
}

fn collect_markdown(
    base: &Path,
    dir: &Path,
    out: &mut Vec<DiscoveredCommand>,
    scope: CommandScope,
    provider: CommandProvider,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_markdown(base, &path, out, scope, provider);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Ok(rel) = path.strip_prefix(base) else {
            continue;
        };
        let stem = rel.with_extension("");
        let name = format!(
            "/{}",
            stem.components()
                .filter_map(|c| c.as_os_str().to_str())
                .collect::<Vec<_>>()
                .join(":")
        );
        let description = std::fs::read_to_string(&path)
            .ok()
            .and_then(|c| front_matter_description(&c));
        out.push(DiscoveredCommand {
            invocation: name.clone(),
            name,
            description,
            scope,
            provider,
            path,
        });
    }
}

/// Extracts a `description:` value from optional YAML front matter.
fn front_matter_description(content: &str) -> Option<String> {
    let rest = content.strip_prefix("---")?;
    let end = rest.find("\n---")?;
    let yaml = &rest[..end];
    let value: serde_yaml::Value = serde_yaml::from_str(yaml.trim()).ok()?;
    value
        .get("description")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;
