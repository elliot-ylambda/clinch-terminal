//! Opt-in, per-pane "auto-continue when the CLI agent's rate limit resets".
//!
//! When a pane's Claude Code or Codex session stops while its usage window is exhausted
//! (see `PlanLimits::exhausted_until` in the `cli_agent_usage` crate), an
//! enabled pane arms exactly one "Continue" to be typed into that pane
//! shortly after the reset time. The scheduling *decision* lives in the pure
//! [`PaneAutoContinue`] state machine (clock injected, tested without real
//! timers in `auto_continue_tests.rs`); the actual timer and PTY write live
//! on the pane's `TerminalView` (`terminal/view/auto_continue.rs`), which
//! subscribes to [`AutoContinueModelEvent::Armed`].
//!
//! Safety rules encoded here:
//! - a pane must be explicitly opted in (non-persisted, per pane);
//! - at most ONE continue is sent per arm ([`PaneAutoContinue::take_fire`]
//!   consumes the armed state);
//! - an unknown reset time never arms, and a reset time already in the past
//!   never arms (stale usage data must not trigger sends);
//! - a usage refresh may confirm exhaustion shortly after the stop event, but
//!   that confirmation window is bounded so an old normal completion cannot
//!   arm much later when another pane exhausts the account;
//! - the armed continue is tied to the exact agent and, when reported, session
//!   id that hit the limit; Codex's ID-less OSC 9 fallback remains scoped to
//!   the pane's session lifecycle. The view re-validates that identity (and
//!   that the agent process is still the foreground command) before typing;
//! - any user input into the pane, any new prompt/status activity, and any
//!   session end or replacement cancels the pending continue.
//!
//! With `show_plan_limits` off, the usage poller never fetches Claude plan
//! data, so Claude cannot arm (the footer toggle and Command Palette entries
//! are hidden behind the same setting). Codex limits come from local rollout
//! files and remain available independently of that Claude-only setting.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use warpui::{Entity, EntityId, ModelContext, SingletonEntity};

use super::{CLIAgentSessionStatus, CLIAgentSessionsModel, CLIAgentSessionsModelEvent};
use crate::ai::blocklist::usage::CliAgentUsageModel;
use crate::settings::CliAgentUsageSettings;
use crate::terminal::CLIAgent;

/// The exact text typed-and-sent when an auto-continue fires. Matches the
/// footer's manual "Continue" quick-reply button.
pub const AUTO_CONTINUE_PROMPT: &str = "Continue";

/// Slack added after the reported reset time before firing, so we never type
/// into a window that hasn't actually reset yet (the usage API timestamps
/// are coarse and clocks skew).
pub const AUTO_CONTINUE_RESET_SLACK_SECS: i64 = 60;

/// How long a stopped session may wait for the usage poll that confirms the
/// stop coincided with an exhausted window. The endpoint cadence is about one
/// minute; two minutes covers one delayed request without letting an old,
/// normally completed session arm much later because another pane hit a limit.
pub const AUTO_CONTINUE_USAGE_CONFIRMATION_GRACE_SECS: i64 = 2 * 60;

/// Whether rate-limit auto-continue may be offered for this pane. Keep every
/// entry point (footer, Command Palette, and the action handler) on this one
/// predicate so hidden controls cannot still be invoked through another path.
pub(crate) fn is_auto_continue_available(
    agent: CLIAgent,
    show_plan_limits: bool,
    is_shared_session_viewer: bool,
) -> bool {
    !is_shared_session_viewer
        && match agent {
            CLIAgent::Claude => show_plan_limits,
            CLIAgent::Codex => true,
            _ => false,
        }
}

fn exhausted_until_for_agent(
    snapshot: &cli_agent_usage::UsageSnapshot,
    agent: CLIAgent,
) -> Option<DateTime<Utc>> {
    match agent {
        CLIAgent::Claude => snapshot.claude.plan.and_then(|plan| plan.exhausted_until()),
        CLIAgent::Codex => snapshot.codex.plan.and_then(|plan| plan.exhausted_until()),
        _ => None,
    }
}

