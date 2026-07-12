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
use crate::terminal::cli_agent_sessions::auto_continue::{
    AutoContinueModel, AutoContinueModelEvent,
};
use crate::terminal::cli_agent_sessions::{CLIAgentSessionStatus, CLIAgentSessionsModel};
use crate::terminal::CLIAgent;

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
    }

    /// Toggle entry point shared by the footer button and the Command
    /// Palette enable/disable pair.
    pub(super) fn toggle_auto_continue_on_limit_reset(&mut self, ctx: &mut ViewContext<Self>) {
        let view_id = self.view_id;
        AutoContinueModel::handle(ctx).update(ctx, |model, ctx| model.toggle(view_id, ctx));
    }

    /// Called on every user-initiated PTY write (typed keys, pastes, footer
    /// sends): any user input into the pane cancels a pending auto-continue.
    /// Cheap read-first so the per-keystroke cost is a singleton lookup.
    pub(super) fn cancel_auto_continue_on_user_input(&mut self, ctx: &mut ViewContext<Self>) {
        let view_id = self.view_id;
        if !AutoContinueModel::as_ref(ctx).is_armed(view_id) {
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
        // Consuming the armed state (exactly-once) BEFORE any send; a stale
        // generation or an intervening disarm yields `None`.
        let Some(armed) = AutoContinueModel::handle(ctx).update(ctx, |model, ctx| {
            model.take_due_fire(view_id, generation, ctx)
        }) else {
            return;
        };

        // The pane must still host the exact Claude session that hit the
        // limit, and it must still be stopped — anything in progress means
        // something else already woke it.
        let session_matches = CLIAgentSessionsModel::as_ref(ctx)
            .session(view_id)
            .is_some_and(|session| {
                session.agent == CLIAgent::Claude
                    && matches!(session.status, CLIAgentSessionStatus::Success)
                    && session.session_context.session_id.as_deref()
                        == Some(armed.session_id.as_str())
            });
        if !session_matches {
            return;
        }

        // The session entry can outlive its foreground block (the agent
        // process may have exited while the timer was pending); never type
        // into a plain shell prompt.
        let agent_still_foreground = self
            .model
            .lock()
            .block_list()
            .active_block()
            .is_active_and_long_running();
        if !agent_still_foreground {
            return;
        }

        // Same pipeline as the footer's manual "Continue" button (per-agent
        // Enter strategy). No local PTY means nothing to type into.
        #[cfg(feature = "local_tty")]
        self.submit_text_to_cli_agent_pty(
            crate::terminal::cli_agent_sessions::auto_continue::AUTO_CONTINUE_PROMPT.to_string(),
            ctx,
        );
    }
}
