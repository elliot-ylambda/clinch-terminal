use std::cell::RefCell;
use std::rc::Rc;

use warpui::{App, SingletonEntity, TypedActionView as _};

use super::*;
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputState, CLIAgentSession, CLIAgentSessionContext, CLIAgentSessionStatus,
    CLIAgentSessionsModel,
};
use crate::terminal::CLIAgent;
use crate::test_util::add_window_with_terminal;
use crate::test_util::terminal::initialize_app_for_terminal_view;

#[derive(Debug, PartialEq, Eq)]
enum ObservedEvent {
    WriteToPty(String),
    SubmitTextToCliAgent(String),
}

#[test]
fn custom_insert_writes_command_in_terminal_and_submits_text_to_cli_agent() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let footer = terminal.read(&app, |view, ctx| {
            view.input().as_ref(ctx).agent_input_footer().clone()
        });

        let observed = Rc::new(RefCell::new(Vec::new()));
        let observed_for_subscription = observed.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&footer, move |_, event, _| match event {
                AgentInputFooterEvent::WriteToPty(text) => observed_for_subscription
                    .borrow_mut()
                    .push(ObservedEvent::WriteToPty(text.clone())),
                AgentInputFooterEvent::SubmitTextToCliAgent(text) => observed_for_subscription
                    .borrow_mut()
                    .push(ObservedEvent::SubmitTextToCliAgent(text.clone())),
                _ => {}
            });
        });

        footer.update(&mut app, |footer, ctx| {
            footer.handle_action(
                &AgentInputFooterAction::InsertCustomText("git status".to_owned()),
                ctx,
            );
        });
        assert_eq!(
            *observed.borrow(),
            vec![ObservedEvent::WriteToPty("git status\n".to_owned())]
        );

        observed.borrow_mut().clear();
        CLIAgentSessionsModel::handle(&app).update(&mut app, |sessions, ctx| {
            sessions.set_session(
                terminal.id(),
                CLIAgentSession {
                    agent: CLIAgent::Codex,
                    status: CLIAgentSessionStatus::InProgress,
                    session_context: CLIAgentSessionContext::default(),
                    input_state: CLIAgentInputState::Closed,
                    listener: None,
                    plugin_version: None,
                    remote_host: None,
                    draft_text: None,
                    custom_command_prefix: None,
                    received_rich_notification: false,
                    has_observed_turn_activity: false,
                    turn_interrupted_by_user: false,
                    prompt_history: Default::default(),
                    prompt_history_load_state: Default::default(),
                    prompt_history_generation: 0,
                    should_auto_toggle_input: false,
                },
                ctx,
            );
        });

        footer.update(&mut app, |footer, ctx| {
            footer.handle_action(
                &AgentInputFooterAction::InsertCustomText("review this".to_owned()),
                ctx,
            );
        });
        assert_eq!(
            *observed.borrow(),
            vec![ObservedEvent::SubmitTextToCliAgent(
                "review this".to_owned()
            )]
        );
    });
}
