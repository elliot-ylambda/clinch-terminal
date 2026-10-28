use warpui::App;

use super::tests::{cli_agent_session_with_id, initialize_app, mock_workspace};
use super::*;
use crate::menu::MenuAction;
use crate::pane_group::Direction;
use crate::terminal::event::BlockMetadataReceivedEvent;
use crate::terminal::model::block::BlockMetadata;
use crate::terminal::model_events::ModelEvent;
use crate::terminal::CLIAgent;
use crate::workspace::tab_settings::VerticalTabsDisplayGranularity;

fn setup_workspace(app: &mut App) -> ViewHandle<Workspace> {
    initialize_app(app);
    TabSettings::handle(app).update(app, |settings, ctx| {
        settings.use_vertical_tabs.set_value(true, ctx).unwrap();
        settings
            .vertical_tabs_display_granularity
            .set_value(VerticalTabsDisplayGranularity::Panes, ctx)
            .unwrap();
    });
    let workspace = mock_workspace(app);
    workspace.update(app, |workspace, ctx| workspace.focus_active_tab(ctx));
    workspace
}

fn dispatch_menu_action(workspace: &ViewHandle<Workspace>, app: &mut App, action: MenuAction) {
    let menu = workspace.read(app, |workspace, _| workspace.tab_right_click_menu.clone());
    app.update(|ctx| {
        ctx.dispatch_typed_action_for_view(menu.window_id(ctx), menu.id(), &action);
    });
}

fn choose_menu_item(workspace: &ViewHandle<Workspace>, app: &mut App, label: &str) {
    let menu = workspace.read(app, |workspace, _| workspace.tab_right_click_menu.clone());
    menu.update(app, |menu, ctx| {
        assert!(menu.set_selected_by_name(label, ctx), "missing {label}");
    });
    // Exercise the menu's normal dispatch and close event, rather than calling
    // Workspace::handle_action with an action extracted from the menu.
    dispatch_menu_action(workspace, app, MenuAction::Enter);
    workspace.read(app, |workspace, _| {
        assert!(workspace.show_tab_right_click_menu.is_none());
    });
}

fn seed_session(terminal_id: EntityId, agent: CLIAgent, id: &str, app: &mut App) {
    CLIAgentSessionsModel::handle(app).update(app, |sessions, ctx| {
        sessions.set_session(terminal_id, cli_agent_session_with_id(agent, Some(id)), ctx);
    });
}

fn seed_directory(terminal: &ViewHandle<TerminalView>, directory: &str, app: &mut App) {
    let (dispatcher, block_index) = terminal.read(app, |terminal, _| {
        (
            terminal.model_event_dispatcher().clone(),
            terminal.model.lock().block_list().active_block_index(),
        )
    });
    dispatcher.update(app, |_, ctx| {
        ctx.emit(ModelEvent::BlockMetadataReceived(
            BlockMetadataReceivedEvent {
                block_metadata: BlockMetadata::new(None, Some(directory.to_owned())),
                block_index,
                is_after_in_band_command: false,
                is_done_bootstrapping: false,
            },
        ));
    });
}

fn menu_signature(items: &[MenuItem<WorkspaceAction>]) -> Vec<String> {
    fn fields_signature(fields: &MenuItemFields<WorkspaceAction>) -> String {
        format!(
            "{}:{}:{:?}",
            fields.label(),
            fields.is_disabled(),
            fields.on_select_action()
        )
    }
    items
        .iter()
        .map(|item| match item {
            MenuItem::Separator => "separator".to_owned(),
            MenuItem::Item(fields) => fields_signature(fields),
            MenuItem::ItemsRow { items } => {
                format!(
                    "row:{:?}",
                    items.iter().map(fields_signature).collect::<Vec<_>>()
                )
            }
            MenuItem::Submenu { fields, menu } => {
                format!(
                    "{}:{:?}",
                    fields_signature(fields),
                    menu_signature(menu.items())
                )
            }
            MenuItem::Header {
                fields,
                clickable,
                right_side_fields,
            } => {
                format!(
                    "header:{clickable}:{}:{:?}",
                    fields_signature(fields),
                    right_side_fields.as_ref().map(fields_signature)
                )
            }
        })
        .collect()
}

