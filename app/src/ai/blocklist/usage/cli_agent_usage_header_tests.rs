use chrono::{Duration, TimeZone, Utc};
use cli_agent_usage::{LimitWindow, PlanLimits, Provider, Severity};

use super::{limit_texts, provider_windows};

#[test]
fn limit_texts_none_is_dash() {
    let now = Utc.with_ymd_and_hms(2026, 7, 2, 12, 0, 0).unwrap();
    let (pct, reset, sev) = limit_texts(None, now, true);
    assert_eq!(pct, "—");
    assert_eq!(reset, None);
    assert_eq!(sev, Severity::Normal);
}

#[test]
fn limit_texts_with_reset() {
    let now = Utc.with_ymd_and_hms(2026, 7, 2, 12, 0, 0).unwrap();
    let w = LimitWindow {
        percent: 47.0,
        resets_at: Some(now + Duration::hours(2)),
        severity: Severity::Warning,
    };
    let (pct, reset, sev) = limit_texts(Some(w), now, true);
    assert_eq!(pct, "47%");
    assert_eq!(reset.as_deref(), Some("in 2h"));
    assert_eq!(sev, Severity::Warning);
}

#[test]
fn limit_texts_without_reset_omits_countdown() {
    let now = Utc.with_ymd_and_hms(2026, 7, 2, 12, 0, 0).unwrap();
    let w = LimitWindow {
        percent: 8.0,
        resets_at: Some(now + Duration::days(6)),
        severity: Severity::Normal,
    };
    let (pct, reset, _sev) = limit_texts(Some(w), now, false);
    assert_eq!(pct, "8%");
    assert_eq!(reset, None);
}

#[test]
fn claude_provider_windows_include_fable_as_a_separate_limit() {
    let window = |percent| LimitWindow {
        percent,
        resets_at: None,
        severity: Severity::Normal,
    };
    let provider = Provider {
        plan: Some(PlanLimits {
            session: Some(window(12.0)),
            weekly: Some(window(34.0)),
            fable_weekly: Some(window(56.0)),
        }),
        ..Provider::default()
    };

    let windows = provider_windows(&provider, true);
    assert_eq!(windows.len(), 3);
    assert_eq!(windows[0], ("5h ", Some(window(12.0))));
    assert_eq!(windows[1], ("wk ", Some(window(34.0))));
    assert_eq!(windows[2], ("Fable ", Some(window(56.0))));

    let codex_windows = provider_windows(&provider, false);
    assert_eq!(codex_windows.len(), 2);
}
