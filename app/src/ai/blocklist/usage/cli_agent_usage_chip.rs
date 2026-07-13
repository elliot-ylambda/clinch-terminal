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
use warpui::Element;

use super::CliAgentUsageProvider;
use crate::appearance::Appearance;

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

/// A focused provider panel: plan limits, local token totals, and a link to the
/// provider's authoritative web usage page.
pub fn render_cli_agent_usage_panel(
    snapshot: &UsageSnapshot,
    appearance: &Appearance,
    kind: CliAgentUsageProvider,
    link_mouse_state: MouseStateHandle,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let bg = theme.surface_2();
    let main = theme.main_text_color(bg);
    let sub = theme.sub_text_color(bg);
    let now = Utc::now();
    let provider = kind.data(snapshot);

    // Header row.
    let mut col = Flex::column().with_spacing(4.);
    col.add_child(span(
        format!("{} usage", kind.display_name()),
        main,
        appearance,
    ));

    // Plan-% rows.
    col.add_child(panel_row(
        span("5h", sub, appearance),
        plan_cell(provider.plan.and_then(|p| p.session), now, appearance, bg),
    ));
    col.add_child(panel_row(
        span("Weekly", sub, appearance),
        plan_cell(provider.plan.and_then(|p| p.weekly), now, appearance, bg),
    ));
    if kind == CliAgentUsageProvider::Claude {
        col.add_child(panel_row(
            span("Fable wk", sub, appearance),
            plan_cell(
                provider.plan.and_then(|p| p.fable_weekly),
                now,
                appearance,
                bg,
            ),
        ));
    }

    // Token rows.
    for (label, pick) in [
        ("Session", 0u8),
        ("Today", 1),
        ("This week", 2),
        ("This month", 3),
    ] {
        col.add_child(panel_row(
            span(label, sub, appearance),
            token_cell(window(provider, pick), appearance, main, sub),
        ));
    }

    col.add_child(
        Container::new(usage_link(kind, appearance, bg, link_mouse_state))
            .with_margin_top(6.)
            .finish(),
    );

    ConstrainedBox::new(
        Container::new(col.finish())
            .with_vertical_padding(12.)
            .with_horizontal_padding(16.)
            .with_background(bg)
            .with_border(Border::all(1.).with_border_fill(theme.accent()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .finish(),
    )
    .with_width(276.)
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
fn panel_row(label: Box<dyn Element>, value: Box<dyn Element>) -> Box<dyn Element> {
    let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
    row.add_child(ConstrainedBox::new(label).with_width(84.).finish());
    row.add_child(ConstrainedBox::new(value).with_width(160.).finish());
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
