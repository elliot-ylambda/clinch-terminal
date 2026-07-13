use std::collections::HashMap;
use std::path::PathBuf;

use ai::skills::SkillReference;
#[cfg(not(feature = "local_fs"))]
use ai::skills::SkillScope;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::AppContext;

use super::SkillsSubtab;
use crate::ai::skills::{SkillDescriptor, SkillManager};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CatalogGroup {
    pub id: String,
    pub label: String,
    pub order: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogStatus {
    Active,
    Contextual { invocation: String },
    Shadowed { by: String },
    Limited { reason: String },
    Disabled { reason: String },
}

impl CatalogStatus {
    pub fn label(&self) -> Option<String> {
        match self {
            Self::Active => None,
            Self::Contextual { invocation } => Some(format!("contextual as /{invocation}")),
            Self::Shadowed { by } => Some(format!("shadowed by {by}")),
            Self::Limited { reason } => Some(format!("limited: {reason}")),
            Self::Disabled { reason } => Some(format!("disabled: {reason}")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatalogKind {
    Skill,
    Command,
    PluginSkill,
    PluginCommand,
    Managed,
    Bundled,
    System,
    Admin,
}

impl CatalogKind {
    fn label(self) -> &'static str {
        match self {
            Self::Skill => "skill",
            Self::Command => "legacy command",
            Self::PluginSkill => "plugin skill",
            Self::PluginCommand => "plugin command",
            Self::Managed => "managed skill",
            Self::Bundled => "bundled skill",
            Self::System => "system skill",
            Self::Admin => "admin skill",
        }
    }

    fn is_skill(self) -> bool {
        matches!(
            self,
            Self::Skill
                | Self::PluginSkill
                | Self::Managed
                | Self::Bundled
                | Self::System
                | Self::Admin
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CatalogOrigin {
    Enterprise,
    Personal,
    Project {
        owner: LocalOrRemotePath,
        nested: bool,
    },
    Plugin {
        name: String,
    },
    Bundled,
    System,
    Admin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogSkill {
    pub descriptor: SkillDescriptor,
    pub display_name: String,
    pub group: CatalogGroup,
    pub source_label: String,
    pub status: CatalogStatus,
    pub kind: CatalogKind,
    origin: CatalogOrigin,
}

impl CatalogSkill {
    pub fn stable_id(&self) -> String {
        format!("{}:{}", self.display_name, self.descriptor.reference)
    }

    pub fn subtitle(&self) -> String {
        let mut parts = vec![self.kind.label().to_string(), self.source_label.clone()];
        if let Some(status) = self.status.label() {
            parts.push(status);
        }
        parts.join(" · ")
    }

    pub fn path(&self) -> Option<LocalOrRemotePath> {
        match &self.descriptor.reference {
            SkillReference::Path(path) => Some(path.clone()),
            SkillReference::BundledSkillId(_) => None,
        }
    }
}

pub fn group_catalog_skills(skills: Vec<CatalogSkill>) -> Vec<(CatalogGroup, Vec<CatalogSkill>)> {
    let mut grouped: HashMap<String, (CatalogGroup, Vec<CatalogSkill>)> = HashMap::new();
    for skill in skills {
        let group = skill.group.clone();
        grouped
            .entry(group.id.clone())
            .or_insert_with(|| (group, Vec::new()))
            .1
            .push(skill);
    }

    let mut grouped: Vec<_> = grouped.into_values().collect();
    for (_, skills) in &mut grouped {
        skills.sort_by(|a, b| {
            a.display_name
                .to_lowercase()
                .cmp(&b.display_name.to_lowercase())
                .then_with(|| a.source_label.cmp(&b.source_label))
        });
    }
    grouped.sort_by(|(a, _), (b, _)| a.order.cmp(&b.order).then_with(|| a.label.cmp(&b.label)));
    grouped
}

#[cfg(feature = "local_fs")]
mod local {
    use std::collections::{HashMap, HashSet};
    use std::fs;
    use std::path::{Component, Path};
    use std::time::SystemTime;

    use ai::skills::{parse_skill_content_at_location, SkillProvider, SkillScope};
    use repo_metadata::repositories::DetectedRepositories;
    use serde_json::Value as JsonValue;
    use toml::Value as TomlValue;
    use walkdir::{DirEntry, WalkDir};
    use warpui::SingletonEntity;

    use super::*;
    use crate::ai::skills::extract_skill_parent_directory;

    const CLAUDE_BUNDLED_SKILLS: &[(&str, &str)] = &[
        ("batch", "Orchestrate large-scale changes across worktrees."),
        (
            "claude-api",
            "Load Claude API reference and migration guidance.",
        ),
        (
            "code-review",
            "Review changes for correctness and cleanup opportunities.",
        ),
        (
            "dataviz",
            "Apply chart, dashboard, palette, and accessibility guidance.",
        ),
        (
            "debug",
            "Diagnose Claude Code runtime issues from debug logs.",
        ),
        (
            "design-sync",
            "Upload a React design system for Claude Design to use.",
        ),
        (
            "doctor",
            "Diagnose Claude Code installation and configuration issues.",
        ),
        (
            "fewer-permission-prompts",
            "Suggest safe permission allowlist entries from prior usage.",
        ),
        (
            "loop",
            "Run a prompt repeatedly while the session remains open.",
        ),
        ("run", "Launch and drive the current project."),
        (
            "run-skill-generator",
            "Record a repeatable project launch and verification recipe.",
        ),
        (
            "simplify",
            "Review changed code and apply focused cleanups.",
        ),
        ("verify", "Build, run, and observe a code change."),
    ];

    #[derive(Default)]
    struct ClaudeSettings {
        enabled_plugins: HashMap<String, bool>,
        skill_overrides: HashMap<String, String>,
        disable_bundled_skills: bool,
        disable_doctor: bool,
        strict_plugin_only_skills: bool,
    }

    pub(super) fn catalog_for_subtab(
        manager: &SkillManager,
        subtab: SkillsSubtab,
        working_directory: Option<&LocalOrRemotePath>,
        app: &AppContext,
    ) -> Vec<CatalogSkill> {
        let home = dirs::home_dir();
        let repo_root = working_directory
            .and_then(|cwd| DetectedRepositories::as_ref(app).get_root_for_path(cwd));

        let mut entries = match subtab {
            SkillsSubtab::Claude => claude_catalog(
                manager,
                working_directory,
                repo_root.as_ref(),
                home.as_deref(),
                app,
            ),
            SkillsSubtab::Codex => codex_catalog(
                manager,
                working_directory,
                repo_root.as_ref(),
                home.as_deref(),
                app,
            ),
            SkillsSubtab::All => {
                let mut all = claude_catalog(
                    manager,
                    working_directory,
                    repo_root.as_ref(),
                    home.as_deref(),
                    app,
                );
                all.extend(codex_catalog(
                    manager,
                    working_directory,
                    repo_root.as_ref(),
                    home.as_deref(),
                    app,
                ));
                all.extend(generic_catalog(
                    manager,
                    working_directory,
                    repo_root.as_ref(),
                    home.as_deref(),
                    app,
                ));
                all
            }
        };

        // Agent-specific catalogs are built before the generic fallback. Keep their richer
        // representation when both catalogs point to the same file.
        let mut deduped = HashMap::new();
        for entry in entries.drain(..) {
            deduped
                .entry(entry.descriptor.reference.to_string())
                .or_insert(entry);
        }
        let mut entries: Vec<_> = deduped.into_values().collect();
        entries.sort_by(|a, b| {
            a.group
                .order
                .cmp(&b.group.order)
                .then_with(|| {
                    a.display_name
                        .to_lowercase()
                        .cmp(&b.display_name.to_lowercase())
                })
                .then_with(|| a.source_label.cmp(&b.source_label))
        });
        entries
    }

    fn claude_catalog(
        manager: &SkillManager,
        working_directory: Option<&LocalOrRemotePath>,
        repo_root: Option<&LocalOrRemotePath>,
        home: Option<&Path>,
        app: &AppContext,
    ) -> Vec<CatalogSkill> {
        let mut entries: Vec<_> = manager
            .file_skill_variants_for_working_directory(working_directory, app)
            .into_iter()
            .filter(|skill| skill.provider == SkillProvider::Claude)
            .map(|descriptor| {
                entry_from_descriptor(
                    descriptor,
                    false,
                    working_directory,
                    repo_root,
                    home,
                    CatalogKind::Skill,
                )
            })
            .collect();

        if let Some(cwd) = working_directory {
            entries.extend(
                manager
                    .descendant_file_skill_variants_for_provider(cwd, SkillProvider::Claude, app)
                    .into_iter()
                    .map(|descriptor| {
                        entry_from_descriptor(
                            descriptor,
                            true,
                            working_directory,
                            repo_root,
                            home,
                            CatalogKind::Skill,
                        )
                    }),
            );
        }

        let Some(home) = home else {
            apply_claude_precedence(&mut entries, &ClaudeSettings::default());
            return entries;
        };
        // Local config and plugin registries describe the local Claude installation. Do not
        // mix them into a remote session's catalog.
        if working_directory.is_some_and(LocalOrRemotePath::is_remote) {
            apply_claude_precedence(&mut entries, &ClaudeSettings::default());
            return entries;
        }

        let local_cwd = working_directory.and_then(LocalOrRemotePath::to_local_path);
        let local_repo_root = repo_root.and_then(LocalOrRemotePath::to_local_path);
        let settings = read_claude_settings(home, local_repo_root);

        if let (Some(cwd), Some(working_directory)) = (local_cwd, working_directory) {
            entries.extend(discover_local_claude_project_skills(
                cwd,
                working_directory,
                repo_root,
                home,
            ));
        }

        for managed_root in claude_managed_roots() {
            entries.extend(discover_recursive_skills(
                &managed_root.join("skills"),
                SkillProvider::Claude,
                SkillScope::Bundled,
                CatalogKind::Managed,
                CatalogOrigin::Enterprise,
                system_group("Enterprise managed", 50),
                home,
                local_repo_root,
            ));
        }

        entries.extend(discover_commands(
            &home.join(".claude/commands"),
            SkillScope::Home,
            CatalogOrigin::Personal,
            personal_group(),
            home,
            local_repo_root,
        ));

        if let Some(cwd) = local_cwd {
            for owner in ancestor_chain(cwd, local_repo_root) {
                let group = project_group(
                    &LocalOrRemotePath::Local(owner.to_path_buf()),
                    false,
                    working_directory,
                    repo_root,
                );
                entries.extend(discover_commands(
                    &owner.join(".claude/commands"),
                    SkillScope::Project,
                    CatalogOrigin::Project {
                        owner: LocalOrRemotePath::Local(owner.to_path_buf()),
                        nested: false,
                    },
                    group,
                    home,
                    local_repo_root,
                ));
            }
        }

        entries.extend(discover_claude_plugins(home, &settings, local_repo_root));
        entries.extend(discover_skills_directory_plugins(
            &home.join(".claude/skills"),
            SkillScope::Home,
            home,
            local_repo_root,
            &settings,
        ));
        if let Some(cwd) = local_cwd {
            entries.extend(discover_skills_directory_plugins(
                &cwd.join(".claude/skills"),
                SkillScope::Project,
                home,
                local_repo_root,
                &settings,
            ));
        }

        entries.extend(claude_bundled_entries());
        apply_claude_precedence(&mut entries, &settings);
        entries
    }

    fn codex_catalog(
        manager: &SkillManager,
        working_directory: Option<&LocalOrRemotePath>,
        repo_root: Option<&LocalOrRemotePath>,
        home: Option<&Path>,
        app: &AppContext,
    ) -> Vec<CatalogSkill> {
        let mut entries: Vec<_> = manager
            .file_skill_variants_for_working_directory(working_directory, app)
            .into_iter()
            .filter(|skill| skill.provider == SkillProvider::Agents)
            .map(|descriptor| {
                entry_from_descriptor(
                    descriptor,
                    false,
                    working_directory,
                    repo_root,
                    home,
                    CatalogKind::Skill,
                )
            })
            .collect();

        let Some(home) = home else {
            return entries;
        };
        if working_directory.is_some_and(LocalOrRemotePath::is_remote) {
            return entries;
        }
        let local_repo_root = repo_root.and_then(LocalOrRemotePath::to_local_path);

        entries.extend(discover_codex_home_skills(home, local_repo_root));
        entries.extend(discover_recursive_skills(
            Path::new("/etc/codex/skills"),
            SkillProvider::Codex,
            SkillScope::Bundled,
            CatalogKind::Admin,
            CatalogOrigin::Admin,
            system_group("Codex admin", 500),
            home,
            local_repo_root,
        ));

        let config = read_toml(&home.join(".codex/config.toml"));
        let disabled = disabled_codex_skill_paths(config.as_ref(), home);
        for entry in &mut entries {
            if entry
                .path()
                .and_then(|path| path.to_local_path().map(normalized_path))
                .is_some_and(|path| disabled.contains(&path))
            {
                entry.status = CatalogStatus::Disabled {
                    reason: "~/.codex/config.toml".to_string(),
                };
            }
        }

        entries.extend(discover_codex_plugins(
            home,
            config.as_ref(),
            local_repo_root,
        ));
        entries
    }

    fn generic_catalog(
        manager: &SkillManager,
        working_directory: Option<&LocalOrRemotePath>,
        repo_root: Option<&LocalOrRemotePath>,
        home: Option<&Path>,
        app: &AppContext,
    ) -> Vec<CatalogSkill> {
        let mut entries: Vec<_> = manager
            .file_skill_variants_for_working_directory(working_directory, app)
            .into_iter()
            .filter(|skill| {
                !matches!(
                    skill.provider,
                    SkillProvider::Agents | SkillProvider::Claude
                )
            })
            .map(|descriptor| {
                entry_from_descriptor(
                    descriptor,
                    false,
                    working_directory,
                    repo_root,
                    home,
                    CatalogKind::Skill,
                )
            })
            .collect();
        entries.extend(
            manager
                .get_skills_for_working_directory(working_directory, app)
                .into_iter()
                .filter(|skill| skill.scope == SkillScope::Bundled)
                .map(|descriptor| CatalogSkill {
                    display_name: descriptor.name.clone(),
                    descriptor,
                    group: system_group("Warp bundled", 750),
                    source_label: "bundled with Warp".to_string(),
                    status: CatalogStatus::Active,
                    kind: CatalogKind::Bundled,
                    origin: CatalogOrigin::Bundled,
                }),
        );
        entries
    }

    fn entry_from_descriptor(
        descriptor: SkillDescriptor,
        nested: bool,
        working_directory: Option<&LocalOrRemotePath>,
        repo_root: Option<&LocalOrRemotePath>,
        home: Option<&Path>,
        kind: CatalogKind,
    ) -> CatalogSkill {
        let path = match &descriptor.reference {
            SkillReference::Path(path) => Some(path),
            SkillReference::BundledSkillId(_) => None,
        };
        let display_name = invocation_name_for_descriptor(&descriptor);
        let owner = path.and_then(|path| extract_skill_parent_directory(path).ok());
        let (group, origin, status) = match descriptor.scope {
            SkillScope::Home => (
                personal_group(),
                CatalogOrigin::Personal,
                CatalogStatus::Active,
            ),
            SkillScope::Project => {
                let owner = owner.unwrap_or_else(|| {
                    working_directory
                        .cloned()
                        .unwrap_or_else(|| LocalOrRemotePath::Local(PathBuf::new()))
                });
                let status = if nested {
                    CatalogStatus::Contextual {
                        invocation: qualified_claude_name(&owner, working_directory, &display_name),
                    }
                } else {
                    CatalogStatus::Active
                };
                (
                    project_group(&owner, nested, working_directory, repo_root),
                    CatalogOrigin::Project { owner, nested },
                    status,
                )
            }
            SkillScope::Bundled => (
                system_group("Bundled", 700),
                CatalogOrigin::Bundled,
                CatalogStatus::Active,
            ),
        };
        let source_label = path
            .map(|path| compact_location(path, home, repo_root))
            .unwrap_or_else(|| "bundled".to_string());
        CatalogSkill {
            display_name,
            descriptor,
            group,
            source_label,
            status,
            kind,
            origin,
        }
    }

    fn invocation_name_for_descriptor(descriptor: &SkillDescriptor) -> String {
        if descriptor.provider == SkillProvider::Claude {
            if let SkillReference::Path(path) = &descriptor.reference {
                if let Some(name) = path
                    .parent()
                    .and_then(|parent| parent.file_name().map(str::to_owned))
                {
                    return name;
                }
            }
        }
        descriptor.name.clone()
    }

    fn personal_group() -> CatalogGroup {
        CatalogGroup {
            id: "personal".to_string(),
            label: "Personal".to_string(),
            order: 100,
        }
    }

    fn project_group(
        owner: &LocalOrRemotePath,
        nested: bool,
        working_directory: Option<&LocalOrRemotePath>,
        repo_root: Option<&LocalOrRemotePath>,
    ) -> CatalogGroup {
        let relative = relative_location(owner, repo_root).unwrap_or_else(|| owner.display_path());
        let location_label = if relative.is_empty() || relative == "." {
            "repo root".to_string()
        } else {
            relative
        };
        let distance = working_directory
            .and_then(|cwd| path_distance(cwd, owner))
            .unwrap_or_default();
        let (prefix, order) = if nested {
            ("Contextual", 400 + distance)
        } else {
            ("Project", 200 + distance)
        };
        CatalogGroup {
            id: format!("{}:{}", prefix.to_lowercase(), owner.display_path()),
            label: format!("{prefix} · {location_label}"),
            order,
        }
    }

    fn plugin_group(enabled: bool, agent: &str, scope: &str) -> CatalogGroup {
        CatalogGroup {
            id: format!(
                "{}-plugins-{}-{}",
                agent.to_lowercase(),
                scope.to_lowercase().replace(' ', "-"),
                enabled
            ),
            label: if enabled {
                format!("{scope} · {agent} plugins")
            } else {
                format!("Disabled · {scope} {agent} plugins")
            },
            order: if enabled { 450 } else { 800 },
        }
    }

    fn system_group(label: &str, order: usize) -> CatalogGroup {
        CatalogGroup {
            id: label.to_lowercase().replace(' ', "-"),
            label: label.to_string(),
            order,
        }
    }

    fn compact_location(
        location: &LocalOrRemotePath,
        home: Option<&Path>,
        repo_root: Option<&LocalOrRemotePath>,
    ) -> String {
        let Some(path) = location.to_local_path() else {
            return location.display_path();
        };
        if let Some(home) = home {
            if let Ok(relative) = path.strip_prefix(home) {
                return format!("~/{}", relative.display());
            }
        }
        if let Some(LocalOrRemotePath::Local(repo_root)) = repo_root {
            if let Ok(relative) = path.strip_prefix(repo_root) {
                return if relative.as_os_str().is_empty() {
                    ".".to_string()
                } else {
                    relative.display().to_string()
                };
            }
        }
        path.display().to_string()
    }

    fn relative_location(
        location: &LocalOrRemotePath,
        root: Option<&LocalOrRemotePath>,
    ) -> Option<String> {
        let root = root?;
        match (location, root) {
            (LocalOrRemotePath::Local(path), LocalOrRemotePath::Local(root)) => path
                .strip_prefix(root)
                .ok()
                .map(|path| path.display().to_string()),
            (LocalOrRemotePath::Remote(path), LocalOrRemotePath::Remote(root))
                if path.host_id == root.host_id =>
            {
                path.path
                    .to_local_path_lossy()
                    .strip_prefix(root.path.to_local_path_lossy())
                    .ok()
                    .map(|path| path.display().to_string())
            }
            _ => None,
        }
    }

    fn path_distance(from: &LocalOrRemotePath, to: &LocalOrRemotePath) -> Option<usize> {
        if from == to {
            return Some(0);
        }
        let mut current = Some(from.clone());
        let mut distance = 0;
        while let Some(path) = current {
            if &path == to {
                return Some(distance);
            }
            current = path.parent();
            distance += 1;
        }
        None
    }

    fn qualified_claude_name(
        owner: &LocalOrRemotePath,
        working_directory: Option<&LocalOrRemotePath>,
        name: &str,
    ) -> String {
        let relative = relative_location(owner, working_directory).unwrap_or_default();
        if relative.is_empty() {
            name.to_string()
        } else {
            format!("{}:{name}", relative.replace('\\', "/"))
        }
    }

    fn ancestor_chain<'a>(cwd: &'a Path, root: Option<&'a Path>) -> Vec<&'a Path> {
        let root = root.unwrap_or(cwd);
        let mut out = Vec::new();
        let mut current = Some(cwd);
        while let Some(path) = current {
            if !path.starts_with(root) {
                break;
            }
            out.push(path);
            if path == root {
                break;
            }
            current = path.parent();
        }
        out
    }

    fn discover_local_claude_project_skills(
        scan_root: &Path,
        working_directory: &LocalOrRemotePath,
        repo_root: Option<&LocalOrRemotePath>,
        home: &Path,
    ) -> Vec<CatalogSkill> {
        let mut entries = Vec::new();
        let mut walker = WalkDir::new(scan_root).follow_links(false).into_iter();
        while let Some(entry) = walker.next() {
            let Ok(entry) = entry else {
                continue;
            };
            if should_skip_nested_skill_scan_entry(&entry) {
                if entry.file_type().is_dir() {
                    walker.skip_current_dir();
                }
                continue;
            }
            if !entry.file_type().is_dir() || !entry.path().ends_with(".claude/skills") {
                continue;
            }

            for skill_path in skill_files_in_component_path(entry.path()) {
                let Some(descriptor) = parse_markdown_descriptor(
                    &skill_path,
                    None,
                    SkillProvider::Claude,
                    SkillScope::Project,
                ) else {
                    continue;
                };
                let owner = skill_path
                    .parent()
                    .and_then(Path::parent)
                    .and_then(Path::parent)
                    .and_then(Path::parent);
                let nested = owner.is_some_and(|owner| owner != scan_root);
                entries.push(entry_from_descriptor(
                    descriptor,
                    nested,
                    Some(working_directory),
                    repo_root,
                    Some(home),
                    CatalogKind::Skill,
                ));
            }
            // A provider root's direct children are the skill directories. Supporting files
            // within those skills cannot define additional skills, so avoid walking them.
            walker.skip_current_dir();
        }
        entries
    }

    fn should_skip_nested_skill_scan_entry(entry: &DirEntry) -> bool {
        if entry.depth() == 0 || !entry.file_type().is_dir() {
            return false;
        }
        let name = entry.file_name().to_string_lossy();
        if matches!(
            name.as_ref(),
            ".git"
                | "target"
                | "node_modules"
                | "vendor"
                | "dist"
                | "build"
                | ".next"
                | ".cache"
                | ".venv"
        ) {
            return true;
        }
        if name.starts_with('.') && name != ".claude" {
            return true;
        }
        entry
            .path()
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .is_some_and(|parent| parent == ".claude" && name != "skills")
    }

    fn parse_markdown_descriptor(
        path: &Path,
        name_override: Option<String>,
        provider: SkillProvider,
        scope: SkillScope,
    ) -> Option<SkillDescriptor> {
        let content = fs::read_to_string(path).ok()?;
        let mut parsed = parse_skill_content_at_location(
            LocalOrRemotePath::Local(path.to_path_buf()),
            &content,
            provider,
            scope,
        )
        .ok()?;
        if let Some(name) = name_override {
            parsed.name = name;
        }
        Some(parsed.into())
    }

    fn discover_commands(
        root: &Path,
        scope: SkillScope,
        origin: CatalogOrigin,
        group: CatalogGroup,
        home: &Path,
        repo_root: Option<&Path>,
    ) -> Vec<CatalogSkill> {
        if !root.is_dir() {
            return Vec::new();
        }
        WalkDir::new(root)
            .follow_links(true)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("md"))
            .filter_map(|entry| {
                let relative = entry.path().strip_prefix(root).ok()?;
                let mut command = relative.with_extension("").display().to_string();
                command = command.replace(['/', '\\'], ":");
                let descriptor = parse_markdown_descriptor(
                    entry.path(),
                    Some(command.clone()),
                    SkillProvider::Claude,
                    scope,
                )?;
                Some(CatalogSkill {
                    display_name: command,
                    source_label: compact_local_path(entry.path(), home, repo_root),
                    descriptor,
                    group: group.clone(),
                    status: CatalogStatus::Active,
                    kind: CatalogKind::Command,
                    origin: origin.clone(),
                })
            })
            .collect()
    }

    fn compact_local_path(path: &Path, home: &Path, repo_root: Option<&Path>) -> String {
        let repo_root = repo_root.map(|root| LocalOrRemotePath::Local(root.to_path_buf()));
        compact_location(
            &LocalOrRemotePath::Local(path.to_path_buf()),
            Some(home),
            repo_root.as_ref(),
        )
    }

    fn claude_managed_roots() -> Vec<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            vec![PathBuf::from("/Library/Application Support/ClaudeCode")]
        }
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            vec![PathBuf::from("/etc/claude-code")]
        }
        #[cfg(target_os = "windows")]
        {
            vec![PathBuf::from(r"C:\Program Files\ClaudeCode")]
        }
        #[cfg(not(any(
            target_os = "macos",
            target_os = "linux",
            target_os = "android",
            target_os = "windows"
        )))]
        {
            Vec::new()
        }
    }

    fn claude_managed_settings_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();
        for root in claude_managed_roots() {
            paths.push(root.join("managed-settings.json"));
            let mut drop_ins: Vec<_> = fs::read_dir(root.join("managed-settings.d"))
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.extension().and_then(|extension| extension.to_str()) == Some("json")
                        && path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| !name.starts_with('.'))
                })
                .collect();
            drop_ins.sort();
            paths.extend(drop_ins);
        }
        paths
    }

    fn read_claude_settings(home: &Path, repo_root: Option<&Path>) -> ClaudeSettings {
        let mut settings = ClaudeSettings::default();
        let mut paths = vec![(home.join(".claude/settings.json"), false)];
        if let Some(repo_root) = repo_root {
            paths.push((repo_root.join(".claude/settings.json"), false));
            paths.push((repo_root.join(".claude/settings.local.json"), false));
        }
        paths.extend(
            claude_managed_settings_paths()
                .into_iter()
                .map(|path| (path, true)),
        );
        let mut env_disable_bundled = std::env::var("CLAUDE_CODE_DISABLE_BUNDLED_SKILLS")
            .ok()
            .is_some_and(|value| truthy_setting(&value));
        let mut env_disable_doctor = std::env::var("DISABLE_DOCTOR_COMMAND")
            .ok()
            .is_some_and(|value| truthy_setting(&value));

        for (path, is_managed) in paths {
            let Ok(content) = fs::read_to_string(path) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<JsonValue>(&content) else {
                continue;
            };
            if let Some(plugins) = value.get("enabledPlugins").and_then(JsonValue::as_object) {
                for (name, enabled) in plugins {
                    if let Some(enabled) = enabled.as_bool() {
                        settings.enabled_plugins.insert(name.clone(), enabled);
                    }
                }
            }
            if let Some(overrides) = value.get("skillOverrides").and_then(JsonValue::as_object) {
                for (name, visibility) in overrides {
                    if let Some(visibility) = visibility.as_str() {
                        settings
                            .skill_overrides
                            .insert(name.clone(), visibility.to_string());
                    }
                }
            }
            if let Some(disabled) = value
                .get("disableBundledSkills")
                .and_then(JsonValue::as_bool)
            {
                settings.disable_bundled_skills = disabled;
            }
            if let Some(env) = value.get("env").and_then(JsonValue::as_object) {
                if let Some(value) = env
                    .get("CLAUDE_CODE_DISABLE_BUNDLED_SKILLS")
                    .and_then(JsonValue::as_str)
                {
                    env_disable_bundled = truthy_setting(value);
                }
                if let Some(value) = env
                    .get("DISABLE_DOCTOR_COMMAND")
                    .and_then(JsonValue::as_str)
                {
                    env_disable_doctor = truthy_setting(value);
                }
            }
            if is_managed {
                if let Some(value) = value.get("strictPluginOnlyCustomization") {
                    settings.strict_plugin_only_skills = customization_locks_skills(value);
                }
            }
        }
        settings.disable_bundled_skills |= env_disable_bundled;
        settings.disable_doctor = env_disable_doctor;
        settings
    }

    fn truthy_setting(value: &str) -> bool {
        matches!(
            value.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    }

    fn customization_locks_skills(value: &JsonValue) -> bool {
        value.as_bool() == Some(true)
            || value
                .as_array()
                .is_some_and(|surfaces| surfaces.iter().any(|surface| surface == "skills"))
    }

    fn discover_claude_plugins(
        home: &Path,
        settings: &ClaudeSettings,
        repo_root: Option<&Path>,
    ) -> Vec<CatalogSkill> {
        let registry_path = home.join(".claude/plugins/installed_plugins.json");
        let Ok(content) = fs::read_to_string(registry_path) else {
            return Vec::new();
        };
        let Ok(registry) = serde_json::from_str::<JsonValue>(&content) else {
            return Vec::new();
        };
        let Some(plugins) = registry.get("plugins").and_then(JsonValue::as_object) else {
            return Vec::new();
        };

        let mut out = Vec::new();
        for (key, installs) in plugins {
            let Some(install) = installs.as_array().and_then(|installs| {
                installs
                    .iter()
                    .filter_map(|install| {
                        claude_plugin_install_rank(install, repo_root).map(|rank| (install, rank))
                    })
                    .max_by(|(a, a_rank), (b, b_rank)| {
                        a_rank.cmp(b_rank).then_with(|| {
                            a.get("lastUpdated")
                                .and_then(JsonValue::as_str)
                                .unwrap_or_default()
                                .cmp(
                                    b.get("lastUpdated")
                                        .and_then(JsonValue::as_str)
                                        .unwrap_or_default(),
                                )
                        })
                    })
                    .map(|(install, _)| install)
            }) else {
                continue;
            };
            let Some(root) = install
                .get("installPath")
                .and_then(JsonValue::as_str)
                .map(PathBuf::from)
            else {
                continue;
            };
            let (scope, scope_label) = match install.get("scope").and_then(JsonValue::as_str) {
                Some("user") | None => (SkillScope::Home, "Personal"),
                Some("local") => (SkillScope::Project, "Local project"),
                Some(_) => (SkillScope::Project, "Project"),
            };
            let enabled = settings.enabled_plugins.get(key).copied().unwrap_or(true);
            let plugin_name = key.split('@').next().unwrap_or(key);
            out.extend(discover_plugin_components(
                &root,
                plugin_name,
                key,
                enabled,
                scope,
                SkillProvider::Claude,
                home,
                repo_root,
                plugin_group(enabled, "Claude", scope_label),
            ));
        }
        out
    }

    fn claude_plugin_install_rank(install: &JsonValue, repo_root: Option<&Path>) -> Option<u8> {
        match install.get("scope").and_then(JsonValue::as_str) {
            Some("user") | None => Some(1),
            Some("project") | Some("local") => {
                let repo_root = repo_root?;
                let project_path = install
                    .get("projectPath")
                    .and_then(JsonValue::as_str)
                    .map(Path::new);
                if project_path
                    .is_some_and(|path| normalized_path(path) != normalized_path(repo_root))
                {
                    return None;
                }
                Some(
                    if install.get("scope").and_then(JsonValue::as_str) == Some("local") {
                        3
                    } else {
                        2
                    },
                )
            }
            Some(_) => None,
        }
    }

    fn discover_skills_directory_plugins(
        skills_root: &Path,
        scope: SkillScope,
        home: &Path,
        repo_root: Option<&Path>,
        settings: &ClaudeSettings,
    ) -> Vec<CatalogSkill> {
        let Ok(entries) = fs::read_dir(skills_root) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.join(".claude-plugin/plugin.json").is_file())
            .flat_map(|root| {
                let manifest = read_json(&root.join(".claude-plugin/plugin.json"));
                let plugin_name = manifest
                    .as_ref()
                    .and_then(|value| value.get("name"))
                    .and_then(JsonValue::as_str)
                    .map(str::to_owned)
                    .or_else(|| root.file_name()?.to_str().map(str::to_owned))
                    .unwrap_or_else(|| "plugin".to_string());
                let key = format!("{plugin_name}@skills-dir");
                let enabled = settings.enabled_plugins.get(&key).copied().unwrap_or(true);
                discover_plugin_components(
                    &root,
                    &plugin_name,
                    &key,
                    enabled,
                    scope,
                    SkillProvider::Claude,
                    home,
                    repo_root,
                    plugin_group(
                        enabled,
                        "Claude",
                        if scope == SkillScope::Home {
                            "Personal"
                        } else {
                            "Project"
                        },
                    ),
                )
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn discover_plugin_components(
        root: &Path,
        plugin_name: &str,
        source_name: &str,
        enabled: bool,
        scope: SkillScope,
        provider: SkillProvider,
        home: &Path,
        repo_root: Option<&Path>,
        group: CatalogGroup,
    ) -> Vec<CatalogSkill> {
        let manifest = read_json(&root.join(if provider == SkillProvider::Claude {
            ".claude-plugin/plugin.json"
        } else {
            ".codex-plugin/plugin.json"
        }));
        let mut skill_roots = vec![root.join("skills")];
        if let Some(manifest) = &manifest {
            skill_roots.extend(json_component_paths(manifest.get("skills"), root));
        }
        let mut command_roots = manifest
            .as_ref()
            .and_then(|manifest| manifest.get("commands"))
            .map(|value| json_component_paths(Some(value), root))
            .unwrap_or_else(|| vec![root.join("commands")]);

        let origin = CatalogOrigin::Plugin {
            name: source_name.to_string(),
        };
        let default_status = if enabled {
            CatalogStatus::Active
        } else {
            CatalogStatus::Disabled {
                reason: "plugin is turned off".to_string(),
            }
        };
        let mut out = Vec::new();
        let mut saw_skill = false;
        let mut seen_paths = HashSet::new();
        for skill_root in skill_roots.drain(..) {
            for skill_path in skill_files_in_component_path(&skill_root) {
                if !seen_paths.insert(skill_path.clone()) {
                    continue;
                }
                let Some(descriptor) =
                    parse_markdown_descriptor(&skill_path, None, provider, scope)
                else {
                    continue;
                };
                let skill_name = if skill_path == root.join("SKILL.md") {
                    descriptor.name.clone()
                } else {
                    skill_path
                        .parent()
                        .and_then(Path::file_name)
                        .and_then(|name| name.to_str())
                        .map(str::to_owned)
                        .unwrap_or_else(|| descriptor.name.clone())
                };
                saw_skill = true;
                out.push(CatalogSkill {
                    display_name: format!("{plugin_name}:{skill_name}"),
                    source_label: format!(
                        "{} · {}",
                        source_name,
                        compact_local_path(&skill_path, home, repo_root)
                    ),
                    descriptor,
                    group: group.clone(),
                    status: default_status.clone(),
                    kind: CatalogKind::PluginSkill,
                    origin: origin.clone(),
                });
            }
        }
        if !saw_skill && root.join("SKILL.md").is_file() {
            if let Some(descriptor) =
                parse_markdown_descriptor(&root.join("SKILL.md"), None, provider, scope)
            {
                out.push(CatalogSkill {
                    display_name: format!("{plugin_name}:{}", descriptor.name),
                    source_label: format!(
                        "{} · {}",
                        source_name,
                        compact_local_path(&root.join("SKILL.md"), home, repo_root)
                    ),
                    descriptor,
                    group: group.clone(),
                    status: default_status.clone(),
                    kind: CatalogKind::PluginSkill,
                    origin: origin.clone(),
                });
            }
        }

        for command_root in command_roots.drain(..) {
            let command_files: Vec<_> = if command_root.is_file() {
                vec![command_root.clone()]
            } else {
                WalkDir::new(&command_root)
                    .follow_links(true)
                    .into_iter()
                    .filter_map(Result::ok)
                    .filter(|entry| entry.file_type().is_file())
                    .filter(|entry| {
                        entry.path().extension().and_then(|ext| ext.to_str()) == Some("md")
                    })
                    .map(|entry| entry.into_path())
                    .collect()
            };
            for command_path in command_files {
                if !seen_paths.insert(command_path.clone()) {
                    continue;
                }
                let command_name = command_path
                    .strip_prefix(&command_root)
                    .ok()
                    .filter(|relative| !relative.as_os_str().is_empty())
                    .map(|relative| {
                        relative
                            .with_extension("")
                            .display()
                            .to_string()
                            .replace(['/', '\\'], ":")
                    })
                    .or_else(|| {
                        command_path
                            .file_stem()
                            .and_then(|name| name.to_str())
                            .map(str::to_owned)
                    });
                let Some(command_name) = command_name else {
                    continue;
                };
                let Some(descriptor) = parse_markdown_descriptor(
                    &command_path,
                    Some(command_name.clone()),
                    provider,
                    scope,
                ) else {
                    continue;
                };
                out.push(CatalogSkill {
                    display_name: format!("{plugin_name}:{command_name}"),
                    source_label: format!(
                        "{} · {}",
                        source_name,
                        compact_local_path(&command_path, home, repo_root)
                    ),
                    descriptor,
                    group: group.clone(),
                    status: default_status.clone(),
                    kind: CatalogKind::PluginCommand,
                    origin: origin.clone(),
                });
            }
        }
        out
    }

    fn json_component_paths(value: Option<&JsonValue>, root: &Path) -> Vec<PathBuf> {
        let values: Vec<&str> = match value {
            Some(JsonValue::String(path)) => vec![path],
            Some(JsonValue::Array(paths)) => paths.iter().filter_map(JsonValue::as_str).collect(),
            _ => Vec::new(),
        };
        values
            .into_iter()
            .filter_map(|path| safe_plugin_path(root, path))
            .collect()
    }

    fn safe_plugin_path(root: &Path, relative: &str) -> Option<PathBuf> {
        let relative = Path::new(relative);
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
        {
            return None;
        }
        Some(root.join(relative))
    }

    fn skill_files_in_component_path(path: &Path) -> Vec<PathBuf> {
        if path.is_file() && path.file_name().and_then(|name| name.to_str()) == Some("SKILL.md") {
            return vec![path.to_path_buf()];
        }
        if path.join("SKILL.md").is_file() {
            return vec![path.join("SKILL.md")];
        }
        let Ok(entries) = fs::read_dir(path) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("SKILL.md"))
            .filter(|path| path.is_file())
            .collect()
    }

    fn claude_bundled_entries() -> Vec<CatalogSkill> {
        CLAUDE_BUNDLED_SKILLS
            .iter()
            .map(|(name, description)| CatalogSkill {
                descriptor: SkillDescriptor {
                    reference: SkillReference::BundledSkillId(format!("claude-code:{name}")),
                    name: (*name).to_string(),
                    description: (*description).to_string(),
                    scope: SkillScope::Bundled,
                    provider: SkillProvider::Claude,
                    icon_override: None,
                },
                display_name: (*name).to_string(),
                group: system_group("Claude Code bundled", 700),
                source_label: "documented by Claude Code; availability varies by version and plan"
                    .to_string(),
                status: CatalogStatus::Active,
                kind: CatalogKind::Bundled,
                origin: CatalogOrigin::Bundled,
            })
            .collect()
    }

    fn apply_claude_precedence(entries: &mut [CatalogSkill], settings: &ClaudeSettings) {
        for entry in entries.iter_mut() {
            // Claude's `skillOverrides` setting explicitly excludes plugin skills; plugin
            // visibility is controlled through `enabledPlugins` instead.
            if !matches!(entry.origin, CatalogOrigin::Plugin { .. }) {
                let override_name = &entry.display_name;
                match settings
                    .skill_overrides
                    .get(override_name)
                    .map(String::as_str)
                {
                    Some("off") => {
                        entry.status = CatalogStatus::Disabled {
                            reason: "skillOverrides sets it to off".to_string(),
                        };
                    }
                    Some("name-only") if matches!(entry.status, CatalogStatus::Active) => {
                        entry.status = CatalogStatus::Limited {
                            reason: "name only in Claude's context".to_string(),
                        };
                    }
                    Some("user-invocable-only")
                        if matches!(entry.status, CatalogStatus::Active) =>
                    {
                        entry.status = CatalogStatus::Limited {
                            reason: "user-invocable only".to_string(),
                        };
                    }
                    _ => {}
                }
            }
            if settings.strict_plugin_only_skills
                && matches!(
                    entry.origin,
                    CatalogOrigin::Personal | CatalogOrigin::Project { .. }
                )
            {
                entry.status = CatalogStatus::Disabled {
                    reason: "enterprise policy allows only plugin and managed skills".to_string(),
                };
            }
            if settings.disable_bundled_skills && matches!(entry.origin, CatalogOrigin::Bundled) {
                if entry.descriptor.name == "doctor"
                    && !settings.disable_doctor
                    && !matches!(entry.status, CatalogStatus::Disabled { .. })
                {
                    entry.status = CatalogStatus::Limited {
                        reason: "user-invocable only while bundled skills are disabled".to_string(),
                    };
                } else {
                    entry.status = CatalogStatus::Disabled {
                        reason: "disableBundledSkills is on".to_string(),
                    };
                }
            } else if settings.disable_doctor
                && matches!(entry.origin, CatalogOrigin::Bundled)
                && entry.descriptor.name == "doctor"
            {
                entry.status = CatalogStatus::Disabled {
                    reason: "DISABLE_DOCTOR_COMMAND is on".to_string(),
                };
            }
        }

        let snapshots: Vec<_> = entries
            .iter()
            .map(|entry| {
                (
                    entry.display_name.clone(),
                    entry.display_name.clone(),
                    entry.source_label.clone(),
                    entry.kind,
                    entry.origin.clone(),
                    entry.status.clone(),
                )
            })
            .collect();
        for entry in entries.iter_mut() {
            if !catalog_status_participates_in_precedence(&entry.status) {
                continue;
            }
            let same_domain = |other_origin: &CatalogOrigin| match (&entry.origin, other_origin) {
                (CatalogOrigin::Plugin { name: a }, CatalogOrigin::Plugin { name: b }) => a == b,
                (CatalogOrigin::Plugin { .. }, _) | (_, CatalogOrigin::Plugin { .. }) => false,
                _ => true,
            };
            let shadow = snapshots.iter().find(|(_, name, _, kind, origin, status)| {
                catalog_status_participates_in_precedence(status)
                    && same_domain(origin)
                    && name == &entry.display_name
                    && claude_entry_outranks(entry.kind, &entry.origin, *kind, origin)
            });
            if let Some((display_name, _, source, _, _, _)) = shadow {
                entry.status = CatalogStatus::Shadowed {
                    by: format!("{display_name} ({source})"),
                };
            }
        }
    }

    fn catalog_status_participates_in_precedence(status: &CatalogStatus) -> bool {
        matches!(
            status,
            CatalogStatus::Active | CatalogStatus::Limited { .. }
        )
    }

    fn claude_entry_outranks(
        target_kind: CatalogKind,
        target_origin: &CatalogOrigin,
        source_kind: CatalogKind,
        source_origin: &CatalogOrigin,
    ) -> bool {
        // Skills always beat legacy commands with the same invocation name, even when the
        // command lives at a nominally higher filesystem scope.
        if target_kind.is_skill() != source_kind.is_skill() {
            return !target_kind.is_skill() && source_kind.is_skill();
        }
        claude_origin_rank(source_origin) < claude_origin_rank(target_origin)
    }

    fn claude_origin_rank(origin: &CatalogOrigin) -> usize {
        match origin {
            CatalogOrigin::Enterprise => 0,
            CatalogOrigin::Personal => 1,
            CatalogOrigin::Project { .. } => 2,
            CatalogOrigin::Bundled => 3,
            CatalogOrigin::Plugin { .. } | CatalogOrigin::System | CatalogOrigin::Admin => 4,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn discover_recursive_skills(
        root: &Path,
        provider: SkillProvider,
        scope: SkillScope,
        kind: CatalogKind,
        origin: CatalogOrigin,
        group: CatalogGroup,
        home: &Path,
        repo_root: Option<&Path>,
    ) -> Vec<CatalogSkill> {
        if !root.is_dir() {
            return Vec::new();
        }
        WalkDir::new(root)
            .follow_links(true)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file() && entry.file_name() == "SKILL.md")
            .filter_map(|entry| {
                let descriptor = parse_markdown_descriptor(entry.path(), None, provider, scope)?;
                let display_name = invocation_name_for_descriptor(&descriptor);
                Some(CatalogSkill {
                    display_name,
                    source_label: compact_local_path(entry.path(), home, repo_root),
                    descriptor,
                    group: group.clone(),
                    status: CatalogStatus::Active,
                    kind,
                    origin: origin.clone(),
                })
            })
            .collect()
    }

    fn discover_codex_home_skills(home: &Path, repo_root: Option<&Path>) -> Vec<CatalogSkill> {
        let skills_root = home.join(".codex/skills");
        let mut entries = discover_recursive_skills(
            &skills_root,
            SkillProvider::Codex,
            SkillScope::Bundled,
            CatalogKind::System,
            CatalogOrigin::System,
            system_group("Codex runtime", 625),
            home,
            repo_root,
        );
        for entry in &mut entries {
            let Some(path) = entry
                .path()
                .and_then(|path| path.to_local_path().map(Path::to_path_buf))
            else {
                continue;
            };
            let relative = path.strip_prefix(&skills_root).ok();
            let is_direct_personal = path.parent().and_then(Path::parent) == Some(&skills_root)
                && path
                    .parent()
                    .and_then(Path::file_name)
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| !name.starts_with('.'));
            if is_direct_personal {
                entry.descriptor.scope = SkillScope::Home;
                entry.group = personal_group();
                entry.kind = CatalogKind::Skill;
                entry.origin = CatalogOrigin::Personal;
            } else if relative
                .and_then(|path| path.components().next())
                .is_some_and(|component| component.as_os_str() == ".system")
            {
                entry.group = system_group("Codex system", 600);
            }
        }
        entries
    }

    fn read_toml(path: &Path) -> Option<TomlValue> {
        fs::read_to_string(path)
            .ok()
            .and_then(|content| toml::from_str(&content).ok())
    }

    fn read_json(path: &Path) -> Option<JsonValue> {
        fs::read_to_string(path)
            .ok()
            .and_then(|content| serde_json::from_str(&content).ok())
    }

    fn disabled_codex_skill_paths(config: Option<&TomlValue>, home: &Path) -> HashSet<PathBuf> {
        config
            .and_then(|config| config.get("skills"))
            .and_then(|skills| skills.get("config"))
            .and_then(TomlValue::as_array)
            .into_iter()
            .flatten()
            .filter(|entry| entry.get("enabled").and_then(TomlValue::as_bool) == Some(false))
            .filter_map(|entry| entry.get("path").and_then(TomlValue::as_str))
            .map(|path| expand_home(path, home))
            .map(|path| normalized_path(&path))
            .collect()
    }

    fn expand_home(path: &str, home: &Path) -> PathBuf {
        path.strip_prefix("~/")
            .map(|path| home.join(path))
            .unwrap_or_else(|| PathBuf::from(path))
    }

    fn normalized_path(path: &Path) -> PathBuf {
        path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
    }

    fn discover_codex_plugins(
        home: &Path,
        config: Option<&TomlValue>,
        repo_root: Option<&Path>,
    ) -> Vec<CatalogSkill> {
        let Some(plugins) = config
            .and_then(|config| config.get("plugins"))
            .and_then(TomlValue::as_table)
        else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (key, value) in plugins {
            let enabled = value
                .get("enabled")
                .and_then(TomlValue::as_bool)
                .unwrap_or(true);
            let (plugin, marketplace) = key
                .split_once('@')
                .map(|(plugin, marketplace)| (plugin, Some(marketplace)))
                .unwrap_or((key.as_str(), None));
            let roots = codex_plugin_roots(home, plugin, marketplace);
            let Some(root) = roots.into_iter().max_by_key(|root| {
                fs::metadata(root)
                    .and_then(|metadata| metadata.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH)
            }) else {
                continue;
            };
            out.extend(discover_plugin_components(
                &root,
                plugin,
                key,
                enabled,
                SkillScope::Bundled,
                SkillProvider::Codex,
                home,
                repo_root,
                plugin_group(enabled, "Codex", "Personal"),
            ));
        }
        out
    }

    fn codex_plugin_roots(home: &Path, plugin: &str, marketplace: Option<&str>) -> Vec<PathBuf> {
        let mut roots = Vec::new();
        let direct = home.join(".codex/plugins").join(plugin);
        if direct.join(".codex-plugin/plugin.json").is_file() {
            roots.push(direct);
        }
        let cache = home.join(".codex/plugins/cache");
        let marketplaces: Vec<PathBuf> = marketplace
            .map(|marketplace| vec![cache.join(marketplace)])
            .unwrap_or_else(|| {
                fs::read_dir(&cache)
                    .into_iter()
                    .flatten()
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .collect()
            });
        for marketplace in marketplaces {
            let versions = marketplace.join(plugin);
            roots.extend(
                fs::read_dir(versions)
                    .into_iter()
                    .flatten()
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .filter(|path| path.join(".codex-plugin/plugin.json").is_file()),
            );
        }
        roots
    }

    #[cfg(test)]
    #[path = "catalog_tests.rs"]
    mod tests;
}

#[cfg(feature = "local_fs")]
pub fn catalog_for_subtab(
    manager: &SkillManager,
    subtab: SkillsSubtab,
    working_directory: Option<&LocalOrRemotePath>,
    app: &AppContext,
) -> Vec<CatalogSkill> {
    local::catalog_for_subtab(manager, subtab, working_directory, app)
}

#[cfg(not(feature = "local_fs"))]
pub fn catalog_for_subtab(
    manager: &SkillManager,
    _subtab: SkillsSubtab,
    working_directory: Option<&LocalOrRemotePath>,
    app: &AppContext,
) -> Vec<CatalogSkill> {
    manager
        .get_skills_for_working_directory(working_directory, app)
        .into_iter()
        .map(|descriptor| CatalogSkill {
            display_name: descriptor.name.clone(),
            source_label: descriptor.reference.to_string(),
            group: CatalogGroup {
                id: format!("{:?}", descriptor.scope),
                label: descriptor.scope.to_string(),
                order: match descriptor.scope {
                    SkillScope::Home => 100,
                    SkillScope::Project => 200,
                    SkillScope::Bundled => 700,
                },
            },
            descriptor,
            status: CatalogStatus::Active,
            kind: CatalogKind::Skill,
            origin: CatalogOrigin::System,
        })
        .collect()
}
