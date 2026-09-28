use std::collections::HashSet;

use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::vec2f;
use warp_core::channel::{Channel, ChannelConfig, ChannelState};
use warp_core::AppId;
use warpui::elements::{ConstrainedBox, Draggable, Empty};
use warpui::keymap::Keystroke;
use warpui::platform::{OperatingSystem, WindowStyle};
use warpui::{
    App, AppContext, Element, Entity, Presenter, SingletonEntity, TypedActionView, View,
    WindowInvalidation,
};

use super::{
    active_index_after_removal, close_project_decision, next_project_index, previous_project_index,
    project_agent_hover_summary, render_project_agent_hover_card, CloseProjectDecision,
    ACTIVATE_NEXT_PROJECT_MAC_KEY_BINDING, ACTIVATE_PREVIOUS_PROJECT_MAC_KEY_BINDING,
    PROJECT_TAB_BORDER_WIDTH, PROJECT_TAB_VERTICAL_PADDING,
};
use crate::appearance::Appearance;
use crate::root_view::{NewWorkspaceSource, RootView};
use crate::terminal::CLIAgent;
use crate::util::bindings::{custom_tag_to_keystroke, trigger_to_keystroke, CustomAction};
use crate::workspace::view::{
    ProjectCliAgentActivity, ProjectCliAgentCounts, ProjectCliAgentSummary,
};
use crate::GlobalResourceHandles;

fn mock_project_window(app: &mut App) -> warpui::ViewHandle<super::ProjectWindow> {
    crate::workspace::view::tests::initialize_app(app);
    app.update(crate::root_view::init);
    ChannelState::set(ChannelState::new(
        Channel::Local,
        ChannelConfig::no_backend(AppId::new("test", "warp", "WarpTest"), "warp-test.log"),
    ));
    let resources = GlobalResourceHandles::mock(app);
    let (_, root) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        RootView::new(
            resources,
            NewWorkspaceSource::Empty {
                previous_active_window: None,
                shell: None,
            },
            ctx,
        )
    });
    root.read(app, |root, _| root.project_window()).unwrap()
}

#[test]
fn new_project_inherits_active_sidebar_width() {
    App::test((), |mut app| async move {
        let parent = mock_project_window(&mut app);
        parent.update(&mut app, |parent, ctx| {
            let first = parent.active_workspace();
            first.update(ctx, |workspace, _| {
                workspace.set_vertical_tabs_panel_width(390.)
            });
            parent.add_project(ctx);
            assert_eq!(
                parent
                    .active_workspace()
                    .as_ref(ctx)
                    .vertical_tabs_panel_width(),
                390.
            );
            parent.active_workspace().update(ctx, |workspace, _| {
                workspace.set_vertical_tabs_panel_width(460.)
            });
            parent.add_project(ctx);
            assert_eq!(
                parent
                    .active_workspace()
                    .as_ref(ctx)
                    .vertical_tabs_panel_width(),
                460.
            );
            assert_eq!(first.as_ref(ctx).vertical_tabs_panel_width(), 390.);
        });
    });
}

fn assert_sidebar_drag_handoff(single_tab_source: bool, has_tasks: bool) {
    App::test((), move |mut app| async move {
        let parent = mock_project_window(&mut app);
        parent.update(&mut app, |parent, ctx| {
            let source = parent.active_workspace();
            if !single_tab_source {
                source.update(ctx, |workspace, ctx| workspace.add_terminal_tab(false, ctx));
            }
            if has_tasks {
                source.update(ctx, |workspace, ctx| {
                    workspace.add_workspace_task("Keep this project task".into(), ctx);
                });
            }
            let transferred = source.read(ctx, |workspace, ctx| {
                workspace
                    .get_tab_transfer_info_for_attach(workspace.tab_count() - 1, ctx)
                    .unwrap()
            });
            let pane_group_id = transferred.pane_group.id();
            transferred
                .draggable_state
                .set_dragging(vec2f(300., 15.), vec2f(-30., -10.));
            let target_id = parent.add_project(ctx);
            let target = parent.active_workspace();
            assert!(parent.move_inner_tab_to_project(source.id(), pane_group_id, target_id, ctx));
            assert_eq!(parent.active_workspace().id(), target.id());
            let removes_source = single_tab_source && !has_tasks;
            assert_eq!(parent.projects.len(), if removes_source { 1 } else { 2 });
            assert_eq!(
                source.as_ref(ctx).tab_count(),
                if removes_source { 0 } else { 1 }
            );
            if has_tasks {
                assert_eq!(source.as_ref(ctx).workspace_tasks().len(), 1);
            }
            target.read(ctx, |workspace, ctx| {
                assert_eq!(workspace.tab_count(), 2);
                let moved = workspace.get_tab_transfer_info_for_attach(1, ctx).unwrap();
                assert_eq!(moved.pane_group.id(), pane_group_id);
                assert!(
                    moved.draggable_state.is_dragging(),
                    "drag must continue in destination sidebar"
                );
            });
            assert!(!parent.move_inner_tab_to_project(source.id(), pane_group_id, target_id, ctx));
            assert_eq!(
                target.as_ref(ctx).tab_count(),
                2,
                "stale hover events must not duplicate the tab"
            );
        });
    });
}

