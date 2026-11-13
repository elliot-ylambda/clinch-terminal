use std::cell::{Cell, RefCell};
use std::rc::Rc;

use chrono::TimeZone as _;
use warpui::{App, ViewHandle};

use super::*;
use crate::agent_resume::AgentResumeProvider;
use crate::terminal::cli_agent_sessions::event::CLIAgentStopReason;
use crate::terminal::cli_agent_sessions::tests::idle_test_session;
use crate::terminal::cli_agent_sessions::{CLIAgentSession, CLIAgentSessionKey};
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

const RESUME_COMMAND: &str = "clinch_agent_resume_launch claude conversation-1";

/// A Claude Code session sitting idle at its prompt after a finished turn.
fn idle_session() -> CLIAgentSession {
    let mut session = idle_test_session(CLIAgent::Claude);
    session.status = CLIAgentSessionStatus::Success;
    session.has_observed_turn_activity = true;
    session.session_context.session_id = Some("conversation-1".into());
    session
}

/// Puts `session` in the pane with its agent running in the foreground, and returns what keeps it
/// from sleeping. The session arrives through the model, so the pane hears about it first.
fn blocker_for(
    app: &mut App,
    terminal: &ViewHandle<TerminalView>,
    session: CLIAgentSession,
) -> Option<SleepBlocker> {
    terminal.update(app, |view, ctx| {
        view.model.lock().simulate_long_running_block("claude", "");
        CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
            sessions.set_session(view.view_id, session, ctx);
        });
    });
    terminal.update(app, |view, ctx| {
        // The session did not just start: that rule has its own check below.
        view.agent_session_started_at = None;
        view.agent_sleep_blocker(ctx)
    })
}

#[test]
fn an_idle_agent_may_sleep_and_anything_in_use_stays_awake() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.update(crate::remote_control::register);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.read(&app, |view, ctx| {
            assert_eq!(view.agent_sleep_blocker(ctx), Some(SleepBlocker::NoAgent));
        });
        assert_eq!(blocker_for(&mut app, &terminal, idle_session()), None);

        let mut working = idle_session();
        working.status = CLIAgentSessionStatus::InProgress;
        assert_eq!(
            blocker_for(&mut app, &terminal, working),
            Some(SleepBlocker::Working)
        );

        let mut asking = idle_session();
        asking.status = CLIAgentSessionStatus::Blocked { message: None };
        assert_eq!(
            blocker_for(&mut app, &terminal, asking),
            Some(SleepBlocker::WaitingOnYou)
        );

        let mut limited = idle_session();
        limited.session_context.stop_reason =
            Some(CLIAgentStopReason::UsageLimit { stated_reset: None });
        assert_eq!(
            blocker_for(&mut app, &terminal, limited),
            Some(SleepBlocker::RateLimited)
        );

        let mut shell = idle_session();
        shell.session_context.screen_background_work.shells = 1;
        assert_eq!(
            blocker_for(&mut app, &terminal, shell),
            Some(SleepBlocker::BackgroundWork)
        );
        let mut monitor = idle_session();
        monitor.session_context.screen_background_work.monitors = 1;
        assert_eq!(
            blocker_for(&mut app, &terminal, monitor),
            Some(SleepBlocker::BackgroundWork)
        );
        let mut subagent = idle_session();
        subagent
            .session_context
            .active_subagent_ids
            .insert("subagent-1".into());
        assert_eq!(
            blocker_for(&mut app, &terminal, subagent),
            Some(SleepBlocker::BackgroundWork)
        );

        let mut draft = idle_session();
        draft.draft_text = Some("half a message".into());
        assert_eq!(
            blocker_for(&mut app, &terminal, draft),
            Some(SleepBlocker::Draft)
        );

        let mut unidentified = idle_session();
        unidentified.session_context.session_id = None;
        assert_eq!(
            blocker_for(&mut app, &terminal, unidentified),
            Some(SleepBlocker::NoSavedSession)
        );

        // Without hooks and with nothing readable on screen, nobody can say it is idle.
        let mut unreadable = idle_session();
        unreadable.received_rich_notification = false;
        assert_eq!(
            blocker_for(&mut app, &terminal, unreadable),
            Some(SleepBlocker::StatusUnknown)
        );

        let mut remote = idle_session();
        remote.remote_host = Some("user@host".into());
        assert_eq!(
            blocker_for(&mut app, &terminal, remote),
            Some(SleepBlocker::NoAgent)
        );
    });
}

