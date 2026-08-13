use super::*;

#[test]
fn default_left_places_skills_and_conversation_finder_after_file_explorer() {
    // The collapse (tabs) button is always leftmost. The three direct-access
    // controls follow without the duplicate generic Tools Panel toggle.
    assert_eq!(
        HeaderToolbarItemKind::default_left(),
        vec![
            HeaderToolbarItemKind::TabsPanel,
            HeaderToolbarItemKind::FileExplorer,
            HeaderToolbarItemKind::Skills,
            HeaderToolbarItemKind::ConversationFinder,
            HeaderToolbarItemKind::AgentManagement,
        ],
    );
}

#[test]
fn all_items_includes_file_explorer() {
    assert!(HeaderToolbarItemKind::all_items().contains(&HeaderToolbarItemKind::FileExplorer));
}

#[test]
fn all_items_includes_skills_and_conversation_finder() {
    let items = HeaderToolbarItemKind::all_items();
    assert!(items.contains(&HeaderToolbarItemKind::Skills));
    assert!(items.contains(&HeaderToolbarItemKind::ConversationFinder));
}

#[test]
fn all_items_keeps_tools_panel_available_for_customization() {
    assert!(HeaderToolbarItemKind::all_items().contains(&HeaderToolbarItemKind::ToolsPanel));
}

#[test]
fn notifications_are_not_exposed_in_clinch_header_toolbars() {
    assert!(!HeaderToolbarItemKind::default_right()
        .contains(&HeaderToolbarItemKind::NotificationsMailbox));
    assert!(
        !HeaderToolbarItemKind::all_items().contains(&HeaderToolbarItemKind::NotificationsMailbox)
    );
}

#[test]
fn removed_imessage_status_is_never_exposed() {
    assert!(
        !HeaderToolbarItemKind::default_right().contains(&HeaderToolbarItemKind::IMessageStatus)
    );
    assert!(!HeaderToolbarItemKind::all_items().contains(&HeaderToolbarItemKind::IMessageStatus));
}

#[test]
fn removed_code_review_button_is_never_exposed() {
    assert!(!HeaderToolbarItemKind::default_right().contains(&HeaderToolbarItemKind::CodeReview));
    assert!(!HeaderToolbarItemKind::all_items().contains(&HeaderToolbarItemKind::CodeReview));
}

#[test]
fn file_explorer_is_a_panel_item() {
    // It renders the shared left panel when the dedicated Tools Panel item is
    // absent from the toolbar configuration.
    assert!(HeaderToolbarItemKind::FileExplorer.is_panel());
}

#[test]
fn skills_is_a_panel_but_conversation_finder_is_not() {
    assert!(HeaderToolbarItemKind::Skills.is_panel());
    assert!(!HeaderToolbarItemKind::ConversationFinder.is_panel());
}

#[test]
fn file_explorer_metadata() {
    assert_eq!(
        HeaderToolbarItemKind::FileExplorer.display_label(),
        "File Explorer"
    );
    assert_eq!(HeaderToolbarItemKind::FileExplorer.icon(), Icon::Folder);
}

#[test]
fn skills_and_conversation_finder_metadata() {
    assert_eq!(HeaderToolbarItemKind::Skills.display_label(), "Skills");
    assert_eq!(HeaderToolbarItemKind::Skills.icon(), Icon::Stars);
    assert_eq!(
        HeaderToolbarItemKind::ConversationFinder.display_label(),
        "Conversation Finder"
    );
    assert_eq!(
        HeaderToolbarItemKind::ConversationFinder.icon(),
        Icon::Conversation
    );
}
