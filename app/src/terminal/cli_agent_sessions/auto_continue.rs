//! Per-pane "auto-continue when the CLI agent's rate limit resets", on by
//! default (`CliAgentUsageSettings::auto_continue_on_limit_reset`) with a
//! per-session footer override.
//!
//! When a pane's Claude Code or Codex session stops at its usage limit, an
//! enabled pane arms exactly one "Continue" to be typed into that pane
//! shortly after the reset. The reset comes from the provider: Claude Code
//! states it in the limit message itself, refined by plan-usage data when
//! the user enabled it; Codex reports it in its rollout `rate_limits` (see
//! `PlanLimits::exhausted_until` in the `cli_agent_usage` crate).
//!
//! The scheduling *decision* lives in the pure [`PaneAutoContinue`] state
//! machine (clock injected, tested without real timers in
//! `auto_continue_tests.rs`); the actual timer and PTY write live on the
//! pane's `TerminalView` (`terminal/view/auto_continue.rs`), which subscribes
//! to [`AutoContinueModelEvent::Armed`].
//!
//! Safety rules encoded here:
//! - a per-session footer choice overrides the global default for that exact
//!   durable identity; the choice and any causal arm are persisted
//!   machine-locally so an app restart cannot silently lose the timer;
//! - at most ONE continue is sent per arm ([`PaneAutoContinue::prepare_due_fire`]
//!   consumes the armed state only after final validation);
//! - an unknown reset time never arms, and a reset time already in the past
//!   never arms (stale usage data must not trigger sends);
//! - only a provider-reported usage-limit Stop may begin confirmation; a normal
//!   Success can never arm from account-wide exhaustion in another pane;
//! - the armed continue is tied to the exact agent and, when reported, session
//!   id that hit the limit. ID-less/legacy and remote sessions are not
//!   eligible. The view re-validates identity, provider usage, and foreground
//!   ownership before typing;
//! - any user input into the pane, any new prompt/status activity, and any
//!   session end or replacement cancels the pending continue. Pointer reports
//!   (wheel/clicks in a mouse-tracking TUI) are not input.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use cli_agent_usage::{ExhaustionStatus, PlanLimits, UsageSnapshot};
use settings::Setting as _;
use warpui::{AppContext, Entity, EntityId, ModelContext, SingletonEntity};

use super::event::CLIAgentStopReason;
use super::{
    CLIAgentSession, CLIAgentSessionStatus, CLIAgentSessionsModel, CLIAgentSessionsModelEvent,
};
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

/// Retry cadence when the reset is due but the provider temporarily omits the
/// reset timestamp, or a validated PTY submission cannot be initiated.
pub const AUTO_CONTINUE_RETRY_SECS: i64 = 60;
const MAX_DELIVERY_ATTEMPTS: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoContinueAvailability {
    Ready,
    Unsupported,
}

impl AutoContinueAvailability {
    pub fn may_render(self, enabled: bool) -> bool {
        enabled || matches!(self, Self::Ready)
    }

