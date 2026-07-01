//! Deterministic per-project color for a window's top header (the tab-bar
//! strip).
//!
//! Maps a project directory to one of the shared tab palette colors
//! ([`TAB_COLOR_OPTIONS`]) with a stable hash, so the same project always gets
//! the same hue across restarts. See
//! `docs/superpowers/specs/2026-06-30-window-header-project-colors-design.md`.

use std::path::Path;

use warp_core::ui::color::blend::Blend;
use warp_core::ui::color::Opacity;
use warp_core::ui::theme::{AnsiColorIdentifier, Fill, WarpTheme};

use crate::ui_components::color_dot::TAB_COLOR_OPTIONS;

/// Opacity of the project tint over the base header background. Subtle by
/// design ("a hint of color, not a bar") — a touch lighter than the 25% used
/// for the per-tab color dots.
const WINDOW_HEADER_TINT_OPACITY: Opacity = 18;

/// Deterministically maps a project directory to one of [`TAB_COLOR_OPTIONS`].
///
/// Uses an explicit FNV-1a hash (deliberately not `DefaultHasher`, whose output
/// is not guaranteed stable across toolchains) so a project's color never
/// drifts between versions or platforms.
pub(crate) fn header_color_for_path(path: &Path) -> AnsiColorIdentifier {
    // Normalize away a trailing separator so ".../clinch" and ".../clinch/"
    // resolve to the same color.
    let key = path.to_string_lossy();
    let key = key.trim_end_matches(['/', '\\']);

    // FNV-1a (64-bit): a fixed function of the input bytes, so the mapping is
    // reproducible across releases and platforms.
    const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = FNV_OFFSET_BASIS;
    for byte in key.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    let index = (hash % TAB_COLOR_OPTIONS.len() as u64) as usize;
    TAB_COLOR_OPTIONS[index]
}

/// Resolves a window's header color: an explicit tab color (a manually chosen
/// color, or a configured directory color) wins; otherwise the project
/// directory is hashed. `None` when there is no project directory to hash
/// (e.g. a remote session with no local working directory).
pub(crate) fn resolve_header_color(
    explicit: Option<AnsiColorIdentifier>,
    project_dir: Option<&Path>,
) -> Option<AnsiColorIdentifier> {
    explicit.or_else(|| project_dir.map(header_color_for_path))
}

/// Builds the subtle header tint: the chosen palette color blended over the
/// theme background at [`WINDOW_HEADER_TINT_OPACITY`]. Mirrors the
/// `internal_colors::accent_bg` idiom so the tint stays theme-aware.
pub(crate) fn header_tint_fill(theme: &WarpTheme, color_id: AnsiColorIdentifier) -> Fill {
    let color: Fill = color_id
        .to_ansi_color(&theme.terminal_colors().normal)
        .into();
    Fill::Solid(theme.background().into_solid())
        .blend(&color.with_opacity(WINDOW_HEADER_TINT_OPACITY))
}

#[cfg(test)]
#[path = "header_color_tests.rs"]
mod tests;
