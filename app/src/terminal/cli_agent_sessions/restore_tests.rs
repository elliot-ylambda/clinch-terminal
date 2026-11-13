use std::cell::{Cell, RefCell};
use std::rc::Rc;

use warpui::App;

use super::*;
use crate::agent_resume::AgentResumeProvider;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

/// A restored pane that a restart cut off mid-turn: the queue starts it.
fn enqueue_running(
    app: &mut App,
    id: &str,
    starts: Rc<Cell<usize>>,
    cancellations: Rc<Cell<usize>>,
) -> ViewHandle<TerminalView> {
    AgentRestoreQueue::handle(app).update(app, |queue, _| {
        queue
            .unclaimed_interrupted_panes
            .insert(id.as_bytes().to_vec());
    });
    enqueue(app, id, starts, cancellations)
}

/// A restored pane. It stays asleep unless the previous run recorded it as mid-turn.
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
            id.as_bytes().to_vec(),
            CLIAgentSessionKey {
                provider: AgentResumeProvider::Claude,
                session_id: id.into(),
            },
            Box::new(move |_| {
                cancellations.set(cancellations.get() + 1);
                true
            }),
            Rc::new(|_| {}),
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
        app.update(crate::remote_control::register);
        let starts = Rc::new(Cell::new(0));
        let cancels = Rc::new(Cell::new(0));
        let panes: Vec<_> = (0..5)
            .map(|i| {
                enqueue_running(
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
        app.update(crate::remote_control::register);
        let starts = Rc::new(Cell::new(0));
        let cancels = Rc::new(Cell::new(0));
        let panes: Vec<_> = (0..3)
            .map(|i| {
                enqueue_running(
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
        app.update(crate::remote_control::register);
        let starts = Rc::new(Cell::new(0));
        let cancels = Rc::new(Cell::new(0));
        let first = enqueue_running(
            &mut app,
            "same-conversation",
            starts.clone(),
            cancels.clone(),
        );
        let duplicate = enqueue_running(
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
        app.update(crate::remote_control::register);
        let starts = Rc::new(Cell::new(0));
        let cancels = Rc::new(Cell::new(0));
        let panes: Vec<_> = (0..3)
            .map(|i| {
                enqueue_running(
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

#[test]
fn continuing_waits_until_the_agent_can_take_a_prompt_and_stops_if_someone_else_took_over() {
    let settled = CONTINUE_SETTLE;
    let early = Duration::from_secs(1);

    assert_eq!(continue_step(None, true, false, early), ContinueStep::Wait);
    assert_eq!(
        continue_step(None, true, false, settled),
        ContinueStep::Send
    );
    // A picker or dialog is on screen rather than an input box.
    assert_eq!(
        continue_step(None, false, false, settled),
        ContinueStep::Wait
    );
    assert_eq!(
        continue_step(Some("starting"), true, false, settled),
        ContinueStep::Wait
    );

    // The user typed first, or left a draft: their input always wins.
    assert_eq!(
        continue_step(None, true, true, settled),
        ContinueStep::Finished
    );
    assert_eq!(
        continue_step(Some("human_input_or_draft"), false, false, early),
        ContinueStep::Finished
    );

    // A spinner may only be the agent loading, until it has run past the timeout.
    assert_eq!(
        continue_step(Some("working"), false, false, settled),
        ContinueStep::Wait
    );
    assert_eq!(
        continue_step(Some("working"), false, false, CONTINUE_TIMEOUT),
        ContinueStep::Finished
    );

    // Never able to take a prompt: flag the tab instead of retrying forever.
    assert_eq!(
        continue_step(Some("agent_not_writable"), false, false, CONTINUE_TIMEOUT),
        ContinueStep::GiveUp
    );
    assert_eq!(
        continue_step(None, false, false, CONTINUE_TIMEOUT),
        ContinueStep::GiveUp
    );
}

#[test]
fn the_continue_prompt_stays_out_of_history() {
    assert!(crate::agent_resume::is_injected_prompt(
        &restart_continue_prompt()
    ));
}

#[test]
fn only_panes_that_were_running_start_and_they_keep_their_continue_until_it_is_sent() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.update(crate::remote_control::register);
        AgentRestoreQueue::handle(&app).update(&mut app, |queue, _| {
            queue.unclaimed_interrupted_panes = HashSet::from([b"running".to_vec()]);
        });
        let starts = Rc::new(Cell::new(0));
        let cancellations = Rc::new(Cell::new(0));
        for id in ["idle-1", "idle-2", "running"] {
            enqueue(&mut app, id, starts.clone(), cancellations.clone());
        }
        AgentRestoreQueue::handle(&app).update(&mut app, |queue, ctx| queue.tick(ctx));
        AgentRestoreQueue::handle(&app).read(&app, |queue, ctx| {
            let state = |id: &str| {
                queue
                    .entries
                    .iter()
                    .find(|entry| entry.key.session_id == id)
                    .map(|entry| entry.state)
                    .unwrap()
            };
            assert_eq!(state("running"), RestoreState::Starting);
            // The sessions that were idle wait for someone to open them.
            assert_eq!(state("idle-1"), RestoreState::Asleep);
            assert_eq!(state("idle-2"), RestoreState::Asleep);
            let _ = ctx;
        });
        assert_eq!(starts.get(), 1, "only the session that was running starts");
        // A second restart before the continue went out must not lose it.
        AgentRestoreQueue::handle(&app).update(&mut app, |queue, ctx| {
            queue.record_interrupted_turns(false, ctx);
            assert!(queue.unclaimed_interrupted_panes.is_empty());
            assert!(queue
                .last_recorded_panes
                .as_ref()
                .unwrap()
                .contains(b"running".as_slice()));
        });
    });
}

/// The whole path for a session without hook status, the state every restored session starts in:
/// it comes back, shows an empty input box, and gets exactly one continue.
#[test]
fn a_restored_session_that_was_running_gets_one_continue_once_its_input_box_shows() {
    use crate::terminal::{CLIAgent, SizeInfo, SizeUpdate, SizeUpdateReason};

    for agent in [CLIAgent::Claude, CLIAgent::Codex] {
        App::test((), |mut app| async move {
            initialize_app_for_terminal_view(&mut app);
            app.update(crate::remote_control::register);
            let queue = AgentRestoreQueue::handle(&app);
            queue.update(&mut app, |queue, _| {
                queue.unclaimed_interrupted_panes = HashSet::from([b"running".to_vec()]);
            });
            let starts = Rc::new(Cell::new(0));
            let terminal = enqueue(&mut app, "running", starts.clone(), Rc::default());
            let writes = Rc::new(RefCell::new(Vec::<u8>::new()));
            let captured = writes.clone();
            app.update(|ctx| {
                ctx.subscribe_to_view(&terminal, move |_, event, _| {
                    if let crate::terminal::view::Event::WriteBytesToPty { bytes } = event {
                        captured.borrow_mut().extend_from_slice(bytes);
                    }
                })
            });
            terminal.update(&mut app, |view, ctx| {
                CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                    let mut session = sessions.session(view.id()).unwrap().clone();
                    session.agent = agent;
                    sessions.set_session(view.id(), session, ctx);
                });
            });
            queue.update(&mut app, |queue, ctx| {
                if agent == CLIAgent::Codex {
                    queue.entries[0].key.provider = AgentResumeProvider::Codex;
                }
                queue.tick(ctx);
            });
            assert_eq!(starts.get(), 1, "the agent starts");

            terminal.update(&mut app, |view, _| {
                view.end_deferred_agent_start_grace_for_test();
                let mut model = view.model.lock();
                model.simulate_long_running_block("agent", "");
                let last_size = *model.block_list().size();
                let new_size = SizeInfo::new_without_font_metrics(24, 120);
                model.resize(SizeUpdate {
                    update_reason: SizeUpdateReason::Refresh,
                    last_size,
                    new_size,
                    new_gap_height: None,
                    natural_rows: new_size.rows(),
                    natural_cols: new_size.columns(),
                });
                model.enter_alt_screen(true);
                let frame = if agent == CLIAgent::Claude {
                    let rule = "─".repeat(120);
                    format!(
                        "\x1b[0m\x1b[2J\x1b[?25h\x1b[3;1H{rule}\x1b[4;1H❯ \x1b[2mAsk Claude\x1b[0m\x1b[5;1H{rule}\x1b[4;3H"
                    )
                } else {
                    "\x1b[0m\x1b[2J\x1b[?25h\x1b[48;2;30;30;30m\x1b[3;1H\x1b[2K\x1b[4;1H\x1b[2K\x1b[5;1H\x1b[2K\x1b[4;1H\x1b[1m›\x1b[22m \x1b[2mAsk Codex\x1b[0m\x1b[6;1HReady\x1b[4;3H"
                        .to_owned()
                };
                model.process_bytes(frame.as_str());
            });

            // Back, but not settled yet: nothing is typed.
            let pending = |app: &App| {
                AgentRestoreQueue::handle(app).read(app, |queue, _| {
                    (queue.entries[0].state, queue.entries[0].continue_pending)
                })
            };
            queue.update(&mut app, |queue, ctx| queue.tick(ctx));
            assert_eq!(pending(&app), (RestoreState::Restored, true));
            assert!(writes.borrow().is_empty());
            terminal.read(&app, |view, ctx| {
                assert_eq!(view.local_control_agent_readiness(ctx).1, None);
                assert!(view.agent_composer_is_empty(ctx));
            });

            // Settled: the prompt goes out once and the pane leaves the record for the next restart.
            queue.update(&mut app, |queue, ctx| {
                queue.entries[0].restored_at =
                    Some(Instant::now().checked_sub(CONTINUE_SETTLE).unwrap());
                queue.tick(ctx);
            });
            assert_eq!(pending(&app), (RestoreState::Restored, false));
            let typed = String::from_utf8_lossy(&writes.borrow()).into_owned();
            assert_eq!(
                typed.matches(RESTART_CONTINUE_MARKER).count(),
                1,
                "{typed:?}"
            );
            queue.update(&mut app, |queue, ctx| queue.tick(ctx));
            let typed = String::from_utf8_lossy(&writes.borrow()).into_owned();
            assert_eq!(
                typed.matches(RESTART_CONTINUE_MARKER).count(),
                1,
                "sent only once"
            );
            queue.update(&mut app, |queue, ctx| {
                queue.record_interrupted_turns(false, ctx);
                assert!(queue.last_recorded_panes.as_ref().unwrap().is_empty());
            });
        });
    }
}

#[test]
fn a_session_counts_as_mid_turn_while_working_or_waiting_on_its_user() {
    use crate::terminal::cli_agent_sessions::CLIAgentSessionStatus;
    use crate::terminal::CLIAgent;

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.update(crate::remote_control::register);
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            assert!(!view.agent_is_mid_turn(ctx), "no agent in the pane");
            let cases = [
                (CLIAgentSessionStatus::InProgress, true, true),
                // A fresh session starts out in progress before anything has happened.
                (CLIAgentSessionStatus::InProgress, false, false),
                (CLIAgentSessionStatus::Blocked { message: None }, true, true),
                (CLIAgentSessionStatus::Success, true, false),
            ];
            for agent in [CLIAgent::Claude, CLIAgent::Codex] {
                for (status, observed, expected) in cases.clone() {
                    CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                        let mut session = super::super::tests::idle_test_session(agent);
                        session.status = status.clone();
                        session.has_observed_turn_activity = observed;
                        sessions.set_session(view.id(), session, ctx);
                    });
                    assert_eq!(
                        view.agent_is_mid_turn(ctx),
                        expected,
                        "{agent:?} {status:?} observed {observed}"
                    );
                }
            }
        });
    });
}

