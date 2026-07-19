use super::*;
use repo_metadata::repositories::DetectedRepositories;
use warpui::App;

fn conversation(agent: &str) -> AgentConversation {
    conversation_in("session-123", agent, Some("/tmp/project"))
}

fn conversation_in(id: &str, agent: &str, cwd: Option<&str>) -> AgentConversation {
    AgentConversation {
        agent: agent.to_string(),
        session_id: id.to_string(),
        cwd: cwd.map(str::to_string),
        bridge: None,
        start_ts: "2026-07-13T12:00:00Z".to_string(),
        first_prompt: Some("fix the flaky test".to_string()),
        local_resumable: true,
        flags: String::new(),
    }
}

fn source_with_roots(fixtures: &[(&str, &str, Option<&str>, Option<&str>)]) -> DataSource {
    DataSource {
        conversations: fixtures
            .iter()
            .map(|(id, agent, cwd, _)| conversation_in(id, agent, *cwd))
            .collect(),
        roots_by_conversation: fixtures
            .iter()
            .map(|(_, _, _, root)| root.map(PathBuf::from))
            .collect(),
        ..Default::default()
    }
}

fn matching_ids(source: &DataSource) -> Vec<&str> {
    source
        .matching_conversations("")
        .into_iter()
        .map(|(conversation, _, _)| conversation.session_id.as_str())
        .collect()
}

#[test]
fn searchable_text_includes_provider_name() {
    let codex = searchable_text(&conversation("codex"));
    assert!(codex.contains("codex"));
    assert!(codex.contains("Codex"));

    let claude = searchable_text(&conversation("claude"));
    assert!(claude.contains("claude"));
    assert!(claude.contains("Claude Code"));
}

#[test]
fn agent_conversation_agent_filters_keep_the_expected_subsets() {
    let mut source = source_with_roots(&[
        ("claude-new", "claude", Some("/repos/a"), Some("/repos/a")),
        ("codex", "codex", Some("/repos/a"), Some("/repos/a")),
        ("claude-old", "claude", Some("/repos/b"), Some("/repos/b")),
    ]);
    source.scope = ScopeFilter::All;

    source.agent = AgentFilter::All;
    assert_eq!(
        matching_ids(&source),
        vec!["claude-new", "codex", "claude-old"]
    );

    source.agent = AgentFilter::Claude;
    assert_eq!(matching_ids(&source), vec!["claude-new", "claude-old"]);

    source.agent = AgentFilter::Codex;
    assert_eq!(matching_ids(&source), vec!["codex"]);
}

#[test]
fn agent_conversation_this_project_scope_matches_cached_repo_roots() {
    let mut source = source_with_roots(&[
        (
            "alpha-subdir",
            "claude",
            Some("/repos/alpha/crates/app"),
            Some("/repos/alpha/"),
        ),
        ("beta", "codex", Some("/repos/beta"), Some("/repos/beta")),
        (
            "alpha-root",
            "codex",
            Some("/repos/alpha"),
            Some("/repos/alpha"),
        ),
    ]);
    source.scope = ScopeFilter::ThisProject;
    source.project_root = Some(PathBuf::from("/repos/alpha"));

    assert_eq!(matching_ids(&source), vec!["alpha-subdir", "alpha-root"]);
}

#[test]
fn agent_conversation_all_scope_keeps_every_directory() {
    let mut source = source_with_roots(&[
        (
            "alpha",
            "claude",
            Some("/repos/alpha"),
            Some("/repos/alpha"),
        ),
        ("beta", "codex", Some("/repos/beta"), Some("/repos/beta")),
        ("unknown", "claude", None, None),
    ]);
    source.scope = ScopeFilter::All;
    source.project_root = Some(PathBuf::from("/repos/alpha"));

    assert_eq!(matching_ids(&source), vec!["alpha", "beta", "unknown"]);
}

