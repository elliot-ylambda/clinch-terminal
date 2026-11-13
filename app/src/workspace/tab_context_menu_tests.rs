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

#[test]
fn tab_menu_new_tab_below_opens_a_tab_directly_under_the_clicked_tab() {
    let _vertical_tabs = FeatureFlag::VerticalTabs.override_enabled(true);
    App::test((), |mut app| async move {
        let workspace = setup_workspace(&mut app);
        let last_tab_id = workspace.update(&mut app, |workspace, ctx| {
            workspace.add_terminal_tab(false, ctx);
            workspace.active_tab_pane_group().id()
        });
        workspace.update(&mut app, |workspace, ctx| {
            workspace.toggle_tab_right_click_menu(0, TabContextMenuAnchor::VerticalTabsKebab, ctx);
        });
        choose_menu_item(&workspace, &mut app, "New tab below");
        workspace.read(&app, |workspace, _| {
            assert_eq!(workspace.tab_count(), 3);
            assert_eq!(workspace.active_tab_index(), 1);
            assert_eq!(workspace.tabs[2].pane_group.id(), last_tab_id);
        });
    });
}

/// An agent tab's menu offers Sleep, or Wake once it is asleep, and the two bulk commands. A
/// plain terminal tab offers none of them.
#[test]
fn an_agent_tab_menu_offers_sleep_or_wake_and_a_plain_terminal_does_not() {
    let _vertical_tabs = FeatureFlag::VerticalTabs.override_enabled(true);
    App::test((), |mut app| async move {
        let workspace = setup_workspace(&mut app);
        let terminal = workspace.read(&app, |workspace, ctx| {
            workspace.active_terminal_view_handle(ctx).unwrap()
        });
        let menu_rows = |workspace: &ViewHandle<Workspace>, app: &mut App| {
            workspace.update(app, |workspace, ctx| {
                workspace.toggle_tab_right_click_menu(
                    0,
                    TabContextMenuAnchor::Pointer(vec2f(80., 100.)),
                    ctx,
                );
            });
            let rows = workspace.read(app, |workspace, ctx| {
                menu_signature(workspace.tab_right_click_menu.as_ref(ctx).items())
            });
            rows.into_iter()
                .filter(|row| row.contains("leep") || row.starts_with("Wake"))
                .collect::<Vec<_>>()
        };

        assert!(
            menu_rows(&workspace, &mut app).is_empty(),
            "a terminal with no agent has nothing to put to sleep"
        );
        dispatch_menu_action(&workspace, &mut app, MenuAction::Close(false));

        // An agent tab with nothing running in it yet: Sleep is there, and says why it can't.
        seed_session(terminal.id(), CLIAgent::Claude, "conversation", &mut app);
        assert_eq!(
            menu_rows(&workspace, &mut app),
            [
                "Sleep:true:Some(SleepTab(0))",
                "Sleep idle sessions in this project:false:Some(SleepIdleTabs { every_project: false })",
                "Sleep idle sessions in all projects:false:Some(SleepIdleTabs { every_project: true })",
            ]
        );
        dispatch_menu_action(&workspace, &mut app, MenuAction::Close(false));

        // Asleep: the same row is Wake, and choosing it starts the agent once.
        let starts = std::rc::Rc::new(std::cell::Cell::new(0));
        let counter = starts.clone();
        terminal.update(&mut app, |view, ctx| {
            view.defer_agent_start(Box::new(move |_| counter.set(counter.get() + 1)));
            view.mark_agent_asleep(chrono::Utc::now(), ctx);
        });
        assert_eq!(
            menu_rows(&workspace, &mut app)[0],
            "Wake:false:Some(WakeTab(0))"
        );
        choose_menu_item(&workspace, &mut app, "Wake");
        assert_eq!(starts.get(), 1);
        terminal.read(&app, |view, _| assert!(!view.agent_is_asleep()));
    });
}

// ---- Renaming from a menu or a section header, driven by real mouse and key events ----

type ScenePresenter = std::rc::Rc<RefCell<warpui::Presenter>>;

