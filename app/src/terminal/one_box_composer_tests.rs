use warpui::geometry::rect::RectF;
use warpui::geometry::vector::vec2f;

use super::{
    adoptable_native_draft, composer_edit_bytes, composer_rect, is_agent_command, joined_drafts,
    writes_text, ComposerKeys, GridRows,
};

#[test]
fn typed_and_pasted_text_counts_as_text() {
    assert!(writes_text(b"/"));
    assert!(writes_text(b"fix the parser"));
    assert!(writes_text(b"\x1b[200~long message\x1b[201~"));
    assert!(writes_text("é".as_bytes()));
}

#[test]
fn keys_and_escape_sequences_do_not_count_as_text() {
    // Escape, Ctrl-U, Enter, Backspace.
    assert!(!writes_text(b"\x1b"));
    assert!(!writes_text(&[0x15; 8]));
    assert!(!writes_text(b"\r\x7f"));
    // Shift+Tab, a kitty-protocol Ctrl-U, an arrow in application mode, Alt+b.
    assert!(!writes_text(b"\x1b[Z"));
    assert!(!writes_text(b"\x1b[117;5u"));
    assert!(!writes_text(b"\x1bOA"));
    assert!(!writes_text(b"\x1bb"));
    // An SGR mouse report and an OSC string.
    assert!(!writes_text(b"\x1b[<0;12;7M"));
    assert!(!writes_text(b"\x1b]0;title\x07"));
    assert!(!writes_text(b"\x1b]0;title\x1b\\"));
}

#[test]
fn image_placeholders_stay_in_the_agent_composer() {
    assert!(adoptable_native_draft("Reply with only the word ok"));
    assert!(!adoptable_native_draft("[Image #1] what is this"));
    assert!(!adoptable_native_draft("  \n "));
}

#[test]
fn adopted_text_goes_ahead_of_the_box_text() {
    assert_eq!(joined_drafts("restored", ""), "restored");
    assert_eq!(joined_drafts("restored", " \n"), "restored");
    assert_eq!(joined_drafts("restored", "typed"), "restored\ntyped");
}

#[test]
fn only_one_line_starting_with_a_slash_is_an_agent_command() {
    assert!(is_agent_command("/"));
    assert!(is_agent_command("/effort high"));
    assert!(!is_agent_command(""));
    assert!(!is_agent_command("fix /tmp paths"));
    assert!(!is_agent_command("/review\nand explain"));
}

#[test]
fn composer_edits_keep_the_shared_start_and_retype_the_rest() {
    let keys = ComposerKeys {
        backspace: b"\x7f",
        clear_line: b"\x15",
    };
    // Typing and deleting at the end.
    assert_eq!(composer_edit_bytes("/ef", "/eff", &keys), b"f");
    assert_eq!(composer_edit_bytes("/eff", "/ef", &keys), b"\x7f");
    assert_eq!(composer_edit_bytes("/ef", "/e!", &keys), b"\x7f!");
    // Deleting more clears the line and types the rest, since the agent's cursor stays at the end.
    assert_eq!(
        composer_edit_bytes("/efort", "/effort", &keys),
        b"\x15/effort"
    );
    assert_eq!(composer_edit_bytes("/diff", "", &keys), b"\x15");
    // Characters wider than a byte, and no change.
    assert_eq!(composer_edit_bytes("/é", "/", &keys), b"\x7f");
    assert_eq!(composer_edit_bytes("/", "/é", &keys), "é".as_bytes());
    assert!(composer_edit_bytes("/model", "/model", &keys).is_empty());
}

fn rows_in(bounds: RectF) -> GridRows {
    GridRows {
        origin: vec2f(4., 100.),
        bounds,
        cell_height: 20.,
    }
}

#[test]
fn composer_rect_spans_the_composer_rows_across_the_grid() {
    let rows = rows_in(RectF::new(vec2f(0., 0.), vec2f(800., 600.)));

    let rect = composer_rect(&(10..=12), &rows).expect("the composer is in view");

    assert_eq!(rect.origin(), vec2f(0., 300.));
    assert_eq!(rect.size(), vec2f(800., 60.));
}

#[test]
fn a_composer_scrolled_partly_out_of_view_is_not_replaced() {
    // Rows 10..=12 span y 300..360; the visible area ends at 340.
    let rows = rows_in(RectF::new(vec2f(0., 0.), vec2f(800., 340.)));
    assert!(composer_rect(&(10..=12), &rows).is_none());

    // And above the top of the visible area.
    let rows = rows_in(RectF::new(vec2f(0., 320.), vec2f(800., 400.)));
    assert!(composer_rect(&(10..=12), &rows).is_none());
}
