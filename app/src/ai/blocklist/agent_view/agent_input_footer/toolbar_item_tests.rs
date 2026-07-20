use super::*;

#[test]
fn terminal_default_left_contains_exact_quick_actions() {
    assert_eq!(
        AgentToolbarItemKind::terminal_default_left(),
        vec![
            AgentToolbarItemKind::CustomInsert {
                label: "Claude".to_owned(),
                text: "claude --dangerously-skip-permissions".to_owned(),
            },
            AgentToolbarItemKind::CustomInsert {
                label: "Codex".to_owned(),
                text: "codex --dangerously-bypass-approvals-and-sandbox".to_owned(),
            },
            AgentToolbarItemKind::CustomInsert {
                label: "Claude resume".to_owned(),
                text: "claude --dangerously-skip-permissions --resume".to_owned(),
            },
            AgentToolbarItemKind::CustomInsert {
                label: "Codex resume".to_owned(),
                text: "codex resume".to_owned(),
            },
            AgentToolbarItemKind::CustomInsert {
                label: "Open".to_owned(),
                text: "open .".to_owned(),
            },
            AgentToolbarItemKind::CustomInsert {
                label: "Commit & Push".to_owned(),
                text: "git add -A && printf 'Commit message [Update changes]: ' && IFS= read -r clinch_commit_message && git commit -m \"${clinch_commit_message:-Update changes}\" && git push".to_owned(),
            },
            AgentToolbarItemKind::CustomInsert {
                label: "Commit".to_owned(),
                text: "git add -A && printf 'Commit message [Update changes]: ' && IFS= read -r clinch_commit_message && git commit -m \"${clinch_commit_message:-Update changes}\"".to_owned(),
            },
        ]
    );
    assert!(AgentToolbarItemKind::terminal_default_right().is_empty());
}

#[test]
fn terminal_availability_admits_only_custom_insert() {
    let custom = AgentToolbarItemKind::CustomInsert {
        label: "Build".to_owned(),
        text: "cargo build".to_owned(),
    };
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
        &items[5..13],
        &[
            AgentToolbarItemKind::CustomInsert {
                label: "/codex".to_owned(),
                text: "/codex".to_owned(),
            },
            AgentToolbarItemKind::CustomInsert {
                label: "Make No Mistakes".to_owned(),
                text: "Do it all for me. I'm stepping away. Don't make any mistakes.".to_owned(),
            },
            AgentToolbarItemKind::CustomInsert {
                label: "Create a PR".to_owned(),
                text: "Create a PR, then merge main into this PR".to_owned(),
            },
            AgentToolbarItemKind::CustomInsert {
                label: "Worktree-Build".to_owned(),
                text: "OK go into an isolated work tree. Plan this out, then implement it and create a pull request.".to_owned(),
            },
            AgentToolbarItemKind::CustomInsert {
                label: "Review w/ Codex Sol Max".to_owned(),
                text: "Review w/ Codex Sol Max".to_owned(),
            },
            AgentToolbarItemKind::CustomInsert {
                label: "Review w/ Claude Code Fable".to_owned(),
                text: "Review w/ Claude Code Fable".to_owned(),
            },
            AgentToolbarItemKind::CustomInsert {
                label: "Debug w/ Ultracode".to_owned(),
                text: "Investigate with Ultra Code and use subagents".to_owned(),
            },
            AgentToolbarItemKind::CustomInsert {
                label: "Git Worktree".to_owned(),
                text: "Move our current work and code into an isolated git work tree. And create a branch. Work out of the git worktree".to_owned(),
            },
        ]
    );
    assert_eq!(
        items.last(),
        Some(&AgentToolbarItemKind::CustomInsert {
            label: "Push2Main".to_owned(),
            text: "Push all these changes to main.".to_owned(),
        })
    );
    assert!(!items.iter().any(|item| {
        matches!(
            item,
            AgentToolbarItemKind::CustomInsert { label, .. } if label == "/codex-build"
        )
    }));
}

#[test]
fn cli_default_places_remote_control_at_the_end_of_the_right_side() {
    let _creating_shared_sessions = FeatureFlag::CreatingSharedSessions.override_enabled(true);
    let _remote_control = FeatureFlag::HOARemoteControl.override_enabled(true);
    let left = AgentToolbarItemKind::cli_default_left();
    let right = AgentToolbarItemKind::cli_default_right();

    assert!(!left.contains(&AgentToolbarItemKind::ShareSession));
    assert_eq!(right.last(), Some(&AgentToolbarItemKind::ShareSession));
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
    let item = AgentToolbarItemKind::CustomInsert {
        label: "Ship it".to_string(),
        text: "/deploy".to_string(),
    };
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
    };
    let json = serde_json::to_string(&item).unwrap();
    let back: AgentToolbarItemKind = serde_json::from_str(&json).unwrap();
    assert_eq!(item, back);
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
