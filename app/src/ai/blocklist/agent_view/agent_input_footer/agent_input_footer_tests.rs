use std::cell::RefCell;
use std::rc::Rc;

use warpui::{App, SingletonEntity, TypedActionView as _};

use super::*;
use crate::ai::blocklist::{InputConfig, InputType};
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputEntrypoint, CLIAgentInputState, CLIAgentSession, CLIAgentSessionContext,
    CLIAgentSessionStatus, CLIAgentSessionsModel,
};
use crate::terminal::CLIAgent;
use crate::test_util::add_window_with_terminal;
use crate::test_util::terminal::initialize_app_for_terminal_view;

#[derive(Debug, PartialEq, Eq)]
enum ObservedEvent {
    WriteToPty(String),
    SubmitTextToCliAgent(String),
    InsertIntoCLIRichInput(String),
    CopyAndClearDraft,
}

fn codex_session(session_id: Option<&str>) -> CLIAgentSession {
    CLIAgentSession {
        agent: CLIAgent::Codex,
        status: CLIAgentSessionStatus::InProgress,
        session_context: CLIAgentSessionContext {
            session_id: session_id.map(str::to_owned),
            ..CLIAgentSessionContext::default()
        },
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
    }
}

#[test]
fn conversation_actions_keep_their_slots_while_session_identity_is_pending() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let footer = terminal.read(&app, |view, ctx| {
            view.input().as_ref(ctx).agent_input_footer().clone()
        });
        let bookmark_button = footer.read(&app, |footer, _| {
            footer.bookmark_conversation_button.clone()
        });

        assert!(footer.read(&app, |footer, ctx| {
            footer
                .render_cli_toolbar_item(
                    &AgentToolbarItemKind::BookmarkConversation,
                    &SharedSessionStatus::NotShared,
                    false,
                    ctx,
                )
                .is_some()
        }));
        assert!(bookmark_button.read(&app, |button, _| button.is_disabled()));

        CLIAgentSessionsModel::handle(&app).update(&mut app, |sessions, ctx| {
            sessions.set_session(terminal.id(), codex_session(None), ctx);
        });
        assert!(footer.read(&app, |footer, ctx| {
            footer
                .render_cli_toolbar_item(
                    &AgentToolbarItemKind::TransferAgent,
                    &SharedSessionStatus::NotShared,
                    false,
                    ctx,
                )
                .is_some()
        }));
        assert!(bookmark_button.read(&app, |button, _| button.is_disabled()));

        CLIAgentSessionsModel::handle(&app).update(&mut app, |sessions, ctx| {
            sessions.set_session(terminal.id(), codex_session(Some("codex-session")), ctx);
        });
        assert!(!bookmark_button.read(&app, |button, _| button.is_disabled()));
    });
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
                AgentInputFooterEvent::InsertIntoCLIRichInput(text) => observed_for_subscription
                    .borrow_mut()
                    .push(ObservedEvent::InsertIntoCLIRichInput(text.clone())),
                AgentInputFooterEvent::CopyAndClearDraft => observed_for_subscription
                    .borrow_mut()
                    .push(ObservedEvent::CopyAndClearDraft),
                _ => {}
            });
        });

        footer.update(&mut app, |footer, ctx| {
            footer.handle_action(
                &AgentInputFooterAction::InsertCustomText {
                    text: "git status".to_owned(),
                    auto_send: true,
                },
                ctx,
            );
        });
        assert_eq!(
            *observed.borrow(),
            vec![ObservedEvent::WriteToPty("git status\n".to_owned())]
        );

        observed.borrow_mut().clear();
        footer.update(&mut app, |footer, ctx| {
            footer.handle_action(
                &AgentInputFooterAction::InsertCustomText {
                    text: "git status".to_owned(),
                    auto_send: false,
                },
                ctx,
            );
        });
        assert_eq!(
            *observed.borrow(),
            vec![ObservedEvent::WriteToPty("git status".to_owned())]
        );

        observed.borrow_mut().clear();
        CLIAgentSessionsModel::handle(&app).update(&mut app, |sessions, ctx| {
            sessions.set_session(terminal.id(), codex_session(None), ctx);
        });

        footer.update(&mut app, |footer, ctx| {
            footer.handle_action(
                &AgentInputFooterAction::InsertCustomText {
                    text: "review this".to_owned(),
                    auto_send: true,
                },
                ctx,
            );
        });
        assert_eq!(
            *observed.borrow(),
            vec![ObservedEvent::SubmitTextToCliAgent(
                "review this".to_owned()
            )]
        );

        observed.borrow_mut().clear();
        footer.update(&mut app, |footer, ctx| {
            for action in [
                AgentInputFooterAction::Compact,
                AgentInputFooterAction::SendContinue,
                AgentInputFooterAction::SendLooksGood,
                AgentInputFooterAction::CopyAndClearDraft,
            ] {
                footer.handle_action(&action, ctx);
            }
        });
        assert_eq!(
            *observed.borrow(),
            vec![
                ObservedEvent::SubmitTextToCliAgent("/compact".to_owned()),
                ObservedEvent::SubmitTextToCliAgent("Continue".to_owned()),
                ObservedEvent::SubmitTextToCliAgent("Looks good to me, continue".to_owned()),
                ObservedEvent::CopyAndClearDraft,
            ]
        );

        observed.borrow_mut().clear();
        footer.update(&mut app, |footer, ctx| {
            footer.handle_action(
                &AgentInputFooterAction::InsertCustomText {
                    text: "review this".to_owned(),
                    auto_send: false,
                },
                ctx,
            );
        });
        assert_eq!(
            *observed.borrow(),
            vec![ObservedEvent::WriteToPty("review this".to_owned())]
        );

        observed.borrow_mut().clear();
        CLIAgentSessionsModel::handle(&app).update(&mut app, |sessions, ctx| {
            sessions.open_input(
                terminal.id(),
                CLIAgentInputEntrypoint::CtrlG,
                InputConfig {
                    input_type: InputType::Shell,
                    is_locked: false,
                },
                false,
                false,
                ctx,
            );
        });
        footer.update(&mut app, |footer, ctx| {
            footer.handle_action(
                &AgentInputFooterAction::InsertCustomText {
                    text: "review in rich input".to_owned(),
                    auto_send: false,
                },
                ctx,
            );
        });
        assert_eq!(
            *observed.borrow(),
            vec![ObservedEvent::InsertIntoCLIRichInput(
                "review in rich input".to_owned()
            )]
        );
    });
}
