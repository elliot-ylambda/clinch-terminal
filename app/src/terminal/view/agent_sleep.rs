//! Sleep for a Claude Code or Codex pane: its agent process is stopped, and the pane keeps its tab,
//! title and conversation until someone wakes it.
//!
//! An asleep pane is in the same state as a pane restored at launch whose agent has not started
//! yet: the session is seeded, and the resume command waits behind `deferred_agent_start`. Waking
//! releases that command, so both go through one path. This file holds the pane's side of it: the
//! rules for when an agent may be stopped, the state while it stops and wakes, and the card drawn
//! over an asleep pane. `PaneGroup::sleep_agent` stops the process and re-arms the resume command.
use std::time::Duration;

use chrono::{DateTime, Local, Utc};
use instant::Instant;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::color::coloru_with_opacity;
use warp_core::ui::theme::Fill;
use warp_core::ui::tokens::{corners_lg, corners_md};
use warp_core::ui::Icon;
use warpui::elements::{
    Border, ConstrainedBox, Container, CrossAxisAlignment, Flex, Hoverable, MainAxisAlignment,
    MainAxisSize, ParentElement as _, Rect, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::platform::Cursor;
use warpui::r#async::Timer;
use warpui::{AppContext, Element, SingletonEntity, ViewContext};

use super::use_agent_footer::ObservedAgentState;
use super::{Event, TerminalAction, TerminalView};
use crate::terminal::cli_agent_sessions::restore::AgentRestoreQueue;
use crate::terminal::cli_agent_sessions::{CLIAgentSessionStatus, CLIAgentSessionsModel};
use crate::terminal::CLIAgent;

/// How long a stopped agent gets to exit before the sleep is called off and it is left running.
const STOP_TIMEOUT: Duration = Duration::from_secs(20);
/// An agent this new may still be loading, so it isn't put to sleep unless its own tab asks.
const RECENT_START: Duration = Duration::from_secs(2 * 60);
/// How long a queued coordinator message keeps its target awake after delivery last checked it.
const QUEUED_MESSAGE_HOLD: Duration = Duration::from_secs(5);
/// How long a wake may show as in progress when nothing else reports how it went.
const WAKE_GRACE: Duration = Duration::from_secs(90);

const CARD_WIDTH: f32 = 300.;
const CARD_PADDING: f32 = 14.;
const CARD_TITLE_SIZE: f32 = 13.;
const CARD_BODY_SIZE: f32 = 12.;
const CARD_ICON_SIZE: f32 = 14.;
/// How much of the scrim color dims the pane behind the card, as a percentage.
const PANE_DIM_OPACITY: u8 = 45;

/// Where a pane's agent is on its way into or out of sleep.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AgentSleepPhase {
    /// Clinch asked the agent to stop and is waiting for it to exit.
    Stopping { resume_command: String },
    /// No agent process. Waking runs the saved resume command.
    Asleep,
    /// The resume command is running and the agent isn't ready yet.
    Waking,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentSleep {
    pub(crate) since: DateTime<Utc>,
    pub(crate) phase: AgentSleepPhase,
}

/// Why a pane's agent can't be put to sleep right now. When in doubt, the answer is a reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SleepBlocker {
    NoAgent,
    AlreadyAsleep,
    Restoring,
    NoSavedSession,
    Working,
    WaitingOnYou,
    RateLimited,
    BackgroundWork,
    Draft,
    QueuedMessage,
    InUseElsewhere,
    StatusUnknown,
    StartedRecently,
    Visible,
    Coordinator,
}