#[test]
fn agent_conversation_selected_folder_overrides_scope() {
    let mut source = source_with_roots(&[
        (
            "alpha",
            "claude",
            Some("/repos/alpha"),
            Some("/repos/alpha"),
        ),
        ("beta", "codex", Some("/repos/beta"), Some("/repos/beta")),
    ]);
    source.scope = ScopeFilter::ThisProject;
    source.project_root = Some(PathBuf::from("/repos/alpha"));
    source.selected_folder = Some(PathBuf::from("/repos/beta"));

    assert_eq!(matching_ids(&source), vec!["beta"]);
}

#[test]
fn agent_conversation_agent_and_directory_filters_intersect() {
    let mut source = source_with_roots(&[
        (
            "alpha-claude",
            "claude",
            Some("/repos/alpha"),
            Some("/repos/alpha"),
        ),
        (
            "alpha-codex",
            "codex",
            Some("/repos/alpha"),
            Some("/repos/alpha"),
        ),
        (
            "beta-codex",
            "codex",
            Some("/repos/beta"),
            Some("/repos/beta"),
        ),
    ]);
    source.scope = ScopeFilter::ThisProject;
    source.project_root = Some(PathBuf::from("/repos/alpha"));
    source.agent = AgentFilter::Codex;

    assert_eq!(matching_ids(&source), vec!["alpha-codex"]);
}

#[test]
fn agent_conversation_pool_is_filtered_before_display_cap() {
    let mut fixtures = (0..MAX_DISPLAYED)
        .map(|index| {
            (
                format!("other-{index}"),
                "claude".to_string(),
                Some("/repos/other".to_string()),
                Some("/repos/other".to_string()),
            )
        })
        .collect::<Vec<_>>();
    fixtures.extend((0..3).map(|index| {
        (
            format!("project-{index}"),
            "codex".to_string(),
            Some("/repos/project/subdir".to_string()),
            Some("/repos/project".to_string()),
        )
    }));
    let borrowed = fixtures
        .iter()
        .map(|(id, agent, cwd, root)| {
            (id.as_str(), agent.as_str(), cwd.as_deref(), root.as_deref())
        })
        .collect::<Vec<_>>();
    let mut source = source_with_roots(&borrowed);
    source.scope = ScopeFilter::ThisProject;
    source.project_root = Some(PathBuf::from("/repos/project"));

    assert_eq!(
        matching_ids(&source),
        vec!["project-0", "project-1", "project-2"]
    );
}

#[test]
fn agent_conversation_unknown_project_target_leaves_results_unfiltered() {
    let source = source_with_roots(&[
        (
            "alpha",
            "claude",
            Some("/repos/alpha"),
            Some("/repos/alpha"),
        ),
        ("beta", "codex", Some("/repos/beta"), Some("/repos/beta")),
    ]);

    assert_eq!(matching_ids(&source), vec!["alpha", "beta"]);
}

#[test]
fn agent_conversation_recent_folders_are_deduped_counted_and_newest_first() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| DetectedRepositories::default());
        let fallback_conversation =
            conversation_in("literal", "claude", Some("/outside-repos/literal"));
        let fallback_root = app.read(|ctx| conversation_root(&fallback_conversation, ctx));

        let folders = build_recent_folders(&[
            Some(PathBuf::from("/repos/alpha")),
            Some(PathBuf::from("/repos/beta")),
            Some(PathBuf::from("/repos/alpha")),
            fallback_root,
            None,
        ]);

        assert_eq!(
            folders,
            vec![
                FolderEntry {
                    root: PathBuf::from("/repos/alpha"),
                    display_name: "alpha".to_string(),
                    count: 2,
                },
                FolderEntry {
                    root: PathBuf::from("/repos/beta"),
                    display_name: "beta".to_string(),
                    count: 1,
                },
                FolderEntry {
                    root: PathBuf::from("/outside-repos/literal"),
                    display_name: "literal".to_string(),
                    count: 1,
                },
            ]
        );
    });
}