#[test]
fn a_shell_that_never_starts_its_command_frees_the_slot_and_can_be_retried() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.update(crate::remote_control::register);
        let starts = Rc::new(Cell::new(0));
        let cancels = Rc::new(Cell::new(0));
        let mut terminals = (0..3)
            .map(|index| {
                enqueue_running(
                    &mut app,
                    &format!("session-{index}"),
                    starts.clone(),
                    cancels.clone(),
                )
            })
            .collect::<Vec<_>>();
        let tick = |app: &mut App| {
            AgentRestoreQueue::handle(app).update(app, |queue, ctx| queue.tick(ctx));
        };
        tick(&mut app);
        assert_eq!(starts.get(), 2, "two sessions start at a time");

        // The first shell is stuck: the third session must not wait out the 90 second timeout.
        let stuck = terminals.remove(0);
        stuck.update(&mut app, |view, ctx| view.agent_command_did_not_start(ctx));
        tick(&mut app);
        assert_eq!(starts.get(), 3, "the stuck session gave up its slot");
        AgentRestoreQueue::handle(&app).read(&app, |queue, ctx| {
            assert_eq!(queue.stalled_count(), 1);
            assert_eq!(
                queue.state(stuck.id()).and_then(RestoreState::label),
                Some(DID_NOT_START_LABEL)
            );
            assert_eq!(queue.first_attention_terminal(ctx), Some(stuck.id()));
        });

        // Retry goes back through the queue. It applies even if the user typed in the tab, which
        // they may well have while trying to answer the shell.
        let retried = Rc::new(Cell::new(false));
        stuck.update(&mut app, |view, _| view.mark_user_input_for_test());
        AgentRestoreQueue::handle(&app).update(&mut app, |queue, ctx| {
            let flag = retried.clone();
            queue.entries[0].retry_stalled_command = Rc::new(move |_| flag.set(true));
            queue.retry_stalled(ctx);
            assert_eq!(queue.stalled_count(), 0);
            assert_eq!(queue.state(stuck.id()), Some(RestoreState::Queued));
        });
        stuck.read(&app, |view, _| assert!(!view.agent_command_stalled()));
        for terminal in &terminals {
            report_ready(&mut app, terminal);
        }
        tick(&mut app);
        assert!(retried.get(), "the retry ran once a slot was free");
        AgentRestoreQueue::handle(&app).read(&app, |queue, _| {
            assert_eq!(queue.state(stuck.id()), Some(RestoreState::Starting));
        });
    });
}

