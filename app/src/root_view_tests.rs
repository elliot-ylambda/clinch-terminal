use std::path::PathBuf;

use settings::Setting;
use warp_core::channel::{Channel, ChannelConfig, ChannelState};
use warp_core::user_preferences::GetUserPreferences as _;
use warp_core::AppId;
use warpui::platform::WindowStyle;
use warpui::{App, SingletonEntity, TypedActionView};

use super::{
    has_completed_local_onboarding, AuthOnboardingState, GlobalResourceHandles, NewWorkspaceSource,
    RootView, HAS_COMPLETED_ONBOARDING_KEY,
};
use crate::auth::auth_manager::AuthManager;
use crate::auth::AuthStateProvider;
use crate::launch_configs::launch_config::{
    LaunchConfig, PaneMode, PaneTemplateType, ProjectWindowTemplate, TabTemplate, WindowTemplate,
};
use crate::server::server_api::ServerApiProvider;
use crate::server::telemetry::LaunchConfigUiLocation;
use crate::workspace::WorkspaceAction;

fn initialize_app(app: &mut App) {
    app.update(crate::settings::init_and_register_user_preferences);
    app.add_singleton_model(|_ctx| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AuthManager::new_for_test);
}

fn set_local_onboarding_completed(app: &mut App, completed: bool) {
    app.update(|ctx| {
        ctx.private_user_preferences()
            .write_value(
                HAS_COMPLETED_ONBOARDING_KEY,
                serde_json::to_string(&completed).unwrap(),
            )
            .unwrap();
    });
}

/// Regression test for the bug fixed by introducing
/// `RootView::sync_local_onboarding_to_server`: when a user completed onboarding
/// pre-login and later authenticated via a non-login-slide entrypoint (i.e. while
/// already in `Terminal` state), the server-side `is_onboarded` flag was never
/// flipped. The helper runs unconditionally on `AuthComplete` and must flip the
/// flag when all preconditions hold.
#[test]
fn test_sync_flips_server_is_onboarded_when_local_onboarding_completed() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Seed the "has_completed_local_onboarding" preference and make the user
        // appear not yet onboarded on the server. The default test user is
        // non-anonymous, so the guards in the helper won't short-circuit.
        set_local_onboarding_completed(&mut app, true);
        app.update(|ctx| {
            AuthStateProvider::as_ref(ctx).get().set_is_onboarded(false);
            assert!(has_completed_local_onboarding(ctx));
            assert_eq!(
                AuthStateProvider::as_ref(ctx).get().is_onboarded(),
                Some(false)
            );
        });

        app.update(|ctx| {
            let auth_state = AuthStateProvider::as_ref(ctx).get().clone();
            RootView::sync_local_onboarding_to_server(&auth_state, ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                AuthStateProvider::as_ref(ctx).get().is_onboarded(),
                Some(true),
                "sync should have invoked AuthManager::set_user_onboarded"
            );
        });
    });
}

/// If the user hasn't completed local onboarding, the helper must leave the
/// server-side flag untouched — onboarding hasn't actually happened yet.
#[test]
fn test_sync_noop_when_local_onboarding_not_completed() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Do not set HAS_COMPLETED_ONBOARDING_KEY; it defaults to false.
        app.update(|ctx| {
            AuthStateProvider::as_ref(ctx).get().set_is_onboarded(false);
        });

        app.update(|ctx| {
            let auth_state = AuthStateProvider::as_ref(ctx).get().clone();
            RootView::sync_local_onboarding_to_server(&auth_state, ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                AuthStateProvider::as_ref(ctx).get().is_onboarded(),
                Some(false),
                "sync should not have changed is_onboarded when local onboarding is incomplete"
            );
        });
    });
}

