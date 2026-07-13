use tempfile::TempDir;

use super::*;

fn write(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

#[test]
fn discovers_enabled_claude_plugin_skills_and_commands_from_current_install() {
    let temp = TempDir::new().unwrap();
    let home = temp.path();
    let plugin = home.join("plugin-v2");
    write(
        &plugin.join("skills/review/SKILL.md"),
        "---\nname: Code review\ndescription: Review code\n---\nReview it.",
    );
    write(
        &plugin.join("commands/ship.md"),
        "---\ndescription: Ship it\n---\nShip it.",
    );
    write(
        &plugin.join(".claude-plugin/plugin.json"),
        r#"{"name":"toolkit"}"#,
    );
    write(
        &home.join(".claude/plugins/installed_plugins.json"),
        &format!(
            r#"{{"plugins":{{"toolkit@market":[{{"scope":"user","installPath":"{}","lastUpdated":"2026-01-01"}}]}}}}"#,
            plugin.display()
        ),
    );
    let settings = ClaudeSettings {
        enabled_plugins: HashMap::from([("toolkit@market".to_string(), true)]),
        ..Default::default()
    };

    let entries = discover_claude_plugins(home, &settings, None);
    let names: HashSet<_> = entries
        .iter()
        .map(|entry| entry.display_name.as_str())
        .collect();
    assert_eq!(names, HashSet::from(["toolkit:review", "toolkit:ship"]));
    assert!(entries
        .iter()
        .all(|entry| matches!(entry.status, CatalogStatus::Active)));
}

#[test]
fn disabled_claude_plugin_is_visible_and_labeled_disabled() {
    let temp = TempDir::new().unwrap();
    let home = temp.path();
    let plugin = home.join("plugin");
    write(
        &plugin.join("skills/review/SKILL.md"),
        "---\nname: review\ndescription: Review code\n---\nReview it.",
    );
    write(
        &home.join(".claude/plugins/installed_plugins.json"),
        &format!(
            r#"{{"plugins":{{"toolkit@market":[{{"scope":"user","installPath":"{}","lastUpdated":"2026-01-01"}}]}}}}"#,
            plugin.display()
        ),
    );
    let settings = ClaudeSettings {
        enabled_plugins: HashMap::from([("toolkit@market".to_string(), false)]),
        ..Default::default()
    };

    let entries = discover_claude_plugins(home, &settings, None);
    assert_eq!(entries.len(), 1);
    assert!(matches!(entries[0].status, CatalogStatus::Disabled { .. }));
    assert_eq!(entries[0].group.label, "Disabled · Personal Claude plugins");
}

#[test]
fn claude_personal_skill_shadows_project_skill_and_skill_shadows_command() {
    let personal = CatalogSkill {
        descriptor: descriptor("deploy", SkillScope::Home),
        display_name: "deploy".to_string(),
        group: personal_group(),
        source_label: "~/.claude/skills/deploy/SKILL.md".to_string(),
        status: CatalogStatus::Active,
        kind: CatalogKind::Skill,
        origin: CatalogOrigin::Personal,
    };
    let project = CatalogSkill {
        descriptor: descriptor("deploy", SkillScope::Project),
        display_name: "deploy".to_string(),
        group: system_group("Project", 200),
        source_label: ".claude/skills/deploy/SKILL.md".to_string(),
        status: CatalogStatus::Active,
        kind: CatalogKind::Skill,
        origin: CatalogOrigin::Project {
            owner: LocalOrRemotePath::Local(PathBuf::from("/repo")),
            nested: false,
        },
    };
    let command = CatalogSkill {
        descriptor: descriptor("deploy", SkillScope::Home),
        display_name: "deploy".to_string(),
        group: personal_group(),
        source_label: "~/.claude/commands/deploy.md".to_string(),
        status: CatalogStatus::Active,
        kind: CatalogKind::Command,
        origin: CatalogOrigin::Personal,
    };
    let mut entries = vec![personal, project, command];

    apply_claude_precedence(&mut entries, &ClaudeSettings::default());

    assert!(matches!(entries[0].status, CatalogStatus::Active));
    assert!(matches!(entries[1].status, CatalogStatus::Shadowed { .. }));
    assert!(matches!(entries[2].status, CatalogStatus::Shadowed { .. }));
}

#[test]
fn claude_limited_personal_skill_still_shadows_project_but_overrides_skip_plugins() {
    let personal = catalog_entry(
        "deploy",
        SkillScope::Home,
        CatalogKind::Skill,
        CatalogOrigin::Personal,
    );
    let project = catalog_entry(
        "deploy",
        SkillScope::Project,
        CatalogKind::Skill,
        CatalogOrigin::Project {
            owner: LocalOrRemotePath::Local(PathBuf::from("/repo")),
            nested: false,
        },
    );
    let plugin = catalog_entry(
        "deploy",
        SkillScope::Home,
        CatalogKind::PluginSkill,
        CatalogOrigin::Plugin {
            name: "toolkit@market".to_string(),
        },
    );
    let mut entries = vec![personal, project, plugin];
    let settings = ClaudeSettings {
        skill_overrides: HashMap::from([("deploy".to_string(), "name-only".to_string())]),
        ..Default::default()
    };

    apply_claude_precedence(&mut entries, &settings);

    assert!(matches!(entries[0].status, CatalogStatus::Limited { .. }));
    assert!(matches!(entries[1].status, CatalogStatus::Shadowed { .. }));
    assert!(matches!(entries[2].status, CatalogStatus::Active));
}

#[test]
fn claude_bundled_disable_keeps_doctor_user_invocable() {
    let mut entries: Vec<_> = claude_bundled_entries()
        .into_iter()
        .filter(|entry| matches!(entry.display_name.as_str(), "doctor" | "debug"))
        .collect();

    apply_claude_precedence(
        &mut entries,
        &ClaudeSettings {
            disable_bundled_skills: true,
            ..Default::default()
        },
    );

    let doctor = entries
        .iter()
        .find(|entry| entry.display_name == "doctor")
        .unwrap();
    let debug = entries
        .iter()
        .find(|entry| entry.display_name == "debug")
        .unwrap();
    assert!(matches!(doctor.status, CatalogStatus::Limited { .. }));
    assert!(matches!(debug.status, CatalogStatus::Disabled { .. }));
}

#[test]
fn claude_enterprise_policy_disables_personal_and_project_but_not_managed_or_plugins() {
    let mut entries = vec![
        catalog_entry(
            "personal",
            SkillScope::Home,
            CatalogKind::Skill,
            CatalogOrigin::Personal,
        ),
        catalog_entry(
            "project",
            SkillScope::Project,
            CatalogKind::Skill,
            CatalogOrigin::Project {
                owner: LocalOrRemotePath::Local(PathBuf::from("/repo")),
                nested: false,
            },
        ),
        catalog_entry(
            "managed",
            SkillScope::Bundled,
            CatalogKind::Managed,
            CatalogOrigin::Enterprise,
        ),
        catalog_entry(
            "plugin",
            SkillScope::Home,
            CatalogKind::PluginSkill,
            CatalogOrigin::Plugin {
                name: "toolkit@market".to_string(),
            },
        ),
    ];

    apply_claude_precedence(
        &mut entries,
        &ClaudeSettings {
            strict_plugin_only_skills: true,
            ..Default::default()
        },
    );

    assert!(matches!(entries[0].status, CatalogStatus::Disabled { .. }));
    assert!(matches!(entries[1].status, CatalogStatus::Disabled { .. }));
    assert!(matches!(entries[2].status, CatalogStatus::Active));
    assert!(matches!(entries[3].status, CatalogStatus::Active));
}

#[test]
fn recursively_discovers_nested_codex_system_skills() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join(".codex/skills");
    write(
        &root.join(".system/review/SKILL.md"),
        "---\nname: review\ndescription: Review code\n---\nReview it.",
    );

    let entries = discover_recursive_skills(
        &root,
        SkillProvider::Codex,
        SkillScope::Bundled,
        CatalogKind::System,
        CatalogOrigin::System,
        system_group("Codex system", 600),
        temp.path(),
        None,
    );

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].display_name, "review");
    assert_eq!(entries[0].descriptor.provider, SkillProvider::Codex);
}

