use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use warpui::windowing::state::ApplicationStage;
use warpui::windowing::WindowManager;
use warpui::App;

use super::*;
use crate::terminal::cli_agent_sessions::tests::idle_test_session;
use crate::terminal::cli_agent_sessions::{CLIAgentSessionStatus, CLIAgentSessionsModel};
use crate::terminal::CLIAgent;
use crate::workspace::view::tests::{initialize_app, mock_workspace};
use crate::workspace::view::PanesLayout;

/// Adds a background tab with a Claude Code agent sitting idle in it.
fn add_idle_agent_tab(
    workspace: &mut Workspace,
    ctx: &mut ViewContext<Workspace>,
) -> (usize, ViewHandle<TerminalView>) {
    let group = workspace.add_tab_with_pane_layout_placed(
        PanesLayout::SingleTerminal(Box::default()),
        Arc::new(HashMap::new()),
        None,
        Some((true, None)),
        None,
        ctx,
    );
    let terminal = group.as_ref(ctx).active_session_view(ctx).unwrap();
    terminal.update(ctx, |view, _| {
        view.model.lock().simulate_long_running_block("claude", "");
    });
    CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
        let mut session = idle_test_session(CLIAgent::Claude);
        session.status = CLIAgentSessionStatus::Success;
        session.has_observed_turn_activity = true;
        session.session_context.session_id = Some(format!("conversation-{}", terminal.id()));
        sessions.set_session(terminal.id(), session, ctx);
    });
    let index = workspace
        .tabs
        .iter()
        .position(|tab| tab.pane_group.id() == group.id())
        .unwrap();
    (index, terminal)
}

fn mark_window_active(ctx: &mut ViewContext<Workspace>) {
    let window_id = ctx.window_id();
    WindowManager::handle(ctx).update(ctx, |state, _| {
        state.overwrite_for_test(ApplicationStage::Active, Some(window_id));
    });
}

#[test]
fn the_coordinator_and_the_tab_on_screen_never_sleep_on_their_own() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);
        let tabs = workspace.update(&mut app, |workspace, ctx| {
            mark_window_active(ctx);
            let coordinator = add_idle_agent_tab(workspace, ctx);
            let on_screen = add_idle_agent_tab(workspace, ctx);
            let background = add_idle_agent_tab(workspace, ctx);
            workspace.tabs[coordinator.0].coordinator = true;
            workspace.activate_tab(on_screen.0, ctx);
            (coordinator, on_screen, background)
        });
        // The sessions arrive through the model; the panes hear about them between updates.
        workspace.update(&mut app, |workspace, ctx| {
            let ((coordinator, _), (on_screen, _), (background, _)) = tabs.clone();
            // "Sleep idle sessions" and the idle timer ask without naming a tab.
            assert_eq!(
                workspace
                    .sleep_tab(coordinator, false, None, ctx)
                    .kept_awake,
                vec![SleepBlocker::Coordinator]
            );
            assert_eq!(
                workspace.sleep_tab(on_screen, false, None, ctx).kept_awake,
                vec![SleepBlocker::StartedRecently]
            );
            for (_, terminal) in [&tabs.0, &tabs.1, &tabs.2] {
                terminal.update(ctx, |view, _| view.forget_recent_agent_start_for_test());
            }
            assert_eq!(
                workspace.sleep_tab(on_screen, false, None, ctx).kept_awake,
                vec![SleepBlocker::Visible]
            );
            // Even the tab's own Sleep command leaves the Coordinator running.
            assert_eq!(
                workspace.sleep_tab(coordinator, true, None, ctx).kept_awake,
                vec![SleepBlocker::Coordinator]
            );

            // A background tab, and the tab on screen when the user asks for it by name, get as
            // far as stopping the agent. In a test there is no saved session to come back to, so
            // that is where they stop: every rule about the tab itself has passed.
            assert_eq!(
                workspace.sleep_tab(background, false, None, ctx).kept_awake,
                vec![SleepBlocker::NoSavedSession]
            );
            assert_eq!(
                workspace.sleep_tab(on_screen, true, None, ctx).kept_awake,
                vec![SleepBlocker::NoSavedSession]
            );
        });
    });
}

