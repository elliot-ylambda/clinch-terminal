use warp_core::channel::{Channel, ChannelConfig, ChannelState};
use warp_core::AppId;
use warpui::platform::WindowStyle;
use warpui::App;

use super::*;
use crate::root_view::NewWorkspaceSource;
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputState, CLIAgentSession, CLIAgentSessionContext,
};
use crate::GlobalResourceHandles;

#[test]
fn discovers_inactive_projects_and_rejects_replaced_conversations() {
    App::test((), |mut app| async move {
        crate::workspace::view::tests::initialize_app(&mut app);
        app.update(crate::root_view::init);
        ChannelState::set(ChannelState::new(
            Channel::Local,
            ChannelConfig::no_backend(AppId::new("test", "warp", "WarpTest"), "warp-test.log"),
        ));
        let resources = GlobalResourceHandles::mock(&mut app);
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
        let project_window = root.read(&app, |root, _| root.project_window()).unwrap();
        project_window.update(&mut app, |window, ctx| window.add_project(ctx));
        let terminals = project_window.read(&app, |window, ctx| {
            window
                .projects()
                .map(|(_, workspace)| {
                    workspace
                        .as_ref(ctx)
                        .active_tab_pane_group()
                        .as_ref(ctx)
                        .terminal_views(ctx)[0]
                        .clone()
                })
                .collect::<Vec<_>>()
        });
        for (index, terminal) in terminals.iter().enumerate() {
            app.update(|ctx| {
                CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                    sessions.set_session(
                        terminal.id(),
                        CLIAgentSession {
                            agent: if index == 0 {
                                CLIAgent::Claude
                            } else {
                                CLIAgent::Codex
                            },
                            status: CLIAgentSessionStatus::Success,
                            session_context: CLIAgentSessionContext {
                                session_id: Some(format!("conversation-{index}")),
                                ..Default::default()
                            },
                            input_state: CLIAgentInputState::Closed,
                            should_auto_toggle_input: false,
                            listener: None,
                            plugin_version: None,
                            remote_host: None,
                            draft_text: None,
                            custom_command_prefix: None,
                            received_rich_notification: true,
                            has_observed_turn_activity: true,
                            turn_interrupted_by_user: false,
                            prompt_history: Default::default(),
                            prompt_history_load_state: Default::default(),
                            prompt_history_generation: 0,
                        },
                        ctx,
                    )
                })
            });
        }
        let instance = InstanceId("test-instance".into());
        let before = project_window.read(&app, |window, _| window.active_project_index());
        let snapshot = app.read(|ctx| snapshot(&instance, &AgentScope::default(), ctx).unwrap());
        assert_eq!(snapshot.agents.len(), 2);
        assert_eq!(
            snapshot.tree["windows"][0]["projects"][0]["tabs"][0]["panes"][0]["state"],
            "turn_complete"
        );
        assert_eq!(
            snapshot.tree["windows"][0]["projects"][0]["tabs"][0]["panes"][0]["conversation_id"],
            "conversation-0"
        );
        assert_eq!(
            snapshot.tree["windows"][0]["projects"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let first = snapshot.agents[0].data["agent_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let project_id = snapshot.agents[0].data["project_id"]
            .as_str()
            .unwrap()
            .to_owned();
        app.read(|ctx| {
            assert!(resolve(&instance, &first, ctx).is_ok());
            assert!(resolve(&InstanceId("other-instance".into()), &first, ctx).is_err());
            let filtered = super::snapshot(
                &instance,
                &AgentScope {
                    projects: vec![project_id],
                    sections: vec![],
                },
                ctx,
            )
            .unwrap();
            assert_eq!(filtered.agents.len(), 1);
            assert!(super::snapshot(
                &instance,
                &AgentScope {
                    projects: vec!["missing".into()],
                    sections: vec![]
                },
                ctx
            )
            .is_err());
        });
        assert_eq!(
            before,
            project_window.read(&app, |window, _| window.active_project_index())
        );
        app.update(|ctx| {
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                let mut replacement = sessions.session(terminals[0].id()).unwrap().clone();
                replacement.session_context.session_id = Some("replacement-conversation".into());
                sessions.set_session(terminals[0].id(), replacement, ctx);
            })
        });
        app.read(|ctx| assert!(resolve(&instance, &first, ctx).is_err()));
    });
}

#[cfg(unix)]
#[test]
fn codex_launch_preserves_pane_hooks_and_supports_older_cli_without_relaunching() {
    use std::os::unix::fs::PermissionsExt as _;

    use ::local_control::agents::AgentProvider;
    for supports_local in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("codex");
        let log = dir.path().join("args");
        std::fs::write(&executable, format!(
            "#!/bin/sh\nif [ \"$1\" = --help ]; then printf '%s\\n' '{}'; exit 0; fi\nprintf '%s\\n' \"$@\" >> \"$CLINCH_LAUNCH_TEST_LOG\"\nexit 7\n",
            if supports_local { "Options: --no-daemon" } else { "Options: --model" }
        )).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let prompt = "literal ' quote; $(touch injected) `touch injected`";
        let status = command::blocking::Command::new("sh")
            .arg("-c")
            .arg(provider_launch_command(AgentProvider::Codex, Some(prompt)))
            .current_dir(dir.path())
            .env("PATH", format!("{}:/usr/bin:/bin", dir.path().display()))
            .env("CLINCH_LAUNCH_TEST_LOG", &log)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(7));
        let expected = if supports_local {
            format!("--no-daemon\n--\n{prompt}\n")
        } else {
            format!("--\n{prompt}\n")
        };
        assert_eq!(std::fs::read_to_string(&log).unwrap(), expected);
        assert!(!dir.path().join("injected").exists());
    }
}