#[test]
fn a_queued_message_a_recent_start_and_a_missing_agent_keep_a_pane_awake() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.update(crate::remote_control::register);
        let terminal = add_window_with_terminal(&mut app, None);
        assert_eq!(blocker_for(&mut app, &terminal, idle_session()), None);

        terminal.update(&mut app, |view, ctx| {
            // Delivery is holding a coordinator message for this agent.
            view.note_queued_agent_message();
            assert_eq!(
                view.agent_sleep_blocker(ctx),
                Some(SleepBlocker::QueuedMessage)
            );
            view.agent_message_queued_at = Instant::now().checked_sub(QUEUED_MESSAGE_HOLD);
            assert_eq!(view.agent_sleep_blocker(ctx), None);

            // An agent that only just started may still be loading.
            view.agent_session_started_at = Some(Instant::now());
            let blocker = view.agent_sleep_blocker(ctx);
            assert_eq!(blocker, Some(SleepBlocker::StartedRecently));
            assert!(blocker.unwrap().yields_to_direct_request());
            view.agent_session_started_at = Instant::now().checked_sub(RECENT_START);
            assert_eq!(view.agent_sleep_blocker(ctx), None);
        });

        // The agent has exited: only a shell is left, and a shell is not put to sleep.
        let shell = add_window_with_terminal(&mut app, None);
        shell.update(&mut app, |view, ctx| {
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(view.view_id, idle_session(), ctx);
            });
        });
        shell.update(&mut app, |view, ctx| {
            view.agent_session_started_at = None;
            assert_eq!(view.agent_sleep_blocker(ctx), Some(SleepBlocker::NoAgent));
        });
    });
}

/// The whole of a sleep and a wake as the pane sees it: the agent is stopped, the session stays
/// with its conversation, and waking runs the resume command for that same conversation once.
#[test]
fn sleep_keeps_the_conversation_and_waking_resumes_it() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.update(crate::remote_control::register);
        let terminal = add_window_with_terminal(&mut app, None);
        assert_eq!(blocker_for(&mut app, &terminal, idle_session()), None);
        let stopped = Rc::new(RefCell::new(Vec::new()));
        let seen = stopped.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if let Event::AgentStoppedForSleep { resume_command, .. } = event {
                    seen.borrow_mut().push(resume_command.clone());
                }
            })
        });

        terminal.update(&mut app, |view, ctx| {
            view.begin_agent_sleep(RESUME_COMMAND.to_owned(), ctx);
            assert_eq!(
                view.local_control_agent_readiness(ctx).1,
                Some("going_to_sleep"),
                "nothing may be typed into an agent that is being stopped"
            );
            assert_eq!(
                view.agent_sleep_blocker(ctx),
                Some(SleepBlocker::AlreadyAsleep)
            );
            assert!(
                !view.agent_is_asleep(),
                "not asleep until the agent has gone"
            );

            // The agent exits. An exit Clinch asked for keeps the session.
            assert!(view.agent_exited_for_sleep(ctx));
        });
        assert_eq!(*stopped.borrow(), vec![RESUME_COMMAND.to_owned()]);
        let session = app
            .read(|ctx| {
                CLIAgentSessionsModel::as_ref(ctx)
                    .session(terminal.id())
                    .cloned()
            })
            .expect("the session stays with the pane");
        assert_eq!(
            session.session_key(),
            Some(CLIAgentSessionKey {
                provider: AgentResumeProvider::Claude,
                session_id: "conversation-1".into()
            })
        );
        assert!(
            !session.received_rich_notification && !session.has_observed_turn_activity,
            "nothing the stopped process reported is trusted after it"
        );

        // The pane group arms the resume command and hands the pane to the queue.
        let starts = Rc::new(Cell::new(0));
        let counter = starts.clone();
        let since = chrono::Utc::now();
        terminal.update(&mut app, |view, _| {
            view.defer_agent_start(Box::new(move |_| counter.set(counter.get() + 1)));
        });
        AgentRestoreQueue::handle(&app).update(&mut app, |queue, ctx| {
            queue.put_to_sleep(
                &terminal,
                b"pane".to_vec(),
                session.session_key().unwrap(),
                since,
                Box::new(|_| false),
                Rc::new(|_| {}),
                ctx,
            );
        });
        terminal.update(&mut app, |view, ctx| {
            assert_eq!(view.agent_asleep_since(), Some(since));
            assert_eq!(
                view.local_control_agent_readiness(ctx).1,
                Some("not_started")
            );
            view.mark_as_visible(ctx);
        });
        assert_eq!(starts.get(), 0, "an asleep pane on screen stays asleep");

        // A key typed into it wakes it, and is not mistaken for the user taking the pane over.
        terminal.update(&mut app, |view, ctx| {
            view.mark_user_input_for_test();
            assert!(view.agent_sleep_saw_typing(ctx));
            // More keys while it comes up are dropped too, and start nothing more.
            view.mark_user_input_for_test();
            assert!(view.agent_sleep_saw_typing(ctx));
            assert!(!view.agent_restore_has_user_input());
            assert_eq!(
                view.agent_sleep().map(|sleep| sleep.phase.clone()),
                Some(AgentSleepPhase::Waking)
            );
            assert!(!view.wake_agent(ctx));
        });
        assert_eq!(starts.get(), 1, "the resume command runs exactly once");
        let conversation = app.read(|ctx| {
            CLIAgentSessionsModel::as_ref(ctx)
                .session(terminal.id())
                .and_then(|session| session.session_key())
                .map(|key| key.session_id)
        });
        assert_eq!(conversation.as_deref(), Some("conversation-1"));

        terminal.update(&mut app, |view, ctx| {
            view.agent_wake_settled(ctx);
            assert!(view.agent_sleep().is_none());
        });
    });
}

