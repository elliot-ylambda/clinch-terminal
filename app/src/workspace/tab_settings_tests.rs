use settings::Setting;
use warpui::{App, SingletonEntity};

use super::*;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspace::header_toolbar_item::HeaderToolbarItemKind;

#[test]
fn use_latest_user_prompt_as_conversation_title_in_tab_names_defaults_to_false() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        TabSettings::handle(&app).read(&app, |settings, _ctx| {
            assert!(!*settings.use_latest_user_prompt_as_conversation_title_in_tab_names);
        });
    });
}

#[test]
fn use_latest_user_prompt_as_conversation_title_in_tab_names_uses_vertical_tabs_path() {
    assert_eq!(
        UseLatestUserPromptAsConversationTitleInTabNames::toml_path(),
        Some("appearance.vertical_tabs.use_latest_prompt_as_title")
    );
    assert_eq!(
        UseLatestUserPromptAsConversationTitleInTabNames::hierarchy(),
        Some("appearance.vertical_tabs")
    );
    assert_eq!(
        UseLatestUserPromptAsConversationTitleInTabNames::toml_key(),
        "use_latest_prompt_as_title"
    );
}

#[test]
fn show_vertical_tab_panel_in_restored_windows_defaults_to_true() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        TabSettings::handle(&app).read(&app, |settings, _ctx| {
            assert!(*settings.show_vertical_tab_panel_in_restored_windows);
        });
    });
}

#[test]
fn show_vertical_tab_panel_in_restored_windows_uses_vertical_tabs_path() {
    assert_eq!(
        ShowVerticalTabPanelInRestoredWindows::toml_path(),
        Some("appearance.vertical_tabs.show_panel_in_restored_windows")
    );
    assert_eq!(
        ShowVerticalTabPanelInRestoredWindows::hierarchy(),
        Some("appearance.vertical_tabs")
    );
    assert_eq!(
        ShowVerticalTabPanelInRestoredWindows::toml_key(),
        "show_panel_in_restored_windows"
    );
}

#[test]
fn hide_title_bar_search_bar_in_vertical_tabs_defaults_to_false() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        TabSettings::handle(&app).read(&app, |settings, _ctx| {
            assert!(!*settings.hide_title_bar_search_bar_in_vertical_tabs);
        });
    });
}

#[test]
fn hide_title_bar_search_bar_in_vertical_tabs_uses_vertical_tabs_path() {
    assert_eq!(
        HideTitleBarSearchBarInVerticalTabs::toml_path(),
        Some("appearance.vertical_tabs.hide_title_bar_search_bar")
    );
    assert_eq!(
        HideTitleBarSearchBarInVerticalTabs::hierarchy(),
        Some("appearance.vertical_tabs")
    );
    assert_eq!(
        HideTitleBarSearchBarInVerticalTabs::toml_key(),
        "hide_title_bar_search_bar"
    );
}

#[test]
fn header_toolbar_chip_selection_default_contains_code_review() {
    let config = HeaderToolbarChipSelection::Default;
    assert!(config.contains_item(&HeaderToolbarItemKind::CodeReview));
}

#[test]
fn default_file_explorer_owns_the_shared_left_panel() {
    let config = HeaderToolbarChipSelection::Default;

    assert!(config.is_shared_left_panel_owner(&HeaderToolbarItemKind::FileExplorer));
    assert!(!config.is_shared_left_panel_owner(&HeaderToolbarItemKind::ToolsPanel));
}

#[test]
fn tools_panel_takes_shared_left_panel_ownership_when_both_items_are_configured() {
    let config = HeaderToolbarChipSelection::Custom {
        left: vec![
            HeaderToolbarItemKind::FileExplorer,
            HeaderToolbarItemKind::ToolsPanel,
        ],
        right: vec![],
    };

    assert!(!config.is_shared_left_panel_owner(&HeaderToolbarItemKind::FileExplorer));
    assert!(config.is_shared_left_panel_owner(&HeaderToolbarItemKind::ToolsPanel));
}