#[test]
fn the_notice_says_a_shell_may_be_waiting_when_that_is_all_that_is_left() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.update(crate::remote_control::register);
        let starts = Rc::new(Cell::new(0));
        let cancels = Rc::new(Cell::new(0));
        let terminal = enqueue_running(&mut app, "session", starts, cancels);
        AgentRestoreQueue::handle(&app).update(&mut app, |queue, ctx| queue.tick(ctx));
        terminal.update(&mut app, |view, ctx| view.agent_command_did_not_start(ctx));
        AgentRestoreQueue::handle(&app).update(&mut app, |queue, ctx| {
            queue.tick(ctx);
            assert_eq!(
                queue.progress_text().as_deref(),
                Some("1 session didn't start: its shell may be waiting for input")
            );
        });
    });
}

/// The launch bug: every restored session used to start its agent, two at a time, whether or not
/// anyone opened its tab. Sessions that were idle now stay asleep through launch, through the
/// layout and focus passes that mark panes visible, and until someone wakes one.
#[test]
fn restored_sessions_that_were_idle_stay_asleep_until_one_is_woken() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.update(crate::remote_control::register);
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
        for _ in 0..3 {
            queue.update(&mut app, |queue, ctx| queue.tick(ctx));
        }
        assert_eq!(starts.get(), 0, "launch starts no idle session");
        assert_eq!(
            queue.read(&app, |queue, _| queue.progress_text()),
            None,
            "nothing is being restored, so there is no notice"
        );

        // Focus and layout mark panes visible in passing. That is not someone opening the tab.
        for pane in &panes {
            pane.update(&mut app, |view, ctx| {
                view.mark_as_visible(ctx);
                view.mark_as_visible(ctx);
                assert!(view.agent_is_asleep());
                assert_eq!(
                    view.local_control_agent_readiness(ctx).1,
                    Some("not_started")
                );
            });
        }
        queue.update(&mut app, |queue, ctx| queue.tick(ctx));
        assert_eq!(starts.get(), 0, "being shown starts nothing");

        // Waking one starts that one, once, and the queue watches it come up.
        panes[2].update(&mut app, |view, ctx| {
            assert!(view.wake_agent(ctx));
            assert!(!view.wake_agent(ctx), "a second wake has nothing to start");
        });
        assert_eq!(starts.get(), 1);
        assert_eq!(
            queue.read(&app, |queue, _| queue.state(panes[2].id())),
            Some(RestoreState::Starting)
        );
        assert_eq!(
            queue.read(&app, |queue, _| queue.label(panes[2].id())),
            Some("Waking up…")
        );
        assert_eq!(
            queue.read(&app, |queue, _| queue.progress_text()),
            None,
            "one tab waking is not Clinch restoring sessions"
        );
        report_ready(&mut app, &panes[2]);
        queue.update(&mut app, |queue, ctx| queue.tick(ctx));
        assert_eq!(
            queue.read(&app, |queue, _| queue.state(panes[2].id())),
            Some(RestoreState::Restored)
        );
        panes[2].read(&app, |view, _| {
            assert!(
                view.agent_sleep().is_none(),
                "the pane stops saying it is waking"
            );
        });
        for (index, pane) in panes.iter().enumerate() {
            if index != 2 {
                assert_eq!(
                    queue.read(&app, |queue, _| queue.state(pane.id())),
                    Some(RestoreState::Asleep)
                );
            }
        }
        assert_eq!(starts.get(), 1, "the other four are still asleep");
        assert_eq!(cancels.get(), 0);
    });
}

