use super::*;

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