/// One armed (scheduled) auto-continue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmedAutoContinue {
    /// When to type the continue: the exhausted window's reset time plus
    /// [`AUTO_CONTINUE_RESET_SLACK_SECS`].
    pub fire_at: DateTime<Utc>,
    /// The provider that hit the limit. The view drops the fire unless the
    /// pane's live session still runs this exact agent.
    pub agent: CLIAgent,
    /// The session id that hit the limit, when the provider reports one. `None`
    /// is allowed only for Codex's OSC 9 fallback; the view then requires the
    /// live Codex session to remain ID-less in this same pane.
    pub session_id: Option<String>,
    /// Monotonic arm counter: a scheduled timer only fires if its generation
    /// still matches, so any disarm or re-arm invalidates older timers.
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingUsageConfirmation {
    agent: CLIAgent,
    session_id: Option<String>,
    expires_at: DateTime<Utc>,
}

/// Pure per-pane auto-continue state machine. Every transition takes explicit
/// timestamps so tests can drive it without real timers.
#[derive(Debug, Default)]
pub struct PaneAutoContinue {
    enabled: bool,
    pending_usage_confirmation: Option<PendingUsageConfirmation>,
    armed: Option<ArmedAutoContinue>,
    generation: u64,
}

impl PaneAutoContinue {
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn armed(&self) -> Option<&ArmedAutoContinue> {
        self.armed.as_ref()
    }

    fn has_pending_continue(&self) -> bool {
        self.pending_usage_confirmation.is_some() || self.armed.is_some()
    }

    fn pending_agent(&self) -> Option<CLIAgent> {
        self.pending_usage_confirmation
            .as_ref()
            .map(|pending| pending.agent)
    }

    /// Turns the opt-in on or off. Turning it off always disarms.
    /// Returns `true` if anything changed.
    pub fn set_enabled(&mut self, enabled: bool) -> bool {
        if self.enabled == enabled {
            return false;
        }
        self.enabled = enabled;
        if !enabled {
            self.disarm();
        }
        true
    }

