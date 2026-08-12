use ::local_control::protocol::{TabCreateParams, TabType, TargetSelector};
use ::local_control::{ErrorCode, InstanceId};
use warpui::App;

use super::{create_tab, tab_create_launch, TerminalLaunch};
use crate::local_control::LocalControlBridge;
use crate::workspace::view::tests::{initialize_app, mock_workspace};

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
