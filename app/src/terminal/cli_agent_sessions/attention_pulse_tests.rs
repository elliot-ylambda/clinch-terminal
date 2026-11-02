use warpui::{App, EntityId, SingletonEntity};

use super::AgentAttentionPulse;
use crate::terminal::cli_agent_sessions::tests::idle_test_session;
use crate::terminal::cli_agent_sessions::{CLIAgentSessionStatus, CLIAgentSessionsModel};
use crate::terminal::CLIAgent;

fn set_status(app: &mut App, terminal: EntityId, status: CLIAgentSessionStatus) {
    CLIAgentSessionsModel::handle(app).update(app, |model, ctx| {
        let mut session = idle_test_session(CLIAgent::Codex);
        session.has_observed_turn_activity = true;
        session.status = status;
        model.set_session(terminal, session, ctx);
    });
}

#[test]
fn pulse_stays_idle_and_lit_while_nothing_is_blocked() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let pulse = app.add_singleton_model(AgentAttentionPulse::new);

        set_status(&mut app, EntityId::new(), CLIAgentSessionStatus::InProgress);

        pulse.read(&app, |pulse, _| {
            assert!(!pulse.ticking);
            assert!(pulse.is_lit());
        });
    });
}

#[test]
fn pulse_blinks_while_a_session_is_blocked_and_settles_lit_afterwards() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let pulse = app.add_singleton_model(AgentAttentionPulse::new);
        let terminal = EntityId::new();

        set_status(
            &mut app,
            terminal,
            CLIAgentSessionStatus::Blocked {
                message: Some("Red or blue?".to_owned()),
            },
        );
        pulse.read(&app, |pulse, _| {
            assert!(pulse.ticking);
            assert!(pulse.is_lit(), "a new block starts visible");
        });

        pulse.update(&mut app, |pulse, ctx| pulse.tick(ctx));
        pulse.read(&app, |pulse, _| assert!(!pulse.is_lit()));
        pulse.update(&mut app, |pulse, ctx| pulse.tick(ctx));
        pulse.read(&app, |pulse, _| assert!(pulse.is_lit()));
        pulse.update(&mut app, |pulse, ctx| pulse.tick(ctx));
        pulse.read(&app, |pulse, _| assert!(!pulse.is_lit()));

        // Answering the question unblocks the session: the next tick stops the clock and
        // restores the lit phase so the next block does not start dimmed.
        set_status(&mut app, terminal, CLIAgentSessionStatus::InProgress);
        pulse.update(&mut app, |pulse, ctx| pulse.tick(ctx));
        pulse.read(&app, |pulse, _| {
            assert!(!pulse.ticking);
            assert!(pulse.is_lit());
        });
    });
}