#[test]
fn an_agent_that_does_not_stop_is_left_running_and_the_pane_says_so() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.update(crate::remote_control::register);
        let terminal = add_window_with_terminal(&mut app, None);
        assert_eq!(blocker_for(&mut app, &terminal, idle_session()), None);
        let failures = Rc::new(Cell::new(0));
        let seen = failures.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if matches!(event, Event::AgentSleepFailed) {
                    seen.set(seen.get() + 1);
                }
            })
        });
        terminal.update(&mut app, |view, ctx| {
            // An exit while nothing is being stopped is the user closing the agent.
            assert!(!view.agent_exited_for_sleep(ctx));

            view.begin_agent_sleep(RESUME_COMMAND.to_owned(), ctx);
            let generation = view.agent_sleep_generation;
            // A timer left over from an earlier attempt does nothing.
            view.give_up_stopping_agent(generation.wrapping_sub(1), ctx);
            assert!(view.agent_sleep().is_some());
            view.give_up_stopping_agent(generation, ctx);
            assert!(view.agent_sleep().is_none());
            assert_eq!(view.agent_sleep_blocker(ctx), None, "it can be tried again");
        });
        assert_eq!(failures.get(), 1);
    });
}

#[test]
fn the_card_says_what_is_happening_in_plain_words() {
    let now = Local.with_ymd_and_hms(2026, 11, 13, 16, 30, 0).unwrap();
    let today = Local
        .with_ymd_and_hms(2026, 11, 13, 14, 14, 0)
        .unwrap()
        .with_timezone(&Utc);
    let last_week = Local
        .with_ymd_and_hms(2026, 11, 7, 15, 8, 0)
        .unwrap()
        .with_timezone(&Utc);
    assert_eq!(asleep_since_label(today, now), "2:14 PM");
    assert_eq!(asleep_since_label(last_week, now), "Nov 7, 3:08 PM");

    let card = |phase| {
        sleep_card_copy(
            &AgentSleep {
                since: today,
                phase,
            },
            now,
        )
    };
    let (title, body) = card(AgentSleepPhase::Asleep);
    assert_eq!(title, "This session is asleep");
    assert!(body.starts_with("Asleep since 2:14 PM."), "{body}");
    assert_eq!(card(AgentSleepPhase::Waking).0, "Waking up…");
    assert_eq!(
        card(AgentSleepPhase::Stopping {
            resume_command: String::new()
        })
        .0,
        "Going to sleep…"
    );
}

#[test]
fn every_reason_has_its_own_name_and_explanation() {
    let all = [
        SleepBlocker::NoAgent,
        SleepBlocker::AlreadyAsleep,
        SleepBlocker::Restoring,
        SleepBlocker::NoSavedSession,
        SleepBlocker::Working,
        SleepBlocker::WaitingOnYou,
        SleepBlocker::RateLimited,
        SleepBlocker::BackgroundWork,
        SleepBlocker::Draft,
        SleepBlocker::QueuedMessage,
        SleepBlocker::InUseElsewhere,
        SleepBlocker::StatusUnknown,
        SleepBlocker::StartedRecently,
        SleepBlocker::Visible,
        SleepBlocker::Coordinator,
    ];
    let codes: std::collections::HashSet<_> = all.iter().map(|blocker| blocker.code()).collect();
    let explanations: std::collections::HashSet<_> =
        all.iter().map(|blocker| blocker.explanation()).collect();
    assert_eq!(codes.len(), all.len());
    assert_eq!(explanations.len(), all.len());
    // Only the two rules that guard against a surprise give way to the user asking by name.
    let soft: Vec<_> = all
        .iter()
        .filter(|blocker| blocker.yields_to_direct_request())
        .collect();
    assert_eq!(
        soft,
        vec![&SleepBlocker::StartedRecently, &SleepBlocker::Visible]
    );
}
