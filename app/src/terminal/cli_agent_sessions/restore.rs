//! Restore saved conversations in their existing panes, independently of focus or rendering.
use std::time::Duration;

use instant::Instant;
use warpui::r#async::Timer;
use warpui::{
    AppContext, Entity, EntityId, ModelContext, SingletonEntity, ViewHandle, WeakViewHandle,
};

use super::{CLIAgentSessionKey, CLIAgentSessionsModel};
use crate::terminal::view::TerminalView;

const CONCURRENT_STARTS: usize = 2;
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const START_TIMEOUT: Duration = Duration::from_secs(90);

type CancelPendingCommand = Box<dyn FnOnce(&mut AppContext) -> bool>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RestoreState {
    Queued,
    Starting,
    Restored,
    Attention(&'static str),
    Closed,
}

impl RestoreState {
    pub(crate) fn label(self) -> Option<&'static str> {
        match self {
            Self::Queued => Some("Queued for restore"),
            Self::Starting => Some("Restoring session…"),
            Self::Attention(reason) => Some(reason),
            Self::Restored | Self::Closed => None,
        }
    }
}

struct RestoreEntry {
    terminal: WeakViewHandle<TerminalView>,
    key: CLIAgentSessionKey,
    state: RestoreState,
    started_at: Option<Instant>,
    cancel_pending_command: Option<CancelPendingCommand>,
}

#[derive(Default)]
pub(crate) struct AgentRestoreQueue {
    entries: Vec<RestoreEntry>,
    ticking: bool,
    completed_at: Option<Instant>,
}

impl Entity for AgentRestoreQueue {
    type Event = ();
}

impl SingletonEntity for AgentRestoreQueue {}