/// Lays out the workspace the way the window would before its next event. Mouse state
/// lives in the views, so a fresh presenter per frame still sees hovers and clicks.
fn render_scene(workspace: &ViewHandle<Workspace>, app: &mut App) -> ScenePresenter {
    let (window_id, views) = workspace.read(app, |workspace, _| {
        (
            workspace.window_id,
            HashSet::from([
                workspace.tab_right_click_menu.id(),
                workspace.tab_rename_editor.id(),
                workspace.tab_group_rename_editor.id(),
                workspace.pane_rename_editor.id(),
            ]),
        )
    });
    let presenter = std::rc::Rc::new(RefCell::new(warpui::Presenter::new(window_id)));
    app.update(|ctx| {
        let mut updated = views;
        updated.insert(workspace.id());
        presenter.borrow_mut().invalidate(
            warpui::WindowInvalidation {
                updated,
                ..Default::default()
            },
            ctx,
        );
        presenter
            .borrow_mut()
            .build_scene(vec2f(1200., 800.), 1., None, ctx);
    });
    presenter
}

fn send_events(
    events: impl IntoIterator<Item = warpui::Event>,
    workspace: &ViewHandle<Workspace>,
    presenter: &ScenePresenter,
    app: &mut App,
) {
    let window_id = workspace.read(app, |workspace, _| workspace.window_id);
    app.update(|ctx| {
        for event in events {
            ctx.simulate_window_event(event, window_id, presenter.clone());
        }
    });
}

/// Two moves, as a real pointer makes: the innermost hover target takes the first for its
/// own hover change, so a target wrapped around it only sees the second.
fn hover(position: Vector2F) -> [warpui::Event; 2] {
    [position, position + vec2f(1., 0.)].map(|position| warpui::Event::MouseMoved {
        position,
        cmd: false,
        shift: false,
        is_synthetic: false,
    })
}

fn left_click(position: Vector2F, click_count: u32) -> [warpui::Event; 2] {
    [
        warpui::Event::LeftMouseDown {
            position,
            modifiers: Default::default(),
            click_count,
            is_first_mouse: false,
        },
        warpui::Event::LeftMouseUp {
            position,
            modifiers: Default::default(),
        },
    ]
}

/// Clicks a row of the open tab or section menu with the mouse.
fn click_menu_row(workspace: &ViewHandle<Workspace>, app: &mut App, label: &str) {
    let menu = workspace.read(app, |workspace, _| workspace.tab_right_click_menu.clone());
    menu.update(app, |menu, ctx| {
        // The selected row publishes its rendered bounds as submenu_0.
        assert!(menu.set_selected_by_name(label, ctx), "missing {label}");
    });
    let presenter = render_scene(workspace, app);
    let row = presenter
        .borrow()
        .position_cache()
        .get_position("submenu_0")
        .expect("menu row must have rendered bounds")
        .center();
    send_events(hover(row), workspace, &presenter, app);
    send_events(left_click(row, 1), workspace, &presenter, app);
}

/// Types into whichever editor holds focus, then presses Enter.
fn type_and_press_enter(workspace: &ViewHandle<Workspace>, app: &mut App, text: &str) {
    let presenter = render_scene(workspace, app);
    send_events(
        [warpui::Event::TypedCharacters {
            chars: text.to_owned(),
        }],
        workspace,
        &presenter,
        app,
    );
    let presenter = render_scene(workspace, app);
    send_events(
        [warpui::Event::KeyDown {
            keystroke: warpui::keymap::Keystroke::parse("enter").unwrap(),
            chars: "\r".to_owned(),
            details: Default::default(),
            is_composing: false,
        }],
        workspace,
        &presenter,
        app,
    );
}

/// A two-tab workspace whose first tab sits in a section named "Backend", with the
/// vertical tabs panel showing.
fn setup_workspace_with_section(app: &mut App) -> (ViewHandle<Workspace>, TabGroupId) {
    let workspace = setup_workspace(app);
    // Enter reaches a rename editor through the editor's own key bindings.
    app.update(crate::editor::init);
    let group_id = workspace.update(app, |workspace, ctx| {
        workspace.vertical_tabs_panel_open = true;
        workspace.should_show_ai_assistant_warm_welcome = false;
        workspace.add_terminal_tab(false, ctx);
        let group_id = workspace
            .create_named_tab_group_from_tab(0, "Backend".to_owned(), ctx)
            .expect("tab becomes a section");
        workspace.focus_active_tab(ctx);
        group_id
    });
    (workspace, group_id)
}

struct SectionHeader {
    /// A point on the header's title.
    title: Vector2F,
    /// The centre of the "..." button, which only renders while the section is hovered.
    kebab: Vector2F,
}

