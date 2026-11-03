use super::*;

fn tab(id: u8, parent: Option<u8>) -> TabLineage {
    TabLineage {
        pane_uuids: vec![PaneUuid(vec![id])],
        sources: parent.map(|id| PaneUuid(vec![id])).into_iter().collect(),
        group_id: None,
        pinned: false,
    }
}

/// A tab whose sources are `sources`, nearest first.
fn tab_from(id: u8, sources: &[u8]) -> TabLineage {
    TabLineage {
        sources: sources.iter().map(|id| PaneUuid(vec![*id])).collect(),
        ..tab(id, None)
    }
}

fn ids(tabs: &[TabLineage]) -> Vec<u8> {
    tabs.iter().map(|tab| tab.pane_uuids[0].0[0]).collect()
}

/// Applies `step` the way `Workspace::move_tab_run` does.
fn apply(tabs: &mut Vec<TabLineage>, step: &LineageStep) {
    let moved: Vec<_> = tabs.drain(step.moving.clone()).collect();
    let at = step.destination();
    tabs.splice(at..at, moved);
}

#[test]
fn fork_of_a_fork_nests_one_level_deeper() {
    let tabs = [
        tab(1, None),
        tab(2, Some(1)),
        tab(3, Some(2)),
        tab(4, Some(1)),
        tab(5, None),
    ];
    let depths = lineage_depths(&tabs);
    assert_eq!(depths, [0, 1, 2, 1, 0]);
    assert_eq!(subtree(&depths, 0), 0..4);
    assert_eq!(subtree(&depths, 1), 1..3);
    assert_eq!(subtree(&depths, 4), 4..5);
    // A new fork goes under its own source; an unrelated tab goes after the family.
    assert_eq!(index_for_child(&depths, 1), 3);
    assert_eq!(index_for_child(&depths, 0), 4);
    assert_eq!(index_after_family(&depths, 1), 4);
    assert_eq!(index_after_family(&depths, 0), 4);
    assert_eq!(index_after_family(&depths, 9), 5);
}

#[test]
fn unrelated_tabs_and_missing_parents_break_families() {
    assert_eq!(
        lineage_depths(&[tab(1, None), tab(2, None), tab(3, Some(1)), tab(4, Some(3))]),
        [0, 0, 0, 1]
    );
    assert_eq!(lineage_depths(&[tab(2, Some(1)), tab(3, Some(1))]), [0, 0]);
    // A grandchild that is not directly under its own source's subtree is never drawn
    // under the wrong parent.
    assert_eq!(
        lineage_depths(&[
            tab(1, None),
            tab(2, Some(1)),
            tab(4, Some(1)),
            tab(3, Some(2))
        ]),
        [0, 1, 1, 0]
    );
}

#[test]
fn section_and_pin_boundaries_break_families() {
    let mut tabs = [tab(1, None), tab(2, Some(1))];
    tabs[0].pinned = true;
    assert_eq!(lineage_depths(&tabs), [0, 0]);
    tabs[0].pinned = false;
    tabs[1].group_id = Some(TabGroupId::new());
    assert_eq!(lineage_depths(&tabs), [0, 0]);
    tabs[0].group_id = tabs[1].group_id;
    assert_eq!(lineage_depths(&tabs), [0, 1]);
}

#[test]
fn source_can_be_any_pane_in_a_split_tab() {
    let mut parent = tab(1, None);
    parent.pane_uuids.push(PaneUuid(vec![2]));
    assert_eq!(lineage_depths(&[parent, tab(3, Some(2))]), [0, 1]);
}

#[test]
fn steps_trade_whole_families_and_sibling_subtrees() {
    // 1 { 2 { 3 }, 4 }, 5 { 6 }
    let depths = [0, 1, 2, 1, 0, 1];
    let step = lineage_step(&depths, 0, true).unwrap();
    assert_eq!((step.moving.clone(), step.neighbor.clone()), (0..4, 4..6));
    assert_eq!(step.destination(), 2);
    let step = lineage_step(&depths, 4, false).unwrap();
    assert_eq!((step.moving.clone(), step.neighbor.clone()), (4..6, 0..4));
    assert_eq!(step.destination(), 0);
    // Siblings inside a family trade places with their subtrees.
    let step = lineage_step(&depths, 1, true).unwrap();
    assert_eq!((step.moving.clone(), step.neighbor.clone()), (1..3, 3..4));
    assert_eq!(step.destination(), 2);
    let step = lineage_step(&depths, 3, false).unwrap();
    assert_eq!((step.moving.clone(), step.neighbor.clone()), (3..4, 1..3));
    assert_eq!(step.destination(), 1);
}

#[test]
fn forks_cannot_be_stepped_out_of_their_source() {
    let depths = [0, 1, 2, 1, 0, 1];
    // Above the source.
    assert_eq!(lineage_step(&depths, 1, false), None);
    assert_eq!(lineage_step(&depths, 2, false), None);
    // Past the source's last descendant.
    assert_eq!(lineage_step(&depths, 2, true), None);
    assert_eq!(lineage_step(&depths, 3, true), None);
    // List ends.
    assert_eq!(lineage_step(&depths, 0, false), None);
    assert_eq!(lineage_step(&depths, 5, true), None);
}

