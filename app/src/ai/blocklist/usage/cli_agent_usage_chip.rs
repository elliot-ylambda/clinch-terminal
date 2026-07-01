use chrono::{DateTime, Utc};
use cli_agent_usage::format::{chip_halves, fmt_pct, fmt_reset, fmt_tokens};
use cli_agent_usage::{LimitWindow, Provider, Severity, UsageSnapshot, WindowTotals};

// Element + theme imports — mirror app/src/context_chips/display_chip.rs.
use warp_core::ui::theme::{Fill, WarpTheme};
use warp_core::ui::Icon;
use warpui::elements::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Flex, ParentElement,
    Radius, Text,
};
use warpui::Element;

use crate::appearance::Appearance;

/// Map a crate `Severity` to a fill against `bg` (the surface the text sits on).
fn severity_fill(severity: Severity, theme: &WarpTheme, bg: Fill) -> Fill {
    match severity {
        Severity::Normal => theme.main_text_color(bg),
        Severity::Warning => Fill::Solid(theme.ui_warning_color()),
        Severity::Critical => Fill::Solid(theme.ui_error_color()),
    }
}

/// A monospace text span in a given color.
fn span(text: impl Into<String>, color: Fill, appearance: &Appearance) -> Box<dyn Element> {
    Text::new_inline(
        text.into(),
        appearance.monospace_font_family(),
        appearance.monospace_font_size(),
    )
    .with_color(color.into())
    .with_line_height_ratio(appearance.line_height_ratio())
    .finish()
}

/// The footer chip: `[clock] cc 47%w · cx 55%w`, each %-half colored by its severity.
/// `None` when neither tool has data (chip hidden).
pub fn render_cli_agent_usage_chip(
    snapshot: &UsageSnapshot,
    appearance: &Appearance,
    bg: Fill,
) -> Option<Box<dyn Element>> {
    let halves = chip_halves(snapshot)?;
    let theme = appearance.theme();
    let neutral = theme.sub_text_color(bg);
    let icon_size = appearance.monospace_font_size();

    let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
    row.add_child(
        Container::new(
            ConstrainedBox::new(Icon::Clock.to_warpui_icon(theme.main_text_color(bg)).finish())
                .with_width(icon_size)
                .with_height(icon_size)
                .finish(),
        )
        .with_margin_right(4.)
        .finish(),
    );

    for (i, half) in halves.iter().enumerate() {
        if i > 0 {
            row.add_child(span(" · ", neutral, appearance));
        }
        row.add_child(span(format!("{} ", half.label), neutral, appearance));
        row.add_child(span(
            half.pct.clone(),
            severity_fill(half.severity, theme, bg),
            appearance,
        ));
    }

    Some(Container::new(row.finish()).with_vertical_padding(4.).finish())
}

/// The expanded panel: two columns (Claude | Codex) — 5h %, weekly %, then
/// session/today/week/month input+output tokens (cache-read dimmed). No cost.
pub fn render_cli_agent_usage_panel(
    snapshot: &UsageSnapshot,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let bg = theme.surface_2();
    let main = theme.main_text_color(bg);
    let sub = theme.sub_text_color(bg);
    let now = Utc::now();

    // Header row.
    let mut col = Flex::column().with_spacing(4.);
    col.add_child(panel_row(
        span("", sub, appearance),
        span("Claude Code", main, appearance),
        span("Codex", main, appearance),
    ));

    // Plan-% rows.
    col.add_child(panel_row(
        span("5h", sub, appearance),
        plan_cell(snapshot.claude.plan.and_then(|p| p.session), now, appearance, bg),
        plan_cell(snapshot.codex.plan.and_then(|p| p.session), now, appearance, bg),
    ));
    col.add_child(panel_row(
        span("Weekly", sub, appearance),
        plan_cell(snapshot.claude.plan.and_then(|p| p.weekly), now, appearance, bg),
        plan_cell(snapshot.codex.plan.and_then(|p| p.weekly), now, appearance, bg),
    ));

    // Token rows.
    for (label, pick) in [
        ("Session", 0u8),
        ("Today", 1),
        ("This week", 2),
        ("This month", 3),
    ] {
        col.add_child(panel_row(
            span(label, sub, appearance),
            token_cell(window(&snapshot.claude, pick), appearance, main, sub),
            token_cell(window(&snapshot.codex, pick), appearance, main, sub),
        ));
    }

    ConstrainedBox::new(
        Container::new(col.finish())
            .with_vertical_padding(12.)
            .with_horizontal_padding(16.)
            .with_background(bg)
            .with_border(Border::all(1.).with_border_fill(theme.accent()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .finish(),
    )
    .with_width(320.)
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

/// A three-cell row: fixed-width label, then two equal provider columns.
fn panel_row(
    label: Box<dyn Element>,
    claude: Box<dyn Element>,
    codex: Box<dyn Element>,
) -> Box<dyn Element> {
    let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
    row.add_child(ConstrainedBox::new(label).with_width(84.).finish());
    row.add_child(ConstrainedBox::new(claude).with_width(108.).finish());
    row.add_child(ConstrainedBox::new(codex).with_width(108.).finish());
    row.finish()
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
            row.add_child(span(fmt_pct(w.percent), severity_fill(w.severity, theme, bg), appearance));
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
