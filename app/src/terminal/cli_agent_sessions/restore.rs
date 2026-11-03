//! Restore saved conversations in their existing panes, independently of focus or rendering, and
//! tell the ones a restart cut off mid-turn to carry on.
use std::collections::{BTreeSet, HashSet};
use std::rc::Rc;
use std::time::Duration;

use instant::Instant;
use warpui::r#async::Timer;
use warpui::{
    AppContext, Entity, EntityId, ModelContext, SingletonEntity, ViewHandle, WeakViewHandle,
};

use super::{CLIAgentSessionKey, CLIAgentSessionsModel};
use crate::agent_resume::{
    read_interrupted_turns, write_interrupted_turns, RESTART_CONTINUE_MARKER,
};
use crate::settings::ClinchSettings;
use crate::terminal::view::TerminalView;
use crate::workspace::WorkspaceRegistry;

const CONCURRENT_STARTS: usize = 2;
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const START_TIMEOUT: Duration = Duration::from_secs(90);
/// How often a tab flagged for attention is rechecked once nothing else is restoring.
const ATTENTION_POLL_INTERVAL: Duration = Duration::from_secs(2);
/// How often the panes a restart would cut off are recorded, for a crash or forced quit.
const RECORD_INTERVAL: Duration = Duration::from_secs(10);
/// How long a restored agent gets to settle before anything is typed into it.
const CONTINUE_SETTLE: Duration = Duration::from_secs(3);
/// How long to keep trying to continue a restored agent before flagging its tab instead.
const CONTINUE_TIMEOUT: Duration = Duration::from_secs(3 * 60);
const CONTINUE_FAILED_LABEL: &str = "Was running but couldn't continue — check this tab";
const DID_NOT_START_LABEL: &str = "The shell didn't start this — it may be waiting for input";

type CancelPendingCommand = Box<dyn FnOnce(&mut AppContext) -> bool>;
type RetryStalledCommand = Rc<dyn Fn(&mut AppContext)>;

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

/// What to do about a restored agent that a restart cut off mid-turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContinueStep {
    Wait,
    Send,
    /// Nothing more to do: someone else took over.
    Finished,
    GiveUp,
}

fn continue_step(
    unavailable: Option<&str>,
    composer_is_empty: bool,
    user_typed: bool,
    elapsed: Duration,
) -> ContinueStep {
    let timed_out = elapsed >= CONTINUE_TIMEOUT;
    match unavailable {
        _ if user_typed => ContinueStep::Finished,
        Some("human_input_or_draft") => ContinueStep::Finished,
        // A spinner at startup can be just the agent loading, so give it time to settle. A turn
        // still running after all that is the agent carrying on by itself.
        Some("working") if timed_out => ContinueStep::Finished,
        None if composer_is_empty && elapsed >= CONTINUE_SETTLE => ContinueStep::Send,
        _ if timed_out => ContinueStep::GiveUp,
        _ => ContinueStep::Wait,
    }
}

fn restart_continue_prompt() -> String {
    format!(
        "{RESTART_CONTINUE_MARKER} Clinch restarted while this session was mid-turn, so its last \
         turn was cut off. Continue where you left off. Before repeating anything with outside \
         effects, such as a deploy, a push, or a message, check whether it already happened."
    )
}

/// Every pane a restart right now would cut off, across every window and project.
pub(crate) fn mid_turn_panes(ctx: &AppContext) -> BTreeSet<Vec<u8>> {
    let mut panes = BTreeSet::new();
    for (_, workspace) in WorkspaceRegistry::as_ref(ctx).all_workspaces(ctx) {
        for group in workspace.as_ref(ctx).tab_views() {
            panes.extend(group.as_ref(ctx).mid_turn_pane_uuids(ctx));
        }
    }
    panes
}

struct RestoreEntry {
    terminal: WeakViewHandle<TerminalView>,
    pane_uuid: Vec<u8>,
    key: CLIAgentSessionKey,
    state: RestoreState,
    started_at: Option<Instant>,
    cancel_pending_command: Option<CancelPendingCommand>,
    retry_stalled_command: RetryStalledCommand,
    /// The user asked to try this one again, knowing what they typed in its tab.
    retrying: bool,
    /// This pane was mid-turn when Clinch stopped, and hasn't been told to continue yet.
    continue_pending: bool,
    /// When the agent came back, which is when the wait to continue it starts.
    restored_at: Option<Instant>,
}

#[derive(Default)]
pub(crate) struct AgentRestoreQueue {
    entries: Vec<RestoreEntry>,
    ticking: bool,
    completed_at: Option<Instant>,
    /// The user closed the notice. Restoring shows it again.
    notice_dismissed: bool,
    /// Panes the previous run recorded as mid-turn that no restored tab has claimed yet.
    unclaimed_interrupted_panes: HashSet<Vec<u8>>,
    last_recorded_panes: Option<BTreeSet<Vec<u8>>>,
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

