use warpui_core::color::ColorU;

use super::*;
use crate::ui::color::contrast::contrast_ratio;
use crate::ui::theme::{mock_terminal_colors, Details, Fill, WarpTheme};

fn theme(background: u32, foreground: u32, accent: u32) -> WarpTheme {
    WarpTheme::new(
        Fill::Solid(ColorU::from_u32(background)),
        ColorU::from_u32(foreground),
        Fill::Solid(ColorU::from_u32(accent)),
        None,
        Some(Details::Darker),
        mock_terminal_colors(),
        None,
        None,
    )
}

fn hex(color: ColorU) -> u32 {
    (color.r as u32) << 16 | (color.g as u32) << 8 | color.b as u32
}

#[test]
fn ink_matches_the_prototype_scale() {
    let tokens = Tokens::new(&theme(0x0D0E11FF, 0xD3D6DDFF, 0xC5F06BFF));
    assert_eq!(hex(tokens.surface_terminal), 0x0D0E11);
    assert_eq!(hex(tokens.surface_chrome), 0x1D1E21);
    assert_eq!(hex(tokens.surface_field), 0x252629);
    assert_eq!(hex(tokens.surface_raised), 0x2D2E32);
    assert_eq!(hex(tokens.control_track), 0x393A3E);
    assert_eq!(hex(tokens.text_on_accent), 0x1D1E21);
    assert_eq!(hex(tokens.status(StatusKind::Working)), 0x5BD08A);
    assert_eq!(hex(tokens.status(StatusKind::NeedsYou)), 0xF2B04B);
    assert_eq!(hex(tokens.status(StatusKind::Done)), 0x6FA8FF);
    assert_eq!(hex(tokens.status(StatusKind::RateLimited)), 0xF08080);
}

#[test]
fn paper_matches_the_prototype_scale() {
    let tokens = Tokens::new(&theme(0xFFFFFFFF, 0x2A2C31FF, 0x4F7D0FFF));
    assert_eq!(hex(tokens.surface_terminal), 0xFFFFFF);
    assert_eq!(hex(tokens.surface_chrome), 0xEFEFF0);
    assert_eq!(hex(tokens.surface_field), 0xE4E5E5);
    assert_eq!(hex(tokens.surface_raised), 0xFFFFFF);
    assert_eq!(hex(tokens.text_on_accent), 0xFFFFFF);
    assert_eq!(hex(tokens.status(StatusKind::NeedsYou)), 0xA8650A);
}

#[test]
fn low_contrast_themes_are_lifted_to_the_floor() {
    // A dim foreground on a dark background: every tier must still read on every surface.
    let dim = theme(0x202020FF, 0x505050FF, 0x808080FF);
    let tokens = Tokens::new(&dim);
    let background = ColorU::from_u32(0x202020FF);
    let foreground = ColorU::from_u32(0x505050FF);
    for amount in [DARK_MIX.chrome, DARK_MIX.field, DARK_MIX.raised] {
        let surface = mix(background, foreground, amount);
        assert!(contrast_ratio(tokens.text_3, surface) >= 4.5);
    }
}

#[test]
fn image_themes_draw_translucent_surfaces_and_opaque_overlays() {
    let gradient = WarpTheme::new(
        Fill::VerticalGradient(crate::ui::theme::VerticalGradient::new(
            ColorU::from_u32(0x101010FF),
            ColorU::from_u32(0x303030FF),
        )),
        ColorU::from_u32(0xE0E0E0FF),
        Fill::Solid(ColorU::from_u32(0xC5F06BFF)),
        None,
        Some(Details::Darker),
        mock_terminal_colors(),
        None,
        None,
    );
    let tokens = Tokens::new(&gradient);
    assert_eq!(tokens.surface_terminal.a, 0);
    assert!(tokens.surface_chrome.a < 0xFF);
    assert_eq!(tokens.surface_overlay.a, 0xFF);
}

#[test]
fn text_on_accent_reads() {
    for accent in [0xC5F06BFF, 0x4F7D0FFF, 0xFF79C6FF, 0x0000FFFF] {
        let tokens = Tokens::new(&theme(0x0D0E11FF, 0xD3D6DDFF, accent));
        assert!(contrast_ratio(tokens.text_on_accent, tokens.accent) >= 4.5);
    }
}
