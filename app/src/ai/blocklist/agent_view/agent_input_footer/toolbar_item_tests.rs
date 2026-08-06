use super::*;

#[test]
fn terminal_default_left_contains_exact_quick_actions() {
    assert_eq!(
        AgentToolbarItemKind::terminal_default_left(),
        vec![
            AgentToolbarItemKind::custom_insert(
                "Claude",
                "claude --dangerously-skip-permissions",
            ),
            AgentToolbarItemKind::custom_insert(
                "Codex",
                "codex --dangerously-bypass-approvals-and-sandbox",
            ),
            AgentToolbarItemKind::custom_insert(
                "Claude resume",
                "claude --dangerously-skip-permissions --resume",
            ),
            AgentToolbarItemKind::custom_insert("Codex resume", "codex resume"),
            AgentToolbarItemKind::custom_insert("Open", "open ."),
            AgentToolbarItemKind::custom_insert(
                "Commit & Push",
                "git add -A && printf 'Commit message [Update changes]: ' && IFS= read -r clinch_commit_message && git commit -m \"${clinch_commit_message:-Update changes}\" && git push",
            ),
            AgentToolbarItemKind::custom_insert(
                "Commit",
                "git add -A && printf 'Commit message [Update changes]: ' && IFS= read -r clinch_commit_message && git commit -m \"${clinch_commit_message:-Update changes}\"",
            ),
            AgentToolbarItemKind::custom_insert("Status", "git status --short --branch"),
        ]
    );
    assert!(AgentToolbarItemKind::terminal_default_right().is_empty());
}

#[test]
fn terminal_availability_admits_only_custom_insert() {
    let custom = AgentToolbarItemKind::custom_insert("Build", "cargo build");
    assert!(custom.is_available_for_terminal());

    let unavailable = [
        AgentToolbarItemKind::ContextChip(ContextChipKind::WorkingDirectory),
        AgentToolbarItemKind::ModelSelector,
        AgentToolbarItemKind::NLDToggle,
        AgentToolbarItemKind::ContextWindowUsage,
        AgentToolbarItemKind::FileExplorer,
        AgentToolbarItemKind::RichInput,
        AgentToolbarItemKind::VoiceInput,
        AgentToolbarItemKind::FileAttach,
        AgentToolbarItemKind::ShareSession,
        AgentToolbarItemKind::Settings,
        AgentToolbarItemKind::Compact,
        AgentToolbarItemKind::ForkSession,
        AgentToolbarItemKind::ContinuePrompt,
        AgentToolbarItemKind::LooksGoodPrompt,
        AgentToolbarItemKind::FastForwardToggle,
        AgentToolbarItemKind::HandoffToCloud,
    ];
    for item in unavailable {
        assert!(!item.is_available_for_terminal(), "{item:?}");
    }
}

/// The two CLI-agent quick-reply buttons added alongside Fork/Compact.
fn quick_reply_kinds() -> [AgentToolbarItemKind; 2] {
    [
        AgentToolbarItemKind::ContinuePrompt,
        AgentToolbarItemKind::LooksGoodPrompt,
    ]
}

#[test]
fn quick_replies_are_cli_agent_only() {
    for kind in quick_reply_kinds() {
        assert_eq!(kind.available_in(), ToolbarAvailability::CLIAgentOnly);
        assert!(kind.available_in().is_available_for_cli());
        assert!(!kind.available_in().is_available_for_agent_view());
    }
}

#[test]
fn quick_replies_are_hidden_from_session_viewers() {
    for kind in quick_reply_kinds() {
        // The host (not a viewer) sees the buttons.
        assert!(kind.available_to_session_viewer(&SharedSessionStatus::NotShared, false));
        // A shared-session viewer must not drive the host's agent.
        assert!(!kind.available_to_session_viewer(&SharedSessionStatus::reader(), false));
    }
}

#[test]
fn quick_replies_have_expected_labels_and_icons() {
    assert_eq!(
        AgentToolbarItemKind::ForkSession.display_label(),
        "Fork in New Tab"
    );
    assert_eq!(
        AgentToolbarItemKind::ContinuePrompt.display_label(),
        "Continue"
    );
    assert_eq!(
        AgentToolbarItemKind::LooksGoodPrompt.display_label(),
        "LGTM"
    );
    assert_eq!(
        AgentToolbarItemKind::ContinuePrompt.icon(),
        Some(Icon::Play)
    );
    assert_eq!(
        AgentToolbarItemKind::LooksGoodPrompt.icon(),
        Some(Icon::ThumbsUp)
    );
}

