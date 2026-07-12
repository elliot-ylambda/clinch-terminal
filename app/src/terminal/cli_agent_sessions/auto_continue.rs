//! Opt-in, per-pane "auto-continue when Claude's rate limit resets".
//!
//! When a pane's Claude session stops while its usage window is exhausted
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
//! - the armed continue is tied to the exact Claude session id that hit the
//!   limit; the view re-validates that id (and that the agent process is
//!   still the foreground command) before typing anything;
//! - any user input into the pane, any new prompt/status activity, and any
//!   session end or replacement cancels the pending continue.
//!
//! The feature is inert while the usage widget is disabled: with
//! `show_plan_limits` off, the usage poller never fetches Claude plan data,
//! so `exhausted_until` is always `None` and nothing can arm (the footer
//! toggle and Command Palette entries are hidden behind the same setting).

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use warpui::{Entity, EntityId, ModelContext, SingletonEntity};

use super::{CLIAgentSessionStatus, CLIAgentSessionsModel, CLIAgentSessionsModelEvent};
use crate::ai::blocklist::usage::CliAgentUsageModel;
use crate::terminal::CLIAgent;

/// The exact text typed-and-sent when an auto-continue fires. Matches the
/// footer's manual "Continue" quick-reply button.
pub const AUTO_CONTINUE_PROMPT: &str = "Continue";

/// Slack added after the reported reset time before firing, so we never type
/// into a window that hasn't actually reset yet (the usage API timestamps
/// are coarse and clocks skew).
pub const AUTO_CONTINUE_RESET_SLACK_SECS: i64 = 60;

/// One armed (scheduled) auto-continue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmedAutoContinue {
    /// When to type the continue: the exhausted window's reset time plus
    /// [`AUTO_CONTINUE_RESET_SLACK_SECS`].
    pub fire_at: DateTime<Utc>,
    /// The Claude session id that hit the limit. The view drops the fire
    /// unless the pane's live session still reports this exact id.
    pub session_id: String,
    /// Monotonic arm counter: a scheduled timer only fires if its generation
    /// still matches, so any disarm or re-arm invalidates older timers.
    pub generation: u64,
}

/// Pure per-pane auto-continue state machine. Every transition takes explicit
/// timestamps so tests can drive it without real timers.
#[derive(Debug, Default)]
pub struct PaneAutoContinue {
    enabled: bool,
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

    /// The pane's Claude session stopped. Arms one continue iff the toggle is
    /// on, nothing is already armed, the stopped session reported an id, and
    /// the usage window is exhausted with a KNOWN reset time still in the
    /// future — a past reset means our usage data is stale, and stale data
    /// must never trigger a send. Returns the newly armed continue.
    pub fn on_claude_session_stopped(
        &mut self,
        session_id: Option<&str>,
        exhausted_until: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Option<&ArmedAutoContinue> {
        if !self.enabled || self.armed.is_some() {
            return None;
        }
        let session_id = session_id?;
        let until = exhausted_until?;
        if until <= now {
            return None;
        }
        self.generation += 1;
        self.armed = Some(ArmedAutoContinue {
            fire_at: until + Duration::seconds(AUTO_CONTINUE_RESET_SLACK_SECS),
            session_id: session_id.to_owned(),
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
        self.armed.take().is_some()
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
        Self {
            panes: HashMap::new(),
        }
    }

    pub fn is_enabled(&self, terminal_view_id: EntityId) -> bool {
        self.panes
            .get(&terminal_view_id)
            .is_some_and(PaneAutoContinue::is_enabled)
    }

    pub fn is_armed(&self, terminal_view_id: EntityId) -> bool {
        self.panes
            .get(&terminal_view_id)
            .is_some_and(|pane| pane.armed().is_some())
    }

    /// The armed continue's fire time, for footer display.
    pub fn armed_fire_at(&self, terminal_view_id: EntityId) -> Option<DateTime<Utc>> {
        self.panes
            .get(&terminal_view_id)
            .and_then(|pane| pane.armed().map(|armed| armed.fire_at))
    }

    /// Flips the pane's opt-in. Turning it ON while the pane's Claude session
    /// is already stopped at an exhausted window arms immediately — the
    /// common flow is the user flipping the toggle only after seeing the
    /// limit message. Turning it OFF cancels any pending continue (one-click
    /// disarm).
    pub fn toggle(&mut self, terminal_view_id: EntityId, ctx: &mut ModelContext<Self>) {
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

    /// Arm path for a toggle that was flipped on after the session had
    /// already stopped at the limit (no further `StatusChanged` will come).
    fn try_arm_for_already_stopped_session(
        &mut self,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        let stopped_claude_session_id = {
            let sessions = CLIAgentSessionsModel::as_ref(ctx);
            let Some(session) = sessions.session(terminal_view_id) else {
                return;
            };
            if session.agent != CLIAgent::Claude
                || !matches!(session.status, CLIAgentSessionStatus::Success)
            {
                return;
            }
            session.session_context.session_id.clone()
        };
        self.arm_if_exhausted(terminal_view_id, stopped_claude_session_id.as_deref(), ctx);
    }

    /// Arms the pane iff Claude's usage window is currently exhausted with a
    /// known, still-future reset time (see `PlanLimits::exhausted_until`).
    fn arm_if_exhausted(
        &mut self,
        terminal_view_id: EntityId,
        session_id: Option<&str>,
        ctx: &mut ModelContext<Self>,
    ) {
        // With the usage widget disabled the poller never fetches Claude plan
        // data, so this is `None` and the feature stays inert.
        let exhausted_until = CliAgentUsageModel::as_ref(ctx)
            .latest()
            .claude
            .plan
            .and_then(|plan| plan.exhausted_until());
        let Some(pane) = self.panes.get_mut(&terminal_view_id) else {
            return;
        };
        let Some(armed) = pane.on_claude_session_stopped(session_id, exhausted_until, Utc::now())
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
                // A Claude session stopped: the only state that can arm.
                CLIAgentSessionStatus::Success if *agent == CLIAgent::Claude => {
                    self.arm_if_exhausted(
                        *terminal_view_id,
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
