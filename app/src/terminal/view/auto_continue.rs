//! TerminalView glue for rate-limit auto-continue: schedules the wall-clock
//! timer when this pane arms, re-validates at fire time, and types the
//! continue into the agent's PTY.
//!
//! The decision logic lives in `cli_agent_sessions::auto_continue` (pure,
//! clock-injected state machine). This file only owns the runtime pieces
//! that need a view: the timer (spawned on the view so a closed pane's timer
//! dies with it) and the PTY submit (reuses the same
//! `submit_text_to_cli_agent_pty` pipeline as the footer's manual "Continue"
//! button, so the per-agent Enter strategy applies). All cross-entity work
//! goes through model handles/events — never a synchronous action dispatch
//! that could re-borrow this view (see the footer "+" circular-borrow fix).

use chrono::{DateTime, Utc};
use warpui::r#async::Timer;
use warpui::{SingletonEntity, ViewContext};

use super::TerminalView;
use crate::ai::blocklist::usage::CliAgentUsageModel;
use crate::terminal::cli_agent_sessions::auto_continue::{
    AutoContinueAvailability, AutoContinueModel, AutoContinueModelEvent, AUTO_CONTINUE_PROMPT,
};
use crate::terminal::cli_agent_sessions::{CLIAgentSessionStatus, CLIAgentSessionsModel};

impl TerminalView {
    pub(super) fn register_subscriptions_for_auto_continue(&mut self, ctx: &mut ViewContext<Self>) {
        let view_id = self.view_id;
        ctx.subscribe_to_model(
            &AutoContinueModel::handle(ctx),
            move |me, _handle, event, ctx| match event {
                AutoContinueModelEvent::Armed {
                    terminal_view_id,
                    fire_at,
                    generation,
                } => {
                    if *terminal_view_id == view_id {
                        me.schedule_auto_continue_timer(*fire_at, *generation, ctx);
                    }
                }
                AutoContinueModelEvent::Changed { .. } => {}
            },
        );
        // Availability depends on live provider plan data. Notify the pane
        // even before it has an AutoContinueModel entry so the initially
        // hidden toggle appears as soon as the first usable snapshot arrives.
        ctx.subscribe_to_model(&CliAgentUsageModel::handle(ctx), |_, _, _, ctx| {
            ctx.notify();
        });
    }

    /// Toggle entry point shared by the footer button and the Command
    /// Palette enable/disable pair.
    pub(super) fn toggle_auto_continue_on_limit_reset(&mut self, ctx: &mut ViewContext<Self>) {
        let view_id = self.view_id;
        let is_shared_session_viewer = self.model.lock().shared_session_status().is_viewer();
        let is_available =
            AutoContinueModel::availability(view_id, is_shared_session_viewer, ctx).may_enable();
        AutoContinueModel::handle(ctx).update(ctx, |model, ctx| {
            if model.is_enabled(view_id) || is_available {
                model.toggle(view_id, ctx);
            } else {
                model.disable(view_id, ctx);
            }
        });
    }

    /// Called on every user-initiated PTY write (typed keys, pastes, footer
    /// sends): any user input into the pane cancels a pending auto-continue.
    /// Cheap read-first so the per-keystroke cost is a singleton lookup.
    pub(super) fn cancel_auto_continue_on_user_input(&mut self, ctx: &mut ViewContext<Self>) {
        let view_id = self.view_id;
        if !AutoContinueModel::as_ref(ctx).has_pending_continue(view_id) {
            return;
        }
        AutoContinueModel::handle(ctx)
            .update(ctx, |model, ctx| model.notice_user_activity(view_id, ctx));
    }

    fn schedule_auto_continue_timer(
        &mut self,
        fire_at: DateTime<Utc>,
        generation: u64,
        ctx: &mut ViewContext<Self>,
    ) {
        // Negative (already due) clamps to zero. The timer is spawned on the
        // view, so closing the pane drops it; a disarm/re-arm is handled by
        // the generation check at fire time rather than by cancelling here.
        let delay = (fire_at - Utc::now()).to_std().unwrap_or_default();
        ctx.spawn(Timer::after(delay), move |me, _, ctx| {
            me.fire_auto_continue_if_still_armed(generation, ctx);
        });
    }

    fn fire_auto_continue_if_still_armed(&mut self, generation: u64, ctx: &mut ViewContext<Self>) {
        let view_id = self.view_id;
        // Recheck availability at the last responsible moment. This protects
        // against a settings or shared-session transition racing the timer,
        // even though both normal transitions proactively disarm the pane.
        let is_shared_session_viewer = self.model.lock().shared_session_status().is_viewer();
        let availability = AutoContinueModel::availability(view_id, is_shared_session_viewer, ctx);
        if matches!(availability, AutoContinueAvailability::Unsupported) {
            AutoContinueModel::handle(ctx).update(ctx, |model, ctx| model.disable(view_id, ctx));
            return;
        }

        // Inspect without consuming. A stale generation or intervening disarm
        // stops here, while later validation failures leave the one-shot
        // intact instead of silently losing it.
        let Some(expected) =
            AutoContinueModel::as_ref(ctx).armed_for_generation(view_id, generation)
        else {
            return;
        };

        // The pane must still host the exact agent session that hit the limit,
        // and it must still be stopped — anything in progress means something
        // else already woke it.
        let session_matches = CLIAgentSessionsModel::as_ref(ctx)
            .session(view_id)
            .is_some_and(|session| {
                session.agent == expected.agent
                    && (matches!(session.status, CLIAgentSessionStatus::Success)
                        || (expected.restored && !session.has_observed_turn_activity))
                    && session.session_context.session_id.as_deref()
                        == expected.session_id.as_deref()
            });
        if !session_matches {
            AutoContinueModel::handle(ctx).update(ctx, |model, ctx| model.disable(view_id, ctx));
            return;
        }

        // The session entry can outlive its foreground block (the agent
        // process may have exited while the timer was pending); never type
        // into a plain shell prompt.
        let agent_still_foreground = {
            let model = self.model.lock();
            let block = model.block_list().active_block();
            block.is_active_and_long_running() && !block.is_agent_in_control()
        };
        if !agent_still_foreground {
            AutoContinueModel::handle(ctx).update(ctx, |model, ctx| {
                model.record_delivery_failure(
                    view_id,
                    expected,
                    "the original agent process is not the writable foreground command".to_owned(),
                    ctx,
                );
            });
            return;
        }

        let Some(fired) = AutoContinueModel::handle(ctx).update(ctx, |model, ctx| {
            model.prepare_due_fire(view_id, generation, ctx)
        }) else {
            return;
        };

        // Same pipeline as the footer's manual "Continue" button (per-agent
        // Enter strategy). If initiation fails, retain a visible error and
        // retry a bounded number of times instead of losing the one-shot.
        #[cfg(feature = "local_tty")]
        let submitted =
            self.submit_external_text_to_cli_agent_pty(AUTO_CONTINUE_PROMPT.to_string(), ctx);
        #[cfg(not(feature = "local_tty"))]
        let submitted = false;
        if !submitted {
            AutoContinueModel::handle(ctx).update(ctx, |model, ctx| {
                model.record_delivery_failure(
                    view_id,
                    fired,
                    "the agent PTY was not writable".to_owned(),
                    ctx,
                );
            });
        }
    }
}