/// Hovers the section and returns where its header controls are.
fn hover_section_header(
    workspace: &ViewHandle<Workspace>,
    group_id: TabGroupId,
    app: &mut App,
) -> SectionHeader {
    let presenter = render_scene(workspace, app);
    let section = presenter
        .borrow()
        .position_cache()
        .get_position(vertical_tabs::vtab_group_position_id(group_id))
        .expect("section must be rendered");
    send_events(hover(section.center()), workspace, &presenter, app);
    let presenter = render_scene(workspace, app);
    let kebab = presenter
        .borrow()
        .position_cache()
        .get_position(vertical_tabs::vtab_group_kebab_position_id(group_id))
        .expect("the section's ... button must render on hover")
        .center();
    let header = SectionHeader {
        title: vec2f(section.origin_x() + 40., kebab.y()),
        kebab,
    };
    send_events(hover(header.title), workspace, &presenter, app);
    header
}

fn assert_section_rename_is_open(
    workspace: &ViewHandle<Workspace>,
    group_id: TabGroupId,
    app: &App,
) {
    let (renaming, editor_id, window_id) = workspace.read(app, |workspace, _| {
        (
            workspace
                .current_workspace_state
                .is_tab_group_being_renamed(group_id),
            workspace.tab_group_rename_editor.id(),
            workspace.window_id,
        )
    });
    assert!(renaming, "the section's name editor must be open");
    assert_eq!(
        app.focused_view_id(window_id),
        Some(editor_id),
        "the section's name editor must hold focus, or the first key press is lost"
    );
}

/// Types a new name into the open section editor and checks it sticks.
fn assert_typing_renames_section(
    workspace: &ViewHandle<Workspace>,
    group_id: TabGroupId,
    input_focus: Option<EntityId>,
    app: &mut App,
) {
    assert_section_rename_is_open(workspace, group_id, app);
    type_and_press_enter(workspace, app, "API");
    let (name, renaming, window_id) = workspace.read(app, |workspace, _| {
        (
            workspace.tab_groups[&group_id].name.clone(),
            workspace
                .current_workspace_state
                .is_any_tab_group_being_renamed(),
            workspace.window_id,
        )
    });
    assert_eq!(name.as_deref(), Some("API"));
    assert!(!renaming);
    assert_eq!(app.focused_view_id(window_id), input_focus);
}

#[test]
fn section_menus_list_rename_directly_above_remove() {
    let _vertical_tabs = FeatureFlag::VerticalTabs.override_enabled(true);
    let _grouped_tabs = FeatureFlag::GroupedTabs.override_enabled(true);
    App::test((), |mut app| async move {
        let (workspace, group_id) = setup_workspace_with_section(&mut app);
        // The "..." button and a right-click on the header open the same menu.
        for anchor in [
            TabContextMenuAnchor::VerticalTabsKebab,
            TabContextMenuAnchor::Pointer(vec2f(80., 100.)),
        ] {
            workspace.update(&mut app, |workspace, ctx| {
                workspace.toggle_tab_group_right_click_menu(group_id, anchor, ctx);
            });
            workspace.read(&app, |workspace, ctx| {
                let labels = workspace
                    .tab_right_click_menu
                    .as_ref(ctx)
                    .items()
                    .iter()
                    .filter_map(|item| item.fields().map(|fields| fields.label()))
                    .collect::<Vec<_>>();
                let rename = labels
                    .iter()
                    .position(|label| *label == "Rename section")
                    .expect("menu offers Rename section");
                assert_eq!(labels.get(rename + 1), Some(&"Remove section"));
            });
            dispatch_menu_action(&workspace, &mut app, MenuAction::Close(false));
        }
    });
}

#[test]
fn clicking_rename_section_in_the_kebab_menu_renames_the_section() {
    let _vertical_tabs = FeatureFlag::VerticalTabs.override_enabled(true);
    let _grouped_tabs = FeatureFlag::GroupedTabs.override_enabled(true);
    App::test((), |mut app| async move {
        let (workspace, group_id) = setup_workspace_with_section(&mut app);
        let window_id = app.read(|ctx| workspace.window_id(ctx));
        let input_focus = app.focused_view_id(window_id);

        let header = hover_section_header(&workspace, group_id, &mut app);
        let presenter = render_scene(&workspace, &mut app);
        send_events(hover(header.kebab), &workspace, &presenter, &mut app);
        send_events(
            left_click(header.kebab, 1),
            &workspace,
            &presenter,
            &mut app,
        );
        workspace.read(&app, |workspace, _| {
            assert!(workspace.show_tab_group_right_click_menu.is_some());
        });

        click_menu_row(&workspace, &mut app, "Rename section");
        assert_typing_renames_section(&workspace, group_id, input_focus, &mut app);
    });
}

