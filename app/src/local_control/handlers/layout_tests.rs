use ::local_control::protocol::{
    ColorValueParams, RenameParams, TabCreateParams, TabSelector, TabTarget, TabType,
    TargetSelector, WindowSelector, WindowTarget,
};
use ::local_control::{Action, ActionKind, ErrorCode, InstanceId};
use warp_core::ui::theme::AnsiColorIdentifier;
use warpui::platform::WindowStyle;
use warpui::App;

use super::{create_tab, tab_create_launch, TerminalLaunch};
use crate::local_control::handlers::metadata_config::{tab_color_set, tab_rename};
use crate::local_control::LocalControlBridge;
use crate::project_window::ProjectWindow;
use crate::root_view::NewWorkspaceSource;
use crate::workspace::view::tests::{initialize_app, mock_workspace};
use crate::GlobalResourceHandles;

#[test]
fn tab_create_handler_adds_and_activates_terminal_tab() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);
        let previous_count = workspace.read(&app, |workspace, _| workspace.tab_count());
        let bridge = app.add_singleton_model(LocalControlBridge::new);
        let instance_id = InstanceId("inst_test".to_owned());

        let response = bridge.update(&mut app, |bridge, ctx| {
            bridge.set_instance_id(instance_id.clone());
            create_tab(
                &Some(instance_id.clone()),
                &serde_json::json!({}),
                &TargetSelector::default(),
                None,
                ctx,
            )
            .expect("tab.create handler succeeds")
        });

        workspace.read(&app, |workspace, _| {
            assert_eq!(workspace.tab_count(), previous_count + 1);
            assert_eq!(workspace.active_tab_index(), previous_count);
        });
        assert_eq!(response["action"], "tab.create");
        assert_eq!(response["created"], true);
        assert_eq!(response["instance_id"], "inst_test");
        assert_eq!(response["tab"]["previous_count"], previous_count);
        assert_eq!(response["tab"]["count"], previous_count + 1);
        assert_eq!(response["tab"]["active_index"], previous_count);
        assert!(response["tab"]["id"].is_string());
    });
}

#[test]
fn tab_create_handler_registers_a_startup_command_in_an_explicit_cwd() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);
        let previous_count = workspace.read(&app, |workspace, _| workspace.tab_count());
        let bridge = app.add_singleton_model(LocalControlBridge::new);
        let instance_id = InstanceId("inst_test".to_owned());
        let cwd = tempfile::tempdir().expect("temporary cwd");

        let response = bridge.update(&mut app, |bridge, ctx| {
            bridge.set_instance_id(instance_id.clone());
            create_tab(
                &Some(instance_id),
                &serde_json::json!({
                    "cwd": cwd.path().to_string_lossy(),
                    "command": ["true"]
                }),
                &TargetSelector::default(),
                None,
                ctx,
            )
            .expect("tab.create startup command succeeds")
        });

        workspace.read(&app, |workspace, _| {
            assert_eq!(workspace.tab_count(), previous_count + 1);
            assert_eq!(workspace.active_tab_index(), previous_count);
        });
        assert_eq!(response["created"], true);
        assert!(response["tab"]["id"].is_string());
    });
}

