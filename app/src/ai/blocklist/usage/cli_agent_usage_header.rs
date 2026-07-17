use chrono::{DateTime, Utc};
use cli_agent_usage::format::{chip_halves, fmt_pct, fmt_reset, fmt_reset_short, fmt_tokens};
use cli_agent_usage::{LimitWindow, Provider, Severity, UsageSnapshot, WindowTotals};
use warp_core::ui::theme::Fill;
use warp_core::ui::Icon;
use warpui::elements::{
    ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Empty, Flex, Hoverable,
    MouseStateHandle, ParentElement, Radius, SizeConstraintCondition, SizeConstraintSwitch,
};
use warpui::platform::Cursor;
use warpui::Element;

use super::cli_agent_usage_chip::{plan_limits_affordance_link, severity_fill, span};
use super::{
    CliAgentUsageHeaderVisibility, CliAgentUsageMetric, CliAgentUsageProvider,
    PlanLimitsAffordance,
};
use crate::appearance::Appearance;
use crate::workspace::WorkspaceAction;

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

fn provider_windows(
    provider: &Provider,
    kind: CliAgentUsageProvider,
    visibility: &CliAgentUsageHeaderVisibility,
) -> Vec<(&'static str, Option<LimitWindow>)> {
    [
        (
            CliAgentUsageMetric::FiveHour,
            "5h ",
            provider.plan.and_then(|plan| plan.session),
        ),
        (
            CliAgentUsageMetric::Weekly,
            "wk ",
            provider.plan.and_then(|plan| plan.weekly),
        ),
        (
            CliAgentUsageMetric::Fable,
            "Fable ",
            provider.plan.and_then(|plan| plan.fable_weekly),
        ),
    ]
    .into_iter()
    .filter(|(metric, _, _)| visibility.is_visible(kind, *metric))
    .map(|(_, label, window)| (label, window))
    .collect()
}

/// The narrow layout keeps one provider-wide plan window, preferring weekly
/// as the previous compact chip did, plus Claude's optional Fable window. If
/// weekly is hidden, an enabled 5-hour window takes its place.
fn compact_provider_windows(
    provider: &Provider,
    kind: CliAgentUsageProvider,
    visibility: &CliAgentUsageHeaderVisibility,
) -> Vec<(CliAgentUsageMetric, Option<LimitWindow>)> {
    let mut windows = Vec::new();
    if visibility.is_visible(kind, CliAgentUsageMetric::Weekly) {
        windows.push((
            CliAgentUsageMetric::Weekly,
            provider.plan.and_then(|plan| plan.weekly),
        ));
    } else if visibility.is_visible(kind, CliAgentUsageMetric::FiveHour) {
        windows.push((
            CliAgentUsageMetric::FiveHour,
            provider.plan.and_then(|plan| plan.session),
        ));
    }
    if visibility.is_visible(kind, CliAgentUsageMetric::Fable) {
        windows.push((
            CliAgentUsageMetric::Fable,
            provider.plan.and_then(|plan| plan.fable_weekly),
        ));
    }
    windows
}

fn visible_exhausted_until(
    provider: &Provider,
    kind: CliAgentUsageProvider,
    visibility: &CliAgentUsageHeaderVisibility,
) -> Option<DateTime<Utc>> {
    let mut latest = None;
    for (metric, window) in [
        (
            CliAgentUsageMetric::FiveHour,
            provider.plan.and_then(|plan| plan.session),
        ),
        (
            CliAgentUsageMetric::Weekly,
            provider.plan.and_then(|plan| plan.weekly),
        ),
    ] {
        if !visibility.is_visible(kind, metric) {
            continue;
        }
        if let Some(window) = window.filter(|window| window.percent >= 100.) {
            let resets_at = window.resets_at?;
            latest =
                Some(latest.map_or(resets_at, |current: DateTime<Utc>| current.max(resets_at)));
        }
    }
    latest
}