    /// The pane's supported CLI-agent session stopped. Arms one continue iff the toggle is
    /// on, nothing is already armed, the session identity is safe to track,
    /// and the usage window is exhausted with a KNOWN reset time still in the
    /// future — a past reset means our usage data is stale, and stale data must
    /// never trigger a send. Returns the newly armed continue.
    pub fn on_agent_session_stopped(
        &mut self,
        agent: CLIAgent,
        session_id: Option<&str>,
        exhausted_until: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Option<&ArmedAutoContinue> {
        if !self.enabled || self.armed.is_some() {
            return None;
        }
        let session_id = match (agent, session_id) {
            (_, Some(session_id)) => Some(session_id.to_owned()),
            // Native Codex OSC 9 notifications predate structured session IDs.
            // This state is still keyed to the pane; Started/Ended, user input,
            // and foreground-process validation prevent it crossing sessions.
            (CLIAgent::Codex, None) => None,
            (_, None) => return None,
        };
        self.pending_usage_confirmation = Some(PendingUsageConfirmation {
            agent,
            session_id,
            expires_at: now + Duration::seconds(AUTO_CONTINUE_USAGE_CONFIRMATION_GRACE_SECS),
        });
        self.retry_pending_usage_confirmation(exhausted_until, now)
    }

    /// Retries a recent stopped session against a newly refreshed usage
    /// snapshot. Keeping a bounded pending stop avoids both event-order races
    /// and surprise sends to sessions that completed normally long ago.
    fn retry_pending_usage_confirmation(
        &mut self,
        exhausted_until: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Option<&ArmedAutoContinue> {
        if !self.enabled || self.armed.is_some() {
            return None;
        }
        if self
            .pending_usage_confirmation
            .as_ref()
            .is_some_and(|pending| pending.expires_at <= now)
        {
            self.pending_usage_confirmation.take();
            return None;
        }
        let until = exhausted_until?;
        if until <= now {
            return None;
        }
        let pending = self.pending_usage_confirmation.take()?;
        self.generation += 1;
        self.armed = Some(ArmedAutoContinue {
            fire_at: until + Duration::seconds(AUTO_CONTINUE_RESET_SLACK_SECS),
            agent: pending.agent,
            session_id: pending.session_id,
            generation: self.generation,
        });
        self.armed.as_ref()
    }

    /// Cancels any pending continue and invalidates all scheduled timers
    /// (the generation advances even when nothing was armed, so a stale
    /// timer can never match a later arm's generation by accident).
    /// Returns `true` if a pending continue was cancelled.
    pub fn disarm(&mut self) -> bool {
        self.generation += 1;
        let cancelled_pending = self.pending_usage_confirmation.take().is_some();
        let cancelled_armed = self.armed.take().is_some();
        cancelled_pending || cancelled_armed
    }

    /// Consumes the armed continue for a firing timer. Returns `None` (never
    /// fire) unless the pane is still opted in and `generation` matches the
    /// live arm. Consuming the state guarantees at most one fire per arm.
    pub fn take_fire(&mut self, generation: u64) -> Option<ArmedAutoContinue> {
        if !self.enabled {
            return None;
        }
        if self.armed.as_ref()?.generation != generation {
            return None;
        }
        self.armed.take()
    }
}

/// Events emitted by [`AutoContinueModel`].
#[derive(Debug, Clone)]
pub enum AutoContinueModelEvent {
    /// The pane's toggle or armed state changed (footers re-render).
    Changed { terminal_view_id: EntityId },
    /// A continue was armed; the pane's `TerminalView` schedules the timer.
    Armed {
        terminal_view_id: EntityId,
        fire_at: DateTime<Utc>,
        generation: u64,
    },
}

impl AutoContinueModelEvent {
    pub fn terminal_view_id(&self) -> EntityId {
        match self {
            AutoContinueModelEvent::Changed { terminal_view_id }
            | AutoContinueModelEvent::Armed {
                terminal_view_id, ..
            } => *terminal_view_id,
        }
    }
}

/// Singleton tracking each pane's auto-continue opt-in and armed state.
/// Entries exist only for panes the user has touched; a session end or
/// replacement drops the pane's entry entirely (the opt-in is v1
/// non-persisted and scoped to the session in the pane).
pub struct AutoContinueModel {
    panes: HashMap<EntityId, PaneAutoContinue>,
}

impl Entity for AutoContinueModel {
    type Event = AutoContinueModelEvent;
}

impl SingletonEntity for AutoContinueModel {}

impl AutoContinueModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(
            &CLIAgentSessionsModel::handle(ctx),
            |me, _handle, event, ctx| {
                me.on_sessions_event(event, ctx);
            },
        );
        // A hard-stop notification can beat the comparatively slow usage
        // poll. Retry stopped, opted-in panes whenever fresh usage arrives so
        // an earlier 99%/missing snapshot cannot leave the toggle silently on
        // but unarmed.
        ctx.subscribe_to_model(
            &CliAgentUsageModel::handle(ctx),
            |me, _handle, _event, ctx| {
                me.retry_pending_usage_confirmations(ctx);
            },
        );
        // This setting gates only Claude's remote plan data. Disabling it hides
        // Claude's cancellation controls, so clear Claude opt-ins while leaving
        // Codex panes alone (their limits come from local rollout files).
        ctx.subscribe_to_model(
            &CliAgentUsageSettings::handle(ctx),
            |me, _handle, _event, ctx| {
                if !*CliAgentUsageSettings::as_ref(ctx).show_plan_limits {
                    me.disable_agent(CLIAgent::Claude, ctx);
                }
            },
        );
        Self {
            panes: HashMap::new(),
        }
    }

    pub fn is_enabled(&self, terminal_view_id: EntityId) -> bool {
        self.panes
            .get(&terminal_view_id)
            .is_some_and(PaneAutoContinue::is_enabled)
    }

    #[cfg(test)]
    pub fn is_armed(&self, terminal_view_id: EntityId) -> bool {
        self.panes
            .get(&terminal_view_id)
            .is_some_and(|pane| pane.armed().is_some())
    }

    pub fn has_pending_continue(&self, terminal_view_id: EntityId) -> bool {
        self.panes
            .get(&terminal_view_id)
            .is_some_and(PaneAutoContinue::has_pending_continue)
    }

    /// The armed continue's fire time, for footer display.
    pub fn armed_fire_at(&self, terminal_view_id: EntityId) -> Option<DateTime<Utc>> {
        self.panes
            .get(&terminal_view_id)
            .and_then(|pane| pane.armed().map(|armed| armed.fire_at))
    }