#[test]
fn clicking_rename_section_in_the_right_click_menu_renames_the_section() {
    let _vertical_tabs = FeatureFlag::VerticalTabs.override_enabled(true);
    let _grouped_tabs = FeatureFlag::GroupedTabs.override_enabled(true);
    App::test((), |mut app| async move {
        let (workspace, group_id) = setup_workspace_with_section(&mut app);
        let window_id = app.read(|ctx| workspace.window_id(ctx));
        let input_focus = app.focused_view_id(window_id);

        let header = hover_section_header(&workspace, group_id, &mut app);
        let presenter = render_scene(&workspace, &mut app);
        send_events(
            [warpui::Event::RightMouseDown {
                position: header.title,
                cmd: false,
                shift: false,
                click_count: 1,
            }],
            &workspace,
            &presenter,
            &mut app,
        );
        workspace.read(&app, |workspace, _| {
            assert!(workspace.show_tab_group_right_click_menu.is_some());
        });

        click_menu_row(&workspace, &mut app, "Rename section");
        assert_typing_renames_section(&workspace, group_id, input_focus, &mut app);
    });
}

#[test]
fn double_clicking_a_section_header_renames_it_without_collapsing_it() {
    let _vertical_tabs = FeatureFlag::VerticalTabs.override_enabled(true);
    let _grouped_tabs = FeatureFlag::GroupedTabs.override_enabled(true);
    App::test((), |mut app| async move {
        let (workspace, group_id) = setup_workspace_with_section(&mut app);
        let window_id = app.read(|ctx| workspace.window_id(ctx));
        let input_focus = app.focused_view_id(window_id);
        let is_collapsed = |app: &App| {
            workspace.read(app, |workspace, _| {
                workspace.tab_groups[&group_id].collapsed
            })
        };

        let header = hover_section_header(&workspace, group_id, &mut app);
        let presenter = render_scene(&workspace, &mut app);
        send_events(
            left_click(header.title, 1),
            &workspace,
            &presenter,
            &mut app,
        );
        // A single click still folds the section away.
        assert!(is_collapsed(&app));

        let presenter = render_scene(&workspace, &mut app);
        send_events(
            left_click(header.title, 2),
            &workspace,
            &presenter,
            &mut app,
        );
        assert!(
            !is_collapsed(&app),
            "the second click of a double-click must undo the first click's collapse"
        );
        assert_typing_renames_section(&workspace, group_id, input_focus, &mut app);
        assert!(!is_collapsed(&app));
    });
}

#[test]
fn clicking_rename_pane_in_the_tab_menu_keeps_its_editor_focused() {
    let _vertical_tabs = FeatureFlag::VerticalTabs.override_enabled(true);
    App::test((), |mut app| async move {
        let workspace = setup_workspace(&mut app);
        let window_id = app.read(|ctx| workspace.window_id(ctx));
        workspace.update(&mut app, |workspace, ctx| {
            workspace.vertical_tabs_panel_open = true;
            workspace.should_show_ai_assistant_warm_welcome = false;
            workspace.toggle_tab_right_click_menu(
                0,
                TabContextMenuAnchor::Pointer(vec2f(300., 100.)),
                ctx,
            );
        });

        click_menu_row(&workspace, &mut app, "Rename pane");

        let (editor_id, renaming) = workspace.read(&app, |workspace, _| {
            (
                workspace.pane_rename_editor.id(),
                workspace
                    .current_workspace_state
                    .is_any_pane_being_renamed(),
            )
        });
        assert!(renaming);
        assert_eq!(app.focused_view_id(window_id), Some(editor_id));
    });
}

