use chrono::{DateTime, Utc};
use cli_agent_usage::format::{chip_halves, fmt_pct, fmt_reset, fmt_reset_short, ChipHalf};
use cli_agent_usage::{LimitWindow, Provider, Severity, UsageSnapshot};
use warp_core::ui::theme::Fill;
use warp_core::ui::Icon;
use warpui::elements::{
    ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Empty, Flex, Hoverable,
    MouseStateHandle, ParentElement, Radius, SizeConstraintCondition, SizeConstraintSwitch,
};
use warpui::platform::Cursor;
use warpui::Element;

use super::cli_agent_usage_chip::{severity_fill, span, turn_on_plan_limits};
use super::CliAgentUsageProvider;
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
    include_fable: bool,
) -> Vec<(&'static str, Option<LimitWindow>)> {
    let mut windows = vec![
        ("5h ", provider.plan.and_then(|plan| plan.session)),
        ("wk ", provider.plan.and_then(|plan| plan.weekly)),
    ];
    if include_fable {
        windows.push(("Fable ", provider.plan.and_then(|plan| plan.fable_weekly)));
    }
    windows
}

/// One provider's inline segment: `{name} 5h {pct}[· {reset}]  wk {pct}[· {reset}]`.
/// Claude also includes its model-scoped `Fable {pct}[· {reset}]` window. Percents
/// are severity-colored; labels and resets are dimmed. The windows are separated
/// by whitespace — the only `│` divider in the widget sits between providers.
/// A `turn_on` mouse state replaces the windows with the gauges' enable
/// affordance (Claude while `show_plan_limits` is off).
fn provider_segment(
    name: &str,
    provider: &Provider,
    include_fable: bool,
    now: DateTime<Utc>,
    include_resets: bool,
    turn_on: Option<MouseStateHandle>,
    appearance: &Appearance,
    bg: Fill,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let main = theme.main_text_color(bg);
    let sub = theme.sub_text_color(bg);

    let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
    row.add_child(span(format!("{name} "), main, appearance));

    if let Some(mouse_state) = turn_on {
        row.add_child(span("limits ", sub, appearance));
        row.add_child(turn_on_plan_limits(appearance, bg, mouse_state));
        return row.finish();
    }

    for (idx, (label, window)) in provider_windows(provider, include_fable)
        .into_iter()
        .enumerate()
    {
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
    now: DateTime<Utc>,
    include_resets: bool,
    plan_limits_enabled: bool,
    appearance: &Appearance,
    bg: Fill,
    mouse_states: &[MouseStateHandle; 2],
    turn_on_mouse_state: &MouseStateHandle,
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
    row.add_child(clickable_provider(
        CliAgentUsageProvider::Claude,
        provider_segment(
            "Claude",
            &snapshot.claude,
            true,
            now,
            include_resets,
            CliAgentUsageProvider::Claude
                .shows_plan_limits_turn_on(plan_limits_enabled)
                .then(|| turn_on_mouse_state.clone()),
            appearance,
            bg,
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
            "Codex",
            &snapshot.codex,
            false,
            now,
            include_resets,
            None,
            appearance,
            bg,
        ),
        mouse_states[1].clone(),
        appearance,
    ));
    row.finish()
}

fn compact_provider_segment(
    kind: CliAgentUsageProvider,
    half: &ChipHalf,
    provider: &Provider,
    now: DateTime<Utc>,
    turn_on: Option<MouseStateHandle>,
    appearance: &Appearance,
    bg: Fill,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let neutral = theme.sub_text_color(bg);
    let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
    row.add_child(span(format!("{} ", half.label), neutral, appearance));

    if let Some(mouse_state) = turn_on {
        row.add_child(turn_on_plan_limits(appearance, bg, mouse_state));
        return row.finish();
    }

    row.add_child(span(
        half.pct.clone(),
        severity_fill(half.severity, theme, bg),
        appearance,
    ));

    if let Some(until) = provider.plan.and_then(|plan| plan.exhausted_until()) {
        row.add_child(span(
            format!(" resets {}", fmt_reset_short(until, now)),
            neutral,
            appearance,
        ));
    }

    if kind == CliAgentUsageProvider::Claude {
        if let Some(fable) = provider.plan.and_then(|plan| plan.fable_weekly) {
            row.add_child(span(" · fb ", neutral, appearance));
            row.add_child(span(
                format!("{}w", fmt_pct(fable.percent)),
                severity_fill(fable.severity, theme, bg),
                appearance,
            ));
        }
    }

    row.finish()
}

fn compact_row(
    snapshot: &UsageSnapshot,
    halves: &[ChipHalf; 2],
    now: DateTime<Utc>,
    plan_limits_enabled: bool,
    appearance: &Appearance,
    bg: Fill,
    mouse_states: &[MouseStateHandle; 2],
    turn_on_mouse_state: &MouseStateHandle,
) -> Box<dyn Element> {
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

    for (index, (kind, half)) in [CliAgentUsageProvider::Claude, CliAgentUsageProvider::Codex]
        .into_iter()
        .zip(halves)
        .enumerate()
    {
        if index > 0 {
            row.add_child(span(" · ", neutral, appearance));
        }
        row.add_child(clickable_provider(
            kind,
            compact_provider_segment(
                kind,
                half,
                kind.data(snapshot),
                now,
                kind.shows_plan_limits_turn_on(plan_limits_enabled)
                    .then(|| turn_on_mouse_state.clone()),
                appearance,
                bg,
            ),
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
    appearance: &Appearance,
    bg: Fill,
    mouse_states: &[MouseStateHandle; 2],
    turn_on_mouse_state: &MouseStateHandle,
) -> Option<Box<dyn Element>> {
    // Hidden when neither tool has data — same rule as the footer chip.
    let halves = chip_halves(snapshot)?;
    let now = Utc::now();

    let full = inline_row(
        snapshot,
        now,
        true,
        plan_limits_enabled,
        appearance,
        bg,
        mouse_states,
        turn_on_mouse_state,
    );
    let medium = inline_row(
        snapshot,
        now,
        false,
        plan_limits_enabled,
        appearance,
        bg,
        mouse_states,
        turn_on_mouse_state,
    );
    let narrow = compact_row(
        snapshot,
        &halves,
        now,
        plan_limits_enabled,
        appearance,
        bg,
        mouse_states,
        turn_on_mouse_state,
    );

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