impl SleepBlocker {
    /// The stable name the control CLI reports.
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::NoAgent => "no_agent",
            Self::AlreadyAsleep => "already_asleep",
            Self::Restoring => "restoring",
            Self::NoSavedSession => "no_saved_session",
            Self::Working => "working",
            Self::WaitingOnYou => "waiting_on_user",
            Self::RateLimited => "rate_limited",
            Self::BackgroundWork => "background_work",
            Self::Draft => "draft",
            Self::QueuedMessage => "queued_message",
            Self::InUseElsewhere => "in_use_elsewhere",
            Self::StatusUnknown => "status_unknown",
            Self::StartedRecently => "started_recently",
            Self::Visible => "visible",
            Self::Coordinator => "coordinator",
        }
    }

    /// The reason in plain words, to finish "This tab can't sleep: it …".
    pub(crate) fn explanation(self) -> &'static str {
        match self {
            Self::NoAgent => "isn't running Claude Code or Codex",
            Self::AlreadyAsleep => "is already asleep",
            Self::Restoring => "is still starting up",
            Self::NoSavedSession => "has no saved conversation to come back to",
            Self::Working => "is working",
            Self::WaitingOnYou => "is waiting for your answer",
            Self::RateLimited => "is waiting for its usage limit to reset",
            Self::BackgroundWork => "is running something in the background",
            Self::Draft => "has a message you haven't sent",
            Self::QueuedMessage => "has a message waiting to be delivered",
            Self::InUseElsewhere => "is being used from another device",
            Self::StatusUnknown => "can't be read right now",
            Self::StartedRecently => "only just started",
            Self::Visible => "is the tab you're looking at",
            Self::Coordinator => "is the project's Coordinator",
        }
    }

    /// Rules that protect against a surprise, which the tab's own Sleep command may override
    /// because there the user asked for exactly this tab.
    pub(crate) fn yields_to_direct_request(self) -> bool {
        matches!(self, Self::StartedRecently | Self::Visible)
    }
}

/// When a pane fell asleep, for a card or tooltip: the time today, or the date for an older one.
pub(crate) fn asleep_since_label(since: DateTime<Utc>, now: DateTime<Local>) -> String {
    let since = since.with_timezone(&Local);
    if since.date_naive() == now.date_naive() {
        since.format("%-I:%M %p").to_string()
    } else {
        since.format("%b %-d, %-I:%M %p").to_string()
    }
}

/// What the card over the pane says for each phase: its title and the line under it.
pub(crate) fn sleep_card_copy(sleep: &AgentSleep, now: DateTime<Local>) -> (String, String) {
    match sleep.phase {
        AgentSleepPhase::Stopping { .. } => (
            "Going to sleep…".to_owned(),
            "Stopping the agent. The conversation is saved.".to_owned(),
        ),
        AgentSleepPhase::Asleep => (
            "This session is asleep".to_owned(),
            format!(
                "Asleep since {}. Nothing is lost: waking it picks the conversation up where it \
                 stopped.",
                asleep_since_label(sleep.since, now)
            ),
        ),
        AgentSleepPhase::Waking => (
            "Waking up…".to_owned(),
            "Reopening the conversation. This takes a few seconds.".to_owned(),
        ),
    }
}

impl TerminalView {
    #[cfg(test)]
    pub(crate) fn agent_sleep(&self) -> Option<&AgentSleep> {
        self.agent_sleep.as_ref()
    }

    /// Since when this pane's agent has been asleep, if it is.
    pub(crate) fn agent_asleep_since(&self) -> Option<DateTime<Utc>> {
        self.agent_sleep
            .as_ref()
            .filter(|sleep| sleep.phase == AgentSleepPhase::Asleep)
            .map(|sleep| sleep.since)
    }

    pub(crate) fn agent_is_asleep(&self) -> bool {
        self.agent_asleep_since().is_some()
    }

    /// Whether Clinch has asked this pane's agent to stop and is waiting for it to exit.
    pub(crate) fn agent_is_stopping_for_sleep(&self) -> bool {
        matches!(
            self.agent_sleep.as_ref().map(|sleep| &sleep.phase),
            Some(AgentSleepPhase::Stopping { .. })
        )
    }

    /// The agent's resume command is armed and nothing is running: show the pane as asleep.
    pub(crate) fn mark_agent_asleep(&mut self, since: DateTime<Utc>, ctx: &mut ViewContext<Self>) {
        self.agent_sleep = Some(AgentSleep {
            since,
            phase: AgentSleepPhase::Asleep,
        });
        // Its tab, a message or the Wake button starts it now, rather than the restore queue.
        self.background_agent_restore = false;
        self.agent_sleep_generation = self.agent_sleep_generation.wrapping_add(1);
        ctx.notify();
    }

