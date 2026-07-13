use std::cell::Cell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::{Duration, TimeZone, Utc};

use super::*;
use crate::{LimitWindow, PlanLimits, Severity};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

fn temp_snapshot_path() -> PathBuf {
    let suffix = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "cli-agent-usage-plan-cache-{}-{suffix}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir.join("snapshot.json")
}

fn plan(percent: f64) -> PlanLimits {
    PlanLimits {
        session: Some(LimitWindow {
            percent,
            resets_at: None,
            severity: Severity::Normal,
        }),
        ..PlanLimits::default()
    }
}

#[test]
fn processes_share_one_attempt_per_wall_clock_minute() {
    let snapshot = temp_snapshot_path();
    let now = Utc.with_ymd_and_hms(2026, 7, 13, 18, 0, 0).unwrap();
    let calls = Cell::new(0);

    let first = refresh_shared(&snapshot, now, || {
        calls.set(calls.get() + 1);
        Some(plan(14.0))
    });
    let second = refresh_shared(&snapshot, now + Duration::seconds(30), || {
        calls.set(calls.get() + 1);
        Some(plan(99.0))
    });

    assert_eq!(calls.get(), 1);
    assert_eq!(first, Some(plan(14.0)));
    assert_eq!(second, Some(plan(14.0)));

    let _ = fs::remove_dir_all(snapshot.parent().unwrap());
}

#[test]
fn transient_failure_retains_last_good_and_advances_backoff() {
    let snapshot = temp_snapshot_path();
    let now = Utc.with_ymd_and_hms(2026, 7, 13, 18, 0, 0).unwrap();
    assert_eq!(
        refresh_shared(&snapshot, now, || Some(plan(55.0))),
        Some(plan(55.0))
    );

    let calls = Cell::new(0);
    assert_eq!(
        refresh_shared(&snapshot, now + Duration::seconds(61), || {
            calls.set(calls.get() + 1);
            None
        }),
        Some(plan(55.0))
    );
    assert_eq!(
        refresh_shared(&snapshot, now + Duration::seconds(90), || {
            calls.set(calls.get() + 1);
            Some(plan(99.0))
        }),
        Some(plan(55.0))
    );
    assert_eq!(calls.get(), 1);

    let _ = fs::remove_dir_all(snapshot.parent().unwrap());
}

#[test]
fn failed_first_attempt_is_shared_without_fabricating_a_plan() {
    let snapshot = temp_snapshot_path();
    let now = Utc.with_ymd_and_hms(2026, 7, 13, 18, 0, 0).unwrap();
    let calls = Cell::new(0);

    assert_eq!(
        refresh_shared(&snapshot, now, || {
            calls.set(calls.get() + 1);
            None
        }),
        None
    );
    assert_eq!(
        refresh_shared(&snapshot, now + Duration::seconds(30), || {
            calls.set(calls.get() + 1);
            Some(plan(14.0))
        }),
        None
    );
    assert_eq!(calls.get(), 1);

    let _ = fs::remove_dir_all(snapshot.parent().unwrap());
}
