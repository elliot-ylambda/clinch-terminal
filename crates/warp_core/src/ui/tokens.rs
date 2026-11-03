//! Soft Graphite design tokens: the one scale of radii and spacing, and the semantic colors every
//! surface of the app draws from.
//!
//! Colors are derived from the existing [`WarpTheme`] fields rather than stored, so bundled themes,
//! user YAML themes and Base16 imports all work without new fields. Status colors are fixed
//! dark/light pairs because they must never collide with a theme's accent. Text tiers are nudged
//! until they reach 4.5:1 on every surface they sit on, so low-contrast third-party themes stay
//! readable without being edited.

use warpui_core::color::ColorU;
use warpui_core::elements::{CornerRadius, DropShadow, Radius};
use warpui_core::geometry::vector::vec2f;

use super::color::contrast::contrast_ratio;
use super::theme::{ColorScheme, Fill, WarpTheme};

/// Chips, small buttons, menu items, section headers.
pub const RADIUS_SM: Radius = Radius::Pixels(8.);
/// Tab rows, fields, pills, buttons.
pub const RADIUS_MD: Radius = Radius::Pixels(12.);
/// Panels, menus, dialogs, the terminal card.
pub const RADIUS_LG: Radius = Radius::Pixels(16.);
/// Dots, meters, switch tracks.
pub const RADIUS_PILL: Radius = Radius::Percentage(50.);

pub const SPACE_1: f32 = 2.;
pub const SPACE_2: f32 = 4.;
pub const SPACE_3: f32 = 6.;
pub const SPACE_4: f32 = 8.;
pub const SPACE_5: f32 = 10.;
pub const SPACE_6: f32 = 12.;
pub const SPACE_7: f32 = 16.;
pub const SPACE_8: f32 = 20.;
pub const SPACE_9: f32 = 24.;
pub const SPACE_10: f32 = 32.;

/// Inset between the window frame and the rounded terminal card, and the gap between split cards.
pub const CARD_INSET: f32 = SPACE_5;
/// Height of a one-line row in the vertical tab list.
pub const ROW_HEIGHT: f32 = 32.;

/// Interface type sizes.
pub const TEXT_XS: f32 = 11.;
pub const TEXT_SM: f32 = 12.;
pub const TEXT_MD: f32 = 13.;
pub const TEXT_LG: f32 = 15.;
pub const TEXT_XL: f32 = 20.;

pub fn corners_sm() -> CornerRadius {
    CornerRadius::with_all(RADIUS_SM)
}

pub fn corners_md() -> CornerRadius {
    CornerRadius::with_all(RADIUS_MD)
}

pub fn corners_lg() -> CornerRadius {
    CornerRadius::with_all(RADIUS_LG)
}

pub fn corners_pill() -> CornerRadius {
    CornerRadius::with_all(RADIUS_PILL)
}

/// The four things a session can be telling you.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusKind {
    Working,
    NeedsYou,
    Done,
    RateLimited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StatusPalette {
    working: ColorU,
    needs_you: ColorU,
    done: ColorU,
    rate_limited: ColorU,
}

impl StatusPalette {
    fn get(&self, kind: StatusKind) -> ColorU {
        match kind {
            StatusKind::Working => self.working,
            StatusKind::NeedsYou => self.needs_you,
            StatusKind::Done => self.done,
            StatusKind::RateLimited => self.rate_limited,
        }
    }
}

const fn rgb(hex: u32) -> ColorU {
    ColorU {
        r: ((hex >> 16) & 0xFF) as u8,
        g: ((hex >> 8) & 0xFF) as u8,
        b: (hex & 0xFF) as u8,
        a: 0xFF,
    }
}

const DARK_STATUS: StatusPalette = StatusPalette {
    working: rgb(0x5BD08A),
    needs_you: rgb(0xF2B04B),
    done: rgb(0x6FA8FF),
    rate_limited: rgb(0xF08080),
};

const LIGHT_STATUS: StatusPalette = StatusPalette {
    working: rgb(0x1D8A4D),
    needs_you: rgb(0xA8650A),
    done: rgb(0x2A64C9),
    rate_limited: rgb(0xC23B3B),
};

/// Contrast floors, a hair over WCAG's 4.5:1 and 3:1 so rounding in alpha blending can't drop a
/// color under them once it is on screen.
const TEXT_FLOOR: f32 = 4.6;
const NON_TEXT_FLOOR: f32 = 3.1;

