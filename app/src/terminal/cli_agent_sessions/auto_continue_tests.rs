//! Tests for the pure [`PaneAutoContinue`] state machine. All transitions
//! take injected timestamps, so no real timers are involved: "the timer
//! fired" is simulated by calling `take_fire` with the armed generation.

use chrono::{DateTime, Duration, TimeZone, Utc};

use super::{ArmedAutoContinue, PaneAutoContinue, AUTO_CONTINUE_RESET_SLACK_SECS};

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 11, 12, 0, 0).unwrap()
}

/// Arms a fresh, enabled pane with a reset one hour out and returns
/// `(pane, armed)`.
fn armed_pane() -> (PaneAutoContinue, ArmedAutoContinue) {
    let mut pane = PaneAutoContinue::default();
    assert!(pane.set_enabled(true));
    let armed = pane
        .on_claude_session_stopped(Some("sess-1"), Some(now() + Duration::hours(1)), now())
        .expect("arms when enabled, exhausted, and reset is known")
        .clone();
    (pane, armed)
}

#[test]
fn arms_with_slack_and_session_id_when_stopped_while_exhausted() {
    let (pane, armed) = armed_pane();
    assert_eq!(
        armed.fire_at,
        now() + Duration::hours(1) + Duration::seconds(AUTO_CONTINUE_RESET_SLACK_SECS)
    );
    assert_eq!(armed.session_id, "sess-1");
    assert!(pane.is_enabled());
    assert_eq!(pane.armed(), Some(&armed));
}

#[test]
fn disabled_pane_never_arms() {
    let mut pane = PaneAutoContinue::default();
    assert!(pane
        .on_claude_session_stopped(Some("sess-1"), Some(now() + Duration::hours(1)), now())
        .is_none());
    assert!(pane.armed().is_none());
}

#[test]
fn unknown_reset_time_never_arms() {
    let mut pane = PaneAutoContinue::default();
    pane.set_enabled(true);
    assert!(pane
        .on_claude_session_stopped(Some("sess-1"), None, now())
        .is_none());
    assert!(pane.armed().is_none());
}

#[test]
fn missing_session_id_never_arms() {
    let mut pane = PaneAutoContinue::default();
    pane.set_enabled(true);
    assert!(pane
        .on_claude_session_stopped(None, Some(now() + Duration::hours(1)), now())
        .is_none());
    assert!(pane.armed().is_none());
}

#[test]
fn stale_past_reset_time_never_arms() {
    let mut pane = PaneAutoContinue::default();
    pane.set_enabled(true);
    // Reset in the past (or exactly now) means the usage data is stale.
    assert!(pane
        .on_claude_session_stopped(Some("sess-1"), Some(now() - Duration::minutes(5)), now())
        .is_none());
    assert!(pane
        .on_claude_session_stopped(Some("sess-1"), Some(now()), now())
        .is_none());
    assert!(pane.armed().is_none());
}

#[test]
fn fires_exactly_once_per_arm() {
    let (mut pane, armed) = armed_pane();
    let fired = pane
        .take_fire(armed.generation)
        .expect("first fire consumes the arm");
    assert_eq!(fired, armed);
    // The arm was consumed: the same timer (or a duplicate) can never fire again.
    assert!(pane.take_fire(armed.generation).is_none());
    assert!(pane.armed().is_none());
}

#[test]
fn user_activity_disarms_and_invalidates_the_scheduled_timer() {
    let (mut pane, armed) = armed_pane();
    assert!(pane.disarm());
    assert!(pane.armed().is_none());
    // The timer scheduled for the old arm wakes later: its generation no
    // longer matches, so nothing fires.
    assert!(pane.take_fire(armed.generation).is_none());
    // Disarming again reports nothing to cancel.
    assert!(!pane.disarm());
}

#[test]
fn stale_timer_generation_cannot_fire_a_newer_arm() {
    let (mut pane, first) = armed_pane();
    // User activity cancels, then a later stop re-arms.
    pane.disarm();
    let second = pane
        .on_claude_session_stopped(Some("sess-1"), Some(now() + Duration::hours(2)), now())
        .expect("re-arms after a fresh stop")
        .clone();
    assert_ne!(first.generation, second.generation);
    // The first (cancelled) timer wakes: must not consume the second arm.
    assert!(pane.take_fire(first.generation).is_none());
    assert_eq!(pane.armed(), Some(&second));
    // The second timer fires normally.
    assert_eq!(pane.take_fire(second.generation), Some(second));
}

#[test]
fn a_second_stop_while_armed_keeps_the_first_arm() {
    let (mut pane, first) = armed_pane();
    assert!(pane
        .on_claude_session_stopped(Some("sess-2"), Some(now() + Duration::hours(3)), now())
        .is_none());
    assert_eq!(pane.armed(), Some(&first));
}

#[test]
fn disabling_disarms_and_blocks_pending_fires() {
    let (mut pane, armed) = armed_pane();
    assert!(pane.set_enabled(false));
    assert!(pane.armed().is_none());
    assert!(pane.take_fire(armed.generation).is_none());
    // Re-enabling does not resurrect the old arm.
    assert!(pane.set_enabled(true));
    assert!(pane.armed().is_none());
    // Setting the same value again reports no change.
    assert!(!pane.set_enabled(true));
}
