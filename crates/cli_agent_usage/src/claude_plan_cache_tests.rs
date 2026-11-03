use std::cell::Cell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::{Duration, TimeZone, Utc};

use super::*;
use crate::{LimitWindow, PlanFetchOutcome, PlanLimits, Severity};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

const AFTER_RETRY: RefreshPolicy<'static> = RefreshPolicy {
    account: None,
    retry_empty: true,
    user_looking: false,
};

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
fn processes_share_one_attempt_per_five_minutes() {
    let snapshot = temp_snapshot_path();
    let now = Utc.with_ymd_and_hms(2026, 7, 13, 18, 0, 0).unwrap();
    let calls = Cell::new(0);

    let first = refresh_shared(&snapshot, now, RefreshPolicy::default(), || {
        calls.set(calls.get() + 1);
        PlanFetchOutcome::Success(plan(14.0))
    });
    let second = refresh_shared(
        &snapshot,
        now + Duration::seconds(30),
        RefreshPolicy::default(),
        || {
            calls.set(calls.get() + 1);
            PlanFetchOutcome::Success(plan(99.0))
        },
    );

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
        refresh_shared(&snapshot, now, RefreshPolicy::default(), || {
            PlanFetchOutcome::Success(plan(55.0))
        }),
        Some(plan(55.0))
    );

    let calls = Cell::new(0);
    assert_eq!(
        refresh_shared(
            &snapshot,
            now + Duration::seconds(301),
            RefreshPolicy::default(),
            || {
                calls.set(calls.get() + 1);
                PlanFetchOutcome::Unavailable
            }
        ),
        Some(plan(55.0))
    );
    assert_eq!(
        refresh_shared(
            &snapshot,
            now + Duration::seconds(400),
            RefreshPolicy::default(),
            || {
                calls.set(calls.get() + 1);
                PlanFetchOutcome::Success(plan(99.0))
            }
        ),
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
        refresh_shared(&snapshot, now, RefreshPolicy::default(), || {
            calls.set(calls.get() + 1);
            PlanFetchOutcome::Unavailable
        }),
        None
    );
    assert_eq!(
        refresh_shared(
            &snapshot,
            now + Duration::seconds(30),
            RefreshPolicy::default(),
            || {
                calls.set(calls.get() + 1);
                PlanFetchOutcome::Success(plan(14.0))
            }
        ),
        None
    );
    assert_eq!(calls.get(), 1);

    let _ = fs::remove_dir_all(snapshot.parent().unwrap());
}

#[test]
fn explicit_retry_retries_an_empty_throttled_attempt_immediately() {
    let snapshot = temp_snapshot_path();
    let now = Utc.with_ymd_and_hms(2026, 7, 13, 18, 0, 0).unwrap();
    assert_eq!(
        refresh_shared(&snapshot, now, RefreshPolicy::default(), || {
            PlanFetchOutcome::Unavailable
        }),
        None
    );

    let calls = Cell::new(0);
    assert_eq!(
        refresh_shared(&snapshot, now + Duration::seconds(30), AFTER_RETRY, || {
            calls.set(calls.get() + 1);
            PlanFetchOutcome::Success(plan(14.0))
        }),
        Some(plan(14.0))
    );
    assert_eq!(calls.get(), 1);

    let _ = fs::remove_dir_all(snapshot.parent().unwrap());
}

#[test]
fn explicit_retry_does_not_bypass_retry_after() {
    let snapshot = temp_snapshot_path();
    let now = Utc.with_ymd_and_hms(2026, 7, 13, 18, 0, 0).unwrap();
    assert_eq!(
        refresh_shared(&snapshot, now, RefreshPolicy::default(), || {
            PlanFetchOutcome::RateLimited(std::time::Duration::from_secs(2_100))
        }),
        None
    );

    let calls = Cell::new(0);
    assert_eq!(
        refresh_shared(&snapshot, now + Duration::seconds(30), AFTER_RETRY, || {
            calls.set(calls.get() + 1);
            PlanFetchOutcome::Success(plan(14.0))
        }),
        None
    );
    assert_eq!(calls.get(), 0, "must still honor the server Retry-After");

    let _ = fs::remove_dir_all(snapshot.parent().unwrap());
}