#[cfg(feature = "local_tty")]
#[test]
fn tab_create_uses_origin_terminal_project_unless_window_is_explicit() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let global_resource_handles = GlobalResourceHandles::mock(&mut app);
        let previous_active_window = app.read(|ctx| ctx.windows().active_window());
        let sources = (0..2)
            .map(|_| NewWorkspaceSource::Empty {
                previous_active_window,
                shell: None,
            })
            .collect();
        let (window_id, project_window) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            ProjectWindow::new(global_resource_handles, None, sources, 1, ctx)
        });
        let (origin_workspace, active_workspace) =
            project_window.read(&app, |project_window, _| {
                let projects = project_window.projects().collect::<Vec<_>>();
                (projects[0].1.clone(), projects[1].1.clone())
            });
        let origin_terminal_session_uuid = origin_workspace.read(&app, |workspace, ctx| {
            let pane_group = workspace
                .get_pane_group_view(workspace.active_tab_index())
                .expect("origin project has an active tab");
            let terminal_view_id = pane_group
                .as_ref(ctx)
                .focused_session_view(ctx)
                .expect("origin project has a terminal session")
                .id();
            let bytes = pane_group
                .as_ref(ctx)
                .agent_session_uuid_for_terminal_view(terminal_view_id, ctx)
                .expect("origin terminal has a durable UUID");
            uuid::Uuid::from_slice(&bytes).expect("terminal session UUID is valid")
        });
        let origin_count = origin_workspace.read(&app, |workspace, _| workspace.tab_count());
        let active_count = active_workspace.read(&app, |workspace, _| workspace.tab_count());
        let bridge = app.add_singleton_model(LocalControlBridge::new);
        let instance_id = InstanceId("inst_test".to_owned());

        let response = bridge.update(&mut app, |bridge, ctx| {
            bridge.set_instance_id(instance_id.clone());
            create_tab(
                &Some(instance_id.clone()),
                &serde_json::json!({}),
                &TargetSelector::default(),
                Some(&origin_terminal_session_uuid),
                ctx,
            )
            .expect("origin-targeted tab.create succeeds")
        });

        assert_eq!(response["window"]["id"], window_id.to_string());
        assert_eq!(
            origin_workspace.read(&app, |workspace, _| workspace.tab_count()),
            origin_count + 1
        );
        assert_eq!(
            active_workspace.read(&app, |workspace, _| workspace.tab_count()),
            active_count
        );

        let created_tab_id = response["tab"]["id"]
            .as_str()
            .expect("tab.create returns an opaque tab id")
            .to_owned();
        let created_tab_target = TargetSelector {
            window: Some(WindowTarget::Id {
                id: WindowSelector(window_id.to_string()),
            }),
            tab: Some(TabTarget::Id {
                id: TabSelector(created_tab_id.clone()),
            }),
            ..TargetSelector::default()
        };
        let rename = Action::with_params(
            ActionKind::TabRename,
            RenameParams {
                title: "Origin project process".to_owned(),
            },
        )
        .expect("rename action serializes");
        let rename_response = bridge.update(&mut app, |_, ctx| {
            tab_rename(
                &Some(instance_id.clone()),
                &created_tab_target,
                &rename,
                ctx,
            )
            .expect("exact created tab remains addressable while its project is inactive")
        });
        assert_eq!(rename_response["tab_id"], created_tab_id);

        let color = Action::with_params(
            ActionKind::TabColorSet,
            ColorValueParams {
                color: "red".to_owned(),
            },
        )
        .expect("color action serializes");
        bridge.update(&mut app, |_, ctx| {
            tab_color_set(&Some(instance_id.clone()), &created_tab_target, &color, ctx)
                .expect("exact created tab color targets its inactive project")
        });
        let created_tab_index = response["tab"]["active_index"]
            .as_u64()
            .and_then(|index| usize::try_from(index).ok())
            .expect("tab.create returns a valid active index");
        assert_eq!(
            origin_workspace.read(&app, |workspace, _| {
                workspace.get_tab_color(created_tab_index)
            }),
            Some(AnsiColorIdentifier::Red)
        );

        let explicit_window_target = TargetSelector {
            window: Some(WindowTarget::Id {
                id: WindowSelector(window_id.to_string()),
            }),
            ..TargetSelector::default()
        };
        bridge.update(&mut app, |_, ctx| {
            create_tab(
                &Some(instance_id),
                &serde_json::json!({}),
                &explicit_window_target,
                Some(&origin_terminal_session_uuid),
                ctx,
            )
            .expect("explicit window overrides origin project")
        });
        assert_eq!(
            active_workspace.read(&app, |workspace, _| workspace.tab_count()),
            active_count + 1
        );

        let stale_origin = uuid::Uuid::nil();
        let error = bridge.update(&mut app, |_, ctx| {
            create_tab(
                &None,
                &serde_json::json!({}),
                &TargetSelector::default(),
                Some(&stale_origin),
                ctx,
            )
            .expect_err("stale origin must not fall back to the active project")
        });
        assert_eq!(error.code, ErrorCode::StaleTarget);
    });
}

#[test]
fn tab_create_launch_requires_and_preserves_an_explicit_directory() {
    let cwd = tempfile::tempdir().expect("temporary cwd");
    let launch = tab_create_launch(&TabCreateParams {
        cwd: Some(cwd.path().to_string_lossy().into_owned()),
        command: vec!["npm".to_owned(), "run".to_owned(), "dev server".to_owned()],
        ..TabCreateParams::default()
    })
    .expect("valid terminal launch")
    .expect("launch options are present");

    assert_eq!(
        launch,
        TerminalLaunch {
            cwd: cwd.path().to_path_buf(),
            command: Some("npm run 'dev server'".to_owned()),
        }
    );
}

#[test]
fn tab_create_launch_rejects_a_command_without_cwd() {
    let error = tab_create_launch(&TabCreateParams {
        command: vec!["npm".to_owned(), "run".to_owned(), "dev".to_owned()],
        ..TabCreateParams::default()
    })
    .expect_err("cwd is required for startup commands");

    assert_eq!(error.code, ErrorCode::InvalidParams);
}

#[test]
fn tab_create_launch_rejects_non_terminal_tab_types() {
    let cwd = tempfile::tempdir().expect("temporary cwd");
    let error = tab_create_launch(&TabCreateParams {
        tab_type: Some(TabType::Agent),
        cwd: Some(cwd.path().to_string_lossy().into_owned()),
        ..TabCreateParams::default()
    })
    .expect_err("agent tabs cannot use terminal launch options");

    assert_eq!(error.code, ErrorCode::InvalidParams);
}