    /// Something happened in this pane: the idle clock that automatic sleep reads starts over.
    pub(crate) fn touch_agent_activity(&mut self) {
        self.agent_last_active_at = Instant::now();
    }

    pub(crate) fn agent_idle_for(&self) -> Duration {
        self.agent_last_active_at.elapsed()
    }

    #[cfg(test)]
    pub(crate) fn set_agent_idle_for_test(&mut self, idle: Duration) {
        self.agent_last_active_at = Instant::now().checked_sub(idle).unwrap();
    }

    #[cfg(test)]
    pub(crate) fn forget_recent_agent_start_for_test(&mut self) {
        self.agent_session_started_at = None;
    }

    /// Delivery found a queued coordinator message for this pane's agent and is holding it.
    pub(crate) fn note_queued_agent_message(&mut self) {
        self.agent_message_queued_at = Some(Instant::now());
    }

    /// Why this pane's agent can't be put to sleep, or `None` if it can. Everything here is about
    /// the pane itself. Its tab adds what only the tab knows: whether it is the Coordinator, and
    /// whether the user is looking at it.
    pub(crate) fn agent_sleep_blocker(&self, ctx: &AppContext) -> Option<SleepBlocker> {
        match self.agent_sleep.as_ref().map(|sleep| &sleep.phase) {
            Some(AgentSleepPhase::Waking) => return Some(SleepBlocker::Restoring),
            Some(_) => return Some(SleepBlocker::AlreadyAsleep),
            None => {}
        }
        if self.deferred_agent_start.is_some()
            || AgentRestoreQueue::as_ref(ctx).is_bringing_back(self.view_id)
        {
            return Some(SleepBlocker::Restoring);
        }
        let Some(session) = CLIAgentSessionsModel::as_ref(ctx).session(self.view_id) else {
            return Some(SleepBlocker::NoAgent);
        };
        // Only the agents whose way of stopping has been checked. Devin comes back asleep after
        // a restart like the others, but a running one is left alone.
        if !matches!(session.agent, CLIAgent::Claude | CLIAgent::Codex) || session.is_remote() {
            return Some(SleepBlocker::NoAgent);
        }
        if session.session_key().is_none() {
            return Some(SleepBlocker::NoSavedSession);
        }
        // These read the terminal model, so they come before the lock below.
        let draft_source = self.local_control_draft_source(ctx);
        let observed =
            (!session.supports_rich_status()).then(|| self.observed_agent_state(session));
        let shows_working = match observed {
            Some(observed) => observed == ObservedAgentState::Working,
            None => self.cli_agent_shows_working(session),
        };
        {
            let model = self.model.lock();
            let block = model.block_list().active_block();
            if !block.is_active_and_long_running()
                || block.is_agent_in_control()
                || model.shared_session_status().is_sharer_or_viewer()
            {
                return Some(SleepBlocker::NoAgent);
            }
        }
        if matches!(session.status, CLIAgentSessionStatus::Blocked { .. }) {
            return Some(SleepBlocker::WaitingOnYou);
        }
        if shows_working || session.turn_blocks_submission() {
            return Some(SleepBlocker::Working);
        }
        if session.session_context.stop_reason.is_some() {
            return Some(SleepBlocker::RateLimited);
        }
        if !session.background_work().is_empty() {
            return Some(SleepBlocker::BackgroundWork);
        }
        if draft_source.is_some()
            || (self.has_active_cli_agent_input_session(ctx)
                && !self.input.as_ref(ctx).buffer_text(ctx).is_empty())
        {
            return Some(SleepBlocker::Draft);
        }
        if self.coordination_send_pending
            || self
                .agent_message_queued_at
                .is_some_and(|at| at.elapsed() < QUEUED_MESSAGE_HOLD)
        {
            return Some(SleepBlocker::QueuedMessage);
        }
        if crate::remote_control::has_writer_for_terminal(self.view_id, ctx) {
            return Some(SleepBlocker::InUseElsewhere);
        }
        if observed == Some(ObservedAgentState::Unknown) {
            return Some(SleepBlocker::StatusUnknown);
        }
        if self.deferred_agent_is_starting()
            || self
                .agent_session_started_at
                .is_some_and(|at| at.elapsed() < RECENT_START)
        {
            return Some(SleepBlocker::StartedRecently);
        }
        None
    }