#[test]
fn codex_home_catalog_separates_personal_system_and_runtime_skills() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join(".codex/skills");
    for (path, name) in [
        ("personal/SKILL.md", "personal"),
        (".system/review/SKILL.md", "review"),
        ("codex-runtime/slides/SKILL.md", "slides"),
    ] {
        write(
            &root.join(path),
            &format!("---\nname: {name}\ndescription: {name} skill\n---\nUse it."),
        );
    }

    let entries = discover_codex_home_skills(temp.path(), None);
    let by_name: HashMap<_, _> = entries
        .iter()
        .map(|entry| (entry.display_name.as_str(), entry))
        .collect();

    assert_eq!(by_name["personal"].group.label, "Personal");
    assert_eq!(by_name["personal"].descriptor.scope, SkillScope::Home);
    assert_eq!(by_name["review"].group.label, "Codex system");
    assert_eq!(by_name["slides"].group.label, "Codex runtime");
}

#[test]
fn discovers_nested_claude_project_skills_before_entering_the_subdirectory() {
    let temp = TempDir::new().unwrap();
    write(
        &temp.path().join(".claude/skills/root/SKILL.md"),
        "---\nname: root\ndescription: Root skill\n---\nUse it.",
    );
    write(
        &temp
            .path()
            .join("packages/web/.claude/skills/deploy/SKILL.md"),
        "---\nname: Pretty deploy\ndescription: Deploy web\n---\nDeploy it.",
    );
    write(
        &temp
            .path()
            .join("target/generated/.claude/skills/ignored/SKILL.md"),
        "---\nname: ignored\ndescription: Ignore\n---\nIgnore it.",
    );
    let cwd = LocalOrRemotePath::Local(temp.path().to_path_buf());

    let entries = discover_local_claude_project_skills(temp.path(), &cwd, Some(&cwd), temp.path());

    assert_eq!(entries.len(), 2);
    assert!(entries.iter().any(|entry| {
        entry.display_name == "root" && matches!(entry.status, CatalogStatus::Active)
    }));
    assert!(entries.iter().any(|entry| {
        entry.display_name == "deploy"
            && matches!(
                &entry.status,
                CatalogStatus::Contextual { invocation }
                    if invocation == "packages/web:deploy"
            )
    }));
}

