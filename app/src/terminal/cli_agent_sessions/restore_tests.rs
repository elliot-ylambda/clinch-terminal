use std::cell::Cell;
use std::rc::Rc;

use warpui::App;

use super::*;
use crate::agent_resume::AgentResumeProvider;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

fn enqueue(
    app: &mut App,
    id: &str,
    starts: Rc<Cell<usize>>,
    cancellations: Rc<Cell<usize>>,
) -> ViewHandle<TerminalView> {
    let terminal = add_window_with_terminal(app, None);
    terminal.update(app, |view, ctx| {
        CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
            let mut session =
                super::super::tests::idle_test_session(crate::terminal::CLIAgent::Claude);
            session.session_context.session_id = Some(id.into());
            session.received_rich_notification = false;
            sessions.set_session(view.id(), session, ctx);
        });
        view.defer_agent_start(Box::new(move |_| starts.set(starts.get() + 1)));
    });
    AgentRestoreQueue::handle(app).update(app, |queue, ctx| {
        queue.enqueue(
            &terminal,
            CLIAgentSessionKey {
                provider: AgentResumeProvider::Claude,
                session_id: id.into(),
            },
            Box::new(move |_| {
                cancellations.set(cancellations.get() + 1);
                true
            }),
            ctx,
        );
    });
    terminal
}

fn report_ready(app: &mut App, terminal: &ViewHandle<TerminalView>) {
    terminal.update(app, |view, ctx| {
        view.model.lock().simulate_long_running_block("claude", "");
        CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
            let mut session = sessions.session(view.id()).unwrap().clone();
            session.received_rich_notification = true;
            sessions.set_session(view.id(), session, ctx);
        });
    });
}

#[test]
fn background_restore_starts_two_across_windows_without_focus_and_advances_once() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let starts = Rc::new(Cell::new(0));
        let cancels = Rc::new(Cell::new(0));
        let panes: Vec<_> = (0..5)
            .map(|i| {
                enqueue(
                    &mut app,
                    &format!("conversation-{i}"),
                    starts.clone(),
                    cancels.clone(),
                )
            })
            .collect();
        let queue = AgentRestoreQueue::handle(&app);
        let active = app.read(|ctx| ctx.windows().active_window());
        assert_eq!(starts.get(), 0);
        queue.update(&mut app, |queue, ctx| queue.tick(ctx));
        assert_eq!(starts.get(), 2);
        assert_eq!(app.read(|ctx| ctx.windows().active_window()), active);
        assert!(queue
            .read(&app, |queue, _| queue.progress_text())
            .unwrap()
            .contains("2 starting · 3 queued"));

        // Focusing a queued tab and local-control start requests cannot escape admission.
        panes[4].update(&mut app, |view, ctx| {
            view.mark_as_visible(ctx);
            assert!(!view.start_deferred_agent(ctx));
        });
        queue.update(&mut app, |queue, ctx| queue.tick(ctx));
        assert_eq!(
            starts.get(),
            2,
            "seeded session identities are not readiness"
        );

        report_ready(&mut app, &panes[0]);
        queue.update(&mut app, |queue, ctx| queue.tick(ctx));
        assert_eq!(starts.get(), 3);
        for pane in &panes[1..3] {
            report_ready(&mut app, pane);
        }
        queue.update(&mut app, |queue, ctx| queue.tick(ctx));
        assert_eq!(starts.get(), 5);
        for pane in &panes[3..] {
            report_ready(&mut app, pane);
        }
        queue.update(&mut app, |queue, ctx| queue.tick(ctx));
        queue.update(&mut app, |queue, ctx| queue.tick(ctx));
        assert_eq!(starts.get(), 5, "never replay a completed start");
        assert!(panes
            .iter()
            .all(|pane| queue.read(&app, |queue, _| queue.state(pane.id()))
                == Some(RestoreState::Restored)));
        assert_eq!(cancels.get(), 0);
        assert_eq!(app.read(|ctx| ctx.windows().active_window()), active);
    });
}

#[test]
fn background_restore_timeout_cancels_pending_bootstrap_and_continues() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let starts = Rc::new(Cell::new(0));
        let cancels = Rc::new(Cell::new(0));
        let panes: Vec<_> = (0..3)
            .map(|i| {
                enqueue(
                    &mut app,
                    &format!("timeout-{i}"),
                    starts.clone(),
                    cancels.clone(),
                )
            })
            .collect();
        let queue = AgentRestoreQueue::handle(&app);
        queue.update(&mut app, |queue, ctx| {
            queue.tick(ctx);
            queue.entries[0].started_at = Instant::now().checked_sub(START_TIMEOUT);
            queue.tick(ctx);
        });
        assert_eq!(starts.get(), 3);
        assert_eq!(cancels.get(), 1);
        assert!(matches!(
            queue.read(&app, |queue, _| queue.state(panes[0].id())),
            Some(RestoreState::Attention(_))
        ));
        assert!(queue
            .read(&app, |queue, _| queue.progress_text())
            .unwrap()
            .contains("1 need attention"));
        panes[0].update(&mut app, |view, ctx| view.mark_as_visible(ctx));
        queue.update(&mut app, |queue, ctx| queue.tick(ctx));
        assert_eq!(starts.get(), 3, "an uncertain start must not be retried");
        assert_eq!(cancels.get(), 1);
    });
}

#[test]
fn background_restore_rejects_duplicate_conversation_writers() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let starts = Rc::new(Cell::new(0));
        let cancels = Rc::new(Cell::new(0));
        let first = enqueue(
            &mut app,
            "same-conversation",
            starts.clone(),
            cancels.clone(),
        );
        let duplicate = enqueue(
            &mut app,
            "same-conversation",
            starts.clone(),
            cancels.clone(),
        );
        let queue = AgentRestoreQueue::handle(&app);
        queue.update(&mut app, |queue, ctx| queue.tick(ctx));
        duplicate.update(&mut app, |view, ctx| view.mark_as_visible(ctx));
        assert_eq!(starts.get(), 1);
        assert_eq!(
            queue.read(&app, |queue, _| queue.state(first.id())),
            Some(RestoreState::Starting)
        );
        assert!(matches!(
            queue.read(&app, |queue, _| queue.state(duplicate.id())),
            Some(RestoreState::Attention(_))
        ));
    });
}

#[test]
fn background_restore_exit_is_visible_and_does_not_stall_other_panes() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let starts = Rc::new(Cell::new(0));
        let cancels = Rc::new(Cell::new(0));
        let panes: Vec<_> = (0..3)
            .map(|i| {
                enqueue(
                    &mut app,
                    &format!("exit-{i}"),
                    starts.clone(),
                    cancels.clone(),
                )
            })
            .collect();
        let queue = AgentRestoreQueue::handle(&app);
        queue.update(&mut app, |queue, ctx| queue.tick(ctx));
        CLIAgentSessionsModel::handle(&app).update(&mut app, |sessions, ctx| {
            sessions.remove_session(panes[0].id(), ctx)
        });
        queue.update(&mut app, |queue, ctx| queue.tick(ctx));
        assert_eq!(starts.get(), 3);
        assert!(matches!(
            queue.read(&app, |queue, _| queue.state(panes[0].id())),
            Some(RestoreState::Attention(_))
        ));
    });
}