#[test]
fn sidebar_drag_moves_live_tab_into_existing_project() {
    assert_sidebar_drag_handoff(false, false);
}

#[test]
fn sidebar_drag_moves_only_tab_without_closing_its_session() {
    assert_sidebar_drag_handoff(true, false);
}

#[test]
fn sidebar_drag_keeps_source_project_tasks_when_moving_last_session() {
    assert_sidebar_drag_handoff(true, true);
}

#[test]
fn sidebar_drag_of_only_session_waits_for_sibling_project_target() {
    let _vertical_tabs_guard =
        warp_core::features::FeatureFlag::VerticalTabs.override_enabled(true);
    let _window_drag_guard =
        warp_core::features::FeatureFlag::DragTabsToWindows.override_enabled(true);
    App::test((), |mut app| async move {
        let parent = mock_project_window(&mut app);
        let (source, window_id) = parent.update(&mut app, |parent, ctx| {
            let source_id = parent.projects[0].id;
            let source = parent.active_workspace();
            parent.add_project(ctx);
            parent.activate_project(source_id, ctx);
            (source, parent.window_id)
        });
        let mut presenter = Presenter::new(window_id);
        app.update(|ctx| {
            presenter.invalidate(
                WindowInvalidation {
                    updated: HashSet::from([parent.id(), source.id()]),
                    ..Default::default()
                },
                ctx,
            );
            presenter.build_scene(vec2f(1200., 800.), 1., None, ctx);
        });
        source.update(&mut app, |workspace, ctx| {
            let bar = crate::workspace::view::tab_bar_rects_for_window(window_id, ctx)[0];
            let state = workspace
                .get_tab_transfer_info_for_attach(0, ctx)
                .unwrap()
                .draggable_state;
            for cursor in [bar.center(), vec2f(600., 400.)] {
                state.set_dragging(cursor, vec2f(-20., -8.));
                workspace.handle_action(
                    &crate::workspace::WorkspaceAction::DragTab {
                        tab_index: 0,
                        tab_position: RectF::new(cursor - vec2f(20., 8.), vec2f(40., 16.)),
                    },
                    ctx,
                );
                assert!(
                    !crate::workspace::cross_window_tab_drag::CrossWindowTabDrag::as_ref(ctx)
                        .is_active(),
                    "moving toward a sibling project must not detach the source window"
                );
                assert!(state.is_dragging());
            }
        });
    });
}

struct ProjectHoverCardDragLayoutView;

impl Entity for ProjectHoverCardDragLayoutView {
    type Event = ();
}

impl View for ProjectHoverCardDragLayoutView {
    fn ui_name() -> &'static str {
        "ProjectHoverCardDragLayoutView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let agents = [ProjectCliAgentSummary {
            agent: CLIAgent::Codex,
            title: "Agent with a title that needs flexible truncation".to_string(),
            activity: ProjectCliAgentActivity::Working,
        }];
        let hover_card = render_project_agent_hover_card(
            "clinch-terminal",
            2,
            &agents,
            ProjectCliAgentCounts {
                working: 1,
                ..Default::default()
            },
            Appearance::as_ref(app),
        );
        let tab = ConstrainedBox::new(Empty::new().finish())
            .with_width(120.)
            .with_height(32.)
            .finish();
        Draggable::new(Default::default(), tab)
            // Drag visuals are deliberately measured without parent bounds.
            .with_alternate_drag_element(hover_card)
            .finish()
    }
}