#[test]
fn quick_replies_hidden_during_handoff_compose() {
    for kind in quick_reply_kinds() {
        assert!(!kind.is_available_during_handoff_compose());
    }
}

#[test]
fn cli_default_left_places_quick_replies_right_after_fork_and_compact() {
    let items = AgentToolbarItemKind::cli_default_left();
    // The leading four are unconditional (feature flags only append later items),
    // so the quick-reply buttons deterministically sit next to Fork/Compact.
    assert_eq!(
        &items[..4],
        &[
            AgentToolbarItemKind::ForkSession,
            AgentToolbarItemKind::Compact,
            AgentToolbarItemKind::ContinuePrompt,
            AgentToolbarItemKind::LooksGoodPrompt,
        ]
    );
}

#[test]
fn agent_transfer_is_a_default_cli_host_control() {
    let items = AgentToolbarItemKind::cli_default_left();
    assert_eq!(items.get(4), Some(&AgentToolbarItemKind::TransferAgent));
    assert_eq!(
        AgentToolbarItemKind::TransferAgent.available_in(),
        ToolbarAvailability::CLIAgentOnly
    );
    assert!(!AgentToolbarItemKind::TransferAgent
        .available_to_session_viewer(&SharedSessionStatus::reader(), false));
    assert_eq!(
        AgentToolbarItemKind::TransferAgent.icon(),
        Some(Icon::SwitchHorizontal01)
    );
    assert!(AgentToolbarItemKind::all_available_for_cli_input()
        .contains(&AgentToolbarItemKind::TransferAgent));
}

#[test]
fn cli_default_left_includes_expected_quick_inserts() {
    let items = AgentToolbarItemKind::cli_default_left();
    assert_eq!(
        &items[5..17],
        &[
            AgentToolbarItemKind::custom_insert("/codex", "/codex"),
            AgentToolbarItemKind::custom_insert(
                "Make No Mistakes",
                "Do it all for me. I'm stepping away. Don't make any mistakes.",
            ),
            AgentToolbarItemKind::custom_insert("Create a Plan", "Create a Plan"),
            AgentToolbarItemKind::custom_insert("Build w/ Sub-agents", "Build w/ Sub-agents"),
            AgentToolbarItemKind::custom_insert(
                "Create a PR",
                "Create a PR, then merge main into this PR",
            ),
            AgentToolbarItemKind::custom_insert(
                "Worktree-Build",
                "OK go into an isolated work tree. Plan this out, then implement it and create a pull request.",
            ),
            AgentToolbarItemKind::custom_insert(
                "Review w/ Codex Sol Max",
                "Review w/ Codex Sol Max",
            ),
            AgentToolbarItemKind::custom_insert(
                "Review w/ Claude Code Fable",
                "Review w/ Claude Code Fable",
            ),
            AgentToolbarItemKind::custom_insert(
                "Debug w/ Ultracode",
                "Investigate with Ultra Code and use subagents",
            ),
            AgentToolbarItemKind::custom_insert(
                "Git Worktree",
                "Move our current work and code into an isolated git work tree. And create a branch. Work out of the git worktree",
            ),
            AgentToolbarItemKind::custom_insert(
                "Fix & Verify",
                "Implement the requested fix, run the most relevant checks, and summarize what changed.",
            ),
            AgentToolbarItemKind::custom_insert(
                "Simplify",
                "Simplify the current implementation without changing behavior, then run the relevant tests.",
            ),
        ]
    );
    assert_eq!(
        items.last(),
        Some(&AgentToolbarItemKind::custom_insert(
            "Push2Main",
            "Push all these changes to main.",
        ))
    );
    assert!(!items.iter().any(|item| {
        matches!(
            item,
            AgentToolbarItemKind::CustomInsert { label, .. } if label == "/codex-build"
        )
    }));
}

