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
//! - a provider session must be explicitly opted in; that exact durable
//!   identity and any causal arm are persisted machine-locally so an app
//!   restart cannot silently lose the timer;
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
//!   session end or replacement cancels the pending continue.
//!
//! With `show_plan_limits` off, the usage poller never fetches Claude plan
//! data, so Claude cannot arm (the footer toggle and Command Palette entries
//! are hidden behind the same setting). Codex limits come from local rollout
//! files and remain available independently of that Claude-only setting.

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
    WaitingForUsageData,
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
pub(crate) fn auto_continue_availability(
    session: &CLIAgentSession,
    snapshot: &UsageSnapshot,
    show_plan_limits: bool,
    is_shared_session_viewer: bool,
) -> AutoContinueAvailability {
    if is_shared_session_viewer || session.is_remote() || !supports_causal_limit_events(session) {
        return AutoContinueAvailability::Unsupported;
    }

    match session.agent {
        CLIAgent::Claude if !show_plan_limits => AutoContinueAvailability::Unsupported,
        CLIAgent::Claude if snapshot.claude.plan.is_none() => {
            AutoContinueAvailability::WaitingForUsageData
        }
        CLIAgent::Codex if snapshot.codex.plan.is_none() => {
            AutoContinueAvailability::WaitingForUsageData
        }
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
    /// live session and foreground process. Unknown usage data defers instead
    /// of consuming the one-shot; a reset that moved later re-arms precisely.
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
            Some(ExhaustionStatus::ResetUnknown) | None => {
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

/// Singleton tracking each pane's auto-continue opt-in and armed state.
/// Pane entries are ephemeral, while exact provider-session opt-ins and armed
/// fire times are stored in private machine-local settings for restoration.
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
        auto_continue_availability(
            session,
            CliAgentUsageModel::as_ref(ctx).latest(),
            *CliAgentUsageSettings::as_ref(ctx).show_plan_limits,
            is_shared_session_viewer,
        )
    }

    /// Flips the pane's opt-in. Turning it ON while the pane's supported session
    /// is already stopped at an exhausted window arms immediately — the
    /// common flow is the user flipping the toggle only after seeing the
    /// limit message. Turning it OFF cancels any pending continue (one-click
    /// disarm).
    pub fn toggle(&mut self, terminal_view_id: EntityId, ctx: &mut ModelContext<Self>) {
        if self.is_enabled(terminal_view_id) {
            self.disable(terminal_view_id, ctx);
            return;
        }

        if !Self::availability(terminal_view_id, false, ctx).may_enable() {
            return;
        }
        let key = CLIAgentSessionsModel::as_ref(ctx)
            .session(terminal_view_id)
            .and_then(persistence_key);
        let pane = self.panes.entry(terminal_view_id).or_default();
        if !pane.set_enabled(true) {
            return;
        }
        pane.set_persistence_key(key.clone());
        if let Some(key) = key {
            Self::set_persisted_opt_in(&key, true, ctx);
        }
        ctx.emit(AutoContinueModelEvent::Changed { terminal_view_id });
        self.try_arm_for_already_stopped_session(terminal_view_id, ctx);
    }

    /// Turns this pane off if it was opted in, invalidating any scheduled
    /// timer generation. Used by defensive runtime guards as well as UI
    /// actions that become unavailable while pending.
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
            Self::set_persisted_opt_in(&key, false, ctx);
            Self::set_persisted_arm(&key, None, ctx);
        }
        if changed {
            ctx.emit(AutoContinueModelEvent::Changed { terminal_view_id });
        }
    }

    fn set_persisted_opt_in(key: &str, enabled: bool, ctx: &mut ModelContext<Self>) {
        CliAgentUsageSettings::handle(ctx).update(ctx, |settings, ctx| {
            let mut sessions = settings.auto_continue_sessions.clone();
            let changed = if enabled {
                sessions
                    .insert(key.to_owned(), Utc::now().timestamp())
                    .is_none()
            } else {
                sessions.remove(key).is_some()
            };
            if changed {
                if let Err(error) = settings.auto_continue_sessions.set_value(sessions, ctx) {
                    log::error!("failed to persist CLI-agent auto-continue opt-in: {error}");
                }
            }
        });
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

    fn restore_persisted_opt_in(
        &mut self,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
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
        ) || !CliAgentUsageSettings::as_ref(ctx)
            .auto_continue_sessions
            .contains_key(&key)
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

    fn disarm(&mut self, terminal_view_id: EntityId, ctx: &mut ModelContext<Self>) {
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
            let availability = Self::availability(view_id, false, ctx);
            if matches!(availability, AutoContinueAvailability::Unsupported) {
                self.disable(view_id, ctx);
                continue;
            }
            if !availability.may_enable() {
                ctx.emit(AutoContinueModelEvent::Changed {
                    terminal_view_id: view_id,
                });
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
                    reset_for_causal_limit_stop(CliAgentUsageModel::as_ref(ctx).latest(), agent),
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
        if matches!(stop_reason, Some(CLIAgentStopReason::UsageLimit)) {
            self.arm_if_exhausted(terminal_view_id, agent, stopped_session_id.as_deref(), ctx);
        }
    }

    /// Records the stopped session and arms immediately iff that provider's usage
    /// window is already known to be exhausted. Otherwise a causal pending
    /// confirmation lets a later, throttled usage refresh complete the arm.
    fn arm_if_exhausted(
        &mut self,
        terminal_view_id: EntityId,
        agent: CLIAgent,
        session_id: Option<&str>,
        ctx: &mut ModelContext<Self>,
    ) {
        if matches!(
            Self::availability(terminal_view_id, false, ctx),
            AutoContinueAvailability::Unsupported
        ) {
            self.disable(terminal_view_id, ctx);
            return;
        }
        let exhausted_until =
            reset_for_causal_limit_stop(CliAgentUsageModel::as_ref(ctx).latest(), agent);
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
                            Some(CLIAgentStopReason::UsageLimit)
                        ) =>
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
            // A view ID is ephemeral. Clear any previous pane state, then
            // restore the explicit opt-in when this is the same durable
            // provider session after an app or pane restore.
            CLIAgentSessionsModelEvent::Started {
                terminal_view_id, ..
            } => {
                if self.panes.remove(terminal_view_id).is_some() {
                    ctx.emit(AutoContinueModelEvent::Changed {
                        terminal_view_id: *terminal_view_id,
                    });
                }
                self.restore_persisted_opt_in(*terminal_view_id, ctx);
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
            } => self.restore_persisted_opt_in(*terminal_view_id, ctx),
            CLIAgentSessionsModelEvent::InputSessionChanged { .. } => {}
        }
    }
}

#[cfg(test)]
#[path = "auto_continue_tests.rs"]
mod tests;
