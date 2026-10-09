use super::*;

fn tab(id: u8, parent: Option<u8>) -> TabLineage {
    TabLineage {
        pane_uuids: vec![PaneUuid(vec![id])],
        parent: parent.map(|id| PaneUuid(vec![id])),
        group_id: None,
        pinned: false,
    }
}

#[test]
fn adjacent_children_and_grandchildren_share_one_level() {
    let tabs = [
        tab(1, None),
        tab(2, Some(1)),
        tab(3, Some(2)),
        tab(4, Some(1)),
        tab(5, None),
    ];
    let nesting = lineage_nesting(&tabs);
    assert_eq!(nesting, [false, true, true, true, false]);
    assert_eq!(index_after_family(&nesting, 0), 4);
    assert_eq!(index_after_family(&nesting, 1), 4);
}

#[test]
fn unrelated_tabs_and_missing_parents_break_families() {
    assert_eq!(
        lineage_nesting(&[tab(1, None), tab(2, None), tab(3, Some(1)), tab(4, Some(3))]),
        [false, false, false, true]
    );
    assert_eq!(
        lineage_nesting(&[tab(2, Some(1)), tab(3, Some(1))]),
        [false, false]
    );
}

#[test]
fn section_and_pin_boundaries_break_families() {
    let mut tabs = [tab(1, None), tab(2, Some(1))];
    tabs[0].pinned = true;
    assert_eq!(lineage_nesting(&tabs), [false, false]);
    tabs[0].pinned = false;
    tabs[1].group_id = Some(TabGroupId::new());
    assert_eq!(lineage_nesting(&tabs), [false, false]);
    tabs[0].group_id = tabs[1].group_id;
    assert_eq!(lineage_nesting(&tabs), [false, true]);
}

#[test]
fn source_can_be_any_pane_in_a_split_tab() {
    let mut parent = tab(1, None);
    parent.pane_uuids.push(PaneUuid(vec![2]));
    assert_eq!(lineage_nesting(&[parent, tab(3, Some(2))]), [false, true]);
}

#[test]
fn origin_tooltip_uses_live_title_or_closed_snapshot() {
    let mut origin = TabOrigin {
        kind: TabOriginKind::Fork,
        parent_pane_uuid: PaneUuid(vec![1]),
        parent_title: "Original".into(),
    };
    assert_eq!(origin.tooltip(Some("Renamed")), "Forked from \"Renamed\"");
    assert_eq!(origin.tooltip(None), "Forked from \"Original\" (closed)");
    origin.kind = TabOriginKind::Transfer {
        from: CLIAgent::Claude,
    };
    assert_eq!(
        origin.tooltip(None),
        "Transferred from Claude Code · \"Original\" (closed)"
    );
}