/// Alpha of a status or accent tint drawn behind a row, as a byte (about 14%).
const SOFT_ALPHA: u8 = 36;

/// How far each surface moves from the theme background toward its foreground.
///
/// Dark themes lift from the terminal, so a raised surface is lighter than chrome. Light themes
/// sink instead: chrome and fields are darker than the terminal, and a raised surface is the
/// terminal color itself, so it reads as a sheet sitting on top.
struct SurfaceMix {
    chrome: f32,
    field: f32,
    raised: f32,
    hover: f32,
    control_track: f32,
}

const DARK_MIX: SurfaceMix = SurfaceMix {
    chrome: 0.08,
    field: 0.12,
    raised: 0.16,
    hover: 0.12,
    control_track: 0.22,
};

const LIGHT_MIX: SurfaceMix = SurfaceMix {
    chrome: 0.075,
    field: 0.125,
    raised: 0.,
    hover: 0.115,
    control_track: 0.22,
};

/// Semantic colors derived from a theme. Cheap to copy; [`super::appearance::Appearance`] caches
/// one per theme so reading it from a render is a field access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tokens {
    /// Terminal and editor content. Fully transparent when the theme draws an image or gradient
    /// there, so it keeps showing through.
    pub surface_terminal: ColorU,
    /// Window frame, sidebar and title bar.
    pub surface_chrome: ColorU,
    /// Inputs, chips, idle buttons.
    pub surface_field: ColorU,
    /// Selected row, active pill.
    pub surface_raised: ColorU,
    pub surface_hover: ColorU,
    /// Popovers, menus, palette, dialogs. Always opaque so text over an image stays legible.
    pub surface_overlay: ColorU,
    pub surface_scrim: ColorU,
    /// The rare divider inside one surface. Surfaces are otherwise separated by shade.
    pub hairline: ColorU,
    /// Off switches, slider and meter tracks.
    pub control_track: ColorU,
    pub text_1: ColorU,
    pub text_2: ColorU,
    pub text_3: ColorU,
    pub accent: ColorU,
    pub accent_soft: ColorU,
    pub text_on_accent: ColorU,
    pub focus_ring: ColorU,
    pub provider_claude: ColorU,
    pub provider_codex: ColorU,
    scheme: ColorScheme,
    status: StatusPalette,
}

impl Tokens {
    pub fn new(theme: &WarpTheme) -> Self {
        let background = theme.background().into_solid();
        let foreground = theme.foreground().into_solid();
        // From the background rather than the foreground: a low-contrast theme can have a dark
        // foreground on a dark background, and surfaces and text must still move the right way.
        let scheme = if contrast_ratio(ColorU::white(), background)
            >= contrast_ratio(ColorU::black(), background)
        {
            ColorScheme::LightOnDark
        } else {
            ColorScheme::DarkOnLight
        };
        let (mix_table, status, provider_claude, extreme) = match scheme {
            ColorScheme::LightOnDark => (DARK_MIX, DARK_STATUS, rgb(0xE8875F), ColorU::white()),
            ColorScheme::DarkOnLight => (LIGHT_MIX, LIGHT_STATUS, rgb(0xC2562C), ColorU::black()),
        };
        let shows_through =
            theme.background_image().is_some() || !matches!(theme.background(), Fill::Solid(_));

        // What each surface looks like on screen. Contrast is always checked against this, even
        // when the surface is really drawn as an alpha overlay.
        let opaque_chrome = mix(background, foreground, mix_table.chrome);
        let opaque_field = mix(background, foreground, mix_table.field);
        let opaque_raised = mix(background, foreground, mix_table.raised);
        let opaque_hover = mix(background, foreground, mix_table.hover);
        let text_surfaces = [
            background,
            opaque_chrome,
            opaque_field,
            opaque_raised,
            opaque_hover,
        ];

        let drawn = |opaque: ColorU, amount: f32| {
            if shows_through && amount > 0. {
                overlay(foreground, amount)
            } else {
                opaque
            }
        };

        let (text_1, text_2, text_3) = match scheme {
            ColorScheme::LightOnDark => (
                mix(foreground, ColorU::white(), 0.45),
                mix(background, foreground, 0.79),
                mix(background, foreground, 0.65),
            ),
            ColorScheme::DarkOnLight => (
                mix(foreground, ColorU::black(), 0.35),
                mix(background, foreground, 0.83),
                mix(background, foreground, 0.74),
            ),
        };

        let clear_text_floor = |color| with_floor(color, extreme, &text_surfaces, TEXT_FLOOR);
        let clear_non_text_floor =
            |color| with_floor(color, extreme, &text_surfaces, NON_TEXT_FLOOR);

        let accent = theme.accent().into_solid();
        Self {
            surface_terminal: if shows_through {
                ColorU::transparent_black()
            } else {
                background
            },
            surface_chrome: drawn(opaque_chrome, mix_table.chrome),
            surface_field: drawn(opaque_field, mix_table.field),
            surface_raised: drawn(opaque_raised, mix_table.raised),
            surface_hover: drawn(opaque_hover, mix_table.hover),
            surface_overlay: opaque_raised,
            surface_scrim: match scheme {
                ColorScheme::LightOnDark => ColorU::new(5, 6, 8, 158),
                ColorScheme::DarkOnLight => ColorU::new(20, 20, 18, 71),
            },
            hairline: match scheme {
                ColorScheme::LightOnDark => ColorU::new(255, 255, 255, 15),
                ColorScheme::DarkOnLight => ColorU::new(0, 0, 0, 20),
            },
            control_track: drawn(
                mix(background, foreground, mix_table.control_track),
                mix_table.control_track,
            ),
            text_1: clear_text_floor(text_1),
            text_2: clear_text_floor(text_2),
            text_3: clear_text_floor(text_3),
            accent,
            accent_soft: with_alpha(accent, SOFT_ALPHA),
            text_on_accent: best_text_on(accent, opaque_chrome),
            focus_ring: accent,
            provider_claude,
            provider_codex: clear_non_text_floor(mix(background, foreground, 0.85)),
            scheme,
            status: StatusPalette {
                working: clear_non_text_floor(status.working),
                needs_you: clear_non_text_floor(status.needs_you),
                done: clear_non_text_floor(status.done),
                rate_limited: clear_non_text_floor(status.rate_limited),
            },
        }
    }

