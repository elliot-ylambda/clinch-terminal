use chrono::{DateTime, Utc};
use cli_agent_usage::format::{chip_halves, fmt_pct, fmt_reset};
use cli_agent_usage::{LimitWindow, Provider, Severity, UsageSnapshot};
use warp_core::ui::theme::Fill;
use warp_core::ui::Icon;
use warpui::elements::{
    ConstrainedBox, Container, CrossAxisAlignment, Empty, Flex, ParentElement,
    SizeConstraintCondition, SizeConstraintSwitch,
};
use warpui::Element;

use super::cli_agent_usage_chip::{render_cli_agent_usage_chip, severity_fill, span};
use crate::appearance::Appearance;

/// Below this available width (px) the widget collapses to the compact chip.
const NARROW_MAX: f32 = 340.;
/// Below this available width (px) the widget drops the reset countdowns.
const MEDIUM_MAX: f32 = 560.;

/// Text pieces for one limit window: the percent (colored by the returned
/// severity), an optional dimmed reset countdown, and the severity. A `None`
/// window renders as `"—"` with `Normal` severity and no reset.
fn limit_texts(
    window: Option<LimitWindow>,
    now: DateTime<Utc>,
    include_reset: bool,
) -> (String, Option<String>, Severity) {
    match window {
        None => ("—".to_string(), None, Severity::Normal),
        Some(w) => {
            let pct = fmt_pct(w.percent);
            let reset = include_reset.then(|| fmt_reset(w.resets_at, now));
            (pct, reset, w.severity)
        }
    }
}

/// One provider's inline segment: `{name} 5h {pct}[· {reset}]  wk {pct}[· {reset}]`.
/// Percents are severity-colored; labels and resets are dimmed. The two windows are
/// separated by whitespace — the only `│` divider in the widget sits between providers.
fn provider_segment(
    name: &str,
    provider: &Provider,
    now: DateTime<Utc>,
    include_resets: bool,
    appearance: &Appearance,
    bg: Fill,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let main = theme.main_text_color(bg);
    let sub = theme.sub_text_color(bg);
    let session = provider.plan.and_then(|p| p.session);
    let weekly = provider.plan.and_then(|p| p.weekly);

    let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
    row.add_child(span(format!("{name} "), main, appearance));

    for (idx, (label, window)) in [("5h ", session), ("wk ", weekly)].into_iter().enumerate() {
        if idx > 0 {
            row.add_child(
                Container::new(Empty::new().finish())
                    .with_margin_right(10.)
                    .finish(),
            );
        }
        let (pct, reset, severity) = limit_texts(window, now, include_resets);
        row.add_child(span(label, sub, appearance));
        row.add_child(span(pct, severity_fill(severity, theme, bg), appearance));
        if let Some(reset) = reset {
            row.add_child(span(format!(" · {reset}"), sub, appearance));
        }
    }
    row.finish()
}

/// The full/medium inline layout: clock icon + Claude segment + `│` divider + Codex segment.
fn inline_row(
    snapshot: &UsageSnapshot,
    now: DateTime<Utc>,
    include_resets: bool,
    appearance: &Appearance,
    bg: Fill,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let sub = theme.sub_text_color(bg);
    let icon_size = appearance.monospace_font_size();
    let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
    row.add_child(
        Container::new(
            ConstrainedBox::new(
                Icon::Clock
                    .to_warpui_icon(theme.main_text_color(bg))
                    .finish(),
            )
            .with_width(icon_size)
            .with_height(icon_size)
            .finish(),
        )
        .with_margin_right(4.)
        .finish(),
    );
    row.add_child(provider_segment(
        "Claude",
        &snapshot.claude,
        now,
        include_resets,
        appearance,
        bg,
    ));
    row.add_child(
        Container::new(span("│", sub, appearance))
            .with_horizontal_margin(12.)
            .finish(),
    );
    row.add_child(provider_segment(
        "Codex",
        &snapshot.codex,
        now,
        include_resets,
        appearance,
        bg,
    ));
    row.finish()
}

/// The always-visible tab-bar usage widget. Three width variants inside a
/// `SizeConstraintSwitch`; `None` when neither provider has data (widget hidden).
pub fn render_cli_agent_usage_header(
    snapshot: &UsageSnapshot,
    appearance: &Appearance,
    bg: Fill,
) -> Option<Box<dyn Element>> {
    // Hidden when neither tool has data — same rule as the footer chip.
    chip_halves(snapshot)?;
    let now = Utc::now();

    let full = inline_row(snapshot, now, true, appearance, bg);
    let medium = inline_row(snapshot, now, false, appearance, bg);
    let narrow = render_cli_agent_usage_chip(snapshot, appearance, bg)
        .unwrap_or_else(|| Empty::new().finish());

    // Conditions are checked in order; the narrower condition must come first so
    // it wins when both are satisfied. `full` is the default (widest) child.
    Some(
        SizeConstraintSwitch::new(
            full,
            vec![
                (SizeConstraintCondition::WidthLessThan(NARROW_MAX), narrow),
                (SizeConstraintCondition::WidthLessThan(MEDIUM_MAX), medium),
            ],
        )
        .finish(),
    )
}

#[cfg(test)]
#[path = "cli_agent_usage_header_tests.rs"]
mod tests;