/// A message sent to an asleep session wakes it, the same way opening its tab does.
#[test]
fn a_message_to_an_asleep_session_wakes_it() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.update(crate::remote_control::register);
        let starts = Rc::new(Cell::new(0));
        let terminal = enqueue(&mut app, "asleep", starts.clone(), Rc::default());
        terminal.update(&mut app, |view, ctx| {
            assert_eq!(
                view.local_control_agent_readiness(ctx).1,
                Some("not_started")
            );
            // This is what local-control delivery calls for a `not_started` target.
            assert!(view.start_deferred_agent(ctx));
            assert_eq!(
                view.local_control_agent_readiness(ctx).1,
                Some("starting"),
                "a queued message waits while the agent comes up"
            );
        });
        assert_eq!(starts.get(), 1);
        assert_eq!(
            AgentRestoreQueue::handle(&app).read(&app, |queue, _| queue.state(terminal.id())),
            Some(RestoreState::Starting)
        );
    });
}

/// A restart keeps asleep tabs asleep, and keeps saying since when.
#[test]
fn a_restart_keeps_asleep_sessions_asleep_since_the_time_they_fell_asleep() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.update(crate::remote_control::register);
        let fell_asleep = Utc::now() - chrono::Duration::hours(5);
        AgentRestoreQueue::handle(&app).update(&mut app, |queue, _| {
            queue
                .unclaimed_asleep_since
                .insert(b"slept-before-quit".to_vec(), fell_asleep);
        });
        let starts = Rc::new(Cell::new(0));
        let slept = enqueue(&mut app, "slept-before-quit", starts.clone(), Rc::default());
        let awake_at_quit = enqueue(&mut app, "awake-at-quit", starts.clone(), Rc::default());
        AgentRestoreQueue::handle(&app).update(&mut app, |queue, ctx| queue.tick(ctx));
        assert_eq!(starts.get(), 0);
        slept.read(&app, |view, _| {
            assert_eq!(view.agent_asleep_since(), Some(fell_asleep));
        });
        awake_at_quit.read(&app, |view, _| {
            let since = view.agent_asleep_since().expect("asleep after the restart");
            assert!(since > fell_asleep, "asleep since this launch, not before");
        });
    });
}