#[test]
fn tab_menu_copy_actions_write_clipboard_and_restore_input_focus() {
    let _vertical_tabs = FeatureFlag::VerticalTabs.override_enabled(true);
    App::test((), |mut app| async move {
        let workspace = setup_workspace(&mut app);
        let (terminal, window_id) = workspace.read(&app, |workspace, ctx| {
            (
                workspace.active_terminal_view_handle(ctx).unwrap(),
                workspace.window_id,
            )
        });
        let configuration =
            terminal.read(&app, |terminal, _| terminal.pane_configuration().clone());
        configuration.update(&mut app, |configuration, ctx| {
            configuration.set_custom_vertical_tabs_title("Fix tab menus", ctx);
        });
        seed_directory(&terminal, "/tmp/tab-menu-project", &mut app);
        workspace.update(&mut app, |workspace, ctx| workspace.focus_active_tab(ctx));
        let input_focus = app
            .focused_view_id(window_id)
            .expect("terminal input is focused");

        for (agent, session_id, label) in [
            (
                CLIAgent::Claude,
                "claude-session-123",
                "Copy Claude Code session ID",
            ),
            (
                CLIAgent::Codex,
                "codex-session-456",
                "Copy Codex session ID",
            ),
        ] {
            seed_session(terminal.id(), agent, session_id, &mut app);
            for (label, expected) in [
                (label, session_id),
                ("Copy pane title", "Fix tab menus"),
                ("Copy working directory", "/tmp/tab-menu-project"),
            ] {
                app.update(|ctx| {
                    ctx.clipboard()
                        .write(ClipboardContent::plain_text("old value".into()))
                });
                workspace.update(&mut app, |workspace, ctx| {
                    workspace.toggle_tab_right_click_menu(
                        0,
                        TabContextMenuAnchor::VerticalTabsKebab,
                        ctx,
                    );
                });
                choose_menu_item(&workspace, &mut app, label);
                assert_eq!(
                    app.update(|ctx| ctx.clipboard().read().plain_text),
                    expected
                );
                assert_eq!(
                    app.focused_view_id(window_id),
                    Some(input_focus),
                    "{label} must restore input focus"
                );
            }
        }
    });
}