#[test]
fn new_section_from_the_tab_menu_opens_its_name_editor_by_mouse_and_by_keyboard() {
    let _vertical_tabs = FeatureFlag::VerticalTabs.override_enabled(true);
    let _grouped_tabs = FeatureFlag::GroupedTabs.override_enabled(true);
    App::test((), |mut app| async move {
        let workspace = setup_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            workspace.vertical_tabs_panel_open = true;
            workspace.should_show_ai_assistant_warm_welcome = false;
            workspace.add_terminal_tab(false, ctx);
            workspace.focus_active_tab(ctx);
        });

        // The new section opens its name editor one step after the menu row's own action,
        // so the menu closing must not hand focus to the terminal in between.
        for (tab_index, by_mouse) in [(0, true), (1, false)] {
            workspace.update(&mut app, |workspace, ctx| {
                workspace.toggle_tab_right_click_menu(
                    tab_index,
                    TabContextMenuAnchor::Pointer(vec2f(300., 100.)),
                    ctx,
                );
            });
            if by_mouse {
                click_menu_row(&workspace, &mut app, "New section with session");
            } else {
                choose_menu_item(&workspace, &mut app, "New section with session");
            }
            let group_id = workspace.read(&app, |workspace, _| {
                workspace.tabs[tab_index]
                    .group_id
                    .expect("the tab moved into a new section")
            });
            assert_section_rename_is_open(&workspace, group_id, &app);
            workspace.update(&mut app, |workspace, ctx| {
                workspace.cancel_tab_group_rename(ctx);
            });
        }
    });
}

#[test]
fn creating_a_section_from_selected_sessions_opens_its_name_editor() {
    let _vertical_tabs = FeatureFlag::VerticalTabs.override_enabled(true);
    let _grouped_tabs = FeatureFlag::GroupedTabs.override_enabled(true);
    App::test((), |mut app| async move {
        let workspace = setup_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            workspace.vertical_tabs_panel_open = true;
            workspace.should_show_ai_assistant_warm_welcome = false;
            workspace.add_terminal_tab(false, ctx);
            for tab in &mut workspace.tabs {
                tab.in_multi_selection = true;
            }
            workspace.focus_active_tab(ctx);
            workspace.toggle_tab_selection_right_click_menu(
                0,
                TabContextMenuAnchor::Pointer(vec2f(300., 100.)),
                ctx,
            );
        });

        click_menu_row(&workspace, &mut app, "Create section from sessions");

        let group_id = workspace.read(&app, |workspace, _| {
            assert_eq!(workspace.tabs[0].group_id, workspace.tabs[1].group_id);
            workspace.tabs[0]
                .group_id
                .expect("the selected sessions moved into a new section")
        });
        assert_section_rename_is_open(&workspace, group_id, &app);
    });
}

#[test]
fn the_new_section_button_opens_the_new_sections_name_editor() {
    let _vertical_tabs = FeatureFlag::VerticalTabs.override_enabled(true);
    let _grouped_tabs = FeatureFlag::GroupedTabs.override_enabled(true);
    App::test((), |mut app| async move {
        let workspace = setup_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            workspace.handle_action(&WorkspaceAction::CreateNewTabGroup, ctx);
        });
        let group_id = workspace.read(&app, |workspace, _| {
            workspace.tabs[workspace.active_tab_index()]
                .group_id
                .expect("the new session starts a new section")
        });
        assert_section_rename_is_open(&workspace, group_id, &app);
    });
}

// ---- Renaming a tab ----

/// A two-tab workspace with the sidebar showing and the second tab active. Returns each
/// tab's input view, which is where focus belongs once a rename is done.
fn setup_workspace_with_two_tabs(app: &mut App) -> (ViewHandle<Workspace>, [Option<EntityId>; 2]) {
    let workspace = setup_workspace(app);
    // Enter reaches a rename editor through the editor's own key bindings.
    app.update(crate::editor::init);
    let window_id = app.read(|ctx| workspace.window_id(ctx));
    workspace.update(app, |workspace, ctx| {
        workspace.vertical_tabs_panel_open = true;
        workspace.should_show_ai_assistant_warm_welcome = false;
        workspace.add_terminal_tab(false, ctx);
    });
    let mut input_focus = [None; 2];
    for tab_index in [0, 1] {
        workspace.update(app, |workspace, ctx| {
            workspace.activate_tab(tab_index, ctx);
            workspace.focus_active_tab(ctx);
        });
        input_focus[tab_index] = app.focused_view_id(window_id);
    }
    (workspace, input_focus)
}

/// Hovers a tab's row in the sidebar and returns its centre.
fn hover_tab_row(workspace: &ViewHandle<Workspace>, tab_index: usize, app: &mut App) -> Vector2F {
    let presenter = render_scene(workspace, app);
    let row = presenter
        .borrow()
        .position_cache()
        .get_position(crate::tab::tab_position_id(tab_index))
        .expect("the tab's row must be rendered")
        .center();
    send_events(hover(row), workspace, &presenter, app);
    row
}