/// A pane put to sleep while Clinch runs is handed to the queue the way a restored one is, even
/// if the queue restored that same pane earlier.
#[test]
fn a_session_put_to_sleep_later_wakes_through_the_queue_on_the_same_conversation() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.update(crate::remote_control::register);
        let starts = Rc::new(Cell::new(0));
        let terminal = enqueue_running(&mut app, "conversation", starts.clone(), Rc::default());
        let queue = AgentRestoreQueue::handle(&app);
        queue.update(&mut app, |queue, ctx| queue.tick(ctx));
        assert_eq!(starts.get(), 1);
        report_ready(&mut app, &terminal);
        queue.update(&mut app, |queue, ctx| {
            queue.entries[0].continue_pending = false;
            queue.tick(ctx);
        });

        // Later, its tab is put to sleep: the agent has exited and its resume command is armed.
        let fell_asleep = Utc::now();
        let resumes = starts.clone();
        terminal.update(&mut app, |view, _| {
            view.defer_agent_start(Box::new(move |_| resumes.set(resumes.get() + 1)));
        });
        queue.update(&mut app, |queue, ctx| {
            queue.put_to_sleep(
                &terminal,
                b"conversation".to_vec(),
                CLIAgentSessionKey {
                    provider: AgentResumeProvider::Claude,
                    session_id: "conversation".into(),
                },
                fell_asleep,
                Box::new(|_| false),
                Rc::new(|_| {}),
                ctx,
            );
            assert_eq!(queue.entries.len(), 1, "the pane keeps a single entry");
            queue.tick(ctx);
        });
        assert_eq!(
            starts.get(),
            1,
            "an asleep session is not restarted by the queue"
        );
        terminal.update(&mut app, |view, ctx| {
            assert_eq!(view.agent_asleep_since(), Some(fell_asleep));
            // The pane was restored by the queue once. That must not stop a message waking it.
            assert!(view.start_deferred_agent(ctx));
        });
        assert_eq!(starts.get(), 2);
        let conversation = app.read(|ctx| {
            CLIAgentSessionsModel::as_ref(ctx)
                .session(terminal.id())
                .and_then(|session| session.session_key())
                .map(|key| key.session_id)
        });
        assert_eq!(conversation.as_deref(), Some("conversation"));
    });
}