/// The server-side flag should also be left untouched when it is already set,
/// even if local onboarding is complete — avoids redundant server calls.
#[test]
fn test_sync_noop_when_already_onboarded_on_server() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        set_local_onboarding_completed(&mut app, true);
        app.update(|ctx| {
            // User::test() defaults to is_onboarded = true; assert that and
            // leave it in place.
            assert_eq!(
                AuthStateProvider::as_ref(ctx).get().is_onboarded(),
                Some(true)
            );
        });

        app.update(|ctx| {
            let auth_state = AuthStateProvider::as_ref(ctx).get().clone();
            RootView::sync_local_onboarding_to_server(&auth_state, ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                AuthStateProvider::as_ref(ctx).get().is_onboarded(),
                Some(true)
            );
        });
    });
}

/// Backendless fork builds (Clinch stable/local, constructed via
/// `ChannelConfig::no_backend`) must launch straight into a blank terminal in
/// every window: no onboarding slides, no login slide, no auth screen. This
/// must hold even if stale `AuthState` would otherwise report the user as
/// logged in, since the backendless branch is checked first in
/// `RootView::new`. It also must hold across `log_out` (e.g. triggered by a
/// stale-auth-state CLI or Settings logout): `AuthOnboardingState::log_out`'s
/// `Terminal` arm is gated the same way, so it stays in `Terminal` instead of
/// re-entering the `Auth` screen.
///
/// Note: this test mutates the process-global `ChannelState` via
/// `ChannelState::set`, relying on `cargo nextest`'s process-per-test
/// isolation (the test runner mandated by this repo) to avoid bleeding into
/// other tests. Do not run this test under plain `cargo test` alongside other
/// tests that read `ChannelState`.
#[test]
fn test_backendless_build_launches_directly_into_terminal() {
    App::test((), |mut app| async move {
        // Reuse the full Workspace test scaffolding: RootView::new creates a
        // Workspace immediately in the backendless branch, which needs all of
        // the singleton models registered there.
        crate::workspace::view::tests::initialize_app(&mut app);

        ChannelState::set(ChannelState::new(
            Channel::Local,
            ChannelConfig::no_backend(AppId::new("test", "warp", "WarpTest"), "warp-test.log"),
        ));

        // `AuthStateProvider::new_for_test` (registered above) seeds
        // credentials, so `auth_state.is_logged_in()` is already `true` here —
        // exactly the "stale auth state" case the backendless branch must
        // still win against, since it's checked first in `RootView::new`.
        app.read(|ctx| {
            assert!(AuthStateProvider::as_ref(ctx).get().is_logged_in());
        });

        let global_resource_handles = GlobalResourceHandles::mock(&mut app);
        let active_window_id = app.read(|ctx| ctx.windows().active_window());
        let (_, root_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            RootView::new(
                global_resource_handles,
                NewWorkspaceSource::Empty {
                    previous_active_window: active_window_id,
                    shell: None,
                },
                ctx,
            )
        });

        app.read(|ctx| {
            root_view.read(ctx, |view, _ctx| {
                assert!(
                    matches!(view.auth_onboarding_state, AuthOnboardingState::Terminal(_)),
                    "backendless builds must launch straight into the workspace"
                );
            });
        });

        // Simulate a logout (e.g. from Settings or the CLI, possibly triggered
        // by stale AuthState) and verify it does not re-enter the Auth screen.
        app.update(|ctx| {
            root_view.update(ctx, |view, ctx| {
                view.log_out(&(), ctx);
            });
        });

        app.read(|ctx| {
            root_view.read(ctx, |view, _ctx| {
                assert!(
                    matches!(view.auth_onboarding_state, AuthOnboardingState::Terminal(_)),
                    "backendless builds must stay in the workspace after log_out"
                );
            });
        });
    });
}