#[test]
fn cli_default_right_side_is_empty_even_with_remote_control_enabled() {
    let _creating_shared_sessions = FeatureFlag::CreatingSharedSessions.override_enabled(true);
    let _remote_control = FeatureFlag::HOARemoteControl.override_enabled(true);
    let left = AgentToolbarItemKind::cli_default_left();
    let right = AgentToolbarItemKind::cli_default_right();

    // The feature flags must not reintroduce a right-side item; the defaults are
    // empty regardless, and nothing migrated to the left side either.
    assert!(right.is_empty());
    assert!(!left.contains(&AgentToolbarItemKind::ShareSession));
    assert!(!left.contains(&AgentToolbarItemKind::Settings));
}

#[test]
fn cli_configurator_still_offers_the_removed_right_side_items() {
    let _creating_shared_sessions = FeatureFlag::CreatingSharedSessions.override_enabled(true);
    let _remote_control = FeatureFlag::HOARemoteControl.override_enabled(true);
    let available = AgentToolbarItemKind::all_available_for_cli_input();

    // Dropping them from the defaults must not make them unreachable.
    assert!(available.contains(&AgentToolbarItemKind::Settings));
    assert!(available.contains(&AgentToolbarItemKind::ShareSession));
    assert!(available.contains(&AgentToolbarItemKind::ContextChip(
        ContextChipKind::WorkingDirectory
    )));
    assert!(available.contains(&AgentToolbarItemKind::ContextChip(
        ContextChipKind::ShellGitBranch
    )));
}

#[test]
fn cli_input_configurator_offers_quick_replies() {
    let available = AgentToolbarItemKind::all_available_for_cli_input();
    assert!(available.contains(&AgentToolbarItemKind::ContinuePrompt));
    assert!(available.contains(&AgentToolbarItemKind::LooksGoodPrompt));
}

#[test]
fn quick_replies_absent_from_agent_view_configurator() {
    let available = AgentToolbarItemKind::all_available();
    assert!(!available.contains(&AgentToolbarItemKind::ContinuePrompt));
    assert!(!available.contains(&AgentToolbarItemKind::LooksGoodPrompt));
}

#[test]
fn custom_insert_is_cli_only_and_host_only() {
    let item = AgentToolbarItemKind::custom_insert("Ship it", "/deploy");
    assert_eq!(item.available_in(), ToolbarAvailability::CLIAgentOnly);
    assert!(!item.available_to_session_viewer(&SharedSessionStatus::reader(), false));
    assert_eq!(item.display_label(), "Ship it");
    assert_eq!(item.icon(), Some(Icon::Play));
}

#[test]
fn custom_insert_round_trips_through_serde() {
    let item = AgentToolbarItemKind::CustomInsert {
        label: "Review".to_string(),
        text: "/review".to_string(),
        auto_send: false,
    };
    let json = serde_json::to_string(&item).unwrap();
    let back: AgentToolbarItemKind = serde_json::from_str(&json).unwrap();
    assert_eq!(item, back);
    assert_eq!(item.icon(), Some(Icon::TextInput));
}

#[test]
fn custom_insert_without_auto_send_field_keeps_legacy_submit_behavior() {
    let json = r#"{"CustomInsert":{"label":"Review","text":"/review"}}"#;
    let item: AgentToolbarItemKind = serde_json::from_str(json).unwrap();
    assert_eq!(
        item,
        AgentToolbarItemKind::custom_insert("Review", "/review")
    );
}

/// Items intentionally dropped from the CLI footer default layout: the file
/// explorer moved to the header toolbar, and the `+` attach button, `±` git
/// diff-stats chip, and Rich Input chip were removed as clutter.
fn removed_cli_default_kinds() -> [AgentToolbarItemKind; 4] {
    [
        AgentToolbarItemKind::FileAttach,
        AgentToolbarItemKind::FileExplorer,
        AgentToolbarItemKind::RichInput,
        AgentToolbarItemKind::ContextChip(ContextChipKind::GitDiffStats),
    ]
}

#[test]
fn removed_items_absent_from_cli_default_left() {
    let items = AgentToolbarItemKind::cli_default_left();
    for kind in removed_cli_default_kinds() {
        assert!(
            !items.contains(&kind),
            "{kind:?} should not be in the CLI footer default layout"
        );
    }
}

#[test]
fn removed_items_still_available_in_cli_configurator() {
    let available = AgentToolbarItemKind::all_available_for_cli_input();
    for kind in removed_cli_default_kinds() {
        assert!(
            available.contains(&kind),
            "{kind:?} should remain re-addable via the CLI footer toolbar editor"
        );
    }
}
