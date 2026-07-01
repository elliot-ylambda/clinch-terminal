use std::path::Path;

use warp_core::ui::theme::AnsiColorIdentifier;

use super::{header_color_for_path, resolve_header_color};

/// Counts the distinct colors produced for a set of paths without requiring
/// `AnsiColorIdentifier: Hash` (it only derives `PartialEq`/`Eq`).
fn distinct_colors(paths: &[&str]) -> Vec<AnsiColorIdentifier> {
    let mut seen: Vec<AnsiColorIdentifier> = Vec::new();
    for p in paths {
        let color = header_color_for_path(Path::new(p));
        if !seen.contains(&color) {
            seen.push(color);
        }
    }
    seen
}

#[test]
fn same_path_always_maps_to_same_color() {
    let path = Path::new("/Users/me/projects/clinch");
    assert_eq!(header_color_for_path(path), header_color_for_path(path));
}

#[test]
fn trailing_separator_does_not_change_color() {
    let without = Path::new("/Users/me/projects/clinch");
    let with = Path::new("/Users/me/projects/clinch/");
    assert_eq!(header_color_for_path(without), header_color_for_path(with));
}

#[test]
fn distinct_projects_spread_across_the_palette() {
    // A degenerate hash (e.g. one that always returns the same color) would
    // collapse all of these into a single bucket. Require a reasonable spread.
    let paths = [
        "/Users/me/projects/clinch",
        "/Users/me/projects/web",
        "/Users/me/work/api",
        "/Users/me/work/infra",
        "/tmp/scratch",
        "/opt/thing",
        "/home/dev/alpha",
        "/home/dev/beta",
        "/home/dev/gamma",
        "/var/data/x",
        "/srv/app/y",
        "/root/z",
    ];
    let distinct = distinct_colors(&paths);
    assert!(
        distinct.len() >= 3,
        "expected header colors to spread across the palette, got {} distinct",
        distinct.len()
    );
}

#[test]
fn color_is_always_from_the_tab_palette() {
    use crate::ui_components::color_dot::TAB_COLOR_OPTIONS;
    for p in ["/a", "/a/b", "/some/deep/project/root", "/x"] {
        let color = header_color_for_path(Path::new(p));
        assert!(
            TAB_COLOR_OPTIONS.contains(&color),
            "{color:?} is not one of the tab palette colors"
        );
    }
}

#[test]
fn explicit_tab_color_overrides_the_directory_hash() {
    let path = Path::new("/Users/me/projects/clinch");
    // An explicit color wins regardless of what the directory would hash to.
    assert_eq!(
        resolve_header_color(Some(AnsiColorIdentifier::Magenta), Some(path)),
        Some(AnsiColorIdentifier::Magenta)
    );
}

#[test]
fn falls_back_to_directory_hash_when_no_explicit_color() {
    let path = Path::new("/Users/me/projects/clinch");
    assert_eq!(
        resolve_header_color(None, Some(path)),
        Some(header_color_for_path(path))
    );
}

#[test]
fn no_color_when_there_is_no_project_directory() {
    assert_eq!(resolve_header_color(None, None), None);
}