    /// Picks up what the previous run recorded and starts recording for the next one. Call once,
    /// before any window restores.
    pub(crate) fn start_tracking_interrupted_turns(&mut self, ctx: &mut ModelContext<Self>) {
        if *ClinchSettings::as_ref(ctx).continue_sessions_after_restart {
            self.unclaimed_interrupted_panes = read_interrupted_turns();
        }
        self.schedule_recording(ctx);
    }

    fn schedule_recording(&self, ctx: &mut ModelContext<Self>) {
        ctx.spawn(
            async { Timer::after(RECORD_INTERVAL).await },
            |queue, _, ctx| {
                queue.record_interrupted_turns(false, ctx);
                queue.schedule_recording(ctx);
            },
        );
    }

    /// Saves the panes a restart right now would cut off, including restored ones still waiting
    /// for their continue, so a second restart can't lose them. `quitting` always writes, which
    /// dates the record from the quit rather than from when the list last changed.
    pub(crate) fn record_interrupted_turns(&mut self, quitting: bool, ctx: &AppContext) {
        // Windows restore synchronously at launch, so nothing can claim these by now.
        self.unclaimed_interrupted_panes.clear();
        let mut panes = mid_turn_panes(ctx);
        panes.extend(
            self.entries
                .iter()
                .filter(|entry| entry.continue_pending)
                .map(|entry| entry.pane_uuid.clone()),
        );
        if quitting || self.last_recorded_panes.as_ref() != Some(&panes) {
            write_interrupted_turns(&panes);
            self.last_recorded_panes = Some(panes);
        }
    }

