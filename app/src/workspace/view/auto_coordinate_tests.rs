use std::time::Duration;

use instant::Instant;
use warpui::EntityId;

use super::{
    wakeup_text, SessionUpdate, Tactic, UpdateKind, WakePlanner, AUTO_COORDINATE_MARKER,
    BATCH_WINDOW, MIN_WAKE_INTERVAL,
};

fn update(session: usize, title: &str, kind: UpdateKind) -> SessionUpdate {
    SessionUpdate {
        session: EntityId::from_usize(session),
        title: title.to_owned(),
        kind,
    }
}

#[test]
fn updates_wait_for_the_batch_window_to_close() {
    let start = Instant::now();
    let mut planner = WakePlanner::default();
    planner.record(update(1, "Fix restore", UpdateKind::FinishedTurn), start);
    planner.record(
        update(2, "Footer editor", UpdateKind::NeedsYou),
        start + Duration::from_secs(2),
    );

    assert_eq!(planner.due_at(), Some(start + BATCH_WINDOW));
    assert!(planner.take_due(start + Duration::from_secs(4)).is_none());
    let updates = planner.take_due(start + BATCH_WINDOW).expect("due");
    assert_eq!(updates.len(), 2);
    assert!(!planner.has_pending());
}

#[test]
fn a_session_appears_once_with_its_latest_change() {
    let start = Instant::now();
    let mut planner = WakePlanner::default();
    planner.record(update(1, "Fix restore", UpdateKind::NeedsYou), start);
    planner.record(update(1, "Fix restore", UpdateKind::FinishedTurn), start);

    let updates = planner.take_due(start + BATCH_WINDOW).expect("due");
    assert_eq!(
        updates,
        vec![update(1, "Fix restore", UpdateKind::FinishedTurn)]
    );
}

#[test]
fn wakeups_are_spaced_by_the_minimum_interval() {
    let start = Instant::now();
    let mut planner = WakePlanner::default();
    planner.mark_woken(start);
    planner.record(
        update(1, "Fix restore", UpdateKind::FinishedTurn),
        start + Duration::from_secs(10),
    );

    assert_eq!(planner.due_at(), Some(start + MIN_WAKE_INTERVAL));
    assert!(planner
        .take_due(start + MIN_WAKE_INTERVAL - Duration::from_secs(1))
        .is_none());
    assert!(planner.take_due(start + MIN_WAKE_INTERVAL).is_some());
}

#[test]
fn held_updates_stay_due_and_newer_ones_win() {
    let start = Instant::now();
    let mut planner = WakePlanner::default();
    planner.record(update(1, "Fix restore", UpdateKind::NeedsYou), start);
    let taken = planner.take_due(start + BATCH_WINDOW).expect("due");

    // While the coordinator was busy, the same session finished its turn.
    let later = start + BATCH_WINDOW + Duration::from_secs(1);
    planner.record(update(1, "Fix restore", UpdateKind::FinishedTurn), later);
    planner.hold(taken, later);

    let retried = planner.take_due(later).expect("held updates stay due");
    assert_eq!(
        retried,
        vec![update(1, "Fix restore", UpdateKind::FinishedTurn)]
    );
}

#[test]
fn a_requested_run_takes_everything_at_once() {
    let start = Instant::now();
    let mut planner = WakePlanner::default();
    planner.mark_woken(start);
    planner.record(update(1, "Fix restore", UpdateKind::FinishedTurn), start);

    assert_eq!(planner.take_all().len(), 1);
    assert!(!planner.has_pending());
    assert_eq!(planner.due_at(), None);
}

#[test]
fn clearing_drops_everything_pending() {
    let start = Instant::now();
    let mut planner = WakePlanner::default();
    planner.record(update(1, "Fix restore", UpdateKind::FinishedTurn), start);
    planner.clear();
    assert_eq!(planner.due_at(), None);
    assert!(planner.take_due(start + BATCH_WINDOW).is_none());
}

#[test]
fn wakeup_names_the_changes_and_the_tactics() {
    let text = wakeup_text(
        &[
            update(1, "Fix restore flag", UpdateKind::FinishedTurn),
            update(2, "Footer editor", UpdateKind::NeedsYou),
        ],
        &[Tactic::Watch, Tactic::LineUpShipping],
        false,
    );
    assert!(text.starts_with(AUTO_COORDINATE_MARKER));
    assert!(text.contains(
        "2 updates: \"Fix restore flag\" finished a turn; \"Footer editor\" is waiting on the user."
    ));
    assert!(text.contains("Tactics on: watch, line up shipping (ask first)."));
    assert!(!text.contains("Run requested"));
}

#[test]
fn wakeup_titles_cannot_break_the_line() {
    let long = "x".repeat(80);
    let text = wakeup_text(
        &[
            update(1, "multi\nline \"quoted\"\ttitle", UpdateKind::FinishedTurn),
            update(2, &long, UpdateKind::FinishedTurn),
            update(3, "   ", UpdateKind::FinishedTurn),
        ],
        &[Tactic::Watch],
        false,
    );
    assert!(!text.chars().any(|ch| ch.is_control()));
    assert!(text.contains("\"multi line 'quoted' title\""));
    assert!(text.contains(&format!("\"{}…\"", "x".repeat(60))));
    assert!(text.contains("\"Untitled session\""));
}

#[test]
fn a_requested_run_says_so_even_with_nothing_new() {
    let text = wakeup_text(&[], &[Tactic::NameSessions], true);
    assert!(text.starts_with(&format!("{AUTO_COORDINATE_MARKER} Run requested.")));
    assert!(!text.contains("update"));
}

/// The coordinator learns what a wakeup means from the managed skill, so the skill must name the
/// marker and every tactic exactly as the app sends them.
#[test]
fn the_coordinate_skill_explains_every_wakeup() {
    let skill = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../resources/bundled/agent-skills/clinch-coordinate/SKILL.md"
    ))
    .expect("bundled clinch-coordinate SKILL.md must exist");
    assert!(skill.contains(&format!("`{AUTO_COORDINATE_MARKER}`")));
    assert!(skill.contains("tab rename --auto"));
    for tactic in Tactic::ALL {
        assert!(
            skill.contains(&format!("**{}**", tactic.wakeup_name())),
            "the skill does not explain the {tactic:?} tactic"
        );
    }
}
