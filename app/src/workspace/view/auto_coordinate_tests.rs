use std::time::Duration;

use instant::Instant;
use warpui::EntityId;

use super::{
    kickoff_text, pause_text, wakeup_text, AutoCoordinateStatus, RunKind, SessionUpdate, Tactic,
    UpdateKind, WakePlanner, AUTO_COORDINATE_MARKER, BATCH_WINDOW, LOOP_INTERVAL,
    MIN_WAKE_INTERVAL,
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
        None,
        &[],
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
        None,
        &[],
    );
    assert!(!text.chars().any(|ch| ch.is_control()));
    assert!(text.contains("\"multi line 'quoted' title\""));
    assert!(text.contains(&format!("\"{}…\"", "x".repeat(60))));
    assert!(text.contains("\"Untitled session\""));
}

#[test]
fn a_requested_run_says_so_even_with_nothing_new() {
    let text = wakeup_text(&[], &[Tactic::NameSessions], Some(RunKind::Requested), &[]);
    assert!(text.starts_with(&format!("{AUTO_COORDINATE_MARKER} Run requested.")));
    assert!(!text.contains("update"));
}

#[test]
fn switching_auto_back_on_asks_it_to_pick_up_again() {
    let text = wakeup_text(
        &[update(1, "Fix restore", UpdateKind::FinishedTurn)],
        &[Tactic::Watch],
        Some(RunKind::Resumed),
        &[],
    );
    assert!(text.starts_with(&format!(
        "{AUTO_COORDINATE_MARKER} Auto-coordinate is back on: restart the loops you paused, then \
         check every session."
    )));
    assert!(text.contains("1 update: \"Fix restore\" finished a turn."));
}

#[test]
fn a_routine_check_names_the_sessions_still_working() {
    let working = ["Billing fix".to_owned(), "multi\nline".to_owned()];
    let text = wakeup_text(&[], &[Tactic::Watch], Some(RunKind::RoutineCheck), &working);
    assert!(text.starts_with(&format!(
        "{AUTO_COORDINATE_MARKER} Routine check: 2 sessions are still working: \"Billing fix\", \
         \"multi line\"."
    )));
    let one = wakeup_text(
        &[],
        &[Tactic::Watch],
        Some(RunKind::RoutineCheck),
        &working[..1],
    );
    assert!(one.contains("Routine check: 1 session is still working: \"Billing fix\"."));
}

#[test]
fn a_resume_or_run_now_covers_a_routine_check_held_with_it() {
    assert_eq!(
        Some(RunKind::RoutineCheck).max(Some(RunKind::Requested)),
        Some(RunKind::Requested)
    );
    assert_eq!(
        Some(RunKind::Resumed).max(Some(RunKind::Requested)),
        Some(RunKind::Resumed)
    );
    assert_eq!(
        None.max(Some(RunKind::RoutineCheck)),
        Some(RunKind::RoutineCheck)
    );
}

#[test]
fn the_loop_checks_in_a_full_interval_after_the_last_wakeup() {
    let start = Instant::now();
    let mut planner = WakePlanner::default();
    // Never woken: a routine check may go out at the first tick.
    assert_eq!(planner.routine_check_in(start), Duration::ZERO);
    planner.mark_woken(start);
    assert_eq!(planner.routine_check_in(start), LOOP_INTERVAL);
    let later = start + Duration::from_secs(60);
    assert_eq!(
        planner.routine_check_in(later),
        LOOP_INTERVAL - Duration::from_secs(60)
    );
    assert_eq!(
        planner.routine_check_in(start + LOOP_INTERVAL),
        Duration::ZERO
    );
    // Clearing pending updates keeps the clock.
    planner.clear();
    assert_eq!(
        planner.routine_check_in(later),
        LOOP_INTERVAL - Duration::from_secs(60)
    );
}

#[test]
fn pausing_asks_the_coordinator_to_pause_all_loops() {
    let text = pause_text();
    assert!(text.starts_with(&format!(
        "{AUTO_COORDINATE_MARKER} Paused. Pause all loops:"
    )));
    assert!(text.contains("don't message, rename or reorganize sessions on your own"));
    assert!(!text.chars().any(|ch| ch.is_control()));
    // Typed into the agent like a wakeup, so it stays out of history and tab topics.
    assert!(crate::agent_resume::is_injected_prompt(&text));
}

#[test]
fn a_coordinator_still_starting_says_so_rather_than_mid_turn() {
    for reason in ["not_started", "starting", "unknown_conversation"] {
        assert_eq!(
            AutoCoordinateStatus::held_by(reason),
            AutoCoordinateStatus::WaitingForAgent,
            "{reason}"
        );
    }
    assert_eq!(
        AutoCoordinateStatus::held_by("working"),
        AutoCoordinateStatus::WaitingForCoordinator
    );
    assert_eq!(
        AutoCoordinateStatus::held_by("no_agent"),
        AutoCoordinateStatus::CoordinatorUnavailable
    );
}

#[test]
fn the_first_message_lists_the_tactics_and_is_the_users_own() {
    assert_eq!(kickoff_text(&[]), None);
    let text = kickoff_text(&[Tactic::NameSessions, Tactic::CleanUp]).unwrap();
    assert!(text.starts_with(
        "Start auto-coordinating this project with these tactics: name sessions, clean up (ask \
         first)."
    ));
    // The user sends it, so it counts as their first prompt and is never a slash command.
    assert!(!crate::agent_resume::is_injected_prompt(&text));
    assert!(!text.starts_with('/'));
    // Clinch runs the loop; naming one would invite the agent to start its own.
    assert!(!text.to_lowercase().contains("loop"));
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
    // Markdown wraps lines anywhere, so phrases are matched with whitespace collapsed.
    let flowing = skill.split_whitespace().collect::<Vec<_>>().join(" ");
    for phrase in [
        "Start auto-coordinating this project",
        "Routine check",
        "Pause all loops",
        "Auto-coordinate is back on",
    ] {
        assert!(
            flowing.contains(phrase),
            "the skill does not explain {phrase:?}"
        );
    }
    assert!(kickoff_text(&Tactic::ALL)
        .unwrap()
        .starts_with("Start auto-coordinating this project"));
    assert!(pause_text().contains("Pause all loops"));
    for tactic in Tactic::ALL {
        assert!(
            skill.contains(&format!("**{}**", tactic.wakeup_name())),
            "the skill does not explain the {tactic:?} tactic"
        );
    }
}
