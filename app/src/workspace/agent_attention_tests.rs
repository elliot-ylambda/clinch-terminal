use super::{attention_tab_indices, next_attention_tab_index};

/// Tabs [A, B, C, D] with waiting = {B, D}: from C the cycle reaches D, then wraps to B, then
/// back to D — visiting every waiting tab in stable tab order.
#[test]
fn cycles_through_waiting_tabs_in_order_and_wraps() {
    let waiting = [1, 3]; // B and D of [A, B, C, D]

    let first = next_attention_tab_index(&waiting, 2);
    assert_eq!(first, Some(3)); // C -> D

    let second = next_attention_tab_index(&waiting, first.unwrap());
    assert_eq!(second, Some(1)); // D wraps -> B

    let third = next_attention_tab_index(&waiting, second.unwrap());
    assert_eq!(third, Some(3)); // B -> D again
}

/// Invoking the cycle while focused on a waiting tab advances past it rather than staying put.
#[test]
fn advances_past_the_focused_waiting_tab() {
    let waiting = [1, 3];
    assert_eq!(next_attention_tab_index(&waiting, 1), Some(3));
}

/// No waiting tabs: the action is a no-op.
#[test]
fn empty_waiting_set_yields_none() {
    assert_eq!(next_attention_tab_index(&[], 2), None);
}

/// The active tab is the only waiting tab: cycling returns it (re-activation is a no-op).
#[test]
fn sole_waiting_tab_is_returned_even_when_active() {
    assert_eq!(next_attention_tab_index(&[2], 2), Some(2));
}

/// Active tab after every waiting tab: wraps to the first waiting tab.
#[test]
fn wraps_when_active_tab_is_after_all_waiting_tabs() {
    let waiting = [0, 2];
    assert_eq!(next_attention_tab_index(&waiting, 3), Some(0));
}

/// The waiting-index projection preserves tab order and drives the "N waiting" count.
#[test]
fn attention_tab_indices_projects_flags_in_tab_order() {
    let flags = [false, true, false, true];
    let indices = attention_tab_indices(flags.into_iter());
    assert_eq!(indices, vec![1, 3]);
    assert_eq!(indices.len(), 2); // the "N waiting" count

    assert_eq!(
        attention_tab_indices([false, false].into_iter()),
        Vec::<usize>::new()
    );
    assert_eq!(
        attention_tab_indices(std::iter::empty::<bool>()),
        Vec::<usize>::new()
    );
}
