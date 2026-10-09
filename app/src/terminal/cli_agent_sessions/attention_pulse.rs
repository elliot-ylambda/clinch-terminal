//! App-wide blink clock for "this agent needs your input" indicators.
//!
//! Every surface that flags a blocked Claude Code / Codex session (a question, a tool approval,
//! a plan approval) reads one shared phase so all of them blink in unison. The clock only runs
//! while at least one session is blocked, so an idle app never schedules a repaint for it.
use std::time::Duration;

use pathfinder_color::ColorU;
use warpui::r#async::Timer;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use super::CLIAgentSessionsModel;

/// Half of a blink cycle: the indicator is lit for this long, then dimmed for this long.
const ATTENTION_PULSE_HALF_PERIOD: Duration = Duration::from_millis(600);

/// Opacity of a blinking indicator during the dimmed half of the cycle. Dimmed rather than
/// hidden, so a quick glance never lands on an empty slot.
const ATTENTION_PULSE_DIMMED_OPACITY: f32 = 0.25;

/// Opacity for a "needs input" indicator at the current blink phase.
pub(crate) fn attention_pulse_opacity(app: &AppContext) -> f32 {
    if AgentAttentionPulse::as_ref(app).is_lit() {
        1.
    } else {
        ATTENTION_PULSE_DIMMED_OPACITY
    }
}

/// `color` faded to the current blink phase.
pub(crate) fn attention_pulse_color(color: ColorU, app: &AppContext) -> ColorU {
    ColorU {
        a: (f32::from(color.a) * attention_pulse_opacity(app)).round() as u8,
        ..color
    }
}

pub struct AgentAttentionPulse {
    lit: bool,
    ticking: bool,
}

/// Emitted whenever the blink phase flips; subscribers repaint.
impl Entity for AgentAttentionPulse {
    type Event = ();
}

impl SingletonEntity for AgentAttentionPulse {}

impl AgentAttentionPulse {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&CLIAgentSessionsModel::handle(ctx), |me, _, _, ctx| {
            me.start_if_needed(ctx);
        });
        Self {
            lit: true,
            ticking: false,
        }
    }

    /// Whether blinking indicators are in the visible half of the cycle. Always `true` while
    /// nothing is blocked, so a newly blocked session starts out visible.
    pub fn is_lit(&self) -> bool {
        self.lit
    }

    fn start_if_needed(&mut self, ctx: &mut ModelContext<Self>) {
        if self.ticking || !CLIAgentSessionsModel::as_ref(ctx).has_blocked_session() {
            return;
        }
        self.ticking = true;
        self.schedule_tick(ctx);
    }

    fn schedule_tick(&mut self, ctx: &mut ModelContext<Self>) {
        ctx.spawn(
            async { Timer::after(ATTENTION_PULSE_HALF_PERIOD).await },
            |me, _, ctx| me.tick(ctx),
        );
    }

    fn tick(&mut self, ctx: &mut ModelContext<Self>) {
        if !CLIAgentSessionsModel::as_ref(ctx).has_blocked_session() {
            self.ticking = false;
            if !self.lit {
                self.lit = true;
                ctx.emit(());
            }
            return;
        }
        self.lit = !self.lit;
        ctx.emit(());
        self.schedule_tick(ctx);
    }
}

#[cfg(test)]
#[path = "attention_pulse_tests.rs"]
mod tests;