    pub(crate) fn enqueue(
        &mut self,
        terminal: &ViewHandle<TerminalView>,
        pane_uuid: Vec<u8>,
        key: CLIAgentSessionKey,
        cancel_pending_command: CancelPendingCommand,
        retry_stalled_command: RetryStalledCommand,
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
        let continue_pending = self.unclaimed_interrupted_panes.remove(&pane_uuid);
        self.entries.push(RestoreEntry {
            terminal: terminal.downgrade(),
            pane_uuid,
            key,
            state: if duplicate {
                RestoreState::Attention("Already restoring in another tab")
            } else {
                RestoreState::Queued
            },
            started_at: None,
            cancel_pending_command: Some(cancel_pending_command),
            retry_stalled_command,
            retrying: false,
            continue_pending,
            restored_at: None,
        });
        self.completed_at = None;
        self.notice_dismissed = false;
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

    /// The first open tab whose restore needs the user, for the notice to jump to.
    pub(crate) fn first_attention_terminal(&self, ctx: &AppContext) -> Option<EntityId> {
        self.entries
            .iter()
            .filter(|entry| matches!(entry.state, RestoreState::Attention(_)))
            .find(|entry| entry.terminal.upgrade(ctx).is_some())
            .map(|entry| entry.terminal.id())
    }

    /// How many restores are stuck because the shell never started their command.
    pub(crate) fn stalled_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.state == RestoreState::Attention(DID_NOT_START_LABEL))
            .count()
    }

    /// Tries again every restore whose shell never started its command, through the queue so they
    /// still start a couple at a time.
    pub(crate) fn retry_stalled(&mut self, ctx: &mut ModelContext<Self>) {
        for entry in &mut self.entries {
            if entry.state != RestoreState::Attention(DID_NOT_START_LABEL) {
                continue;
            }
            let Some(terminal) = entry.terminal.upgrade(ctx) else {
                continue;
            };
            let retry = entry.retry_stalled_command.clone();
            terminal.update(ctx, |view, _| {
                view.clear_agent_command_stalled();
                view.defer_agent_start(Box::new(move |ctx| retry(ctx)));
            });
            entry.state = RestoreState::Queued;
            entry.started_at = None;
            entry.retrying = true;
        }
        self.completed_at = None;
        self.notice_dismissed = false;
        ctx.emit(());
        if !self.ticking {
            self.ticking = true;
            self.schedule_tick(ctx);
        }
    }

    pub(crate) fn dismiss_notice(&mut self, ctx: &mut ModelContext<Self>) {
        self.notice_dismissed = true;
        ctx.emit(());
    }

    pub(crate) fn progress_text(&self) -> Option<String> {
        if self.notice_dismissed {
            return None;
        }
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
        let stalled = self.stalled_count();
        if queued + starting == 0 && stalled > 0 && stalled == attention {
            Some(match stalled {
                1 => "1 session didn't start: its shell may be waiting for input".to_owned(),
                count => {
                    format!("{count} sessions didn't start: their shells may be waiting for input")
                }
            })
        } else if queued + starting > 0 {
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

    /// Tells a restored agent to carry on once it can safely take a prompt.
    fn continue_turn(
        entry: &mut RestoreEntry,
        terminal: &ViewHandle<TerminalView>,
        ctx: &mut ModelContext<Self>,
    ) {
        let restored_at = *entry.restored_at.get_or_insert_with(Instant::now);
        let view = terminal.as_ref(ctx);
        let (revision, unavailable) = view.local_control_agent_readiness(ctx);
        match continue_step(
            unavailable,
            view.agent_composer_is_empty(ctx),
            view.agent_restore_has_user_input(),
            restored_at.elapsed(),
        ) {
            ContinueStep::Wait => {}
            ContinueStep::Send => {
                entry.continue_pending = !terminal.update(ctx, |view, ctx| {
                    view.local_control_send_agent_text(
                        restart_continue_prompt(),
                        &revision,
                        |_| {},
                        ctx,
                    )
                });
            }
            ContinueStep::Finished => entry.continue_pending = false,
            ContinueStep::GiveUp => {
                entry.continue_pending = false;
                entry.state = RestoreState::Attention(CONTINUE_FAILED_LABEL);
            }
        }
    }

    fn tick(&mut self, ctx: &mut ModelContext<Self>) {
        let mut changed = false;
        for entry in &mut self.entries {
            let before = entry.state;
            let Some(terminal) = entry.terminal.upgrade(ctx) else {
                entry.state = RestoreState::Closed;
                entry.continue_pending = false;
                changed |= before != entry.state;
                continue;
            };
            // A flagged tab whose agent is up and waiting for input is fine, even when it came
            // back under a new session id (a teleported or forked resume) or took longer than the
            // start timeout. There is nothing left for the user to check. A failed continue stays
            // flagged, because an agent that's up says nothing about the turn it never resumed.
            if matches!(entry.state, RestoreState::Attention(_))
                && entry.state != RestoreState::Attention(CONTINUE_FAILED_LABEL)
                && terminal.as_ref(ctx).agent_restore_is_ready(ctx)
            {
                entry.state = RestoreState::Restored;
                entry.cancel_pending_command = None;
            }
            if entry.state == RestoreState::Starting {
                let session = CLIAgentSessionsModel::as_ref(ctx).session(terminal.id());
                if terminal.as_ref(ctx).agent_restore_is_ready(ctx) {
                    entry.state = RestoreState::Restored;
                    entry.cancel_pending_command = None;
                } else if terminal.as_ref(ctx).agent_command_stalled() {
                    // Don't keep a start slot for the full timeout: every other session is waiting
                    // behind this one, and this one isn't coming without help.
                    entry.state = RestoreState::Attention(DID_NOT_START_LABEL);
                } else if session.and_then(|session| session.session_key()).as_ref()
                    != Some(&entry.key)
                {
                    entry.state = RestoreState::Attention("Restore stopped — check this tab");
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
            if entry.state == RestoreState::Restored && entry.continue_pending {
                Self::continue_turn(entry, &terminal, ctx);
            }
            changed |= before != entry.state;
        }

        let mut available = CONCURRENT_STARTS.saturating_sub(
            self.entries
                .iter()
                .filter(|entry| entry.state == RestoreState::Starting)
                .count(),
        );
        // Whatever was running comes back first. A stable sort keeps tab order within each group.
        let mut start_order: Vec<usize> = (0..self.entries.len()).collect();
        start_order.sort_by_key(|&index| !self.entries[index].continue_pending);
        for index in start_order {
            let entry = &mut self.entries[index];
            if available == 0 {
                break;
            }
            if entry.state != RestoreState::Queued {
                continue;
            }
            let Some(terminal) = entry.terminal.upgrade(ctx) else {
                continue;
            };
            if (!entry.retrying && terminal.as_ref(ctx).agent_restore_has_user_input())
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
        self.ticking = self.entries.iter().any(|entry| {
            matches!(entry.state, RestoreState::Queued | RestoreState::Starting)
                || (entry.state == RestoreState::Restored && entry.continue_pending)
        });
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
        } else if self
            .entries
            .iter()
            .any(|entry| matches!(entry.state, RestoreState::Attention(_)))
        {
            // Keep rechecking flagged tabs. An idle or hookless agent sends no events that would
            // otherwise wake the queue, and the notice would stay up after the tab recovered.
            self.ticking = true;
            ctx.spawn(
                async { Timer::after(ATTENTION_POLL_INTERVAL).await },
                |queue, _, ctx| queue.tick(ctx),
            );
        }
    }
}

#[cfg(test)]
#[path = "restore_tests.rs"]
mod tests;
