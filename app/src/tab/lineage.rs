use std::collections::HashSet;

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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TabOriginKind {
    Fork,
    Transfer { from: CLIAgent },
}

impl TabOrigin {
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
    pub parent: Option<PaneUuid>,
    pub group_id: Option<TabGroupId>,
    pub pinned: bool,
}

impl TabLineage {
    pub(crate) fn for_tab(tab: &TabData, app: &AppContext) -> Self {
        Self {
            pane_uuids: tab.pane_group.as_ref(app).terminal_pane_uuids(),
            parent: tab
                .origin
                .as_ref()
                .map(|origin| origin.parent_pane_uuid.clone()),
            group_id: tab.group_id,
            pinned: FeatureFlag::PinnedTabs.is_enabled() && tab.pinned,
        }
    }
}

/// A contiguous family is rendered at one level of indentation. Reordering never
/// moves related tabs automatically: only the current visible adjacency matters.
pub(crate) fn lineage_nesting(tabs: &[TabLineage]) -> Vec<bool> {
    let mut family = HashSet::new();
    let mut group_id = None;
    let mut pinned = false;
    tabs.iter()
        .map(|tab| {
            let nested = tab.group_id == group_id
                && tab.pinned == pinned
                && tab
                    .parent
                    .as_ref()
                    .is_some_and(|parent| family.contains(parent));
            if !nested {
                family.clear();
                group_id = tab.group_id;
                pinned = tab.pinned;
            }
            family.extend(tab.pane_uuids.iter().cloned());
            nested
        })
        .collect()
}

/// Insert after the source's current family, including siblings of a nested source.
pub(crate) fn index_after_family(nesting: &[bool], parent_index: usize) -> usize {
    parent_index
        + 1
        + nesting
            .iter()
            .skip(parent_index + 1)
            .take_while(|nested| **nested)
            .count()
}

#[cfg(test)]
#[path = "lineage_tests.rs"]
mod tests;