/// Opens the tab's menu from the "..." button that shows while its row is hovered.
fn open_tab_menu_from_kebab(workspace: &ViewHandle<Workspace>, tab_index: usize, app: &mut App) {
    hover_tab_row(workspace, tab_index, app);
    let presenter = render_scene(workspace, app);
    let buttons = presenter
        .borrow()
        .position_cache()
        .get_position(vertical_tabs::vtab_action_buttons_position_id(tab_index))
        .expect("the row's buttons must render on hover");
    // The "..." button is the first of the row's square buttons.
    let kebab = vec2f(
        buttons.origin_x() + buttons.height() / 2.,
        buttons.center().y(),
    );
    send_events(hover(kebab), workspace, &presenter, app);
    send_events(left_click(kebab, 1), workspace, &presenter, app);
}

fn open_tab_menu_by_right_click(
    workspace: &ViewHandle<Workspace>,
    tab_index: usize,
    app: &mut App,
) {
    let row = hover_tab_row(workspace, tab_index, app);
    let presenter = render_scene(workspace, app);
    send_events(
        [warpui::Event::RightMouseDown {
            position: row,
            cmd: false,
            shift: false,
            click_count: 1,
        }],
        workspace,
        &presenter,
        app,
    );
}

/// Checks the tab's name editor is open and focused, types `name`, presses Enter, and checks
/// the name sticks and focus goes back to the tab's input.
fn assert_typing_renames_tab(
    workspace: &ViewHandle<Workspace>,
    tab_index: usize,
    name: &str,
    input_focus: Option<EntityId>,
    app: &mut App,
) {
    let (renaming, editor_id, window_id) = workspace.read(app, |workspace, _| {
        (
            workspace.current_workspace_state.tab_being_renamed(),
            workspace.tab_rename_editor.id(),
            workspace.window_id,
        )
    });
    assert_eq!(
        renaming,
        Some(tab_index),
        "the tab's name editor must be open"
    );
    assert_eq!(
        app.focused_view_id(window_id),
        Some(editor_id),
        "the tab's name editor must hold focus, or the first key press is lost"
    );
    type_and_press_enter(workspace, app, name);
    let (title, renaming) = workspace.read(app, |workspace, ctx| {
        (
            workspace.tabs[tab_index]
                .pane_group
                .as_ref(ctx)
                .custom_title(ctx),
            workspace.current_workspace_state.is_tab_being_renamed(),
        )
    });
    assert_eq!(title.as_deref(), Some(name));
    assert!(!renaming);
    assert_eq!(
        app.focused_view_id(window_id),
        input_focus,
        "after a rename, typing must reach the tab's terminal again"
    );
}

#[test]
fn clicking_rename_tab_in_the_right_click_menu_renames_the_tab() {
    let _vertical_tabs = FeatureFlag::VerticalTabs.override_enabled(true);
    App::test((), |mut app| async move {
        let (workspace, input_focus) = setup_workspace_with_two_tabs(&mut app);
        // The active tab first, then one in the background.
        for (tab_index, name) in [(1, "Active"), (0, "Background")] {
            open_tab_menu_by_right_click(&workspace, tab_index, &mut app);
            workspace.read(&app, |workspace, _| {
                assert!(workspace.show_tab_right_click_menu.is_some());
            });
            click_menu_row(&workspace, &mut app, "Rename tab");
            assert_typing_renames_tab(
                &workspace,
                tab_index,
                name,
                input_focus[tab_index],
                &mut app,
            );
        }
    });
}

#[test]
fn clicking_rename_tab_in_the_kebab_menu_renames_the_tab() {
    let _vertical_tabs = FeatureFlag::VerticalTabs.override_enabled(true);
    App::test((), |mut app| async move {
        let (workspace, input_focus) = setup_workspace_with_two_tabs(&mut app);
        for (tab_index, name) in [(1, "Active"), (0, "Background")] {
            open_tab_menu_from_kebab(&workspace, tab_index, &mut app);
            workspace.read(&app, |workspace, _| {
                assert!(workspace.show_tab_right_click_menu.is_some());
            });
            click_menu_row(&workspace, &mut app, "Rename tab");
            assert_typing_renames_tab(
                &workspace,
                tab_index,
                name,
                input_focus[tab_index],
                &mut app,
            );
        }
    });
}

