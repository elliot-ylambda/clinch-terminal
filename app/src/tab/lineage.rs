use std::collections::{HashMap, HashSet};
use std::ops::Range;

use serde::{Deserialize, Serialize};
use warpui::AppContext;

use super::TabData;
use crate::app_state::PaneUuid;
use crate::features::FeatureFlag;
use crate::terminal::CLIAgent;
use crate::workspace::tab_group::TabGroupId;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabOrigin {
    pub kind: TabOriginKind,
    pub parent_pane_uuid: PaneUuid,
    /// Fallback when the source pane is no longer open.
    pub parent_title: String,
    /// The source's own sources, nearest first, recorded at creation. When the source
    /// closes, this tab takes its place under the nearest ancestor that is still open.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ancestors: Vec<PaneUuid>,
}

/// How many of its source's own sources a new tab remembers.
const MAX_ANCESTORS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TabOriginKind {
    Fork,
    Transfer { from: CLIAgent },
}

impl TabOrigin {
    /// The source pane, then the source's own sources, nearest first.
    fn sources(&self) -> impl Iterator<Item = &PaneUuid> {
        std::iter::once(&self.parent_pane_uuid).chain(&self.ancestors)
    }

    /// `ancestors` for a tab created from a tab whose own origin is `source`.
    pub(crate) fn ancestry_from(source: Option<&TabOrigin>) -> Vec<PaneUuid> {
        source.map_or_else(Vec::new, |source| {
            source.sources().take(MAX_ANCESTORS).cloned().collect()
        })
    }

    pub(crate) fn tooltip(&self, live_parent_title: Option<&str>) -> String {
        let title = live_parent_title.unwrap_or(&self.parent_title);
        let closed = if live_parent_title.is_none() {
            " (closed)"
        } else {
            ""
        };
        match self.kind {
            TabOriginKind::Fork => format!("Forked from \"{title}\"{closed}"),
            TabOriginKind::Transfer { from } => {
                format!(
                    "Transferred from {} · \"{title}\"{closed}",
                    from.display_name()
                )
            }
        }
    }
}

/// The view-independent inputs shared by insertion and sidebar rendering.
pub(crate) struct TabLineage {
    pub pane_uuids: Vec<PaneUuid>,
    /// The source pane, then the source's own sources, nearest first.
    pub sources: Vec<PaneUuid>,
    pub group_id: Option<TabGroupId>,
    pub pinned: bool,
}

impl TabLineage {
    pub(crate) fn for_tab(tab: &TabData, app: &AppContext) -> Self {
        Self {
            pane_uuids: tab.pane_group.as_ref(app).terminal_pane_uuids(),
            sources: tab
                .origin
                .as_ref()
                .map_or_else(Vec::new, |origin| origin.sources().cloned().collect()),
            group_id: tab.group_id,
            pinned: FeatureFlag::PinnedTabs.is_enabled() && tab.pinned,
        }
    }
}

/// The deepest indent the sidebar draws. Deeper forks keep their place in the tree and
/// still move with their source; they share the last visible level so a narrow panel
/// keeps room for titles.
pub(crate) const MAX_VISIBLE_LINEAGE_DEPTH: usize = 3;

/// The nearest of `tab`'s sources that some tab in `open` still owns. A closed source
/// hands its place to its own source.
fn open_source<'a>(tab: &'a TabLineage, open: &HashSet<&PaneUuid>) -> Option<&'a PaneUuid> {
    tab.sources.iter().find(|source| open.contains(source))
}

/// Each tab's depth in its fork/transfer tree, in list order (0 = not nested). Pass the
/// whole tab list: a source missing from it counts as closed.
///
/// A family is stored in pre-order: the source, then each child followed by that child's
/// own subtree. A tab sits one level under the nearest tab on the current ancestor path
/// that owns its nearest open source. An unrelated tab, a section boundary, or a pin
/// boundary ends the family above it, which is why reorders move whole subtrees
/// ([`subtree`], [`lineage_step`]) rather than single tabs.
pub(crate) fn lineage_depths(tabs: &[TabLineage]) -> Vec<usize> {
    let open: HashSet<&PaneUuid> = tabs.iter().flat_map(|tab| &tab.pane_uuids).collect();
    // `path[d]` is the tab at depth `d` on the way down to the previous tab.
    let mut path: Vec<&TabLineage> = Vec::new();
    tabs.iter()
        .map(|tab| {
            let same_region = path
                .first()
                .is_some_and(|root| root.group_id == tab.group_id && root.pinned == tab.pinned);
            let depth = open_source(tab, &open)
                .filter(|_| same_region)
                .and_then(|parent| {
                    path.iter()
                        .rposition(|ancestor| ancestor.pane_uuids.contains(parent))
                })
                .map_or(0, |parent_depth| parent_depth + 1);
            path.truncate(depth);
            path.push(tab);
            depth
        })
        .collect()
}