impl TypedActionView for ProjectHoverCardDragLayoutView {
    type Action = ();
}

#[test]
fn project_navigation_owns_command_brackets_on_mac() {
    App::test((), |mut app| async move {
        app.update(super::init);

        app.update(|ctx| {
            let previous = ctx
                .editable_bindings()
                .find(|binding| binding.name == "project_window:activate_previous_project")
                .and_then(|binding| trigger_to_keystroke(binding.trigger));
            let next = ctx
                .editable_bindings()
                .find(|binding| binding.name == "project_window:activate_next_project")
                .and_then(|binding| trigger_to_keystroke(binding.trigger));

            if OperatingSystem::get().is_mac() {
                assert_eq!(
                    previous,
                    Keystroke::parse(ACTIVATE_PREVIOUS_PROJECT_MAC_KEY_BINDING).ok()
                );
                assert_eq!(
                    next,
                    Keystroke::parse(ACTIVATE_NEXT_PROJECT_MAC_KEY_BINDING).ok()
                );
                assert_eq!(
                    custom_tag_to_keystroke(CustomAction::ActivatePreviousPane.into()),
                    None
                );
                assert_eq!(
                    custom_tag_to_keystroke(CustomAction::ActivateNextPane.into()),
                    None
                );
            } else {
                assert_eq!(previous, None);
                assert_eq!(next, None);
                assert_eq!(
                    custom_tag_to_keystroke(CustomAction::ActivatePreviousPane.into()),
                    Keystroke::parse("ctrl-shift-{").ok()
                );
                assert_eq!(
                    custom_tag_to_keystroke(CustomAction::ActivateNextPane.into()),
                    Keystroke::parse("ctrl-shift-}").ok()
                );
            }
        });
    });
}

#[test]
fn project_navigation_wraps_and_singletons_are_noops() {
    assert_eq!(previous_project_index(0, 3), Some(2));
    assert_eq!(previous_project_index(2, 3), Some(1));
    assert_eq!(next_project_index(2, 3), Some(0));
    assert_eq!(next_project_index(0, 3), Some(1));

    assert_eq!(previous_project_index(0, 1), None);
    assert_eq!(next_project_index(0, 1), None);
}

#[test]
fn project_agent_hover_summary_matches_project_badge_counts() {
    assert_eq!(
        project_agent_hover_summary(
            3,
            2,
            ProjectCliAgentCounts {
                working: 1,
                done: 1,
                running_commands: 2,
            },
        ),
        "3 open tabs · 2 agents · 1 working · 1 done · 2 commands running"
    );
    assert_eq!(
        project_agent_hover_summary(1, 0, ProjectCliAgentCounts::default()),
        "1 open tab"
    );
}

#[test]
fn closing_active_project_prefers_the_project_at_the_same_position() {
    assert_eq!(active_index_after_removal(1, 1, 2), Some(1));
    assert_eq!(active_index_after_removal(2, 2, 2), Some(1));
}

#[test]
fn removing_inactive_project_preserves_active_project_identity() {
    assert_eq!(active_index_after_removal(2, 0, 2), Some(1));
    assert_eq!(active_index_after_removal(0, 2, 2), Some(0));
    assert_eq!(active_index_after_removal(0, 0, 0), None);
}

#[test]
fn close_project_guard_distinguishes_missing_singleton_and_grouped_projects() {
    assert_eq!(
        close_project_decision(2, None),
        CloseProjectDecision::NotFound
    );
    assert_eq!(
        close_project_decision(1, Some(0)),
        CloseProjectDecision::CloseWindow
    );
    assert_eq!(
        close_project_decision(3, Some(1)),
        CloseProjectDecision::Project(1)
    );
}

/// Mirrors the tab strip's vertical composition inside the title bar. `Text`
/// drops a single-line label entirely (not just clips it) when the line height
/// exceeds its max-height constraint, so if this budget dips below one UI line
/// the project tabs render as blank pills. Note the strip's `ClippedScrollable`
/// must keep zero scrollbar gutter padding (see `render_project_tab_strip`) or
/// 4px silently disappear from this budget.
#[test]
fn project_tab_label_height_budget_fits_one_ui_line() {
    let label_budget = crate::workspace::view::TAB_BAR_HEIGHT
        - 2. * (PROJECT_TAB_VERTICAL_PADDING + PROJECT_TAB_BORDER_WIDTH);
    let ui_line_height = warp_core::ui::appearance::DEFAULT_UI_FONT_SIZE
        * warpui::elements::DEFAULT_UI_LINE_HEIGHT_RATIO;
    assert!(
        label_budget >= ui_line_height,
        "project tab label budget ({label_budget}px) no longer fits one UI line \
         ({ui_line_height}px); the labels will disappear entirely"
    );
}