    pub fn color_scheme(&self) -> ColorScheme {
        self.scheme
    }

    pub fn status(&self, kind: StatusKind) -> ColorU {
        self.status.get(kind)
    }

    /// A tint of the status color, for a row or banner background.
    pub fn status_soft(&self, kind: StatusKind) -> ColorU {
        with_alpha(self.status.get(kind), SOFT_ALPHA)
    }

    /// The shadow under menus, popovers and dialogs.
    pub fn overlay_shadow(&self) -> DropShadow {
        DropShadow {
            color: match self.scheme {
                ColorScheme::LightOnDark => ColorU::new(0, 0, 0, 115),
                ColorScheme::DarkOnLight => ColorU::new(0, 0, 0, 36),
            },
            offset: vec2f(0., 12.),
            blur_radius: 32.,
            spread_radius: 0.,
        }
    }
}

/// Moves `amount` of the way from `from` to `to`, rounding each channel.
fn mix(from: ColorU, to: ColorU, amount: f32) -> ColorU {
    let channel = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * amount).round() as u8;
    ColorU::new(
        channel(from.r, to.r),
        channel(from.g, to.g),
        channel(from.b, to.b),
        0xFF,
    )
}

fn overlay(color: ColorU, amount: f32) -> ColorU {
    with_alpha(color, (amount * 255.).round() as u8)
}

fn with_alpha(color: ColorU, alpha: u8) -> ColorU {
    ColorU::new(color.r, color.g, color.b, alpha)
}

/// Moves `color` toward `extreme` just far enough to clear `floor` on every surface. Falls back to
/// the extreme itself on a mid-gray theme where nothing clears it.
fn with_floor(color: ColorU, extreme: ColorU, surfaces: &[ColorU], floor: f32) -> ColorU {
    (0..=100)
        .map(|step| mix(color, extreme, step as f32 / 100.))
        .find(|candidate| {
            surfaces
                .iter()
                .all(|surface| contrast_ratio(*candidate, *surface) >= floor)
        })
        .unwrap_or(extreme)
}

/// Text drawn on an accent fill: the chrome color when it reads, otherwise white or black. Black
/// needs no check: its ratio times white's is always 21, so if white is under 4.5, black is over.
fn best_text_on(accent: ColorU, chrome: ColorU) -> ColorU {
    [chrome, ColorU::white()]
        .into_iter()
        .find(|candidate| contrast_ratio(*candidate, accent) >= 4.5)
        .unwrap_or(ColorU::black())
}

#[cfg(test)]
#[path = "tokens_tests.rs"]
mod tests;