#[test]
fn codex_disabled_paths_are_read_from_config() {
    let temp = TempDir::new().unwrap();
    let home = temp.path();
    let path = home.join(".codex/skills/.system/review/SKILL.md");
    let config: TomlValue = toml::from_str(&format!(
        "[[skills.config]]\npath = \"{}\"\nenabled = false\n",
        path.display()
    ))
    .unwrap();

    assert_eq!(
        disabled_codex_skill_paths(Some(&config), home),
        HashSet::from([path])
    );
}

fn descriptor(name: &str, scope: SkillScope) -> SkillDescriptor {
    SkillDescriptor {
        reference: SkillReference::Path(LocalOrRemotePath::Local(PathBuf::from(format!(
            "/{name}/SKILL.md"
        )))),
        name: name.to_string(),
        description: String::new(),
        scope,
        provider: SkillProvider::Claude,
        icon_override: None,
    }
}

fn catalog_entry(
    name: &str,
    scope: SkillScope,
    kind: CatalogKind,
    origin: CatalogOrigin,
) -> CatalogSkill {
    CatalogSkill {
        descriptor: descriptor(name, scope),
        display_name: name.to_string(),
        group: system_group("Test", 0),
        source_label: format!("/{name}/SKILL.md"),
        status: CatalogStatus::Active,
        kind,
        origin,
    }
}
