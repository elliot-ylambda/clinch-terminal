//! Brings saved conversations back in their existing panes. A session a restart cut off mid-turn
//! starts again at launch and is told to carry on. Every other session stays asleep, with no agent
//! process, until its tab is opened or something is sent to it.
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use instant::Instant;
use warpui::r#async::Timer;
use warpui::{
    AppContext, Entity, EntityId, ModelContext, SingletonEntity, ViewHandle, WeakViewHandle,
};

use super::{CLIAgentSessionKey, CLIAgentSessionsModel};
use crate::agent_resume::{
    read_asleep_panes, read_interrupted_turns, write_asleep_panes, write_interrupted_turns,
    RESTART_CONTINUE_MARKER,
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
    /// No agent process: the tab waits for someone to open it or send to it.
    Asleep,
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
            Self::Asleep | Self::Restored | Self::Closed => None,
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

/// Every asleep pane and when it went to sleep, across every window and project.
fn asleep_panes(ctx: &AppContext) -> BTreeMap<Vec<u8>, DateTime<Utc>> {
    let mut panes = BTreeMap::new();
    for (_, workspace) in WorkspaceRegistry::as_ref(ctx).all_workspaces(ctx) {
        for group in workspace.as_ref(ctx).tab_views() {
            panes.extend(group.as_ref(ctx).asleep_agent_panes(ctx));
        }
    }
    panes
}

/// Running commands that a restart right now would end for good, across every window and project.
pub(crate) fn running_commands_outside_agents(ctx: &AppContext) -> usize {
    WorkspaceRegistry::as_ref(ctx)
        .all_workspaces(ctx)
        .into_iter()
        .flat_map(|(_, workspace)| workspace.as_ref(ctx).tab_views().collect::<Vec<_>>())
        .map(|group| group.as_ref(ctx).running_commands_outside_agents(ctx))
        .sum()
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
    /// The agent was asleep and is starting because its tab was opened or sent to, not because
    /// Clinch is restoring it at launch.
    woken: bool,
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
    /// Since when the previous run's asleep panes were asleep, until a restored tab claims it.
    unclaimed_asleep_since: HashMap<Vec<u8>, DateTime<Utc>>,
    last_recorded_asleep: Option<BTreeMap<Vec<u8>, DateTime<Utc>>>,
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
        self.unclaimed_asleep_since = read_asleep_panes();
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
        self.unclaimed_asleep_since.clear();
        let asleep = asleep_panes(ctx);
        if self.last_recorded_asleep.as_ref() != Some(&asleep) {
            write_asleep_panes(&asleep);
            self.last_recorded_asleep = Some(asleep);
        }
    }

    /// Takes charge of a pane restored at launch, whose agent is not running yet. A pane cut off
    /// mid-turn starts through the queue, a couple at a time. Every other pane stays asleep until
    /// its tab is opened or something is sent to it: starting them all is what once left dozens of
    /// unused agents running after every launch.
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
        let continue_pending = self.unclaimed_interrupted_panes.remove(&pane_uuid);
        let asleep_since = self
            .unclaimed_asleep_since
            .remove(&pane_uuid)
            .unwrap_or_else(Utc::now);
        self.insert(
            terminal,
            pane_uuid,
            key,
            cancel_pending_command,
            retry_stalled_command,
            (!continue_pending).then_some(asleep_since),
            continue_pending,
            true,
            ctx,
        );
    }

    /// Takes charge of a pane whose agent Clinch just stopped to put its tab to sleep. Waking it
    /// goes through the same checks as a restore at launch.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn put_to_sleep(
        &mut self,
        terminal: &ViewHandle<TerminalView>,
        pane_uuid: Vec<u8>,
        key: CLIAgentSessionKey,
        asleep_since: DateTime<Utc>,
        cancel_pending_command: CancelPendingCommand,
        retry_stalled_command: RetryStalledCommand,
        ctx: &mut ModelContext<Self>,
    ) {
        self.entries
            .retain(|entry| entry.terminal.id() != terminal.id());
        self.record_asleep(pane_uuid.clone(), asleep_since);
        self.insert(
            terminal,
            pane_uuid,
            key,
            cancel_pending_command,
            retry_stalled_command,
            Some(asleep_since),
            false,
            // Its agent has already stopped, so it is asleep whatever else is open. Another tab
            // on the same conversation is no reason to leave this one in no state at all.
            false,
            ctx,
        );
    }

    /// Writes down right away that a pane is asleep, or about to be. The regular record is only
    /// taken every few seconds, and the updater reads this list to know which panes without an
    /// agent process must keep their saved session.
    pub(crate) fn record_asleep(&mut self, pane_uuid: Vec<u8>, since: DateTime<Utc>) {
        let mut asleep = self.last_recorded_asleep.clone().unwrap_or_default();
        if asleep.insert(pane_uuid, since) != Some(since) {
            write_asleep_panes(&asleep);
            self.last_recorded_asleep = Some(asleep);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn insert(
        &mut self,
        terminal: &ViewHandle<TerminalView>,
        pane_uuid: Vec<u8>,
        key: CLIAgentSessionKey,
        cancel_pending_command: CancelPendingCommand,
        retry_stalled_command: RetryStalledCommand,
        asleep_since: Option<DateTime<Utc>>,
        continue_pending: bool,
        refuse_duplicates: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        // A second writer of the same conversation must never be started, by the queue or by
        // opening its tab.
        let duplicate = refuse_duplicates
            && self
                .entries
                .iter()
                .any(|entry| entry.key == key && entry.terminal.upgrade(ctx).is_some());
        let state = match (duplicate, asleep_since) {
            (true, _) => RestoreState::Attention("Already restoring in another tab"),
            (false, Some(_)) => RestoreState::Asleep,
            (false, None) => RestoreState::Queued,
        };
        terminal.update(ctx, |view, ctx| match (state, asleep_since) {
            (RestoreState::Asleep, Some(since)) => view.mark_agent_asleep(since, ctx),
            _ => view.manage_agent_restore_in_background(),
        });
        self.entries.push(RestoreEntry {
            terminal: terminal.downgrade(),
            pane_uuid,
            key,
            state,
            started_at: None,
            cancel_pending_command: Some(cancel_pending_command),
            retry_stalled_command,
            retrying: false,
            continue_pending,
            restored_at: None,
            woken: false,
        });
        if state == RestoreState::Asleep {
            ctx.emit(());
            return;
        }
        self.completed_at = None;
        self.notice_dismissed = false;
        ctx.emit(());
        if !self.ticking {
            self.ticking = true;
            self.schedule_tick(ctx);
        }
    }

    /// An asleep pane's agent was just started, by its tab or by a message to it. From here the
    /// queue watches it come up the way it watches a restore.
    pub(crate) fn woke(&mut self, terminal: EntityId, ctx: &mut ModelContext<Self>) {
        let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.terminal.id() == terminal && entry.state == RestoreState::Asleep)
        else {
            return;
        };
        entry.state = RestoreState::Starting;
        entry.started_at = Some(Instant::now());
        entry.woken = true;
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

    /// What this pane's tab says under its title while its agent comes back.
    pub(crate) fn label(&self, terminal: EntityId) -> Option<&'static str> {
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.terminal.id() == terminal)?;
        match entry.state {
            RestoreState::Starting if entry.woken => Some("Waking up…"),
            state => state.label(),
        }
    }

    /// Whether the queue is still starting this pane's agent, or has it flagged for the user.
    pub(crate) fn is_bringing_back(&self, terminal: EntityId) -> bool {
        self.entries
            .iter()
            .find(|entry| entry.terminal.id() == terminal)
            .is_some_and(|entry| match entry.state {
                RestoreState::Queued | RestoreState::Starting | RestoreState::Attention(_) => true,
                RestoreState::Restored => entry.continue_pending,
                RestoreState::Asleep | RestoreState::Closed => false,
            })
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
        // The notice is about sessions Clinch is restoring at launch. An asleep tab isn't one of
        // them, and a tab someone woke says so on its own row, unless its start needs the user.
        let noticed = |entry: &&RestoreEntry| {
            entry.state != RestoreState::Asleep
                && (!entry.woken || matches!(entry.state, RestoreState::Attention(_)))
        };
        let count = |state| {
            self.entries
                .iter()
                .filter(noticed)
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
        let total = self.entries.iter().filter(noticed).count() - count(RestoreState::Closed);
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
            if before == RestoreState::Starting && entry.state != RestoreState::Starting {
                // Up, or flagged for the user: either way the tab stops saying it is waking.
                terminal.update(ctx, |view, ctx| view.agent_wake_settled(ctx));
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
            && self.entries.iter().all(|entry| {
                matches!(
                    entry.state,
                    RestoreState::Asleep | RestoreState::Restored | RestoreState::Closed
                )
            })
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