    /// Flips the pane's opt-in. Turning it ON while the pane's supported session
    /// is already stopped at an exhausted window arms immediately — the
    /// common flow is the user flipping the toggle only after seeing the
    /// limit message. Turning it OFF cancels any pending continue (one-click
    /// disarm).
    pub fn toggle(&mut self, terminal_view_id: EntityId, ctx: &mut ModelContext<Self>) {
        let is_available = CLIAgentSessionsModel::as_ref(ctx)
            .session(terminal_view_id)
            .is_some_and(|session| {
                is_auto_continue_available(
                    session.agent,
                    *CliAgentUsageSettings::as_ref(ctx).show_plan_limits,
                    false,
                )
            });
        if !is_available {
            self.disable(terminal_view_id, ctx);
            return;
        }

        let enable = !self.is_enabled(terminal_view_id);
        let pane = self.panes.entry(terminal_view_id).or_default();
        if !pane.set_enabled(enable) {
            return;
        }
        ctx.emit(AutoContinueModelEvent::Changed { terminal_view_id });
        if enable {
            self.try_arm_for_already_stopped_session(terminal_view_id, ctx);
        }
    }

    /// Turns this pane off if it was opted in, invalidating any scheduled
    /// timer generation. Used by defensive runtime guards as well as UI
    /// actions that become unavailable while pending.
    pub fn disable(&mut self, terminal_view_id: EntityId, ctx: &mut ModelContext<Self>) {
        if self
            .panes
            .get_mut(&terminal_view_id)
            .is_some_and(|pane| pane.set_enabled(false))
        {
            ctx.emit(AutoContinueModelEvent::Changed { terminal_view_id });
        }
    }

    /// The user typed or sent something into the pane — cancel any pending
    /// continue (the user has taken over).
    pub fn notice_user_activity(
        &mut self,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.disarm(terminal_view_id, ctx);
    }

    /// Consumes a due fire for the view-side timer. See
    /// [`PaneAutoContinue::take_fire`] for the exactly-once guarantee.
    pub fn take_due_fire(
        &mut self,
        terminal_view_id: EntityId,
        generation: u64,
        ctx: &mut ModelContext<Self>,
    ) -> Option<ArmedAutoContinue> {
        let fired = self.panes.get_mut(&terminal_view_id)?.take_fire(generation);
        if fired.is_some() {
            ctx.emit(AutoContinueModelEvent::Changed { terminal_view_id });
        }
        fired
    }

    fn disarm(&mut self, terminal_view_id: EntityId, ctx: &mut ModelContext<Self>) {
        if self
            .panes
            .get_mut(&terminal_view_id)
            .is_some_and(|pane| pane.disarm())
        {
            ctx.emit(AutoContinueModelEvent::Changed { terminal_view_id });
        }
    }

    fn disable_agent(&mut self, agent: CLIAgent, ctx: &mut ModelContext<Self>) {
        let enabled_view_ids: Vec<_> = {
            let sessions = CLIAgentSessionsModel::as_ref(ctx);
            self.panes
                .iter()
                .filter_map(|(view_id, pane)| {
                    (pane.is_enabled()
                        && sessions
                            .session(*view_id)
                            .is_some_and(|session| session.agent == agent))
                    .then_some(*view_id)
                })
                .collect()
        };
        for view_id in enabled_view_ids {
            self.disable(view_id, ctx);
        }
    }

    fn retry_pending_usage_confirmations(&mut self, ctx: &mut ModelContext<Self>) {
        let show_plan_limits = *CliAgentUsageSettings::as_ref(ctx).show_plan_limits;
        let (claude_exhausted_until, codex_exhausted_until) = {
            let snapshot = CliAgentUsageModel::as_ref(ctx).latest();
            (
                exhausted_until_for_agent(snapshot, CLIAgent::Claude),
                exhausted_until_for_agent(snapshot, CLIAgent::Codex),
            )
        };
        let now = Utc::now();
        let pending_panes: Vec<_> = self
            .panes
            .iter()
            .filter_map(|(view_id, pane)| {
                if pane.is_enabled() && pane.armed().is_none() {
                    pane.pending_agent().map(|agent| (*view_id, agent))
                } else {
                    None
                }
            })
            .collect();
        for (view_id, agent) in pending_panes {
            if !is_auto_continue_available(agent, show_plan_limits, false) {
                self.disable(view_id, ctx);
                continue;
            }
            let exhausted_until = match agent {
                CLIAgent::Claude => claude_exhausted_until,
                CLIAgent::Codex => codex_exhausted_until,
                _ => None,
            };
            let Some(armed) = self
                .panes
                .get_mut(&view_id)
                .and_then(|pane| pane.retry_pending_usage_confirmation(exhausted_until, now))
            else {
                continue;
            };
            let (fire_at, generation) = (armed.fire_at, armed.generation);
            ctx.emit(AutoContinueModelEvent::Armed {
                terminal_view_id: view_id,
                fire_at,
                generation,
            });
            ctx.emit(AutoContinueModelEvent::Changed {
                terminal_view_id: view_id,
            });
        }
    }