    /// Clinch is about to ask this pane's agent to stop. `resume_command` is what brings the same
    /// conversation back. If the agent hasn't exited after [`STOP_TIMEOUT`] the sleep is called
    /// off and the pane says so.
    pub(crate) fn begin_agent_sleep(
        &mut self,
        resume_command: String,
        ctx: &mut ViewContext<Self>,
    ) {
        self.agent_sleep = Some(AgentSleep {
            since: Utc::now(),
            phase: AgentSleepPhase::Stopping { resume_command },
        });
        self.agent_sleep_generation = self.agent_sleep_generation.wrapping_add(1);
        let generation = self.agent_sleep_generation;
        ctx.spawn(Timer::after(STOP_TIMEOUT), move |me, _, ctx| {
            me.give_up_stopping_agent(generation, ctx);
        });
        ctx.notify();
    }

    /// Calls off a sleep that could not get as far as stopping the agent, or whose pane can't be
    /// woken afterwards. The pane goes back to not being asleep, without a failure notice.
    pub(crate) fn abandon_agent_sleep(&mut self, ctx: &mut ViewContext<Self>) {
        self.agent_sleep = None;
        self.agent_sleep_generation = self.agent_sleep_generation.wrapping_add(1);
        ctx.notify();
    }

    fn give_up_stopping_agent(&mut self, generation: u64, ctx: &mut ViewContext<Self>) {
        if self.agent_sleep_generation != generation || !self.agent_is_stopping_for_sleep() {
            return;
        }
        self.agent_sleep = None;
        self.touch_agent_activity();
        ctx.emit(Event::AgentSleepFailed);
        ctx.notify();
    }