fn token_windows<'a>(
    provider: &'a Provider,
    kind: CliAgentUsageProvider,
    visibility: &CliAgentUsageHeaderVisibility,
) -> Vec<(CliAgentUsageMetric, &'a WindowTotals)> {
    [
        (CliAgentUsageMetric::SessionTokens, &provider.session),
        (CliAgentUsageMetric::TodayTokens, &provider.today),
        (CliAgentUsageMetric::WeekTokens, &provider.week),
        (CliAgentUsageMetric::MonthTokens, &provider.month),
    ]
    .into_iter()
    .filter(|(metric, _)| visibility.is_visible(kind, *metric))
    .collect()
}

fn token_label(metric: CliAgentUsageMetric, compact: bool) -> &'static str {
    match (metric, compact) {
        (CliAgentUsageMetric::SessionTokens, false) => "sess tok ",
        (CliAgentUsageMetric::TodayTokens, false) => "today tok ",
        (CliAgentUsageMetric::WeekTokens, false) => "week tok ",
        (CliAgentUsageMetric::MonthTokens, false) => "month tok ",
        (CliAgentUsageMetric::SessionTokens, true) => "s ",
        (CliAgentUsageMetric::TodayTokens, true) => "d ",
        (CliAgentUsageMetric::WeekTokens, true) => "w ",
        (CliAgentUsageMetric::MonthTokens, true) => "m ",
        _ => "",
    }
}

fn token_text(totals: &WindowTotals) -> String {
    if totals.tokens.total() == 0 {
        "—".to_string()
    } else {
        fmt_tokens(totals.tokens.io())
    }
}

/// Whether Claude's opt-in plan gauges are on, plus the shared mouse handle
/// for the "Turn on"/"Authorize" affordance rendered in their place while the
/// gauges are off or the Keychain read awaits a sanctioning click.
struct PlanLimitsGate<'a> {
    enabled: bool,
    turn_on_mouse_state: &'a MouseStateHandle,
}

impl PlanLimitsGate<'_> {
    /// The affordance (and its mouse handle) when `kind`'s gauge area should
    /// show a clickable link instead of limit windows.
    fn affordance(
        &self,
        kind: CliAgentUsageProvider,
        provider: &Provider,
    ) -> Option<(PlanLimitsAffordance, MouseStateHandle)> {
        kind.plan_limits_affordance(self.enabled, provider)
            .map(|affordance| (affordance, self.turn_on_mouse_state.clone()))
    }
}

struct HeaderRenderContext<'a> {
    now: DateTime<Utc>,
    visibility: &'a CliAgentUsageHeaderVisibility,
    gate: PlanLimitsGate<'a>,
    appearance: &'a Appearance,
    bg: Fill,
}