#[test]
fn reordering_keeps_every_fork_under_its_source() {
    // The reported bug: moving a source, or moving an unrelated tab past a family,
    // used to strand the forks without their indent.
    let mut tabs = vec![tab(1, None), tab(2, Some(1)), tab(3, Some(2)), tab(4, None)];
    let step = lineage_step(&lineage_depths(&tabs), 0, true).unwrap();
    apply(&mut tabs, &step);
    assert_eq!(ids(&tabs), [4, 1, 2, 3]);
    assert_eq!(lineage_depths(&tabs), [0, 0, 1, 2]);

    let step = lineage_step(&lineage_depths(&tabs), 0, true).unwrap();
    apply(&mut tabs, &step);
    assert_eq!(ids(&tabs), [1, 2, 3, 4]);
    assert_eq!(lineage_depths(&tabs), [0, 1, 2, 0]);

    let step = lineage_step(&lineage_depths(&tabs), 3, false).unwrap();
    apply(&mut tabs, &step);
    assert_eq!(ids(&tabs), [4, 1, 2, 3]);
    assert_eq!(lineage_depths(&tabs), [0, 0, 1, 2]);
}

#[test]
fn any_sequence_of_steps_preserves_every_depth() {
    // 1 { 2 { 3 { 4 } }, 5 }, 6, 7 { 8, 9 { 10 } }
    let mut tabs = vec![
        tab(1, None),
        tab(2, Some(1)),
        tab(3, Some(2)),
        tab(4, Some(3)),
        tab(5, Some(1)),
        tab(6, None),
        tab(7, None),
        tab(8, Some(7)),
        tab(9, Some(7)),
        tab(10, Some(9)),
    ];
    let depth_of = |tabs: &[TabLineage]| {
        let mut by_id: Vec<_> = ids(tabs).into_iter().zip(lineage_depths(tabs)).collect();
        by_id.sort();
        by_id
    };
    let expected = depth_of(&tabs);
    let mut seed = 0x2545_f491_u32;
    for _ in 0..500 {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let index = seed as usize % tabs.len();
        let forward = seed & 0x100 != 0;
        if let Some(step) = lineage_step(&lineage_depths(&tabs), index, forward) {
            apply(&mut tabs, &step);
        }
        assert_eq!(depth_of(&tabs), expected);
    }
}

#[test]
fn a_fork_whose_source_closed_takes_the_sources_place() {
    // 1 { 2 (closed) { 3 }, 4 }: 3 moves up under 1 and 4 stays in the family.
    assert_eq!(
        lineage_depths(&[tab(1, None), tab_from(3, &[2, 1]), tab(4, Some(1))]),
        [0, 1, 1]
    );
    // An open source elsewhere is not skipped for an ancestor that happens to be near.
    assert_eq!(
        lineage_depths(&[tab(1, None), tab_from(3, &[2, 1]), tab(2, Some(1))]),
        [0, 0, 0]
    );
}

#[test]
fn ancestry_records_the_sources_own_sources_nearest_first() {
    let source = TabOrigin {
        kind: TabOriginKind::Fork,
        parent_pane_uuid: PaneUuid(vec![2]),
        parent_title: "Source".into(),
        ancestors: (3..20).map(|id| PaneUuid(vec![id])).collect(),
    };
    let ancestry = TabOrigin::ancestry_from(Some(&source));
    assert_eq!(ancestry.len(), MAX_ANCESTORS);
    assert_eq!(ancestry[..2], [PaneUuid(vec![2]), PaneUuid(vec![3])]);
    assert!(TabOrigin::ancestry_from(None).is_empty());
}

#[test]
fn preorder_moves_a_legacy_fork_of_a_fork_under_its_own_source() {
    // Older builds appended 3 (a fork of 2) after 2's later sibling 4.
    let legacy = [
        tab(1, None),
        tab(2, Some(1)),
        tab(4, Some(1)),
        tab(3, Some(2)),
        tab(5, None),
    ];
    assert_eq!(preorder(&legacy), [0, 1, 3, 2, 4]);
    let tabs = [
        tab(1, None),
        tab(2, Some(1)),
        tab(3, Some(2)),
        tab(4, Some(1)),
        tab(5, None),
        tab(6, Some(5)),
    ];
    assert_eq!(preorder(&tabs), [0, 1, 2, 3, 4, 5]);
    // A fork outside its source's section is never pulled into the section.
    let group = Some(TabGroupId::new());
    let mut sectioned = legacy;
    for tab in &mut sectioned[..3] {
        tab.group_id = group;
    }
    assert_eq!(preorder(&sectioned), [0, 1, 2, 3, 4]);
}

#[test]
fn origin_tooltip_uses_live_title_or_closed_snapshot() {
    let mut origin = TabOrigin {
        kind: TabOriginKind::Fork,
        parent_pane_uuid: PaneUuid(vec![1]),
        parent_title: "Original".into(),
        ancestors: Vec::new(),
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