#[test]
fn test_closing_only_tab_in_project_preserves_sibling_project() {
    App::test((), |mut app| async move {
        crate::workspace::view::tests::initialize_app(&mut app);
        app.update(super::init);

        ChannelState::set(ChannelState::new(
            Channel::Local,
            ChannelConfig::no_backend(AppId::new("test", "warp", "WarpTest"), "warp-test.log"),
        ));
        crate::terminal::general_settings::GeneralSettings::handle(&app).update(
            &mut app,
            |settings, ctx| {
                settings
                    .show_warning_before_quitting
                    .set_value(false, ctx)
                    .expect("failed to disable quit warning");
            },
        );

        let global_resource_handles = GlobalResourceHandles::mock(&mut app);
        let active_window_id = app.read(|ctx| ctx.windows().active_window());
        let (window_id, root_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
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
        });
        let (surviving_project_id, closing_project_id, closing_workspace) =
            project_window.read(&app, |project_window, _| {
                let projects = project_window.projects().collect::<Vec<_>>();
                assert_eq!(projects.len(), 2);
                (projects[0].0, projects[1].0, projects[1].1.clone())
            });

        // This update deliberately exercises the circular-borrow boundary: the
        // close path must not ask ProjectWindow to update this workspace again.
        closing_workspace.update(&mut app, |workspace, ctx| {
            assert_eq!(workspace.tab_count(), 1);
            workspace.handle_action(&WorkspaceAction::CloseTab(0), ctx);
        });

        app.read(|ctx| {
            assert!(ctx.is_window_open(window_id));
            project_window.read(ctx, |project_window, _| {
                let project_ids = project_window
                    .projects()
                    .map(|(project_id, _)| project_id)
                    .collect::<Vec<_>>();
                assert_eq!(project_ids, vec![surviving_project_id]);
                assert!(!project_ids.contains(&closing_project_id));
                assert_eq!(project_window.active_project_index(), 0);
            });
        });
    });
}