    /// Called when the pane's foreground command ends. Returns whether that was the agent exiting
    /// because Clinch stopped it for sleep, in which case its session stays with the pane.
    pub(super) fn agent_exited_for_sleep(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let Some(AgentSleep {
            since,
            phase: AgentSleepPhase::Stopping { resume_command },
        }) = self.agent_sleep.clone()
        else {
            return false;
        };
        let view_id = self.view_id;
        let kept = CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
            sessions.mark_session_asleep(view_id, ctx)
        });
        if !kept {
            // The session went away while its agent was stopping: nothing is left to wake.
            self.agent_sleep = None;
            ctx.emit(Event::AgentSleepFailed);
            ctx.notify();
            return false;
        }
        ctx.emit(Event::AgentStoppedForSleep {
            resume_command,
            since,
        });
        ctx.notify();
        true
    }

    /// Starts this asleep pane's agent again, on the same conversation. Returns whether it was
    /// asleep with a resume command to run.
    pub(crate) fn wake_agent(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let Some(since) = self.agent_asleep_since() else {
            return false;
        };
        if !self.release_deferred_agent(ctx) {
            return false;
        }
        self.agent_sleep = Some(AgentSleep {
            since,
            phase: AgentSleepPhase::Waking,
        });
        self.agent_sleep_generation = self.agent_sleep_generation.wrapping_add(1);
        // Whatever was typed before this, including the keystroke that woke the pane, is not the
        // user taking the pane over from the resume command.
        self.agent_restore_input_baseline = self.coordination_user_input_epoch;
        self.agent_session_started_at = Some(Instant::now());
        self.touch_agent_activity();
        let view_id = self.view_id;
        AgentRestoreQueue::handle(ctx).update(ctx, |queue, ctx| queue.woke(view_id, ctx));
        let generation = self.agent_sleep_generation;
        ctx.spawn(Timer::after(WAKE_GRACE), move |me, _, ctx| {
            if me.agent_sleep_generation == generation {
                me.agent_wake_settled(ctx);
            }
        });
        ctx.notify();
        true
    }

    /// The woken agent is up, or its start needs the user: either way the pane stops saying it is
    /// waking.
    pub(crate) fn agent_wake_settled(&mut self, ctx: &mut ViewContext<Self>) {
        if matches!(
            self.agent_sleep.as_ref().map(|sleep| &sleep.phase),
            Some(AgentSleepPhase::Waking)
        ) {
            self.agent_sleep = None;
            ctx.notify();
        }
    }

    /// The user typed in this pane. Typing in an asleep pane wakes it instead of reaching the
    /// shell under it, and keys typed while it comes up belong to that same request. Returns
    /// whether the typed text should be dropped rather than left in the shell's input.
    pub(super) fn agent_sleep_saw_typing(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        match self.agent_sleep.as_ref().map(|sleep| &sleep.phase) {
            Some(AgentSleepPhase::Asleep) => self.wake_agent(ctx),
            Some(AgentSleepPhase::Waking) => {
                // Not the user taking the pane over from the resume command.
                self.agent_restore_input_baseline = self.coordination_user_input_epoch;
                true
            }
            _ => false,
        }
    }

    /// A translucent layer that dims an asleep pane, so it reads as off at a glance.
    pub(super) fn render_agent_sleep_dim(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        self.agent_sleep.as_ref()?;
        let scrim = Appearance::as_ref(app).tokens().surface_scrim;
        Some(
            Rect::new()
                .with_background(Fill::Solid(coloru_with_opacity(scrim, PANE_DIM_OPACITY)))
                .finish(),
        )
    }

    /// The card over an asleep pane: that it is asleep, since when, and how to wake it.
    pub(super) fn render_agent_sleep_card(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        let sleep = self.agent_sleep.as_ref()?;
        let appearance = Appearance::as_ref(app);
        let tokens = *appearance.tokens();
        let font_family = appearance.ui_font_family();
        let (title, body) = sleep_card_copy(sleep, Local::now());

        let header = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.)
            .with_child(
                ConstrainedBox::new(
                    Icon::Moon
                        .to_warpui_icon(Fill::Solid(tokens.text_2))
                        .finish(),
                )
                .with_width(CARD_ICON_SIZE)
                .with_height(CARD_ICON_SIZE)
                .finish(),
            )
            .with_child(
                Text::new_inline(title, font_family, CARD_TITLE_SIZE)
                    .with_style(Properties::default().weight(Weight::Medium))
                    .with_color(tokens.text_1)
                    .finish(),
            )
            .finish();

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(8.)
            .with_child(header)
            .with_child(
                Text::new(body, font_family, CARD_BODY_SIZE)
                    .soft_wrap(true)
                    .with_color(tokens.text_2)
                    .finish(),
            );

        if sleep.phase == AgentSleepPhase::Asleep {
            let wake = Hoverable::new(self.agent_sleep_wake_mouse_state.clone(), move |_| {
                Container::new(
                    Text::new_inline("Wake", font_family, CARD_BODY_SIZE)
                        .with_style(Properties::default().weight(Weight::Medium))
                        .with_color(tokens.text_on_accent)
                        .finish(),
                )
                .with_horizontal_padding(12.)
                .with_vertical_padding(6.)
                .with_background(Fill::Solid(tokens.accent))
                .with_corner_radius(corners_md())
                .finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(TerminalAction::WakeAgent))
            .finish();
            column.add_child(
                Container::new(
                    Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_child(
                            Text::new_inline("or just start typing", font_family, CARD_BODY_SIZE)
                                .with_color(tokens.text_3)
                                .finish(),
                        )
                        .with_child(wake)
                        .finish(),
                )
                .with_margin_top(4.)
                .finish(),
            );
        }

        Some(
            ConstrainedBox::new(
                Container::new(column.finish())
                    .with_uniform_padding(CARD_PADDING)
                    .with_background(Fill::Solid(tokens.surface_overlay))
                    .with_corner_radius(corners_lg())
                    .with_border(Border::all(1.).with_border_color(tokens.hairline))
                    .with_drop_shadow(tokens.overlay_shadow())
                    .finish(),
            )
            .with_width(CARD_WIDTH)
            .finish(),
        )
    }
}

#[cfg(test)]
#[path = "agent_sleep_tests.rs"]
mod tests;
