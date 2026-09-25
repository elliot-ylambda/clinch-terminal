use warp_core::channel::{Channel, ChannelConfig};
use warp_core::AppId;
use warpui::App;

use super::*;

#[derive(Default)]
struct CapturedEvents(Vec<Option<SessionId>>);

impl Entity for CapturedEvents {
    type Event = ();
}

// ChannelState is process-global; nextest isolates this test from other channels.
#[test]
fn backend_free_ssh_bootstraps_without_extension_even_with_feature_override() {
    let _flag = FeatureFlag::SshRemoteServer.override_enabled(true);
    ChannelState::set(ChannelState::new(
        Channel::Local,
        ChannelConfig::no_backend(
            AppId::new("test", "clinch", "ClinchTest"),
            "clinch-test.log",
        ),
    ));

    App::test((), |mut app| async move {
        let sessions = app.add_model(|_| Sessions::new_for_test());
        let (_sender, receiver) = async_channel::unbounded();
        let dispatcher = app.add_model(|ctx| ModelEventDispatcher::new(receiver, sessions, ctx));
        let captured = app.add_model(|_| CapturedEvents::default());
        captured.update(&mut app, |_, ctx| {
            ctx.subscribe_to_model(&dispatcher, |captured, _, event, _| {
                captured.0.push(match event {
                    ModelEvent::Handler(AnsiHandlerEvent::InitShell {
                        pending_session_info,
                    }) => Some(pending_session_info.session_id),
                    _ => None,
                });
            });
        });

        let mut info = SessionInfo::new_for_test();
        info.is_ssh_wrapper_session = IsSSHWrapperSession::Yes {
            socket_path: "/tmp/test-ssh.sock".into(),
            external_control_master: false,
        };
        let session_id = info.session_id;
        dispatcher.update(&mut app, |dispatcher, ctx| {
            dispatcher.handle_terminal_model_event(
                Event::Handler(HandlerEvent::InitShell {
                    pending_session_info: Box::new(info),
                }),
                ctx,
            );
        });

        captured.read(&app, |captured, _| {
            assert_eq!(captured.0, [Some(session_id)]);
        });
    });
}
