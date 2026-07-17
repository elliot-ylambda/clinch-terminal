use chrono::{DateTime, Utc};
use cli_agent_usage::format::{fmt_pct, fmt_reset, fmt_tokens};
use cli_agent_usage::{LimitWindow, Provider, Severity, UsageSnapshot, WindowTotals};
// Element + theme imports — mirror app/src/context_chips/display_chip.rs.
use warp_core::ui::theme::{Fill, WarpTheme};
use warp_core::ui::Icon;
use warpui::elements::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Flex, Hoverable,
    MouseStateHandle, ParentElement, Radius, Text,
};
use warpui::platform::Cursor;
use warpui::ui_components::components::UiComponent;
use warpui::Element;

use super::{
    CliAgentUsageHeaderVisibility, CliAgentUsageMetric, CliAgentUsageProvider, PlanLimitsAffordance,
};
use crate::appearance::Appearance;
use crate::workspace::WorkspaceAction;

// Column widths are the baseline (default 13px monospace) sizes; the panel
// scales them up with the user's monospace font so larger fonts don't clip the
// value text. The panel width is derived from the columns so they always match.
const PANEL_VALUE_WIDTH: f32 = 232.;
const PANEL_LABEL_WIDTH: f32 = 80.;
const PANEL_ROW_LABEL_WIDTH: f32 = 84.;
const PANEL_CHECKBOX_WIDTH: f32 = 20.;
const PANEL_H_PADDING: f32 = 16.;
/// The monospace size the baseline column widths were tuned for.
const PANEL_BASELINE_FONT_SIZE: f32 = 13.;

/// Grows the panel's fixed columns with the user's monospace font size (never
/// below the baseline) so wider text keeps fitting instead of clipping.
fn panel_scale(appearance: &Appearance) -> f32 {
    (appearance.monospace_font_size() / PANEL_BASELINE_FONT_SIZE).max(1.0)
}

/// Map a crate `Severity` to a fill against `bg` (the surface the text sits on).
pub(super) fn severity_fill(severity: Severity, theme: &WarpTheme, bg: Fill) -> Fill {
    match severity {
        Severity::Normal => theme.main_text_color(bg),
        Severity::Warning => Fill::Solid(theme.ui_warning_color()),
        Severity::Critical => Fill::Solid(theme.ui_error_color()),
    }
}

/// A monospace text span in a given color.
pub(super) fn span(
    text: impl Into<String>,
    color: Fill,
    appearance: &Appearance,
) -> Box<dyn Element> {
    Text::new_inline(
        text.into(),
        appearance.monospace_font_family(),
        appearance.monospace_font_size(),
    )
    .with_color(color.into())
    .with_line_height_ratio(appearance.line_height_ratio())
    .finish()
}

/// The plan gauges' clickable affordance ("Turn on" / "Authorize"): accent
/// text that ensures the `show_plan_limits` setting is on and sanctions one
/// Keychain read. If macOS needs to raise its credential prompt, it does so
/// right after this click — never unprompted at launch.
pub(super) fn plan_limits_affordance_link(
    affordance: PlanLimitsAffordance,
    appearance: &Appearance,
    bg: Fill,
    mouse_state: MouseStateHandle,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    Hoverable::new(mouse_state, move |state| {
        let color = if state.is_hovered() {
            theme.main_text_color(bg)
        } else {
            theme.accent()
        };
        span(affordance.label(), color, appearance)
    })
    .on_click(move |ctx, _app, _position| {
        ctx.dispatch_typed_action(WorkspaceAction::EnableCliAgentPlanLimits);
    })
    .with_cursor(Cursor::PointingHand)
    .finish()
}

/// A focused provider panel: plan limits, local token totals, and a link to the
/// provider's authoritative web usage page.
pub struct CliAgentUsagePanelMouseStates<'a> {
    pub metric_checkboxes: &'a [MouseStateHandle; CliAgentUsageMetric::COUNT],
    pub usage_link_mouse_state: MouseStateHandle,
    pub turn_on_mouse_state: MouseStateHandle,
}