#[test]
fn project_hover_card_layout_is_safe_as_unbounded_drag_visual() {
    App::test((), |mut app| async move {
        crate::workspace::view::tests::initialize_app(&mut app);
        let (window_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| {
            ProjectHoverCardDragLayoutView
        });
        let mut presenter = Presenter::new(window_id);
        let invalidation = WindowInvalidation {
            updated: HashSet::from([app
                .root_view_id(window_id)
                .expect("test window should have a root view")]),
            ..Default::default()
        };

        app.update(move |ctx| {
            presenter.invalidate(invalidation, ctx);
            presenter.build_scene(vec2f(500., 300.), 1., None, ctx);
        });
    });
}

#[test]
fn dragging_project_out_creates_window_without_losing_workspace() {
    App::test((), |mut app| async move {
        crate::workspace::view::tests::initialize_app(&mut app);
        app.update(crate::root_view::init);
        ChannelState::set(ChannelState::new(
            Channel::Local,
            ChannelConfig::no_backend(AppId::new("test", "warp", "WarpTest"), "warp-test.log"),
        ));

        let global_resource_handles = GlobalResourceHandles::mock(&mut app);
        let active_window_id = app.read(|ctx| ctx.windows().active_window());
        let (source_window_id, root_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            RootView::new(
                global_resource_handles,
                NewWorkspaceSource::Empty {
                    previous_active_window: active_window_id,
                    shell: None,
                },
                ctx,
            )
        });
        let project_window = root_view
            .read(&app, |root_view, _| root_view.project_window())
            .expect("backendless root view should contain a project window");
        project_window.update(&mut app, |project_window, ctx| {
            project_window.add_project(ctx);
            ctx.update_window_bounds(
                source_window_id,
                RectF::new(vec2f(100., 100.), vec2f(1200., 800.)),
            );
        });
        let (dragged_project_id, dragged_workspace_id) =
            project_window.read(&app, |project_window, _| {
                let projects = project_window.projects().collect::<Vec<_>>();
                assert_eq!(projects.len(), 2);
                (projects[1].0, projects[1].1.id())
            });

        project_window.update(&mut app, |project_window, ctx| {
            let last_position = RectF::new(vec2f(300., 250.), vec2f(140., 32.));
            // The test platform's ordered-window list is empty. Include the source
            // explicitly while its view is leased, as macOS does during a real drag.
            assert!(project_window
                .attach_target_in_windows(last_position.center(), [source_window_id], ctx)
                .is_none());
            project_window.active_drag = Some(super::ProjectDragState {
                id: dragged_project_id,
                original_index: 1,
                last_position,
                detached_from_source_strip: true,
                hover_target_window_id: None,
            });
            project_window.finish_project_drag(ctx);
        });

        app.read(|ctx| {
            let window_ids = ctx.window_ids().collect::<Vec<_>>();
            assert_eq!(window_ids.len(), 2);
            let target_window_id = *window_ids
                .iter()
                .find(|window_id| **window_id != source_window_id)
                .expect("detached project should create a target window");

            let source_projects = project_window.as_ref(ctx).projects().collect::<Vec<_>>();
            assert_eq!(source_projects.len(), 1);
            assert_ne!(source_projects[0].0, dragged_project_id);

            let target_root = ctx
                .root_view::<RootView>(target_window_id)
                .expect("target window should have a root view");
            let target_project_window = target_root
                .as_ref(ctx)
                .project_window()
                .expect("target root should contain projects");
            let target_projects = target_project_window
                .as_ref(ctx)
                .projects()
                .collect::<Vec<_>>();
            assert_eq!(target_projects.len(), 1);
            assert_eq!(target_projects[0].0, dragged_project_id);
            assert_eq!(target_projects[0].1.id(), dragged_workspace_id);
        });
    });
}