/// One provider's inline segment: `{name} 5h {pct}[· {reset}]  wk {pct}[· {reset}]`.
/// Claude also includes its model-scoped `Fable {pct}[· {reset}]` window. Percents
/// are severity-colored; labels and resets are dimmed. The windows are separated
/// by whitespace — the only `│` divider in the widget sits between providers.
/// While the gate keeps `kind`'s gauges off, the windows give way to the
/// "Turn on" affordance.
fn provider_segment(
    kind: CliAgentUsageProvider,
    provider: &Provider,
    include_resets: bool,
    render: &HeaderRenderContext<'_>,
) -> Box<dyn Element> {
    let appearance = render.appearance;
    let bg = render.bg;
    let theme = appearance.theme();
    let main = theme.main_text_color(bg);
    let sub = theme.sub_text_color(bg);
    let name = match kind {
        CliAgentUsageProvider::Claude => "Claude",
        CliAgentUsageProvider::Codex => "Codex",
    };

    let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
    row.add_child(span(format!("{name} "), main, appearance));

    let windows = provider_windows(provider, kind, render.visibility);
    let mut item_count = 0;
    if !windows.is_empty() {
        if let Some((affordance, mouse_state)) = render.gate.affordance(kind, provider) {
            row.add_child(span("limits ", sub, appearance));
            row.add_child(plan_limits_affordance_link(
                affordance, appearance, bg, mouse_state,
            ));
            item_count += 1;
        } else {
            for (label, window) in windows {
                if item_count > 0 {
                    row.add_child(
                        Container::new(Empty::new().finish())
                            .with_margin_right(10.)
                            .finish(),
                    );
                }
                let show_reset = include_resets
                    && render
                        .visibility
                        .is_visible(kind, CliAgentUsageMetric::ResetTimes);
                let (pct, reset, severity) = limit_texts(window, render.now, show_reset);
                row.add_child(span(label, sub, appearance));
                row.add_child(span(pct, severity_fill(severity, theme, bg), appearance));
                if let Some(reset) = reset {
                    row.add_child(span(format!(" · {reset}"), sub, appearance));
                }
                item_count += 1;
            }
        }
    }

    for (metric, totals) in token_windows(provider, kind, render.visibility) {
        if item_count > 0 {
            row.add_child(
                Container::new(Empty::new().finish())
                    .with_margin_right(10.)
                    .finish(),
            );
        }
        row.add_child(span(token_label(metric, false), sub, appearance));
        row.add_child(span(token_text(totals), main, appearance));
        item_count += 1;
    }

    row.finish()
}