pub fn render_cli_agent_usage_panel(
    snapshot: &UsageSnapshot,
    appearance: &Appearance,
    kind: CliAgentUsageProvider,
    plan_limits_enabled: bool,
    visibility: &CliAgentUsageHeaderVisibility,
    mouse_states: CliAgentUsagePanelMouseStates<'_>,
) -> Box<dyn Element> {
    let CliAgentUsagePanelMouseStates {
        metric_checkboxes,
        usage_link_mouse_state,
        turn_on_mouse_state,
    } = mouse_states;
    let theme = appearance.theme();
    let bg = theme.surface_2();
    let main = theme.main_text_color(bg);
    let sub = theme.sub_text_color(bg);
    let now = Utc::now();
    let provider = kind.data(snapshot);
    let scale = panel_scale(appearance);

    // Header row.
    let mut col = Flex::column().with_spacing(4.);
    col.add_child(span(
        format!("{} usage", kind.display_name()),
        main,
        appearance,
    ));

    // Claude's live plan data is opt-in, but its visibility controls remain
    // available even while collection is off.
    if let Some(affordance) = kind.plan_limits_affordance(plan_limits_enabled, provider) {
        col.add_child(panel_row(
            span("Limits", sub, appearance),
            plan_limits_affordance_link(affordance, appearance, bg, turn_on_mouse_state),
            scale,
        ));
    }
    col.add_child(configurable_panel_row(
        kind,
        CliAgentUsageMetric::FiveHour,
        visibility,
        metric_checkboxes,
        plan_cell(provider.plan.and_then(|p| p.session), now, appearance, bg),
        appearance,
        bg,
    ));
    col.add_child(configurable_panel_row(
        kind,
        CliAgentUsageMetric::Weekly,
        visibility,
        metric_checkboxes,
        plan_cell(provider.plan.and_then(|p| p.weekly), now, appearance, bg),
        appearance,
        bg,
    ));
    if kind == CliAgentUsageProvider::Claude {
        col.add_child(configurable_panel_row(
            kind,
            CliAgentUsageMetric::Fable,
            visibility,
            metric_checkboxes,
            plan_cell(
                provider.plan.and_then(|p| p.fable_weekly),
                now,
                appearance,
                bg,
            ),
            appearance,
            bg,
        ));
    }
    col.add_child(configurable_panel_row(
        kind,
        CliAgentUsageMetric::ResetTimes,
        visibility,
        metric_checkboxes,
        span("Show countdowns", sub, appearance),
        appearance,
        bg,
    ));

    // Token rows.
    for (metric, pick) in [
        (CliAgentUsageMetric::SessionTokens, 0u8),
        (CliAgentUsageMetric::TodayTokens, 1),
        (CliAgentUsageMetric::WeekTokens, 2),
        (CliAgentUsageMetric::MonthTokens, 3),
    ] {
        col.add_child(configurable_panel_row(
            kind,
            metric,
            visibility,
            metric_checkboxes,
            token_cell(window(provider, pick), appearance, main, sub),
            appearance,
            bg,
        ));
    }

    col.add_child(
        Container::new(usage_link(kind, appearance, bg, usage_link_mouse_state))
            .with_margin_top(6.)
            .finish(),
    );

    // Width tracks the widest (checkbox + label + value) row plus padding so the
    // columns always fit the panel exactly, at any monospace font size.
    let panel_width =
        (2. * PANEL_H_PADDING + PANEL_CHECKBOX_WIDTH + PANEL_LABEL_WIDTH + PANEL_VALUE_WIDTH)
            * scale;
    ConstrainedBox::new(
        Container::new(col.finish())
            .with_vertical_padding(12.)
            .with_horizontal_padding(PANEL_H_PADDING * scale)
            .with_background(bg)
            .with_border(Border::all(1.).with_border_fill(theme.accent()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .finish(),
    )
    .with_width(panel_width)
    .finish()
}

fn window(p: &Provider, pick: u8) -> &WindowTotals {
    match pick {
        0 => &p.session,
        1 => &p.today,
        2 => &p.week,
        _ => &p.month,
    }
}

/// A two-cell row: fixed-width label followed by the selected provider's value.
fn panel_row(label: Box<dyn Element>, value: Box<dyn Element>, scale: f32) -> Box<dyn Element> {
    let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
    row.add_child(
        ConstrainedBox::new(label)
            .with_width(PANEL_ROW_LABEL_WIDTH * scale)
            .finish(),
    );
    row.add_child(
        ConstrainedBox::new(value)
            .with_width(PANEL_VALUE_WIDTH * scale)
            .finish(),
    );
    row.finish()
}

/// A provider statistic with a controlled checkbox that toggles whether the
/// statistic appears in the tab-bar header. The current value remains visible
/// in the panel regardless of the checkbox state.
fn configurable_panel_row(
    kind: CliAgentUsageProvider,
    metric: CliAgentUsageMetric,
    visibility: &CliAgentUsageHeaderVisibility,
    metric_mouse_states: &[MouseStateHandle; CliAgentUsageMetric::COUNT],
    value: Box<dyn Element>,
    appearance: &Appearance,
    bg: Fill,
) -> Box<dyn Element> {
    let sub = appearance.theme().sub_text_color(bg);
    let scale = panel_scale(appearance);
    let checkbox = appearance
        .ui_builder()
        .checkbox(
            metric_mouse_states[metric.index()].clone(),
            Some(11. * scale),
        )
        .check(visibility.is_visible(kind, metric))
        .build()
        .on_click(move |ctx, _app, _position| {
            ctx.dispatch_typed_action(WorkspaceAction::ToggleCliAgentUsageHeaderMetric {
                provider: kind,
                metric,
            });
        })
        .with_cursor(Cursor::PointingHand)
        .finish();

    let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
    row.add_child(
        ConstrainedBox::new(checkbox)
            .with_width(PANEL_CHECKBOX_WIDTH * scale)
            .finish(),
    );
    row.add_child(
        ConstrainedBox::new(span(metric.label(), sub, appearance))
            .with_width(PANEL_LABEL_WIDTH * scale)
            .finish(),
    );
    row.add_child(
        ConstrainedBox::new(value)
            .with_width(PANEL_VALUE_WIDTH * scale)
            .finish(),
    );
    row.finish()
}

fn usage_link(
    kind: CliAgentUsageProvider,
    appearance: &Appearance,
    bg: Fill,
    mouse_state: MouseStateHandle,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let label = kind.usage_link_label();
    let url = kind.usage_url();
    let icon_size = appearance.monospace_font_size();

    Hoverable::new(mouse_state, move |state| {
        let color = if state.is_hovered() {
            theme.main_text_color(bg)
        } else {
            theme.accent()
        };
        let content = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(4.)
            .with_child(span(label, color, appearance))
            .with_child(
                ConstrainedBox::new(Icon::LinkExternal.to_warpui_icon(color).finish())
                    .with_width(icon_size)
                    .with_height(icon_size)
                    .finish(),
            )
            .finish();
        Container::new(content)
            .with_vertical_padding(2.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .finish()
    })
    .on_click(move |_ctx, app, _position| app.open_url(url))
    .with_cursor(Cursor::PointingHand)
    .finish()
}

/// `{pct}% · resets {when}` colored by severity, or `—` when absent.
fn plan_cell(
    limit: Option<LimitWindow>,
    now: DateTime<Utc>,
    appearance: &Appearance,
    bg: Fill,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let sub = theme.sub_text_color(bg);
    match limit {
        None => span("—", sub, appearance),
        Some(w) => {
            let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
            row.add_child(span(
                fmt_pct(w.percent),
                severity_fill(w.severity, theme, bg),
                appearance,
            ));
            row.add_child(span(
                format!(" · {}", fmt_reset(w.resets_at, now)),
                sub,
                appearance,
            ));
            row.finish()
        }
    }
}

/// `{io} · {cache} cache` — headline io in main color, cache-read dimmed.
fn token_cell(
    totals: &WindowTotals,
    appearance: &Appearance,
    main: Fill,
    sub: Fill,
) -> Box<dyn Element> {
    if totals.tokens.total() == 0 {
        return span("—", sub, appearance);
    }
    let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
    row.add_child(span(fmt_tokens(totals.tokens.io()), main, appearance));
    if totals.tokens.cache_read > 0 {
        row.add_child(span(
            format!(" · {} cache", fmt_tokens(totals.tokens.cache_read)),
            sub,
            appearance,
        ));
    }
    row.finish()
}