impl AgentRestoreQueue {
    pub(crate) fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(
            &CLIAgentSessionsModel::handle(ctx),
            |queue, _, event, ctx| {
                // A late provider notification can resolve a timed-out start. Resume observation,
                // never execution, and avoid leaving a stale warning after the user fixes a pane.
                if !queue.ticking
                    && queue.entries.iter().any(|entry| {
                        entry.terminal.id() == event.terminal_view_id()
                            && matches!(entry.state, RestoreState::Attention(_))
                    })
                {
                    queue.ticking = true;
                    queue.schedule_tick(ctx);
                }
            },
        );
        Self::default()
    }

    pub(crate) fn enqueue(
        &mut self,
        terminal: &ViewHandle<TerminalView>,
        key: CLIAgentSessionKey,
        cancel_pending_command: CancelPendingCommand,
        ctx: &mut ModelContext<Self>,
    ) {
        if self
            .entries
            .iter()
            .any(|entry| entry.terminal.id() == terminal.id())
        {
            return;
        }
        // A second writer of the same conversation must never be started automatically.
        let duplicate = self
            .entries
            .iter()
            .any(|entry| entry.key == key && entry.terminal.upgrade(ctx).is_some());
        terminal.update(ctx, |view, _| view.manage_agent_restore_in_background());
        self.entries.push(RestoreEntry {
            terminal: terminal.downgrade(),
            key,
            state: if duplicate {
                RestoreState::Attention("Already restoring in another tab")
            } else {
                RestoreState::Queued
            },
            started_at: None,
            cancel_pending_command: Some(cancel_pending_command),
        });
        self.completed_at = None;
        ctx.emit(());
        if !self.ticking {
            self.ticking = true;
            self.schedule_tick(ctx);
        }
    }

    pub(crate) fn state(&self, terminal: EntityId) -> Option<RestoreState> {
        self.entries
            .iter()
            .find(|entry| entry.terminal.id() == terminal)
            .map(|entry| entry.state)
    }

    pub(crate) fn progress_text(&self) -> Option<String> {
        let count = |state| {
            self.entries
                .iter()
                .filter(|entry| entry.state == state)
                .count()
        };
        let queued = count(RestoreState::Queued);
        let starting = count(RestoreState::Starting);
        let restored = count(RestoreState::Restored);
        let attention = self
            .entries
            .iter()
            .filter(|entry| matches!(entry.state, RestoreState::Attention(_)))
            .count();
        let total = self.entries.len() - count(RestoreState::Closed);
        if queued + starting > 0 {
            let mut text = format!("Restoring sessions: {restored} of {total} restored · {starting} starting · {queued} queued");
            if attention > 0 {
                text.push_str(&format!(" · {attention} need attention"));
            }
            Some(text)
        } else if attention > 0 {
            Some(format!("Session restore: {restored} of {total} restored · {attention} need attention — check the marked tabs"))
        } else if restored > 0
            && self
                .completed_at
                .is_some_and(|completed| completed.elapsed() < Duration::from_secs(5))
        {
            Some(format!("Restored all {restored} sessions"))
        } else {
            None
        }
    }

    fn schedule_tick(&self, ctx: &mut ModelContext<Self>) {
        ctx.spawn(
            async { Timer::after(POLL_INTERVAL).await },
            |queue, _, ctx| queue.tick(ctx),
        );
    }

    fn tick(&mut self, ctx: &mut ModelContext<Self>) {
        let mut changed = false;
        for entry in &mut self.entries {
            let before = entry.state;
            let Some(terminal) = entry.terminal.upgrade(ctx) else {
                entry.state = RestoreState::Closed;
                changed |= before != entry.state;
                continue;
            };
            if matches!(entry.state, RestoreState::Attention(_))
                && CLIAgentSessionsModel::as_ref(ctx)
                    .session(terminal.id())
                    .and_then(|session| session.session_key())
                    .as_ref()
                    == Some(&entry.key)
                && terminal.as_ref(ctx).agent_restore_is_ready(ctx)
            {
                entry.state = RestoreState::Restored;
                entry.cancel_pending_command = None;
            }
            if entry.state == RestoreState::Starting {
                let session = CLIAgentSessionsModel::as_ref(ctx).session(terminal.id());
                if session.and_then(|session| session.session_key()).as_ref() != Some(&entry.key) {
                    entry.state = RestoreState::Attention("Restore stopped — check this tab");
                } else if terminal.as_ref(ctx).agent_restore_is_ready(ctx) {
                    entry.state = RestoreState::Restored;
                    entry.cancel_pending_command = None;
                } else if terminal.as_ref(ctx).agent_restore_has_user_input()
                    && entry
                        .cancel_pending_command
                        .take()
                        .is_some_and(|cancel| cancel(ctx))
                {
                    entry.state =
                        RestoreState::Attention("Restore paused — input changed in this tab");
                } else if entry
                    .started_at
                    .is_some_and(|started| started.elapsed() >= START_TIMEOUT)
                {
                    entry.state = RestoreState::Attention("Restore timed out — check this tab");
                }
                if matches!(entry.state, RestoreState::Attention(_)) {
                    // Cancel a command still waiting for shell bootstrap. An already dispatched
                    // command is never replayed or interrupted: the user can inspect its output.
                    if let Some(cancel) = entry.cancel_pending_command.take() {
                        cancel(ctx);
                    }
                }
            }
            changed |= before != entry.state;
        }

        let mut available = CONCURRENT_STARTS.saturating_sub(
            self.entries
                .iter()
                .filter(|entry| entry.state == RestoreState::Starting)
                .count(),
        );
        for entry in &mut self.entries {
            if available == 0 {
                break;
            }
            if entry.state != RestoreState::Queued {
                continue;
            }
            let Some(terminal) = entry.terminal.upgrade(ctx) else {
                continue;
            };
            if terminal.as_ref(ctx).agent_restore_has_user_input()
                || CLIAgentSessionsModel::as_ref(ctx)
                    .session(terminal.id())
                    .and_then(|session| session.session_key())
                    .as_ref()
                    != Some(&entry.key)
            {
                entry.state = RestoreState::Attention("Restore paused — input changed in this tab");
                if let Some(cancel) = entry.cancel_pending_command.take() {
                    cancel(ctx);
                }
            } else if terminal.update(ctx, |view, ctx| view.release_deferred_agent(ctx)) {
                entry.state = RestoreState::Starting;
                entry.started_at = Some(Instant::now());
                available -= 1;
            } else {
                entry.state = RestoreState::Attention("Restore could not start — check this tab");
            }
            changed = true;
        }
        if changed {
            ctx.emit(());
        }
        self.ticking = self
            .entries
            .iter()
            .any(|entry| matches!(entry.state, RestoreState::Queued | RestoreState::Starting));
        if !self.ticking && self.completed_at.is_none() {
            self.completed_at = Some(Instant::now());
        }
        if !self.ticking
            && self
                .entries
                .iter()
                .all(|entry| matches!(entry.state, RestoreState::Restored | RestoreState::Closed))
        {
            self.ticking = self
                .completed_at
                .is_some_and(|completed| completed.elapsed() < Duration::from_secs(5));
            if !self.ticking {
                ctx.emit(());
            }
        }
        if self.ticking {
            self.schedule_tick(ctx);
        }
    }
}

#[cfg(test)]
#[path = "restore_tests.rs"]
mod tests;