#[test]
fn double_clicking_a_tab_renames_it() {
    let _vertical_tabs = FeatureFlag::VerticalTabs.override_enabled(true);
    App::test((), |mut app| async move {
        let (workspace, input_focus) = setup_workspace_with_two_tabs(&mut app);
        for (tab_index, name) in [(1, "Active"), (0, "Background")] {
            let row = hover_tab_row(&workspace, tab_index, &mut app);
            let presenter = render_scene(&workspace, &mut app);
            send_events(left_click(row, 1), &workspace, &presenter, &mut app);
            let presenter = render_scene(&workspace, &mut app);
            send_events(left_click(row, 2), &workspace, &presenter, &mut app);
            assert_typing_renames_tab(
                &workspace,
                tab_index,
                name,
                input_focus[tab_index],
                &mut app,
            );
        }
    });
}

#[test]
fn rename_tab_from_the_keyboard_renames_the_active_tab() {
    let _vertical_tabs = FeatureFlag::VerticalTabs.override_enabled(true);
    App::test((), |mut app| async move {
        let (workspace, input_focus) = setup_workspace_with_two_tabs(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            workspace.handle_action(&WorkspaceAction::RenameActiveTab, ctx);
        });
        assert_typing_renames_tab(&workspace, 1, "Active", input_focus[1], &mut app);
    });
}

/// The view focus returns to once `tab_index` is shown. Leaves that tab active.
fn input_focus_of_tab(
    workspace: &ViewHandle<Workspace>,
    tab_index: usize,
    app: &mut App,
) -> Option<EntityId> {
    workspace.update(app, |workspace, ctx| {
        workspace.activate_tab(tab_index, ctx);
        workspace.focus_active_tab(ctx);
    });
    let window_id = app.read(|ctx| workspace.window_id(ctx));
    app.focused_view_id(window_id)
}

#[test]
fn renaming_a_background_tab_inside_a_section_by_menu_and_by_double_click() {
    let _vertical_tabs = FeatureFlag::VerticalTabs.override_enabled(true);
    let _grouped_tabs = FeatureFlag::GroupedTabs.override_enabled(true);
    App::test((), |mut app| async move {
        let (workspace, group_id) = setup_workspace_with_section(&mut app);
        let tab_index = workspace.read(&app, |workspace, _| {
            workspace
                .tabs
                .iter()
                .position(|tab| tab.group_id == Some(group_id))
                .expect("a tab sits in the section")
        });
        let other_tab = 1 - tab_index;
        let input_focus = input_focus_of_tab(&workspace, tab_index, &mut app);

        input_focus_of_tab(&workspace, other_tab, &mut app);
        open_tab_menu_by_right_click(&workspace, tab_index, &mut app);
        click_menu_row(&workspace, &mut app, "Rename tab");
        assert_typing_renames_tab(&workspace, tab_index, "From menu", input_focus, &mut app);

        input_focus_of_tab(&workspace, other_tab, &mut app);
        let row = hover_tab_row(&workspace, tab_index, &mut app);
        let presenter = render_scene(&workspace, &mut app);
        send_events(left_click(row, 1), &workspace, &presenter, &mut app);
        let presenter = render_scene(&workspace, &mut app);
        send_events(left_click(row, 2), &workspace, &presenter, &mut app);
        assert_typing_renames_tab(
            &workspace,
            tab_index,
            "By double-click",
            input_focus,
            &mut app,
        );
    });
}

#[test]
fn renaming_a_split_tab_from_its_menus() {
    let _vertical_tabs = FeatureFlag::VerticalTabs.override_enabled(true);
    App::test((), |mut app| async move {
        let (workspace, _) = setup_workspace_with_two_tabs(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            let group = workspace.active_tab_pane_group().clone();
            group.update(ctx, |group, ctx| {
                group.add_terminal_pane(Direction::Right, None, ctx);
            });
        });
        let input_focus = input_focus_of_tab(&workspace, 1, &mut app);

        open_tab_menu_by_right_click(&workspace, 1, &mut app);
        click_menu_row(&workspace, &mut app, "Rename tab");
        assert_typing_renames_tab(&workspace, 1, "Split", input_focus, &mut app);

        open_tab_menu_from_kebab(&workspace, 1, &mut app);
        click_menu_row(&workspace, &mut app, "Rename tab");
        assert_typing_renames_tab(&workspace, 1, "Split again", input_focus, &mut app);
    });
}