#[test]
fn skills_owns_shared_left_panel_when_it_is_the_only_direct_panel_button() {
    let config = HeaderToolbarChipSelection::Custom {
        left: vec![HeaderToolbarItemKind::Skills],
        right: vec![],
    };

    assert!(config.is_shared_left_panel_owner(&HeaderToolbarItemKind::Skills));
}

#[test]
fn file_explorer_takes_shared_left_panel_ownership_from_skills() {
    let config = HeaderToolbarChipSelection::Custom {
        left: vec![
            HeaderToolbarItemKind::FileExplorer,
            HeaderToolbarItemKind::Skills,
        ],
        right: vec![],
    };

    assert!(config.is_shared_left_panel_owner(&HeaderToolbarItemKind::FileExplorer));
    assert!(!config.is_shared_left_panel_owner(&HeaderToolbarItemKind::Skills));
}

#[test]
fn shared_left_panel_side_follows_its_actual_owner() {
    let left_skills = HeaderToolbarChipSelection::Custom {
        left: vec![HeaderToolbarItemKind::Skills],
        right: vec![],
    };
    assert!(left_skills.is_shared_left_panel_on_left());

    let right_tools_with_left_file_explorer = HeaderToolbarChipSelection::Custom {
        left: vec![HeaderToolbarItemKind::FileExplorer],
        right: vec![HeaderToolbarItemKind::ToolsPanel],
    };
    assert!(!right_tools_with_left_file_explorer.is_shared_left_panel_on_left());
}

#[test]
fn header_toolbar_chip_selection_custom_without_code_review_reports_absent() {
    let config = HeaderToolbarChipSelection::Custom {
        left: vec![
            HeaderToolbarItemKind::TabsPanel,
            HeaderToolbarItemKind::ToolsPanel,
        ],
        right: vec![HeaderToolbarItemKind::NotificationsMailbox],
    };
    assert!(!config.contains_item(&HeaderToolbarItemKind::CodeReview));
    assert!(config.contains_item(&HeaderToolbarItemKind::TabsPanel));
    assert!(config.contains_item(&HeaderToolbarItemKind::ToolsPanel));
    assert!(config.contains_item(&HeaderToolbarItemKind::NotificationsMailbox));
    assert!(!config.contains_item(&HeaderToolbarItemKind::AgentManagement));
}

#[test]
fn header_toolbar_chip_selection_custom_with_code_review_on_left_reports_present() {
    let config = HeaderToolbarChipSelection::Custom {
        left: vec![HeaderToolbarItemKind::CodeReview],
        right: vec![],
    };
    assert!(config.contains_item(&HeaderToolbarItemKind::CodeReview));
}

#[test]
fn header_toolbar_chip_selection_custom_empty_reports_only_tabs_panel_present() {
    // The tabs panel is locked to the left, so it is always injected into the
    // left zone even for an otherwise-empty custom config.
    let config = HeaderToolbarChipSelection::Custom {
        left: vec![],
        right: vec![],
    };
    for item in HeaderToolbarItemKind::all_items() {
        if item == HeaderToolbarItemKind::TabsPanel {
            assert!(config.contains_item(&item));
        } else {
            assert!(!config.contains_item(&item));
        }
    }
}

#[test]
fn header_toolbar_chip_selection_locks_tabs_panel_to_left() {
    // A legacy config that placed the tabs panel on the right must be
    // normalized: TabsPanel is forced into the left zone and stripped from the
    // right, so the panel and its toggle button always render on the left.
    let config = HeaderToolbarChipSelection::Custom {
        left: vec![HeaderToolbarItemKind::ToolsPanel],
        right: vec![
            HeaderToolbarItemKind::TabsPanel,
            HeaderToolbarItemKind::NotificationsMailbox,
        ],
    };
    assert!(config
        .left_items()
        .contains(&HeaderToolbarItemKind::TabsPanel));
    assert!(!config
        .right_items()
        .contains(&HeaderToolbarItemKind::TabsPanel));
    // Other items keep their configured placement.
    assert!(config
        .left_items()
        .contains(&HeaderToolbarItemKind::ToolsPanel));
    assert!(config
        .right_items()
        .contains(&HeaderToolbarItemKind::NotificationsMailbox));
}