/// Give each provider its own click target so opening one usage dropdown never
/// exposes the other provider's details.
fn clickable_provider(
    provider: CliAgentUsageProvider,
    content: Box<dyn Element>,
    mouse_state: MouseStateHandle,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    Hoverable::new(mouse_state, move |state| {
        let mut container = Container::new(content)
            .with_horizontal_padding(4.)
            .with_vertical_padding(2.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
        if state.is_hovered() {
            container = container.with_background(theme.surface_overlay_1());
        }
        container.finish()
    })
    .on_click(move |ctx, _app, _position| {
        ctx.dispatch_typed_action(WorkspaceAction::ToggleCliAgentUsagePanel(provider));
    })
    // The nested "Turn on" affordance owns its click; without deferring, the
    // same click would also toggle this provider's panel.
    .with_defer_events_to_children()
    .with_cursor(Cursor::PointingHand)
    .finish()
}

/// The full/medium inline layout: clock icon + Claude/Fable segment + `│`
/// divider + Codex segment.
fn inline_row(
    snapshot: &UsageSnapshot,
    include_resets: bool,
    render: &HeaderRenderContext<'_>,
    mouse_states: &[MouseStateHandle; 2],
) -> Box<dyn Element> {
    let appearance = render.appearance;
    let bg = render.bg;
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
    row.add_child(clickable_provider(
        CliAgentUsageProvider::Claude,
        provider_segment(
            CliAgentUsageProvider::Claude,
            &snapshot.claude,
            include_resets,
            render,
        ),
        mouse_states[0].clone(),
        appearance,
    ));
    row.add_child(
        Container::new(span("│", sub, appearance))
            .with_horizontal_margin(12.)
            .finish(),
    );
    row.add_child(clickable_provider(
        CliAgentUsageProvider::Codex,
        provider_segment(
            CliAgentUsageProvider::Codex,
            &snapshot.codex,
            include_resets,
            render,
        ),
        mouse_states[1].clone(),
        appearance,
    ));
    row.finish()
}

fn compact_provider_segment(
    kind: CliAgentUsageProvider,
    provider: &Provider,
    render: &HeaderRenderContext<'_>,
) -> Box<dyn Element> {
    let appearance = render.appearance;
    let bg = render.bg;
    let theme = appearance.theme();
    let neutral = theme.sub_text_color(bg);
    let main = theme.main_text_color(bg);
    let provider_label = match kind {
        CliAgentUsageProvider::Claude => "cc ",
        CliAgentUsageProvider::Codex => "cx ",
    };
    let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
    row.add_child(span(provider_label, neutral, appearance));

    let windows = compact_provider_windows(provider, kind, render.visibility);
    let mut item_count = 0;
    if !windows.is_empty() {
        if let Some((affordance, mouse_state)) = render.gate.affordance(kind, provider) {
            row.add_child(span("limits ", neutral, appearance));
            row.add_child(plan_limits_affordance_link(
                affordance, appearance, bg, mouse_state,
            ));
            item_count += 1;
        } else {
            for (metric, window) in windows {
                if item_count > 0 {
                    row.add_child(span(" · ", neutral, appearance));
                }
                let (pct, _, severity) = limit_texts(window, render.now, false);
                let (label, pct) = match metric {
                    CliAgentUsageMetric::FiveHour => ("5h ", pct),
                    CliAgentUsageMetric::Weekly => ("", format!("{pct}w")),
                    CliAgentUsageMetric::Fable => ("fb ", format!("{pct}w")),
                    _ => ("", pct),
                };
                row.add_child(span(label, neutral, appearance));
                row.add_child(span(pct, severity_fill(severity, theme, bg), appearance));

                if metric != CliAgentUsageMetric::Fable
                    && render
                        .visibility
                        .is_visible(kind, CliAgentUsageMetric::ResetTimes)
                {
                    if let Some(until) = visible_exhausted_until(provider, kind, render.visibility)
                    {
                        row.add_child(span(
                            format!(" resets {}", fmt_reset_short(until, render.now)),
                            neutral,
                            appearance,
                        ));
                    }
                }
                item_count += 1;
            }
        }
    }

    for (metric, totals) in token_windows(provider, kind, render.visibility) {
        if item_count > 0 {
            row.add_child(span(" · ", neutral, appearance));
        }
        row.add_child(span(token_label(metric, true), neutral, appearance));
        row.add_child(span(token_text(totals), main, appearance));
        item_count += 1;
    }

    row.finish()
}

fn compact_row(
    snapshot: &UsageSnapshot,
    render: &HeaderRenderContext<'_>,
    mouse_states: &[MouseStateHandle; 2],
) -> Box<dyn Element> {
    let appearance = render.appearance;
    let bg = render.bg;
    let theme = appearance.theme();
    let neutral = theme.sub_text_color(bg);
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
        .with_margin_right(2.)
        .finish(),
    );

    for (index, kind) in [CliAgentUsageProvider::Claude, CliAgentUsageProvider::Codex]
        .into_iter()
        .enumerate()
    {
        if index > 0 {
            row.add_child(span(" · ", neutral, appearance));
        }
        row.add_child(clickable_provider(
            kind,
            compact_provider_segment(kind, kind.data(snapshot), render),
            mouse_states[index].clone(),
            appearance,
        ));
    }

    row.finish()
}

/// The always-visible tab-bar usage widget. Three width variants inside a
/// `SizeConstraintSwitch`; `None` when neither provider has data (widget hidden).
pub fn render_cli_agent_usage_header(
    snapshot: &UsageSnapshot,
    plan_limits_enabled: bool,
    visibility: &CliAgentUsageHeaderVisibility,
    appearance: &Appearance,
    bg: Fill,
    mouse_states: &[MouseStateHandle; 2],
    turn_on_mouse_state: &MouseStateHandle,
) -> Option<Box<dyn Element>> {
    // Hidden when neither tool has data — same rule as the footer chip.
    chip_halves(snapshot)?;
    let render = HeaderRenderContext {
        now: Utc::now(),
        visibility,
        gate: PlanLimitsGate {
            enabled: plan_limits_enabled,
            turn_on_mouse_state,
        },
        appearance,
        bg,
    };

    let full = inline_row(snapshot, true, &render, mouse_states);
    let medium = inline_row(snapshot, false, &render, mouse_states);
    let narrow = compact_row(snapshot, &render, mouse_states);

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
