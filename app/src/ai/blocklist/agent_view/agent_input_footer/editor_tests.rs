use settings_value::SettingsValue;
use warpui::platform::WindowStyle;
use warpui::App;

use super::*;
use crate::chip_configurator::ChipLocation;
use crate::test_util::settings::initialize_settings_for_tests;

#[test]
fn next_selection_with_custom_button_appends_after_live_defaults() {
    let next = next_selection_with_custom_button(
        CLIAgentToolbarChipSelection::Default,
        "Ship".into(),
        "/deploy".into(),
        true,
        true,
    );
    let CLIAgentToolbarChipSelection::Custom { left, .. } = next else {
        panic!("expected Custom");
    };
    // Defaults remain live and the new button is persisted after them.
    assert_eq!(
        left.last(),
        Some(&AgentToolbarItemKind::CustomInsert {
            label: "Ship".into(),
            text: "/deploy".into(),
            auto_send: true,
        })
    );
    assert!(left.contains(&AgentToolbarItemKind::ForkSession));
}

#[test]
fn next_terminal_selection_with_custom_button_preserves_defaults_and_appends() {
    let mut expected_left = AgentToolbarItemKind::terminal_default_left();
    expected_left.push(AgentToolbarItemKind::CustomInsert {
        label: "Status".into(),
        text: "git status".into(),
        auto_send: true,
    });

    assert_eq!(
        next_terminal_selection_with_custom_button(
            TerminalToolbarChipSelection::Default,
            "Status".into(),
            "git status".into(),
            true,
            true,
        ),
        TerminalToolbarChipSelection::custom_from_effective_items(
            expected_left,
            AgentToolbarItemKind::terminal_default_right(),
        )
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
                Vec::new(),
                ctx,
            );
        });
        SessionSettings::handle(&app).read(&app, |settings, _| {
            assert_eq!(
                settings.terminal_footer_chip_selection.value(),
                &TerminalToolbarChipSelection::custom_from_effective_items(
                    custom_left,
                    defaults_right.clone(),
                )
            );
        });

        editor.update(&mut app, |_, ctx| {
            save_toolbar_selection(
                AgentToolbarEditorMode::Terminal,
                defaults_left,
                defaults_right,
                Vec::new(),
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

#[test]
fn custom_button_can_be_saved_without_showing_in_footer() {
    let button = AgentToolbarItemKind::CustomInsert {
        label: "Later".into(),
        text: "Review this later".into(),
        auto_send: false,
    };
    let next = next_selection_with_custom_button(
        CLIAgentToolbarChipSelection::Default,
        "Later".into(),
        "Review this later".into(),
        false,
        false,
    );

    assert!(!next.left_items().contains(&button));
    assert_eq!(next.hidden_custom_inserts(), vec![button.clone()]);
    let restored = CLIAgentToolbarChipSelection::from_file_value(&next.to_file_value()).unwrap();
    assert_eq!(restored.hidden_custom_inserts(), vec![button]);
}

#[test]
fn provider_alias_opens_shared_footer_and_persists_controls() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| Appearance::mock());
        let (_, editor) = app.add_window(WindowStyle::NotStealFocus, AgentToolbarEditorModal::new);

        editor.update(&mut app, |editor, ctx| {
            editor.open(AgentToolbarEditorMode::ClaudeCode, ctx);
            assert_eq!(editor.mode, AgentToolbarEditorMode::CLIAgent);
            assert_eq!(
                editor.chip_configurator.layout(),
                ChipConfiguratorLayout::SingleZone
            );

            let compact_index = editor
                .chip_configurator
                .used_item_kinds()
                .iter()
                .position(|item| item == &AgentToolbarItemKind::Compact)
                .unwrap();
            editor.handle_action(
                &AgentToolbarEditorAction::Chip(ChipConfiguratorAction::ToggleAutoSend {
                    location: ChipLocation::Used {
                        index: compact_index,
                    },
                }),
                ctx,
            );

            let preset_index = editor
                .chip_configurator
                .unused_item_kinds()
                .iter()
                .position(|item| item.display_label() == "/codex")
                .unwrap();
            editor.handle_action(
                &AgentToolbarEditorAction::Chip(ChipConfiguratorAction::AddFromUnused {
                    index: preset_index,
                }),
                ctx,
            );
            editor.handle_action(&AgentToolbarEditorAction::Save, ctx);
        });

        let stale_provider_selection = CLIAgentToolbarChipSelection::custom_from_effective_items(
            vec![AgentToolbarItemKind::custom_insert(
                "Stale provider button",
                "stale prompt",
            )],
            Vec::new(),
        );
        SessionSettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .claude_code_footer_chip_selection
                .set_value(Some(stale_provider_selection.clone()), ctx)
                .unwrap();
            settings
                .codex_footer_chip_selection
                .set_value(Some(stale_provider_selection), ctx)
                .unwrap();
        });

        SessionSettings::handle(&app).read(&app, |settings, _| {
            let selection = settings
                .coding_agent_footer_chip_selection
                .value()
                .as_ref()
                .expect("shared coding-agent layout should be persisted");
            assert!(selection.right_items().is_empty());
            assert!(selection.left_items().iter().any(|item| {
                matches!(
                    item,
                    AgentToolbarItemKind::CustomInsert {
                        label,
                        text,
                        auto_send: false,
                    } if label == "Compact" && text == "/compact"
                )
            }));
            assert!(selection
                .left_items()
                .iter()
                .any(|item| item.display_label() == "/codex"));
            assert_eq!(
                settings.claude_code_footer_chip_selection_value(),
                selection
            );
            assert_eq!(settings.codex_footer_chip_selection_value(), selection);
        });
    });
}

