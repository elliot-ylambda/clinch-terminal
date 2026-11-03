//! Dragging footer buttons: where a drop lands, and what it does to the saved arrangement. A footer
//! is one ordered list split in two: the first buttons sit on the row and the rest open from More.

use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::Vector2F;

/// Where a dragged button would land.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FooterDrop {
    /// Before the row's button at this index, or after the last one at the row's length.
    Row(usize),
    More,
}

/// Anything over More lands in it. Anywhere else counts as the row, at the slot nearest the button,
/// so a button dragged out of More doesn't need to be dropped precisely.
pub(super) fn footer_drop_target(
    button_center: Vector2F,
    row: &[RectF],
    more_areas: &[RectF],
) -> FooterDrop {
    if more_areas
        .iter()
        .any(|area| area.contains_point(button_center))
    {
        return FooterDrop::More;
    }
    FooterDrop::Row(
        row.iter()
            .position(|button| button_center.x() < button.center().x())
            .unwrap_or(row.len()),
    )
}

/// A footer arrangement after a drop, with `to` as an index into the list without the moved button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FooterMove {
    pub(super) to: usize,
    pub(super) visible_count: usize,
}

/// What dropping the button at `from` does, or `None` if it changes nothing.
///
/// `shown` is how many buttons are on the row now, which a narrow pane keeps below `visible_count`.
/// There is no limit on how many a user can bring out of More; a pane that can't fit them all shows
/// the rest under More until it's wide enough.
pub(super) fn apply_footer_drop(
    from: usize,
    drop: FooterDrop,
    shown: usize,
    visible_count: usize,
    len: usize,
) -> Option<FooterMove> {
    if from >= len {
        return None;
    }
    let count = visible_count.min(len);
    let result = match drop {
        FooterDrop::More => {
            // Already under More, and not just because the pane is narrow.
            if from >= count {
                return None;
            }
            FooterMove {
                to: count - 1,
                visible_count: count - 1,
            }
        }
        FooterDrop::Row(slot) => {
            let mut to = if from < slot { slot - 1 } else { slot };
            // A full row can't take one more at its far end: it would land just out of sight.
            if from >= shown && shown < count {
                to = to.min(shown.saturating_sub(1));
            }
            FooterMove {
                to,
                visible_count: if from >= count { count + 1 } else { count },
            }
        }
    };
    (result.to != from || result.visible_count != count).then_some(result)
}

/// Moves a button in a saved list, given which entries were drawn. Saved entries a footer doesn't
/// draw right now, such as a button a session can't use, keep their place among the others.
pub(super) fn move_saved_entry<T>(
    saved: &mut Vec<T>,
    drawn: &[usize],
    from: usize,
    to: usize,
) -> bool {
    let Some(&moved) = drawn.get(from) else {
        return false;
    };
    let others: Vec<usize> = drawn.iter().copied().filter(|&i| i != moved).collect();
    let target = others
        .get(to)
        .copied()
        .or_else(|| others.last().map(|last| last + 1))
        .unwrap_or(moved);
    let entry = saved.remove(moved);
    let target = if target > moved { target - 1 } else { target };
    saved.insert(target.min(saved.len()), entry);
    true
}

/// `keys` arranged by a saved order, as indices into `keys`. Anything the saved order doesn't know
/// yet, such as a button added since, goes at the end.
pub(super) fn saved_order_indices(keys: &[String], saved: &[String]) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..keys.len()).collect();
    indices.sort_by_key(|&index| {
        saved
            .iter()
            .position(|key| key == &keys[index])
            .unwrap_or(saved.len())
    });
    indices
}

#[cfg(test)]
#[path = "footer_drag_tests.rs"]
mod tests;