    /// Arm path for a toggle that was flipped on after the session had
    /// already stopped at the limit (no further `StatusChanged` will come).
    fn try_arm_for_already_stopped_session(
        &mut self,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        let (agent, stopped_session_id) = {
            let sessions = CLIAgentSessionsModel::as_ref(ctx);
            let Some(session) = sessions.session(terminal_view_id) else {
                return;
            };
            if !is_auto_continue_available(
                session.agent,
                *CliAgentUsageSettings::as_ref(ctx).show_plan_limits,
                false,
            ) || !matches!(session.status, CLIAgentSessionStatus::Success)
            {
                return;
            }
            (session.agent, session.session_context.session_id.clone())
        };
        self.arm_if_exhausted(terminal_view_id, agent, stopped_session_id.as_deref(), ctx);
    }

    /// Records the stopped session and arms immediately iff that provider's usage
    /// window is already known to be exhausted. Otherwise a short-lived
    /// pending confirmation lets the next usage refresh complete the arm.
    fn arm_if_exhausted(
        &mut self,
        terminal_view_id: EntityId,
        agent: CLIAgent,
        session_id: Option<&str>,
        ctx: &mut ModelContext<Self>,
    ) {
        if !is_auto_continue_available(
            agent,
            *CliAgentUsageSettings::as_ref(ctx).show_plan_limits,
            false,
        ) {
            self.disable(terminal_view_id, ctx);
            return;
        }
        let exhausted_until =
            exhausted_until_for_agent(CliAgentUsageModel::as_ref(ctx).latest(), agent);
        let Some(pane) = self.panes.get_mut(&terminal_view_id) else {
            return;
        };
        let Some(armed) =
            pane.on_agent_session_stopped(agent, session_id, exhausted_until, Utc::now())
        else {
            return;
        };
        let (fire_at, generation) = (armed.fire_at, armed.generation);
        ctx.emit(AutoContinueModelEvent::Armed {
            terminal_view_id,
            fire_at,
            generation,
        });
        ctx.emit(AutoContinueModelEvent::Changed { terminal_view_id });
    }

    fn on_sessions_event(
        &mut self,
        event: &CLIAgentSessionsModelEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            CLIAgentSessionsModelEvent::StatusChanged {
                terminal_view_id,
                agent,
                status,
                session_context,
            } => match status {
                // A supported session stopped: the only state that can arm.
                CLIAgentSessionStatus::Success
                    if matches!(*agent, CLIAgent::Claude | CLIAgent::Codex) =>
                {
                    self.arm_if_exhausted(
                        *terminal_view_id,
                        *agent,
                        session_context.session_id.as_deref(),
                        ctx,
                    );
                }
                // Any other transition means the session is active again
                // (new prompt submitted, permission replied, waiting on the
                // user) — cancel a pending continue.
                CLIAgentSessionStatus::Success
                | CLIAgentSessionStatus::InProgress
                | CLIAgentSessionStatus::Blocked { .. } => {
                    self.disarm(*terminal_view_id, ctx);
                }
            },
            // Session ended (agent exited or pane closed) or replaced by a
            // new session: drop the pane's opt-in and any pending continue.
            CLIAgentSessionsModelEvent::Started {
                terminal_view_id, ..
            }
            | CLIAgentSessionsModelEvent::Ended {
                terminal_view_id, ..
            } => {
                if self.panes.remove(terminal_view_id).is_some() {
                    ctx.emit(AutoContinueModelEvent::Changed {
                        terminal_view_id: *terminal_view_id,
                    });
                }
            }
            CLIAgentSessionsModelEvent::InputSessionChanged { .. }
            | CLIAgentSessionsModelEvent::SessionUpdated { .. } => {}
        }
    }
}

#[cfg(test)]
#[path = "auto_continue_tests.rs"]
mod tests;