#[test]
fn right_click_tab_menu_mouse_selection_copies_session_and_restores_focus() {
    let _vertical_tabs = FeatureFlag::VerticalTabs.override_enabled(true);
    App::test((), |mut app| async move {
        let workspace = setup_workspace(&mut app);
        let (terminal_id, locator, window_id) = workspace.read(&app, |workspace, ctx| {
            let group = workspace.active_tab_pane_group();
            (
                group.as_ref(ctx).focused_session_view(ctx).unwrap().id(),
                PaneViewLocator {
                    pane_group_id: group.id(),
                    pane_id: group.as_ref(ctx).focused_pane_id(ctx),
                },
                workspace.window_id,
            )
        });
        seed_session(
            terminal_id,
            CLIAgent::Codex,
            "mouse-selected-session",
            &mut app,
        );
        workspace.update(&mut app, |workspace, ctx| workspace.focus_active_tab(ctx));
        let input_focus = app.focused_view_id(window_id);
        workspace.update(&mut app, |workspace, ctx| {
            workspace.vertical_tabs_panel_open = true;
            workspace.should_show_ai_assistant_warm_welcome = false;
            workspace.toggle_vertical_tabs_pane_context_menu(
                0,
                VerticalTabsPaneContextMenuTarget::ClickedPane(locator),
                vec2f(300., 100.),
                ctx,
            );
        });
        let menu = workspace.read(&app, |workspace, _| workspace.tab_right_click_menu.clone());
        menu.update(&mut app, |menu, ctx| {
            // Selected rows publish their rendered bounds as submenu_0. This
            // identifies the actual copy row without relying on font metrics.
            assert!(menu.set_selected_by_name("Copy Codex session ID", ctx));
        });
        let presenter = std::rc::Rc::new(RefCell::new(warpui::Presenter::new(window_id)));
        app.update(|ctx| {
            ctx.clipboard()
                .write(ClipboardContent::plain_text("old value".into()));
            presenter.borrow_mut().invalidate(
                warpui::WindowInvalidation {
                    updated: HashSet::from([workspace.id(), menu.id()]),
                    ..Default::default()
                },
                ctx,
            );
            presenter
                .borrow_mut()
                .build_scene(vec2f(1200., 800.), 1., None, ctx);
            let position = presenter
                .borrow()
                .position_cache()
                .get_position("submenu_0")
                .expect("copy row must have rendered bounds")
                .center();
            for event in [
                warpui::Event::MouseMoved {
                    position,
                    cmd: false,
                    shift: false,
                    is_synthetic: false,
                },
                warpui::Event::LeftMouseDown {
                    position,
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                warpui::Event::LeftMouseUp {
                    position,
                    modifiers: Default::default(),
                },
            ] {
                ctx.simulate_window_event(event, window_id, presenter.clone());
            }
        });
        assert_eq!(
            app.update(|ctx| ctx.clipboard().read().plain_text),
            "mouse-selected-session"
        );
        assert_eq!(app.focused_view_id(window_id), input_focus);
        workspace.read(&app, |workspace, _| {
            assert!(workspace.show_tab_right_click_menu.is_none())
        });
    });
}

#[test]
fn tab_right_click_and_kebab_have_identical_ordered_menus() {
    let _vertical_tabs = FeatureFlag::VerticalTabs.override_enabled(true);
    App::test((), |mut app| async move {
        let workspace = setup_workspace(&mut app);
        let locator = workspace.read(&app, |workspace, ctx| {
            let group = workspace.active_tab_pane_group();
            PaneViewLocator {
                pane_group_id: group.id(),
                pane_id: group.as_ref(ctx).focused_pane_id(ctx),
            }
        });
        for granularity in [
            VerticalTabsDisplayGranularity::Panes,
            VerticalTabsDisplayGranularity::Tabs,
        ] {
            TabSettings::handle(&app).update(&mut app, |settings, ctx| {
                settings
                    .vertical_tabs_display_granularity
                    .set_value(granularity, ctx)
                    .unwrap();
            });
            workspace.update(&mut app, |workspace, ctx| {
                workspace.toggle_tab_right_click_menu(
                    0,
                    TabContextMenuAnchor::VerticalTabsKebab,
                    ctx,
                );
            });
            let expected = workspace.read(&app, |workspace, ctx| {
                let items = workspace.tab_right_click_menu.as_ref(ctx).items();
                assert!(!matches!(items.first(), Some(MenuItem::Separator)));
                assert!(!matches!(items.last(), Some(MenuItem::Separator)));
                assert!(items
                    .windows(2)
                    .all(|pair| !pair.iter().all(|item| matches!(item, MenuItem::Separator))));
                let labels = items
                    .iter()
                    .filter_map(|item| item.fields().map(|fields| fields.label()))
                    .collect::<Vec<_>>();
                assert!(labels.first().unwrap().starts_with("Copy "));
                assert!(labels.last().unwrap().starts_with("Close "));
                let rename = labels
                    .iter()
                    .position(|label| *label == "Rename pane")
                    .unwrap();
                let close = labels
                    .iter()
                    .position(|label| label.starts_with("Close "))
                    .unwrap();
                assert!(rename < close);
                menu_signature(items)
            });
            dispatch_menu_action(&workspace, &mut app, MenuAction::Close(false));
            for target in [
                VerticalTabsPaneContextMenuTarget::ClickedPane(locator),
                VerticalTabsPaneContextMenuTarget::ActivePane(locator),
            ] {
                workspace.update(&mut app, |workspace, ctx| {
                    workspace.toggle_vertical_tabs_pane_context_menu(
                        0,
                        target,
                        vec2f(80., 100.),
                        ctx,
                    );
                });
                workspace.read(&app, |workspace, ctx| {
                    assert_eq!(
                        menu_signature(workspace.tab_right_click_menu.as_ref(ctx).items()),
                        expected
                    );
                });
                dispatch_menu_action(&workspace, &mut app, MenuAction::Close(false));
            }
        }
    });
}

#[test]
fn tab_menu_copies_clicked_background_pane_without_changing_active_pane() {
    let _vertical_tabs = FeatureFlag::VerticalTabs.override_enabled(true);
    App::test((), |mut app| async move {
        let workspace = setup_workspace(&mut app);
        let (locator, background_terminal, active_terminal, window_id) =
            workspace.update(&mut app, |workspace, ctx| {
                let group = workspace.active_tab_pane_group().clone();
                let (pane_id, background_terminal) = group.read(ctx, |group, ctx| {
                    (
                        group.focused_pane_id(ctx),
                        group.focused_session_view(ctx).unwrap(),
                    )
                });
                group.update(ctx, |group, ctx| {
                    group.add_terminal_pane(Direction::Right, None, ctx);
                });
                let active_terminal = group.as_ref(ctx).focused_session_view(ctx).unwrap();
                workspace.focus_active_tab(ctx);
                (
                    PaneViewLocator {
                        pane_group_id: group.id(),
                        pane_id,
                    },
                    background_terminal,
                    active_terminal,
                    ctx.window_id(),
                )
            });
        assert_ne!(background_terminal.id(), active_terminal.id());
        seed_session(
            background_terminal.id(),
            CLIAgent::Codex,
            "background-session",
            &mut app,
        );
        seed_session(
            active_terminal.id(),
            CLIAgent::Codex,
            "active-session",
            &mut app,
        );
        let input_focus = app.focused_view_id(window_id);
        workspace.update(&mut app, |workspace, ctx| {
            workspace.toggle_vertical_tabs_pane_context_menu(
                0,
                VerticalTabsPaneContextMenuTarget::ClickedPane(locator),
                vec2f(80., 100.),
                ctx,
            );
        });
        choose_menu_item(&workspace, &mut app, "Copy Codex session ID");
        assert_eq!(
            app.update(|ctx| ctx.clipboard().read().plain_text),
            "background-session"
        );
        assert_eq!(app.focused_view_id(window_id), input_focus);
        workspace.read(&app, |workspace, ctx| {
            assert_eq!(
                workspace.active_terminal_view_handle(ctx).unwrap().id(),
                active_terminal.id()
            );
        });
    });
}

#[test]
fn claude_and_codex_have_identical_tab_menus_before_and_after_identification() {
    let _vertical_tabs = FeatureFlag::VerticalTabs.override_enabled(true);
    App::test((), |mut app| async move {
        let workspace = setup_workspace(&mut app);
        let terminal = workspace.read(&app, |workspace, ctx| {
            workspace.active_terminal_view_handle(ctx).unwrap()
        });
        for id in ["", "identified-conversation"] {
            let mut signatures = Vec::new();
            for agent in [CLIAgent::Claude, CLIAgent::Codex] {
                seed_session(terminal.id(), agent, id, &mut app);
                workspace.update(&mut app, |workspace, ctx| {
                    workspace.toggle_tab_right_click_menu(
                        0,
                        TabContextMenuAnchor::Pointer(vec2f(80., 100.)),
                        ctx,
                    );
                });
                signatures.push(workspace.read(&app, |workspace, ctx| {
                    menu_signature(workspace.tab_right_click_menu.as_ref(ctx).items())
                        .into_iter()
                        .map(|row| {
                            row.replace("Claude Code", "Agent")
                                .replace("Codex", "Agent")
                        })
                        .collect::<Vec<_>>()
                }));
                dispatch_menu_action(&workspace, &mut app, MenuAction::Close(false));
            }
            assert_eq!(signatures[0], signatures[1]);
        }
    });
}

#[test]
fn tab_menu_dismissal_restores_focus_but_rename_keeps_its_editor_focused() {
    let _vertical_tabs = FeatureFlag::VerticalTabs.override_enabled(true);
    App::test((), |mut app| async move {
        let workspace = setup_workspace(&mut app);
        let window_id = app.read(|ctx| workspace.window_id(ctx));
        let input_focus = app.focused_view_id(window_id);
        workspace.update(&mut app, |workspace, ctx| {
            workspace.toggle_tab_right_click_menu(0, TabContextMenuAnchor::VerticalTabsKebab, ctx);
        });
        dispatch_menu_action(&workspace, &mut app, MenuAction::Close(false));
        assert_eq!(app.focused_view_id(window_id), input_focus);

        workspace.update(&mut app, |workspace, ctx| {
            workspace.toggle_tab_right_click_menu(0, TabContextMenuAnchor::VerticalTabsKebab, ctx);
        });
        workspace.update(&mut app, |workspace, ctx| {
            workspace.toggle_tab_right_click_menu(0, TabContextMenuAnchor::VerticalTabsKebab, ctx);
        });
        assert_eq!(app.focused_view_id(window_id), input_focus);

        workspace.update(&mut app, |workspace, ctx| {
            workspace.toggle_tab_right_click_menu(0, TabContextMenuAnchor::VerticalTabsKebab, ctx);
        });
        choose_menu_item(&workspace, &mut app, "Rename pane");
        let editor_id = workspace.read(&app, |workspace, _| workspace.pane_rename_editor.id());
        assert_eq!(app.focused_view_id(window_id), Some(editor_id));
    });
}