#[test]
fn test_moving_inner_tab_to_new_project_preserves_live_pane_group() {
    App::test((), |mut app| async move {
        initialize_backendless_workspace_app(&mut app);

        let global_resource_handles = GlobalResourceHandles::mock(&mut app);
        let active_window_id = app.read(|ctx| ctx.windows().active_window());
        let (window_id, root_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
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
        let source_workspace =
            project_window.read(&app, |project_window, _| project_window.active_workspace());

        let transferred_pane_group_id = source_workspace.update(&mut app, |workspace, ctx| {
            workspace.add_terminal_tab(false, ctx);
            assert_eq!(workspace.tab_count(), 2);
            let pane_group_id = workspace.active_tab_pane_group().id();
            assert!(workspace.move_tab_to_new_project(1, None, ctx));
            pane_group_id
        });

        app.read(|ctx| {
            assert_eq!(ctx.window_ids().collect::<Vec<_>>(), vec![window_id]);
            let projects = project_window
                .as_ref(ctx)
                .projects()
                .map(|(_, workspace)| workspace.clone())
                .collect::<Vec<_>>();
            assert_eq!(projects.len(), 2);
            assert_eq!(projects[0].as_ref(ctx).tab_count(), 1);
            assert_eq!(projects[1].as_ref(ctx).tab_count(), 1);
            assert_eq!(
                projects[1].as_ref(ctx).active_tab_pane_group().id(),
                transferred_pane_group_id
            );
            assert_eq!(project_window.as_ref(ctx).active_project_index(), 1);
        });
    });
}

#[test]
fn test_project_close_with_long_running_process_waits_for_confirmation() {
    App::test((), |mut app| async move {
        crate::workspace::view::tests::initialize_app(&mut app);
        app.update(super::init);

        ChannelState::set(ChannelState::new(
            Channel::Local,
            ChannelConfig::no_backend(AppId::new("test", "warp", "WarpTest"), "warp-test.log"),
        ));

        let global_resource_handles = GlobalResourceHandles::mock(&mut app);
        let active_window_id = app.read(|ctx| ctx.windows().active_window());
        let (window_id, root_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
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
        let (project_id, workspace) = project_window.read(&app, |project_window, _| {
            project_window
                .projects()
                .next()
                .map(|(project_id, workspace)| (project_id, workspace.clone()))
                .expect("project window should contain a project")
        });
        let terminal = workspace.read(&app, |workspace, ctx| {
            workspace
                .active_tab_pane_group()
                .as_ref(ctx)
                .focused_session_view(ctx)
                .expect("project should contain a terminal")
        });
        terminal
            .read(&app, |terminal, _| terminal.model.clone())
            .lock()
            .simulate_long_running_block("sleep 10", "running");

        let can_close_immediately = workspace.update(&mut app, |workspace, ctx| {
            workspace.request_project_close(project_id, ctx)
        });

        assert!(!can_close_immediately);
        app.read(|ctx| {
            assert!(ctx.is_window_open(window_id));
            assert_eq!(project_window.as_ref(ctx).projects().count(), 1);
        });
    });
}

fn launch_project_template(cwd: &str) -> WindowTemplate {
    WindowTemplate {
        active_tab_index: Some(0),
        tabs: vec![TabTemplate {
            title: None,
            layout: PaneTemplateType::PaneTemplate {
                cwd: PathBuf::from(cwd),
                commands: Vec::new(),
                is_focused: Some(true),
                pane_mode: PaneMode::Terminal,
                shell: None,
            },
            color: None,
        }],
    }
}

fn grouped_launch_config() -> LaunchConfig {
    LaunchConfig {
        name: "Grouped".to_string(),
        active_window_index: Some(0),
        windows: vec![ProjectWindowTemplate::grouped(
            vec![
                launch_project_template("/project/one"),
                launch_project_template("/project/two"),
            ],
            Some(1),
        )],
    }
}

fn initialize_backendless_workspace_app(app: &mut App) {
    crate::workspace::view::tests::initialize_app(app);
    app.update(super::init);
    ChannelState::set(ChannelState::new(
        Channel::Local,
        ChannelConfig::no_backend(AppId::new("test", "warp", "WarpTest"), "warp-test.log"),
    ));
}

#[test]
fn grouped_launch_config_opens_projects_in_one_window() {
    App::test((), |mut app| async move {
        initialize_backendless_workspace_app(&mut app);

        app.update(|ctx| {
            super::open_launch_config(
                &super::OpenLaunchConfigArg {
                    launch_config: grouped_launch_config(),
                    ui_location: LaunchConfigUiLocation::Uri,
                    open_in_active_window: false,
                },
                ctx,
            );
        });

        app.read(|ctx| {
            let window_ids = ctx.window_ids().collect::<Vec<_>>();
            assert_eq!(window_ids.len(), 1);
            let root_view = ctx
                .root_view::<RootView>(window_ids[0])
                .expect("launch config window should have a root view");
            let project_window = root_view
                .as_ref(ctx)
                .project_window()
                .expect("launch config window should have projects");
            let project_window = project_window.as_ref(ctx);
            assert_eq!(project_window.projects().count(), 2);
            assert_eq!(project_window.active_project_index(), 1);
        });
    });
}

#[test]
fn grouped_launch_config_can_open_in_active_window() {
    App::test((), |mut app| async move {
        initialize_backendless_workspace_app(&mut app);

        let global_resource_handles = GlobalResourceHandles::mock(&mut app);
        let active_window_id = app.read(|ctx| ctx.windows().active_window());
        let (window_id, root_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            RootView::new(
                global_resource_handles,
                NewWorkspaceSource::Empty {
                    previous_active_window: active_window_id,
                    shell: None,
                },
                ctx,
            )
        });
        let active_workspace = root_view
            .read(&app, |root_view, ctx| root_view.workspace_view(ctx))
            .expect("root view should contain an active workspace");
        let launch_config = grouped_launch_config();

        app.update(|ctx| {
            assert!(super::open_launch_config_in_active_window(
                &launch_config.windows[0],
                window_id,
                &active_workspace,
                ctx,
            ));
        });

        app.read(|ctx| {
            let window_ids = ctx.window_ids().collect::<Vec<_>>();
            assert_eq!(window_ids.len(), 1);
            let root_view = ctx
                .root_view::<RootView>(window_ids[0])
                .expect("active window should have a root view");
            let project_window = root_view
                .as_ref(ctx)
                .project_window()
                .expect("active window should have projects");
            let project_window = project_window.as_ref(ctx);
            assert_eq!(project_window.projects().count(), 2);
            assert_eq!(project_window.active_project_index(), 1);
        });
    });
}