#[test]
fn rate_limit_retry_after_is_shared_and_honored() {
    let snapshot = temp_snapshot_path();
    let now = Utc.with_ymd_and_hms(2026, 7, 13, 18, 0, 0).unwrap();
    assert_eq!(
        refresh_shared(&snapshot, now, RefreshPolicy::default(), || {
            PlanFetchOutcome::Success(plan(49.0))
        }),
        Some(plan(49.0))
    );

    let calls = Cell::new(0);
    assert_eq!(
        refresh_shared(
            &snapshot,
            now + Duration::seconds(301),
            RefreshPolicy::default(),
            || {
                calls.set(calls.get() + 1);
                PlanFetchOutcome::RateLimited(std::time::Duration::from_secs(2_100))
            }
        ),
        Some(plan(49.0))
    );
    assert_eq!(
        refresh_shared(
            &snapshot,
            now + Duration::seconds(2_400),
            RefreshPolicy::default(),
            || {
                calls.set(calls.get() + 1);
                PlanFetchOutcome::Success(plan(100.0))
            }
        ),
        Some(plan(49.0))
    );
    assert_eq!(calls.get(), 1, "must not fetch before Retry-After");

    assert_eq!(
        refresh_shared(
            &snapshot,
            now + Duration::seconds(2_402),
            RefreshPolicy::default(),
            || {
                calls.set(calls.get() + 1);
                PlanFetchOutcome::Success(plan(100.0))
            }
        ),
        Some(plan(100.0))
    );
    assert_eq!(calls.get(), 2);

    let _ = fs::remove_dir_all(snapshot.parent().unwrap());
}

#[test]
fn cached_window_is_removed_after_its_reset() {
    let snapshot = temp_snapshot_path();
    let now = Utc.with_ymd_and_hms(2026, 7, 13, 18, 0, 0).unwrap();
    let expiring = PlanLimits {
        session: Some(LimitWindow {
            percent: 49.0,
            resets_at: Some(now + Duration::seconds(10)),
            severity: Severity::Normal,
        }),
        ..PlanLimits::default()
    };
    assert_eq!(
        refresh_shared(&snapshot, now, RefreshPolicy::default(), || {
            PlanFetchOutcome::Success(expiring)
        }),
        Some(expiring)
    );

    let calls = Cell::new(0);
    assert_eq!(
        refresh_shared(
            &snapshot,
            now + Duration::seconds(30),
            RefreshPolicy::default(),
            || {
                calls.set(calls.get() + 1);
                PlanFetchOutcome::Success(plan(100.0))
            }
        ),
        None
    );
    assert_eq!(calls.get(), 0, "normal throttle still applies");

    let _ = fs::remove_dir_all(snapshot.parent().unwrap());
}

#[test]
fn plan_older_than_one_hour_is_not_retained() {
    let snapshot = temp_snapshot_path();
    let now = Utc.with_ymd_and_hms(2026, 7, 13, 18, 0, 0).unwrap();
    assert_eq!(
        refresh_shared(&snapshot, now, RefreshPolicy::default(), || {
            PlanFetchOutcome::Success(plan(49.0))
        }),
        Some(plan(49.0))
    );

    assert_eq!(
        refresh_shared(
            &snapshot,
            now + Duration::minutes(61),
            RefreshPolicy::default(),
            || { PlanFetchOutcome::Unavailable }
        ),
        None
    );

    let _ = fs::remove_dir_all(snapshot.parent().unwrap());
}

#[test]
fn another_accounts_plan_and_throttle_do_not_apply() {
    let snapshot = temp_snapshot_path();
    let now = Utc::now();
    let account = |account| RefreshPolicy {
        account: Some(account),
        ..Default::default()
    };
    assert_eq!(
        refresh_shared(&snapshot, now, account("a"), || {
            PlanFetchOutcome::RateLimited(std::time::Duration::from_secs(3_600))
        }),
        None
    );

    // Just switched: fetches at once, despite the other account's retry deadline.
    assert_eq!(
        refresh_shared(&snapshot, now + Duration::seconds(5), account("b"), || {
            PlanFetchOutcome::Success(plan(10.0))
        }),
        Some(plan(10.0))
    );

    // Within the throttle, the same account reads the cache without fetching.
    assert_eq!(
        refresh_shared(&snapshot, now + Duration::seconds(10), account("b"), || {
            panic!("must not fetch")
        }),
        Some(plan(10.0))
    );

    // Switching back asks again rather than showing the other account's numbers.
    assert_eq!(
        refresh_shared(&snapshot, now + Duration::seconds(15), account("a"), || {
            PlanFetchOutcome::Success(plan(20.0))
        }),
        Some(plan(20.0))
    );
}

#[test]
fn looking_at_the_panel_shortens_the_throttle_but_not_retry_after() {
    let snapshot = temp_snapshot_path();
    let now = Utc::now();
    let looking = RefreshPolicy {
        user_looking: true,
        ..Default::default()
    };
    refresh_shared(&snapshot, now, RefreshPolicy::default(), || {
        PlanFetchOutcome::Success(plan(10.0))
    });
    assert_eq!(
        refresh_shared(&snapshot, now + Duration::seconds(59), looking, || {
            panic!("must not fetch")
        }),
        Some(plan(10.0))
    );
    assert_eq!(
        refresh_shared(&snapshot, now + Duration::seconds(60), looking, || {
            PlanFetchOutcome::RateLimited(std::time::Duration::from_secs(600))
        }),
        Some(plan(10.0))
    );
    assert_eq!(
        refresh_shared(&snapshot, now + Duration::seconds(120), looking, || {
            panic!("must honor Retry-After")
        }),
        Some(plan(10.0))
    );
}