#[test]
fn footer_tabs_preserve_one_coding_agent_draft_and_terminal_draft() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| Appearance::mock());

        let legacy_button = AgentToolbarItemKind::custom_insert("Legacy", "legacy prompt");
        let mut legacy_left = AgentToolbarItemKind::cli_default_left();
        legacy_left.push(legacy_button.clone());
        let legacy_selection =
            CLIAgentToolbarChipSelection::custom_from_effective_items(legacy_left, Vec::new());
        SessionSettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .cli_agent_footer_chip_selection
                .set_value(legacy_selection.clone(), ctx)
                .unwrap();
        });

        let (_, editor) = app.add_window(WindowStyle::NotStealFocus, AgentToolbarEditorModal::new);
        editor.update(&mut app, |editor, ctx| {
            editor.open(AgentToolbarEditorMode::ClaudeCode, ctx);
            assert_eq!(editor.mode, AgentToolbarEditorMode::CLIAgent);
            assert!(editor
                .chip_configurator
                .used_item_kinds()
                .contains(&legacy_button));

            let compact_index = editor
                .chip_configurator
                .used_item_kinds()
                .iter()
                .position(|item| item == &AgentToolbarItemKind::Compact)
                .unwrap();
            editor.handle_action(
                &AgentToolbarEditorAction::Chip(ChipConfiguratorAction::ToggleAutoSend {
                    location: ChipLocation::Used {
                        index: compact_index,
                    },
                }),
                ctx,
            );

            editor.handle_action(
                &AgentToolbarEditorAction::SelectMode(AgentToolbarEditorMode::Codex),
                ctx,
            );
            assert_eq!(editor.mode, AgentToolbarEditorMode::CLIAgent);
            assert!(editor
                .chip_configurator
                .used_item_kinds()
                .iter()
                .any(|item| matches!(
                    item,
                    AgentToolbarItemKind::CustomInsert {
                        label,
                        text,
                        auto_send: false,
                    } if label == "Compact" && text == "/compact"
                )));
            assert!(editor
                .chip_configurator
                .used_item_kinds()
                .contains(&legacy_button));
            let preset_index = editor
                .chip_configurator
                .unused_item_kinds()
                .iter()
                .position(|item| item.display_label() == "/codex")
                .unwrap();
            editor.handle_action(
                &AgentToolbarEditorAction::Chip(ChipConfiguratorAction::AddFromUnused {
                    index: preset_index,
                }),
                ctx,
            );

            assert!(editor
                .chip_configurator
                .used_item_kinds()
                .iter()
                .any(|item| matches!(
                    item,
                    AgentToolbarItemKind::CustomInsert {
                        label,
                        text,
                        auto_send: false,
                    } if label == "Compact" && text == "/compact"
                )));

            editor.handle_action(
                &AgentToolbarEditorAction::SelectMode(AgentToolbarEditorMode::Terminal),
                ctx,
            );
            editor.handle_action(
                &AgentToolbarEditorAction::Chip(ChipConfiguratorAction::RemoveFromUsed {
                    location: ChipLocation::Used { index: 0 },
                }),
                ctx,
            );
            editor.handle_action(&AgentToolbarEditorAction::Save, ctx);
        });

        SessionSettings::handle(&app).read(&app, |settings, _| {
            assert_eq!(
                settings.cli_agent_footer_chip_selection.value(),
                &legacy_selection
            );
            assert!(settings.claude_code_footer_chip_selection.value().is_none());
            assert!(settings.codex_footer_chip_selection.value().is_none());

            let shared = settings
                .coding_agent_footer_chip_selection
                .value()
                .as_ref()
                .expect("coding agents should have one saved layout");
            assert!(shared.left_items().iter().any(|item| matches!(
                item,
                AgentToolbarItemKind::CustomInsert {
                    label,
                    text,
                    auto_send: false,
                } if label == "Compact" && text == "/compact"
            )));
            assert!(shared.left_items().contains(&legacy_button));
            assert!(shared
                .left_items()
                .iter()
                .any(|item| item.display_label() == "/codex"));
            assert_eq!(
                settings.footer_chip_selection_for_cli_agent(crate::terminal::CLIAgent::Claude),
                shared
            );
            assert_eq!(
                settings.footer_chip_selection_for_cli_agent(crate::terminal::CLIAgent::Codex),
                shared
            );
            assert_eq!(
                settings.footer_chip_selection_for_cli_agent(crate::terminal::CLIAgent::Gemini),
                shared
            );

            assert_eq!(
                settings.terminal_footer_chip_selection.left_items(),
                AgentToolbarItemKind::terminal_default_left()
                    .into_iter()
                    .skip(1)
                    .collect::<Vec<_>>()
            );
        });
    });
}
