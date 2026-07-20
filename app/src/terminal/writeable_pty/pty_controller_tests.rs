use std::sync::{mpsc, Arc};

use parking_lot::FairMutex;
use warpui::App;

use super::*;
use crate::terminal::model::ansi::{Handler, PreexecValue};
use crate::terminal::model::block::BlockId;

/// A [`EventLoopSender`] backed by an mpsc channel so tests can observe the
/// bytes that would have been written to the PTY.
struct TestEventLoopSender(mpsc::Sender<Message>);

impl EventLoopSender for TestEventLoopSender {
    fn send(&self, message: Message) -> Result<(), EventLoopSendError> {
        self.0
            .send(message)
            .map_err(|_| EventLoopSendError::Disconnected)
    }
}

struct TestPtyController {
    controller: ModelHandle<PtyController<TestEventLoopSender>>,
    event_loop_rx: mpsc::Receiver<Message>,
    terminal_model: Arc<FairMutex<TerminalModel>>,
    line_editor_status: ModelHandle<LineEditorStatus>,
    // Keep the model-event channel open for the lifetime of the test so the
    // dispatcher's stream doesn't terminate early.
    _model_events_tx: async_channel::Sender<crate::terminal::event::Event>,
}

fn build_pty_controller(app: &mut App) -> TestPtyController {
    let (event_loop_tx, event_loop_rx) = mpsc::channel();
    let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));

    let (executor_command_tx, executor_command_rx) = async_channel::unbounded();
    let sessions = app.add_model(|ctx| Sessions::new(executor_command_tx, ctx));

    let (model_events_tx, model_events_rx) = async_channel::unbounded();
    let model_event_dispatcher = {
        let sessions = sessions.clone();
        app.add_model(|ctx| ModelEventDispatcher::new(model_events_rx, sessions, ctx))
    };

    let line_editor_status = {
        let model_event_dispatcher = model_event_dispatcher.clone();
        let sessions = sessions.clone();
        app.add_model(|ctx| LineEditorStatus::new(model_event_dispatcher, sessions, ctx))
    };

    let controller = {
        let model_event_dispatcher = model_event_dispatcher.clone();
        let line_editor_status = line_editor_status.clone();
        let terminal_model = terminal_model.clone();
        app.add_model(|ctx| {
            PtyController::new(
                TestEventLoopSender(event_loop_tx),
                model_event_dispatcher,
                line_editor_status,
                sessions,
                executor_command_rx,
                terminal_model,
                ctx,
            )
        })
    };

    TestPtyController {
        controller,
        event_loop_rx,
        terminal_model,
        line_editor_status,
        _model_events_tx: model_events_tx,
    }
}

fn active_block_is_started(terminal_model: &Arc<FairMutex<TerminalModel>>) -> bool {
    terminal_model.lock().block_list().active_block().started()
}

fn active_block_id(terminal_model: &Arc<FairMutex<TerminalModel>>) -> BlockId {
    terminal_model.lock().block_list().active_block_id().clone()
}

#[test]
fn write_command_while_line_editor_inactive_defers_block_start() {
    App::test((), |mut app| async move {
        let harness = build_pty_controller(&mut app);

        // The line editor has not reported active yet (e.g. a cold shell that
        // is still bootstrapping), so the write must be parked.
        harness.controller.update(&mut app, |controller, ctx| {
            controller.write_command("ca", ShellType::Zsh, CommandExecutionSource::User, ctx);
        });

        // Nothing was written to the PTY...
        assert!(
            harness.event_loop_rx.try_recv().is_err(),
            "parked command must not reach the PTY while the line editor is inactive"
        );
        // ...so the block must not be considered started yet. Marking it
        // started here is what wedges a restored pane in a phantom
        // "command already running" state if the write never drains.
        assert!(
            !active_block_is_started(&harness.terminal_model),
            "block must not start until the command bytes are actually written"
        );

        // Once the shell reports its line editor as active, the queued command
        // drains: the bytes reach the PTY and the block starts.
        harness
            .line_editor_status
            .update(&mut app, |status, ctx| status.set_active_for_test(ctx));

        assert!(
            matches!(harness.event_loop_rx.try_recv(), Ok(Message::Input(_))),
            "queued command should be written once the line editor is active"
        );
        assert!(
            active_block_is_started(&harness.terminal_model),
            "block should start when the queued command is written"
        );
    });
}

#[test]
fn write_command_while_line_editor_active_starts_block_immediately() {
    App::test((), |mut app| async move {
        let harness = build_pty_controller(&mut app);

        harness
            .line_editor_status
            .update(&mut app, |status, ctx| status.set_active_for_test(ctx));

        harness.controller.update(&mut app, |controller, ctx| {
            controller.write_command(
                "echo foo",
                ShellType::Zsh,
                CommandExecutionSource::User,
                ctx,
            );
        });

        assert!(
            matches!(harness.event_loop_rx.try_recv(), Ok(Message::Input(_))),
            "command should be written directly when the line editor is active"
        );
        assert!(
            active_block_is_started(&harness.terminal_model),
            "block should be started as soon as the command is written"
        );
    });
}

#[test]
fn abort_command_start_recovers_block_when_command_never_executes() {
    App::test((), |mut app| async move {
        let harness = build_pty_controller(&mut app);

        harness
            .line_editor_status
            .update(&mut app, |status, ctx| status.set_active_for_test(ctx));

        harness.controller.update(&mut app, |controller, ctx| {
            controller.write_command("ca", ShellType::Zsh, CommandExecutionSource::User, ctx);
        });
        assert!(active_block_is_started(&harness.terminal_model));
        let block_id = active_block_id(&harness.terminal_model);

        // No preexec ever arrives (the shell never executed the line). The
        // reconciler must revert the phantom start so the pane accepts
        // commands again instead of reporting "command already running".
        harness.controller.update(&mut app, |controller, ctx| {
            controller.abort_command_start_if_unexecuted(&block_id, ctx);
        });

        let terminal_model = harness.terminal_model.lock();
        let active_block = terminal_model.block_list().active_block();
        assert!(
            !active_block.started(),
            "phantom start should be reverted when the command never executed"
        );
        assert!(
            !active_block.is_active_and_long_running(),
            "an aborted block must not read as an active long-running command"
        );
    });
}

#[test]
fn abort_command_start_is_noop_once_command_is_executing() {
    App::test((), |mut app| async move {
        let harness = build_pty_controller(&mut app);

        harness
            .line_editor_status
            .update(&mut app, |status, ctx| status.set_active_for_test(ctx));

        harness.controller.update(&mut app, |controller, ctx| {
            controller.write_command(
                "sleep 100",
                ShellType::Zsh,
                CommandExecutionSource::User,
                ctx,
            );
        });
        let block_id = active_block_id(&harness.terminal_model);

        // The shell acknowledges execution via the preexec hook.
        harness.terminal_model.lock().preexec(PreexecValue {
            command: "sleep 100".to_owned(),
            session_id: None,
        });

        harness.controller.update(&mut app, |controller, ctx| {
            controller.abort_command_start_if_unexecuted(&block_id, ctx);
        });

        let terminal_model = harness.terminal_model.lock();
        let active_block = terminal_model.block_list().active_block();
        assert!(
            active_block.started(),
            "a genuinely executing command must not be aborted"
        );
        assert!(active_block.is_executing());
    });
}