    pub fn may_enable(self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// Whether rate-limit auto-continue may be offered for this pane. Keep every
/// entry point (footer, Command Palette, and the action handler) on this one
/// predicate so hidden controls cannot still be invoked through another path.
///
/// Plan-usage data is deliberately not required: Claude Code states its reset
/// in the limit message, and Codex writes its windows into the rollout at the
/// moment it stops. Without a known reset a stop stays pending (visible in the
/// footer) instead of arming.
pub(crate) fn auto_continue_availability(
    session: &CLIAgentSession,
    is_shared_session_viewer: bool,
) -> AutoContinueAvailability {
    if is_shared_session_viewer || session.is_remote() || !supports_causal_limit_events(session) {
        return AutoContinueAvailability::Unsupported;
    }
    match session.agent {
        CLIAgent::Claude | CLIAgent::Codex => AutoContinueAvailability::Ready,
        _ => AutoContinueAvailability::Unsupported,
    }
}

fn supports_causal_limit_events(session: &CLIAgentSession) -> bool {
    if !session.received_rich_notification
        || session
            .session_context
            .session_id
            .as_deref()
            .is_none_or(|id| id.trim().is_empty())
    {
        return false;
    }

    #[cfg(target_family = "wasm")]
    {
        false
    }
    #[cfg(not(target_family = "wasm"))]
    {
        let minimum = match session.agent {
            CLIAgent::Claude => super::plugin_manager::claude::MINIMUM_PLUGIN_VERSION,
            CLIAgent::Codex => super::plugin_manager::codex::MINIMUM_PLUGIN_VERSION,
            _ => return false,
        };
        session.plugin_version.as_deref().is_some_and(|version| {
            super::plugin_manager::compare_versions(version, minimum).is_ge()
        })
    }
}

fn plan_for_agent(snapshot: &UsageSnapshot, agent: CLIAgent) -> Option<PlanLimits> {
    match agent {
        CLIAgent::Claude => snapshot.claude.plan,
        CLIAgent::Codex => snapshot.codex.plan,
        _ => None,
    }
}

fn exhaustion_for_agent(snapshot: &UsageSnapshot, agent: CLIAgent) -> Option<ExhaustionStatus> {
    plan_for_agent(snapshot, agent).map(|plan| plan.exhaustion_status())
}

/// A causal provider stop can arrive just before the next usage refresh. If a
/// provider-wide window is already rounded to at least 99%, its known reset is
/// better evidence than waiting for a later sample that may occur after the
/// reset and never show 100%. A genuinely full window with no timestamp stays
/// unschedulable.
fn reset_for_causal_limit_stop(snapshot: &UsageSnapshot, agent: CLIAgent) -> Option<DateTime<Utc>> {
    let plan = plan_for_agent(snapshot, agent)?;
    match plan.exhaustion_status() {
        ExhaustionStatus::ResetsAt(reset) => Some(reset),
        ExhaustionStatus::ResetUnknown => None,
        ExhaustionStatus::NotExhausted => [plan.session, plan.weekly]
            .into_iter()
            .flatten()
            .filter(|window| window.percent >= 99.0)
            .filter_map(|window| window.resets_at)
            .max(),
    }
}

/// The reset to schedule for a causal limit stop: the later of what the
/// provider stated in its limit message and what plan-usage data shows, since
/// continuing before either has passed would hit the limit again.
fn reset_for_limit_stop(
    snapshot: &UsageSnapshot,
    agent: CLIAgent,
    stated_reset: Option<DateTime<Utc>>,
) -> Option<DateTime<Utc>> {
    [reset_for_causal_limit_stop(snapshot, agent), stated_reset]
        .into_iter()
        .flatten()
        .max()
}

fn stated_reset(stop_reason: Option<CLIAgentStopReason>) -> Option<DateTime<Utc>> {
    match stop_reason {
        Some(CLIAgentStopReason::UsageLimit { stated_reset }) => stated_reset,
        None => None,
    }
}

fn persistence_key(session: &CLIAgentSession) -> Option<String> {
    let provider = match session.agent {
        CLIAgent::Claude => "claude",
        CLIAgent::Codex => "codex",
        _ => return None,
    };
    let id = session.session_context.session_id.as_deref()?.trim();
    (!id.is_empty()).then(|| format!("{provider}:{id}"))
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
    /// The durable session id that hit the limit. Eligibility requires one;
    /// legacy ID-less notifications cannot authorize an automatic PTY write.
    pub session_id: Option<String>,
    /// Monotonic arm counter: a scheduled timer only fires if its generation
    /// still matches, so any disarm or re-arm invalidates older timers.
    pub generation: u64,
    /// Failed submission initiations already retried for this arm.
    pub delivery_attempts: u8,
    /// Reconstructed from machine-local state after an app restart. The view
    /// may validate an untouched restored session even before a fresh Stop
    /// event re-establishes `Success` status.
    pub restored: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DueFireDecision {
    Fire(ArmedAutoContinue),
    Rearmed(ArmedAutoContinue),
    Ignore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingUsageConfirmation {
    agent: CLIAgent,
    session_id: Option<String>,
}

/// Pure per-pane auto-continue state machine. Every transition takes explicit
/// timestamps so tests can drive it without real timers.
#[derive(Debug, Default)]
pub struct PaneAutoContinue {
    enabled: bool,
    pending_usage_confirmation: Option<PendingUsageConfirmation>,
    armed: Option<ArmedAutoContinue>,
    generation: u64,
    persistence_key: Option<String>,
    last_delivery_error: Option<String>,
}

impl PaneAutoContinue {
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn armed(&self) -> Option<&ArmedAutoContinue> {
        self.armed.as_ref()
    }

    pub fn persistence_key(&self) -> Option<&str> {
        self.persistence_key.as_deref()
    }

    pub fn delivery_error(&self) -> Option<&str> {
        self.last_delivery_error.as_deref()
    }

    pub fn set_persistence_key(&mut self, key: Option<String>) {
        self.persistence_key = key;
    }

    fn has_pending_continue(&self) -> bool {
        self.pending_usage_confirmation.is_some() || self.armed.is_some()
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
            self.last_delivery_error = None;
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
        let session_id = session_id
            .map(str::trim)
            .filter(|session_id| !session_id.is_empty())
            .map(str::to_owned)?;
        self.pending_usage_confirmation = Some(PendingUsageConfirmation {
            agent,
            session_id: Some(session_id),
        });
        self.retry_pending_usage_confirmation(exhausted_until, now)
    }

    /// Retries a recent stopped session against a newly refreshed usage
    /// snapshot. Only a causally classified provider limit can create this
    /// state, so it remains pending across arbitrary cache/Retry-After delays
    /// until activity, session replacement, or an explicit disable cancels it.
    fn retry_pending_usage_confirmation(
        &mut self,
        exhausted_until: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Option<&ArmedAutoContinue> {
        if !self.enabled || self.armed.is_some() {
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
            delivery_attempts: 0,
            restored: false,
        });
        self.last_delivery_error = None;
        self.armed.as_ref()
    }

    fn restore_arm(
        &mut self,
        agent: CLIAgent,
        session_id: String,
        fire_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Option<&ArmedAutoContinue> {
        if !self.enabled || self.armed.is_some() {
            return None;
        }
        self.generation += 1;
        self.armed = Some(ArmedAutoContinue {
            fire_at: fire_at.max(now),
            agent,
            session_id: Some(session_id),
            generation: self.generation,
            delivery_attempts: 0,
            restored: true,
        });
        self.armed.as_ref()
    }

    /// Keeps an existing arm aligned with the provider's latest reset. A
    /// later reset invalidates the old timer by advancing the generation.
    fn reconcile_reset(
        &mut self,
        exhausted_until: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Option<&ArmedAutoContinue> {
        let reset = exhausted_until?;
        if reset <= now {
            return None;
        }
        let desired_fire_at = reset + Duration::seconds(AUTO_CONTINUE_RESET_SLACK_SECS);
        let armed = self.armed.as_ref()?;
        if armed.fire_at == desired_fire_at {
            return None;
        }

        self.generation += 1;
        let mut rearmed = armed.clone();
        rearmed.fire_at = desired_fire_at;
        rearmed.generation = self.generation;
        self.armed = Some(rearmed);
        self.armed.as_ref()
    }

    pub fn armed_for_generation(&self, generation: u64) -> Option<&ArmedAutoContinue> {
        self.enabled
            .then_some(self.armed.as_ref())
            .flatten()
            .filter(|armed| armed.generation == generation)
    }

    /// Final provider-state check performed only after the view validates the
    /// live session and foreground process. A reset that moved later re-arms
    /// precisely, and a window still exhausted without a reset time defers.
    /// Missing plan data fires: the arm already came from a causal stop whose
    /// known reset has passed, Codex drops a window from its data once it has
    /// reset (and reports nothing new until the next request), and Claude plan
    /// data is optional. A premature send just produces another limit stop,
    /// which re-arms.
    pub fn prepare_due_fire(
        &mut self,
        generation: u64,
        exhaustion: Option<ExhaustionStatus>,
        now: DateTime<Utc>,
    ) -> DueFireDecision {
        let Some(current) = self.armed_for_generation(generation).cloned() else {
            return DueFireDecision::Ignore;
        };

        match exhaustion {
            Some(ExhaustionStatus::NotExhausted) => {
                self.armed.take();
                self.last_delivery_error = None;
                DueFireDecision::Fire(current)
            }
            Some(ExhaustionStatus::ResetsAt(reset)) => {
                let fire_at = reset + Duration::seconds(AUTO_CONTINUE_RESET_SLACK_SECS);
                if fire_at <= now {
                    self.armed.take();
                    self.last_delivery_error = None;
                    DueFireDecision::Fire(current)
                } else {
                    self.generation += 1;
                    let mut rearmed = current;
                    rearmed.fire_at = fire_at;
                    rearmed.generation = self.generation;
                    self.armed = Some(rearmed.clone());
                    DueFireDecision::Rearmed(rearmed)
                }
            }
            None => {
                self.armed.take();
                self.last_delivery_error = None;
                DueFireDecision::Fire(current)
            }
            Some(ExhaustionStatus::ResetUnknown) => {
                self.generation += 1;
                let mut rearmed = current;
                rearmed.fire_at = now + Duration::seconds(AUTO_CONTINUE_RETRY_SECS);
                rearmed.generation = self.generation;
                self.armed = Some(rearmed.clone());
                DueFireDecision::Rearmed(rearmed)
            }
        }
    }

    pub fn rearm_after_delivery_failure(
        &mut self,
        mut fired: ArmedAutoContinue,
        now: DateTime<Utc>,
        message: String,
    ) -> Option<&ArmedAutoContinue> {
        self.last_delivery_error = Some(message);
        if !self.enabled || fired.delivery_attempts >= MAX_DELIVERY_ATTEMPTS {
            self.armed.take();
            return None;
        }
        fired.delivery_attempts += 1;
        self.generation += 1;
        fired.generation = self.generation;
        fired.fire_at = now + Duration::seconds(AUTO_CONTINUE_RETRY_SECS);
        self.armed = Some(fired);
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
        let cleared_error = self.last_delivery_error.take().is_some();
        cancelled_pending || cancelled_armed || cleared_error
    }

    /// Consumes the armed continue for a firing timer. Returns `None` (never
    /// fire) unless the pane is still opted in and `generation` matches the
    /// live arm. Consuming the state guarantees at most one fire per arm.
    #[cfg(test)]
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

/// Singleton tracking each pane's auto-continue enablement and armed state.
/// Pane entries are ephemeral, while per-session overrides and armed fire
/// times are stored in private machine-local settings for restoration.
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
        // Every usage scan (about every 5 s) carries Codex's rollout-recorded
        // limit stops; Codex runs no hook for them, so close those turns here.
        // A hard-stop notification can also beat the comparatively slow usage
        // poll, so retry stopped, enabled panes whenever fresh usage arrives.
        ctx.subscribe_to_model(
            &CliAgentUsageModel::handle(ctx),
            |me, _handle, _event, ctx| {
                let codex_stops = CliAgentUsageModel::as_ref(ctx)
                    .latest()
                    .codex
                    .usage_limit_stops
                    .clone();
                if !codex_stops.is_empty() {
                    CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                        sessions.apply_codex_usage_limit_stops(&codex_stops, ctx);
                    });
                }
                me.retry_pending_usage_confirmations(ctx);
            },
        );
        // Re-apply the global default whenever it (or a per-session override)
        // changes. Idempotent: panes already in the effective state are left alone.
        ctx.subscribe_to_model(
            &CliAgentUsageSettings::handle(ctx),
            |me, _handle, _event, ctx| {
                me.sync_all_panes_with_settings(ctx);
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

    pub fn is_waiting_for_reset(&self, terminal_view_id: EntityId) -> bool {
        self.panes
            .get(&terminal_view_id)
            .is_some_and(|pane| pane.pending_usage_confirmation.is_some())
    }

    /// The armed continue's fire time, for footer display.
    pub fn armed_fire_at(&self, terminal_view_id: EntityId) -> Option<DateTime<Utc>> {
        self.panes
            .get(&terminal_view_id)
            .and_then(|pane| pane.armed().map(|armed| armed.fire_at))
    }

    pub fn delivery_error(&self, terminal_view_id: EntityId) -> Option<&str> {
        self.panes
            .get(&terminal_view_id)
            .and_then(PaneAutoContinue::delivery_error)
    }

    pub fn availability(
        terminal_view_id: EntityId,
        is_shared_session_viewer: bool,
        ctx: &AppContext,
    ) -> AutoContinueAvailability {
        let Some(session) = CLIAgentSessionsModel::as_ref(ctx).session(terminal_view_id) else {
            return AutoContinueAvailability::Unsupported;
        };
        auto_continue_availability(session, is_shared_session_viewer)
    }

    /// Whether `key`'s session should auto-continue: its footer override if
    /// the user set one, otherwise the global default.
    fn effective_enabled(key: Option<&str>, ctx: &AppContext) -> bool {
        let settings = CliAgentUsageSettings::as_ref(ctx);
        key.and_then(|key| settings.auto_continue_session_overrides.get(key).copied())
            .unwrap_or(*settings.auto_continue_on_limit_reset)
    }

    /// The footer toggle: flips this pane and records the choice as a
    /// per-session override. Turning it ON while the pane's supported session
    /// is already stopped at its limit arms immediately. Turning it OFF
    /// cancels any pending continue (one-click disarm).
    pub fn toggle(&mut self, terminal_view_id: EntityId, ctx: &mut ModelContext<Self>) {
        let key = CLIAgentSessionsModel::as_ref(ctx)
            .session(terminal_view_id)
            .and_then(persistence_key);
        if self.is_enabled(terminal_view_id) {
            self.disable(terminal_view_id, ctx);
            if let Some(key) = key {
                Self::set_persisted_override(&key, false, ctx);
            }
            return;
        }

        if !Self::availability(terminal_view_id, false, ctx).may_enable() {
            return;
        }
        if let Some(key) = key.as_deref() {
            Self::set_persisted_override(key, true, ctx);
        }
        self.enable(terminal_view_id, key, ctx);
    }

    fn enable(
        &mut self,
        terminal_view_id: EntityId,
        key: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        let pane = self.panes.entry(terminal_view_id).or_default();
        pane.set_persistence_key(key);
        if !pane.set_enabled(true) {
            return;
        }
        ctx.emit(AutoContinueModelEvent::Changed { terminal_view_id });
        self.try_arm_for_already_stopped_session(terminal_view_id, ctx);
    }

    /// Turns this pane off and invalidates any scheduled timer generation,
    /// without recording a per-session override (callers that act for the
    /// user record it themselves).
    pub fn disable(&mut self, terminal_view_id: EntityId, ctx: &mut ModelContext<Self>) {
        let persisted_key = self
            .panes
            .get(&terminal_view_id)
            .and_then(PaneAutoContinue::persistence_key)
            .map(str::to_owned);
        let changed = self
            .panes
            .get_mut(&terminal_view_id)
            .is_some_and(|pane| pane.set_enabled(false));
        if let Some(key) = persisted_key {
            Self::set_persisted_arm(&key, None, ctx);
        }
        if changed {
            ctx.emit(AutoContinueModelEvent::Changed { terminal_view_id });
        }
    }

    /// Stores a footer choice only when it differs from the global default,
    /// so following the default never pins a session to an old value.
    fn set_persisted_override(key: &str, enabled: bool, ctx: &mut ModelContext<Self>) {
        CliAgentUsageSettings::handle(ctx).update(ctx, |settings, ctx| {
            let mut overrides = settings.auto_continue_session_overrides.clone();
            let changed = if enabled == *settings.auto_continue_on_limit_reset {
                overrides.remove(key).is_some()
            } else {
                overrides.insert(key.to_owned(), enabled) != Some(enabled)
            };
            if changed {
                if let Err(error) = settings
                    .auto_continue_session_overrides
                    .set_value(overrides, ctx)
                {
                    log::error!("failed to persist CLI-agent auto-continue choice: {error}");
                }
            }
        });
    }

    /// Brings every tracked pane in line with the global default and its
    /// per-session override.
    fn sync_all_panes_with_settings(&mut self, ctx: &mut ModelContext<Self>) {
        let view_ids: Vec<_> = CLIAgentSessionsModel::as_ref(ctx)
            .terminal_view_ids()
            .collect();
        for view_id in view_ids {
            let key = CLIAgentSessionsModel::as_ref(ctx)
                .session(view_id)
                .and_then(persistence_key);
            let should_enable = Self::effective_enabled(key.as_deref(), ctx);
            if should_enable && !self.is_enabled(view_id) {
                self.restore_pane_state(view_id, ctx);
            } else if !should_enable && self.is_enabled(view_id) {
                self.disable(view_id, ctx);
            }
        }
    }

    fn set_persisted_arm(key: &str, fire_at: Option<DateTime<Utc>>, ctx: &mut ModelContext<Self>) {
        CliAgentUsageSettings::handle(ctx).update(ctx, |settings, ctx| {
            let mut sessions = settings.auto_continue_armed_sessions.clone();
            let changed = match fire_at {
                Some(fire_at) => {
                    sessions.insert(key.to_owned(), fire_at.timestamp())
                        != Some(fire_at.timestamp())
                }
                None => sessions.remove(key).is_some(),
            };
            if changed {
                if let Err(error) = settings
                    .auto_continue_armed_sessions
                    .set_value(sessions, ctx)
                {
                    log::error!("failed to persist CLI-agent auto-continue arm: {error}");
                }
            }
        });
    }

    /// Enables a pane whose session should auto-continue (global default or
    /// per-session override) and restores a persisted arm across app restarts.
    fn restore_pane_state(&mut self, terminal_view_id: EntityId, ctx: &mut ModelContext<Self>) {
        let Some((key, agent, session_id)) = CLIAgentSessionsModel::as_ref(ctx)
            .session(terminal_view_id)
            .and_then(|session| {
                Some((
                    persistence_key(session)?,
                    session.agent,
                    session.session_context.session_id.clone()?,
                ))
            })
        else {
            return;
        };
        if matches!(
            Self::availability(terminal_view_id, false, ctx),
            AutoContinueAvailability::Unsupported
        ) || !Self::effective_enabled(Some(&key), ctx)
        {
            return;
        }

        let persisted_fire_at = CliAgentUsageSettings::as_ref(ctx)
            .auto_continue_armed_sessions
            .get(&key)
            .and_then(|timestamp| DateTime::from_timestamp(*timestamp, 0));
        let pane = self.panes.entry(terminal_view_id).or_default();
        pane.set_persistence_key(Some(key));
        let newly_enabled = pane.set_enabled(true);
        let restored_arm = persisted_fire_at.and_then(|fire_at| {
            pane.restore_arm(agent, session_id, fire_at, Utc::now())
                .map(|armed| (armed.fire_at, armed.generation))
        });
        if newly_enabled {
            ctx.emit(AutoContinueModelEvent::Changed { terminal_view_id });
        }
        if let Some((fire_at, generation)) = restored_arm {
            ctx.emit(AutoContinueModelEvent::Armed {
                terminal_view_id,
                fire_at,
                generation,
            });
            ctx.emit(AutoContinueModelEvent::Changed { terminal_view_id });
        } else if newly_enabled {
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

    pub fn armed_for_generation(
        &self,
        terminal_view_id: EntityId,
        generation: u64,
    ) -> Option<ArmedAutoContinue> {
        self.panes
            .get(&terminal_view_id)?
            .armed_for_generation(generation)
            .cloned()
    }

    /// Performs the final usage check and either claims the one-shot or emits
    /// a replacement timer. The view calls this only after validating session
    /// identity and foreground ownership, so failed validation never consumes
    /// the arm.
    pub fn prepare_due_fire(
        &mut self,
        terminal_view_id: EntityId,
        generation: u64,
        ctx: &mut ModelContext<Self>,
    ) -> Option<ArmedAutoContinue> {
        let agent = self
            .panes
            .get(&terminal_view_id)?
            .armed_for_generation(generation)?
            .agent;
        let persisted_key = self
            .panes
            .get(&terminal_view_id)
            .and_then(PaneAutoContinue::persistence_key)
            .map(str::to_owned);
        let exhaustion = exhaustion_for_agent(CliAgentUsageModel::as_ref(ctx).latest(), agent);
        match self.panes.get_mut(&terminal_view_id)?.prepare_due_fire(
            generation,
            exhaustion,
            Utc::now(),
        ) {
            DueFireDecision::Fire(fired) => {
                if let Some(key) = persisted_key.as_deref() {
                    Self::set_persisted_arm(key, None, ctx);
                }
                ctx.emit(AutoContinueModelEvent::Changed { terminal_view_id });
                Some(fired)
            }
            DueFireDecision::Rearmed(armed) => {
                if let Some(key) = persisted_key.as_deref() {
                    Self::set_persisted_arm(key, Some(armed.fire_at), ctx);
                }
                ctx.emit(AutoContinueModelEvent::Armed {
                    terminal_view_id,
                    fire_at: armed.fire_at,
                    generation: armed.generation,
                });
                ctx.emit(AutoContinueModelEvent::Changed { terminal_view_id });
                None
            }
            DueFireDecision::Ignore => None,
        }
    }

    pub fn record_delivery_failure(
        &mut self,
        terminal_view_id: EntityId,
        fired: ArmedAutoContinue,
        message: String,
        ctx: &mut ModelContext<Self>,
    ) {
        log::error!("CLI-agent auto-continue delivery failed: {message}");
        let persisted_key = self
            .panes
            .get(&terminal_view_id)
            .and_then(PaneAutoContinue::persistence_key)
            .map(str::to_owned);
        let rearmed = self
            .panes
            .get_mut(&terminal_view_id)
            .and_then(|pane| pane.rearm_after_delivery_failure(fired, Utc::now(), message))
            .map(|armed| (armed.fire_at, armed.generation));
        if let Some((fire_at, generation)) = rearmed {
            if let Some(key) = persisted_key.as_deref() {
                Self::set_persisted_arm(key, Some(fire_at), ctx);
            }
            ctx.emit(AutoContinueModelEvent::Armed {
                terminal_view_id,
                fire_at,
                generation,
            });
        } else if let Some(key) = persisted_key.as_deref() {
            Self::set_persisted_arm(key, None, ctx);
        }
        ctx.emit(AutoContinueModelEvent::Changed { terminal_view_id });
    }

    /// Cancels this pane's pending continue while leaving it enabled for the
    /// next limit stop. Runtime guards use this rather than `disable`.
    pub fn disarm(&mut self, terminal_view_id: EntityId, ctx: &mut ModelContext<Self>) {
        let persisted_key = self
            .panes
            .get(&terminal_view_id)
            .and_then(PaneAutoContinue::persistence_key)
            .map(str::to_owned);
        if self
            .panes
            .get_mut(&terminal_view_id)
            .is_some_and(|pane| pane.disarm())
        {
            if let Some(key) = persisted_key {
                Self::set_persisted_arm(&key, None, ctx);
            }
            ctx.emit(AutoContinueModelEvent::Changed { terminal_view_id });
        }
    }

    fn retry_pending_usage_confirmations(&mut self, ctx: &mut ModelContext<Self>) {
        let (claude_exhaustion, codex_exhaustion) = {
            let snapshot = CliAgentUsageModel::as_ref(ctx).latest();
            (
                exhaustion_for_agent(snapshot, CLIAgent::Claude),
                exhaustion_for_agent(snapshot, CLIAgent::Codex),
            )
        };
        let now = Utc::now();
        let enabled_panes: Vec<_> = self
            .panes
            .iter()
            .filter_map(|(view_id, pane)| {
                pane.is_enabled().then(|| {
                    CLIAgentSessionsModel::as_ref(ctx)
                        .session(*view_id)
                        .map(|session| (*view_id, session.agent))
                })?
            })
            .collect();
        for (view_id, agent) in enabled_panes {
            if !Self::availability(view_id, false, ctx).may_enable() {
                self.disarm(view_id, ctx);
                continue;
            }
            let exhaustion = match agent {
                CLIAgent::Claude => claude_exhaustion,
                CLIAgent::Codex => codex_exhaustion,
                _ => None,
            };
            let exhausted_until = exhaustion.and_then(|status| match status {
                ExhaustionStatus::ResetsAt(reset) => Some(reset),
                ExhaustionStatus::NotExhausted | ExhaustionStatus::ResetUnknown => None,
            });
            let Some(pane) = self.panes.get_mut(&view_id) else {
                continue;
            };
            let armed = if pane.armed().is_some() {
                pane.reconcile_reset(exhausted_until, now)
            } else {
                pane.retry_pending_usage_confirmation(
                    reset_for_limit_stop(CliAgentUsageModel::as_ref(ctx).latest(), agent, None),
                    now,
                )
            };
            let Some(armed) = armed else { continue };
            let (fire_at, generation) = (armed.fire_at, armed.generation);
            let persisted_key = pane.persistence_key().map(str::to_owned);
            if let Some(key) = persisted_key {
                Self::set_persisted_arm(&key, Some(fire_at), ctx);
            }
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
        let (agent, stopped_session_id, stop_reason) = {
            let sessions = CLIAgentSessionsModel::as_ref(ctx);
            let Some(session) = sessions.session(terminal_view_id) else {
                return;
            };
            if !Self::availability(terminal_view_id, false, ctx).may_enable()
                || !matches!(session.status, CLIAgentSessionStatus::Success)
            {
                return;
            }
            (
                session.agent,
                session.session_context.session_id.clone(),
                session.session_context.stop_reason,
            )
        };
        if matches!(stop_reason, Some(CLIAgentStopReason::UsageLimit { .. })) {
            self.arm_if_exhausted(
                terminal_view_id,
                agent,
                stopped_session_id.as_deref(),
                stated_reset(stop_reason),
                ctx,
            );
        }
    }

    /// Records the stopped session and arms immediately iff a reset is known
    /// (stated by the provider or shown by its usage data). Otherwise a causal
    /// pending confirmation lets a later usage refresh complete the arm.
    fn arm_if_exhausted(
        &mut self,
        terminal_view_id: EntityId,
        agent: CLIAgent,
        session_id: Option<&str>,
        stated_reset: Option<DateTime<Utc>>,
        ctx: &mut ModelContext<Self>,
    ) {
        if !Self::availability(terminal_view_id, false, ctx).may_enable() {
            self.disarm(terminal_view_id, ctx);
            return;
        }
        // The pane may not have been evaluated yet (its first rich event can be
        // the limit stop itself, e.g. right after a Clinch restart).
        if !self.panes.contains_key(&terminal_view_id) {
            self.restore_pane_state(terminal_view_id, ctx);
        }
        let exhausted_until = reset_for_limit_stop(
            CliAgentUsageModel::as_ref(ctx).latest(),
            agent,
            stated_reset,
        );
        let Some(pane) = self.panes.get_mut(&terminal_view_id) else {
            return;
        };
        let Some(armed) =
            pane.on_agent_session_stopped(agent, session_id, exhausted_until, Utc::now())
        else {
            return;
        };
        let (fire_at, generation) = (armed.fire_at, armed.generation);
        let persisted_key = pane.persistence_key().map(str::to_owned);
        if let Some(key) = persisted_key {
            Self::set_persisted_arm(&key, Some(fire_at), ctx);
        }
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
                    if matches!(*agent, CLIAgent::Claude | CLIAgent::Codex)
                        && matches!(
                            session_context.stop_reason,
                            Some(CLIAgentStopReason::UsageLimit { .. })
                        ) =>
                {
                    self.arm_if_exhausted(
                        *terminal_view_id,
                        *agent,
                        session_context.session_id.as_deref(),
                        stated_reset(session_context.stop_reason),
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
            // A view ID is ephemeral. Clear any previous pane state, then
            // apply the default/override and restore a persisted arm when this
            // is the same durable provider session after an app or pane restore.
            CLIAgentSessionsModelEvent::Started {
                terminal_view_id, ..
            } => {
                if self.panes.remove(terminal_view_id).is_some() {
                    ctx.emit(AutoContinueModelEvent::Changed {
                        terminal_view_id: *terminal_view_id,
                    });
                }
                self.restore_pane_state(*terminal_view_id, ctx);
            }
            CLIAgentSessionsModelEvent::Ended {
                terminal_view_id, ..
            } => {
                if self.panes.remove(terminal_view_id).is_some() {
                    ctx.emit(AutoContinueModelEvent::Changed {
                        terminal_view_id: *terminal_view_id,
                    });
                }
            }
            CLIAgentSessionsModelEvent::SessionUpdated {
                terminal_view_id, ..
            } => self.restore_pane_state(*terminal_view_id, ctx),
            CLIAgentSessionsModelEvent::InputSessionChanged { .. } => {}
        }
    }
}

#[cfg(test)]
#[path = "auto_continue_tests.rs"]
mod tests;
