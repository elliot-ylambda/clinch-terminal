use super::*;
use crate::test_util::settings::initialize_settings_for_tests;
use warpui::platform::WindowStyle;
use warpui::App;

#[test]
fn next_selection_with_custom_button_appends_and_materializes_default() {
    let next = next_selection_with_custom_button(
        CLIAgentToolbarChipSelection::Default,
        "Ship".into(),
        "/deploy".into(),
    );
    let CLIAgentToolbarChipSelection::Custom { left, .. } = next else {
        panic!("expected Custom");
    };
    // Default left items are materialized, then the new button is appended last.
    assert_eq!(
        left.last(),
        Some(&AgentToolbarItemKind::CustomInsert {
            label: "Ship".into(),
            text: "/deploy".into()
        })
    );
    assert!(left.contains(&AgentToolbarItemKind::ForkSession)); // materialized default
}

#[test]
fn next_terminal_selection_with_custom_button_preserves_defaults_and_appends() {
    let mut expected_left = AgentToolbarItemKind::terminal_default_left();
    expected_left.push(AgentToolbarItemKind::CustomInsert {
        label: "Status".into(),
        text: "git status".into(),
    });

    assert_eq!(
        next_terminal_selection_with_custom_button(
            TerminalToolbarChipSelection::Default,
            "Status".into(),
            "git status".into(),
        ),
        TerminalToolbarChipSelection::Custom {
            left: expected_left,
            right: AgentToolbarItemKind::terminal_default_right(),
        }
    );
}

#[test]
fn terminal_editor_defaults_match_and_save_round_trip() {
    let defaults_left = AgentToolbarItemKind::terminal_default_left();
    let defaults_right = AgentToolbarItemKind::terminal_default_right();
    assert!(toolbar_items_match_defaults(
        AgentToolbarEditorMode::Terminal,
        &defaults_left,
        &defaults_right,
    ));

    let mut custom_left = defaults_left.clone();
    custom_left.swap(0, 1);
    assert!(!toolbar_items_match_defaults(
        AgentToolbarEditorMode::Terminal,
        &custom_left,
        &defaults_right,
    ));

    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| Appearance::mock());
        let (_, editor) = app.add_window(WindowStyle::NotStealFocus, AgentToolbarEditorModal::new);

        editor.update(&mut app, |_, ctx| {
            save_toolbar_selection(
                AgentToolbarEditorMode::Terminal,
                custom_left.clone(),
                defaults_right.clone(),
                ctx,
            );
        });
        SessionSettings::handle(&app).read(&app, |settings, _| {
            assert_eq!(
                settings.terminal_footer_chip_selection.value(),
                &TerminalToolbarChipSelection::Custom {
                    left: custom_left,
                    right: defaults_right.clone(),
                }
            );
        });

        editor.update(&mut app, |_, ctx| {
            save_toolbar_selection(
                AgentToolbarEditorMode::Terminal,
                defaults_left,
                defaults_right,
                ctx,
            );
        });
        SessionSettings::handle(&app).read(&app, |settings, _| {
            assert_eq!(
                settings.terminal_footer_chip_selection.value(),
                &TerminalToolbarChipSelection::Default
            );
        });
    });
}