/// `index` and every tab nested under it: the unit that moves when `index` moves.
pub(crate) fn subtree(depths: &[usize], index: usize) -> Range<usize> {
    let Some(&depth) = depths.get(index) else {
        return index..index;
    };
    let descendants = depths[index + 1..]
        .iter()
        .take_while(|nested| **nested > depth)
        .count();
    index..index + 1 + descendants
}

/// The slot after the top-level family containing `index`. A tab unrelated to that
/// family lands here, so it never splits a source from its forks.
pub(crate) fn index_after_family(depths: &[usize], index: usize) -> usize {
    let Some(after) = depths.get(index + 1..) else {
        return depths.len();
    };
    index + 1 + after.iter().take_while(|depth| **depth > 0).count()
}

/// Where a new fork of the tab at `parent_index` goes: right after the parent's own
/// subtree, so it nests one level under its source, ahead of the source's later siblings.
pub(crate) fn index_for_child(depths: &[usize], parent_index: usize) -> usize {
    subtree(depths, parent_index).end
}

/// One reorder step: `moving` trades places with the adjacent `neighbor` run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LineageStep {
    pub moving: Range<usize>,
    pub neighbor: Range<usize>,
}

impl LineageStep {
    /// Where `moving` starts once the step is applied, as an index into the list with
    /// `moving` removed. Pass to `Workspace::move_tab_run`.
    pub(crate) fn destination(&self) -> usize {
        if self.neighbor.start < self.moving.start {
            self.neighbor.start
        } else {
            self.neighbor.end - self.moving.len()
        }
    }
}

/// The step that moves the subtree at `index` past its neighbouring unit: the previous
/// or next family at the top level, or the previous or next sibling subtree inside a
/// family. `None` at the end of the list, and for a fork at the edge of its source's
/// subtree: a fork is never dragged above its source or out past the source's last
/// descendant, so it keeps its place in the family.
pub(crate) fn lineage_step(depths: &[usize], index: usize, forward: bool) -> Option<LineageStep> {
    let depth = *depths.get(index)?;
    let moving = subtree(depths, index);
    let neighbor_start = if forward {
        moving.end
    } else {
        // Walk back past the previous sibling's descendants. A shallower tab is the source.
        depths[..index]
            .iter()
            .rposition(|earlier| *earlier <= depth)?
    };
    (depths.get(neighbor_start) == Some(&depth)).then(|| LineageStep {
        neighbor: subtree(depths, neighbor_start),
        moving,
    })
}

/// The list order that puts every family in pre-order: each tab right under its own
/// source's subtree, siblings in their current order. Builds before nested forks placed
/// a fork of a fork after its source's later siblings, so restore runs this once.
pub(crate) fn preorder(tabs: &[TabLineage]) -> Vec<usize> {
    let open: HashSet<&PaneUuid> = tabs.iter().flat_map(|tab| &tab.pane_uuids).collect();
    let mut order = Vec::with_capacity(tabs.len());
    let mut start = 0;
    while start < tabs.len() {
        // A family as those builds drew it: a contiguous run in one region where each
        // tab's source is an earlier member.
        let root = &tabs[start];
        let mut owner: HashMap<&PaneUuid, usize> =
            root.pane_uuids.iter().map(|pane| (pane, start)).collect();
        let mut children: Vec<Vec<usize>> = vec![Vec::new()];
        let mut end = start + 1;
        while let Some(tab) = tabs.get(end) {
            let Some(parent) = open_source(tab, &open)
                .filter(|_| tab.group_id == root.group_id && tab.pinned == root.pinned)
                .and_then(|source| owner.get(source).copied())
            else {
                break;
            };
            children[parent - start].push(end);
            children.push(Vec::new());
            owner.extend(tab.pane_uuids.iter().map(|pane| (pane, end)));
            end += 1;
        }
        let mut stack = vec![start];
        while let Some(index) = stack.pop() {
            order.push(index);
            stack.extend(children[index - start].iter().rev());
        }
        start = end;
    }
    order
}

#[cfg(test)]
#[path = "lineage_tests.rs"]
mod tests;
