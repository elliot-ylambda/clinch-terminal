use super::{
    active_index_after_removal, close_project_decision, next_project_index, previous_project_index,
    CloseProjectDecision,
};

#[test]
fn project_navigation_wraps_and_singletons_are_noops() {
    assert_eq!(previous_project_index(0, 3), Some(2));
    assert_eq!(previous_project_index(2, 3), Some(1));
    assert_eq!(next_project_index(2, 3), Some(0));
    assert_eq!(next_project_index(0, 3), Some(1));

    assert_eq!(previous_project_index(0, 1), None);
    assert_eq!(next_project_index(0, 1), None);
}

#[test]
fn closing_active_project_prefers_the_project_at_the_same_position() {
    assert_eq!(active_index_after_removal(1, 1, 2), Some(1));
    assert_eq!(active_index_after_removal(2, 2, 2), Some(1));
}

#[test]
fn removing_inactive_project_preserves_active_project_identity() {
    assert_eq!(active_index_after_removal(2, 0, 2), Some(1));
    assert_eq!(active_index_after_removal(0, 2, 2), Some(0));
    assert_eq!(active_index_after_removal(0, 0, 0), None);
}

#[test]
fn close_project_guard_distinguishes_missing_singleton_and_grouped_projects() {
    assert_eq!(
        close_project_decision(2, None),
        CloseProjectDecision::NotFound
    );
    assert_eq!(
        close_project_decision(1, Some(0)),
        CloseProjectDecision::CloseWindow
    );
    assert_eq!(
        close_project_decision(3, Some(1)),
        CloseProjectDecision::Project(1)
    );
}
