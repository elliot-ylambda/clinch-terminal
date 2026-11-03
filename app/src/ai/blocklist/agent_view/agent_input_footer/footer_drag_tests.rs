use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::vec2f;

use super::*;

fn button(x: f32) -> RectF {
    RectF::new(vec2f(x, 100.), vec2f(40., 24.))
}

#[test]
fn a_drop_picks_the_slot_nearest_the_button() {
    let row = [button(0.), button(50.), button(100.)];
    let target = |x| footer_drop_target(vec2f(x, 112.), &row, &[]);
    assert_eq!(target(5.), FooterDrop::Row(0));
    assert_eq!(target(30.), FooterDrop::Row(1));
    assert_eq!(target(80.), FooterDrop::Row(2));
    assert_eq!(target(500.), FooterDrop::Row(3));
}

#[test]
fn a_button_dropped_well_away_from_the_row_still_comes_out_of_more() {
    assert_eq!(
        footer_drop_target(vec2f(30., 10.), &[button(0.), button(50.)], &[]),
        FooterDrop::Row(1)
    );
    assert_eq!(
        footer_drop_target(vec2f(0., 0.), &[], &[]),
        FooterDrop::Row(0)
    );
}

#[test]
fn anything_over_more_or_its_menu_lands_in_more() {
    let more = button(200.);
    let menu = RectF::new(vec2f(200., 0.), vec2f(120., 90.));
    let row = [button(0.)];
    assert_eq!(
        footer_drop_target(vec2f(210., 112.), &row, &[more, menu]),
        FooterDrop::More
    );
    assert_eq!(
        footer_drop_target(vec2f(250., 40.), &row, &[more, menu]),
        FooterDrop::More
    );
}

#[test]
fn dragging_out_of_more_adds_one_to_the_row_with_no_limit() {
    // Six on the row, two under More: the seventh and eighth can both come out.
    assert_eq!(
        apply_footer_drop(6, FooterDrop::Row(1), 6, 6, 8),
        Some(FooterMove {
            to: 1,
            visible_count: 7
        })
    );
    assert_eq!(
        apply_footer_drop(7, FooterDrop::Row(7), 7, 7, 8),
        Some(FooterMove {
            to: 7,
            visible_count: 8
        })
    );
}

#[test]
fn the_first_button_under_more_can_join_the_end_of_the_row_without_moving_in_the_list() {
    assert_eq!(
        apply_footer_drop(2, FooterDrop::Row(2), 2, 2, 4),
        Some(FooterMove {
            to: 2,
            visible_count: 3
        })
    );
}

#[test]
fn dragging_along_the_row_reorders_it() {
    assert_eq!(
        apply_footer_drop(0, FooterDrop::Row(3), 4, 4, 6),
        Some(FooterMove {
            to: 2,
            visible_count: 4
        })
    );
    assert_eq!(
        apply_footer_drop(3, FooterDrop::Row(0), 4, 4, 6),
        Some(FooterMove {
            to: 0,
            visible_count: 4
        })
    );
}

#[test]
fn dropping_a_button_back_where_it_was_changes_nothing() {
    assert_eq!(apply_footer_drop(2, FooterDrop::Row(2), 4, 4, 6), None);
    assert_eq!(apply_footer_drop(2, FooterDrop::Row(3), 4, 4, 6), None);
    assert_eq!(apply_footer_drop(5, FooterDrop::More, 4, 4, 6), None);
}

#[test]
fn dragging_into_more_makes_it_the_first_button_there() {
    assert_eq!(
        apply_footer_drop(1, FooterDrop::More, 4, 4, 6),
        Some(FooterMove {
            to: 3,
            visible_count: 3
        })
    );
}

#[test]
fn the_last_button_on_a_full_footer_can_go_into_more() {
    // The saved count can be larger than the list.
    assert_eq!(
        apply_footer_drop(4, FooterDrop::More, 5, 6, 5),
        Some(FooterMove {
            to: 4,
            visible_count: 4
        })
    );
}

#[test]
fn a_narrow_pane_still_shows_the_button_you_just_dragged_out() {
    // Only three of six fit: dropped at the end of the row, it takes the last visible place.
    assert_eq!(
        apply_footer_drop(7, FooterDrop::Row(3), 3, 6, 8),
        Some(FooterMove {
            to: 2,
            visible_count: 7
        })
    );
    // A button only under More because of the width doesn't change the count.
    assert_eq!(
        apply_footer_drop(4, FooterDrop::Row(0), 3, 6, 8),
        Some(FooterMove {
            to: 0,
            visible_count: 6
        })
    );
}

#[test]
fn undrawn_entries_keep_their_place_when_a_button_moves() {
    // `x` isn't drawn. Moving `c` first leaves it between `a` and `b`.
    let mut saved = vec!["a", "x", "b", "c"];
    assert!(move_saved_entry(&mut saved, &[0, 2, 3], 2, 0));
    assert_eq!(saved, ["c", "a", "x", "b"]);

    let mut saved = vec!["a", "x", "b", "c"];
    assert!(move_saved_entry(&mut saved, &[0, 2, 3], 0, 2));
    assert_eq!(saved, ["x", "b", "c", "a"]);

    let mut saved = vec!["a", "b", "c"];
    assert!(move_saved_entry(&mut saved, &[0, 1, 2], 2, 1));
    assert_eq!(saved, ["a", "c", "b"]);

    assert!(!move_saved_entry(&mut saved, &[0, 1, 2], 9, 0));
}

#[test]
fn a_saved_order_applies_and_new_buttons_go_last() {
    let keys: Vec<String> = ["a", "b", "new", "c"].map(str::to_owned).to_vec();
    let saved: Vec<String> = ["c", "gone", "a", "b"].map(str::to_owned).to_vec();
    assert_eq!(saved_order_indices(&keys, &saved), [3, 0, 1, 2]);
    assert_eq!(saved_order_indices(&keys, &[]), [0, 1, 2, 3]);
}
