use super::*;

#[test]
fn default_left_uses_file_explorer_without_tools_panel() {
    // The collapse (tabs) button is always leftmost; the File Explorer toggle
    // sits immediately to its right without the duplicate Tools Panel toggle.
    assert_eq!(
        HeaderToolbarItemKind::default_left(),
        vec![
            HeaderToolbarItemKind::TabsPanel,
            HeaderToolbarItemKind::FileExplorer,
            HeaderToolbarItemKind::AgentManagement,
        ],
    );
}

#[test]
fn all_items_includes_file_explorer() {
    assert!(HeaderToolbarItemKind::all_items().contains(&HeaderToolbarItemKind::FileExplorer));
}

#[test]
fn all_items_keeps_tools_panel_available_for_customization() {
    assert!(HeaderToolbarItemKind::all_items().contains(&HeaderToolbarItemKind::ToolsPanel));
}

#[test]
fn file_explorer_is_a_panel_item() {
    // It renders the shared left panel when the dedicated Tools Panel item is
    // absent from the toolbar configuration.
    assert!(HeaderToolbarItemKind::FileExplorer.is_panel());
}

#[test]
fn file_explorer_metadata() {
    assert_eq!(
        HeaderToolbarItemKind::FileExplorer.display_label(),
        "File Explorer"
    );
    assert_eq!(HeaderToolbarItemKind::FileExplorer.icon(), Icon::Folder);
}