#[test]
fn automatic_sleep_waits_for_the_idle_time_and_restarts_it_for_a_session_in_use() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);
        let (idle, busy) = workspace.update(&mut app, |workspace, ctx| {
            mark_window_active(ctx);
            (
                add_idle_agent_tab(workspace, ctx),
                add_idle_agent_tab(workspace, ctx),
            )
        });
        workspace.update(&mut app, |workspace, ctx| {
            for (_, terminal) in [&idle, &busy] {
                terminal.update(ctx, |view, _| view.forget_recent_agent_start_for_test());
            }
            // Used a moment ago: not a candidate yet, so nothing is even attempted.
            let two_hours = idle_sleep_threshold(2).unwrap();
            assert_eq!(
                workspace.sleep_idle_tabs(Some(two_hours), ctx),
                SleepSummary::default()
            );

            // Both have sat for three hours, but one of them is working.
            for (_, terminal) in [&idle, &busy] {
                terminal.update(ctx, |view, _| {
                    view.set_agent_idle_for_test(Duration::from_secs(3 * 60 * 60));
                });
            }
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                let mut session = sessions.session(busy.1.id()).unwrap().clone();
                session.status = CLIAgentSessionStatus::InProgress;
                sessions.set_session(busy.1.id(), session, ctx);
            });
        });
        workspace.update(&mut app, |workspace, ctx| {
            busy.1
                .update(ctx, |view, _| view.forget_recent_agent_start_for_test());
            let summary = workspace.run_idle_sleep_check(ctx);
            assert_eq!(
                summary.kept_awake,
                vec![SleepBlocker::NoSavedSession],
                "only the idle session was a candidate: {summary:?}"
            );
            assert!(
                busy.1.as_ref(ctx).agent_idle_for() < Duration::from_secs(60),
                "a working session's idle time starts over"
            );
            assert!(idle.1.as_ref(ctx).agent_idle_for() >= Duration::from_secs(3 * 60 * 60));
        });
    });
}

#[test]
fn waking_a_tab_starts_each_asleep_agent_in_it_once() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);
        let starts = Rc::new(Cell::new(0));
        workspace.update(&mut app, |workspace, ctx| {
            mark_window_active(ctx);
            let (index, terminal) = add_idle_agent_tab(workspace, ctx);
            assert_eq!(workspace.wake_tab(index, ctx), 0, "nothing asleep to wake");
            let counter = starts.clone();
            terminal.update(ctx, |view, ctx| {
                view.defer_agent_start(Box::new(move |_| counter.set(counter.get() + 1)));
                view.mark_agent_asleep(chrono::Utc::now(), ctx);
            });
            assert_eq!(workspace.wake_tab(index, ctx), 1);
            assert_eq!(workspace.wake_tab(index, ctx), 0);
        });
        assert_eq!(starts.get(), 1);
    });
}

#[test]
fn the_confirmation_counts_what_went_to_sleep_and_what_stayed_awake() {
    let summary = |slept, kept_awake: &[SleepBlocker]| SleepSummary {
        slept,
        kept_awake: kept_awake.to_vec(),
    };
    assert_eq!(
        bulk_sleep_message(&summary(0, &[])),
        "No idle sessions to put to sleep"
    );
    assert_eq!(
        bulk_sleep_message(&summary(1, &[])),
        "Put 1 session to sleep"
    );
    assert_eq!(
        bulk_sleep_message(&summary(
            12,
            &[SleepBlocker::AlreadyAsleep, SleepBlocker::NoAgent]
        )),
        "Put 12 sessions to sleep",
        "tabs that were never candidates aren't mentioned"
    );
    assert_eq!(
        bulk_sleep_message(&summary(3, &[SleepBlocker::Working])),
        "Put 3 sessions to sleep. 1 stayed awake because it's in use."
    );
    assert_eq!(
        bulk_sleep_message(&summary(
            0,
            &[SleepBlocker::Visible, SleepBlocker::WaitingOnYou]
        )),
        "No idle sessions to put to sleep. 2 stayed awake because they're in use."
    );
    assert_eq!(
        tab_stayed_awake_message(SleepBlocker::Working),
        "This session can't sleep yet: it is working."
    );
}

#[test]
fn zero_hours_turns_automatic_sleep_off() {
    assert_eq!(idle_sleep_threshold(0), None);
    assert_eq!(
        idle_sleep_threshold(2),
        Some(Duration::from_secs(2 * 60 * 60))
    );
}
