//! Warp UI Components module contains functions and structs that implement our internal components
//! used for the apps design (our buttons with styling, headers and panels etc.) as well definition
//! of colors (aka blended colors from the figma designs derived from Warp theme) and icons used
//! within the app.
use pathfinder_color::ColorU;
use warp_core::ui::theme::Fill;

pub(crate) mod agent_icon;
pub(crate) mod avatar;
pub(crate) mod blended_colors;
pub(crate) mod breadcrumb;
pub mod buttons;
pub(crate) mod color_dot;
pub(crate) mod dialog;
pub(crate) mod icon_with_status;
pub(crate) mod item_highlight;
pub(crate) mod menu_button;
pub(crate) mod red_notification_dot;
pub(crate) mod render_file_search_row;
pub mod tab_selector;
pub(crate) mod window_focus_dimming;

pub use warp_core::ui::icons;

/// Green used by the stable Clinch app icon's glyph and cursor.
pub(crate) const CLINCH_LOGO_GREEN: ColorU = ColorU {
    r: 0xBF,
    g: 0xFF,
    b: 0x00,
    a: 0xFF,
};

/// Blue used for "agent done / unread notification" indicators (project-tab
/// done counts, tab notification dots). Deliberately NOT `theme.accent()`:
/// themes may set the accent to the same lime as [`CLINCH_LOGO_GREEN`], which
/// makes "working" (green) and "done" (blue) badges indistinguishable.
pub(crate) const CLINCH_DONE_BLUE: ColorU = ColorU {
    r: 0x37,
    g: 0x80,
    b: 0xE9,
    a: 0xFF,
};

/// Thickness of the accent rules that tie the app chrome together: the
/// highlighted project tab's outline, the CLI-agent message-history header
/// underline, and the agent/terminal footer's top rule. Shared so the three
/// always read as one system.
pub(crate) const ACCENT_CHROME_RULE_WIDTH: f32 = 1.;

/// Fill for those same rules — the theme accent at full opacity, flattened to a
/// solid so a gradient accent still paints an even line. Pass
/// `theme.accent()`.
pub(crate) fn accent_chrome_rule_fill(accent: Fill) -> Fill {
    Fill::Solid(accent.into_solid())
}

const BORDER_RADIUS: f32 = 4.;
