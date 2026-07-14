use chrono::{Duration, TimeZone, Utc};
use cli_agent_usage::{LimitWindow, PlanLimits, Provider, Severity, TokenCounts, WindowTotals};

use super::{
    compact_provider_windows, limit_texts, provider_windows, token_text, token_windows,
    visible_exhausted_until,
};
use crate::ai::blocklist::usage::{
    CliAgentUsageHeaderVisibility, CliAgentUsageMetric, CliAgentUsageProvider,
};

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

    let visibility = CliAgentUsageHeaderVisibility::default();
    let windows = provider_windows(&provider, CliAgentUsageProvider::Claude, &visibility);
    assert_eq!(windows.len(), 3);
    assert_eq!(windows[0], ("5h ", Some(window(12.0))));
    assert_eq!(windows[1], ("wk ", Some(window(34.0))));
    assert_eq!(windows[2], ("Fable ", Some(window(56.0))));

    let codex_windows = provider_windows(&provider, CliAgentUsageProvider::Codex, &visibility);
    assert_eq!(codex_windows.len(), 2);
}

#[test]
fn provider_windows_respect_independent_visibility_overrides() {
    let provider = Provider {
        plan: Some(PlanLimits {
            session: Some(LimitWindow {
                percent: 12.,
                resets_at: None,
                severity: Severity::Normal,
            }),
            weekly: Some(LimitWindow {
                percent: 34.,
                resets_at: None,
                severity: Severity::Normal,
            }),
            fable_weekly: None,
        }),
        ..Provider::default()
    };
    let overrides = [
        ("claude.5_hour".to_string(), false),
        ("claude.fable".to_string(), false),
    ]
    .into_iter()
    .collect();
    let visibility = CliAgentUsageHeaderVisibility::from_overrides(&overrides);

    let windows = provider_windows(&provider, CliAgentUsageProvider::Claude, &visibility);
    assert_eq!(windows, vec![("wk ", provider.plan.unwrap().weekly)]);
}

#[test]
fn compact_windows_preserve_weekly_default_and_fall_back_to_five_hour() {
    let window = |percent| LimitWindow {
        percent,
        resets_at: None,
        severity: Severity::Normal,
    };
    let provider = Provider {
        plan: Some(PlanLimits {
            session: Some(window(12.)),
            weekly: Some(window(34.)),
            fable_weekly: Some(window(56.)),
        }),
        ..Provider::default()
    };

    let default_windows = compact_provider_windows(
        &provider,
        CliAgentUsageProvider::Claude,
        &CliAgentUsageHeaderVisibility::default(),
    );
    assert_eq!(
        default_windows,
        vec![
            (CliAgentUsageMetric::Weekly, Some(window(34.))),
            (CliAgentUsageMetric::Fable, Some(window(56.))),
        ]
    );

    let overrides = [("claude.weekly".to_string(), false)].into_iter().collect();
    let visibility = CliAgentUsageHeaderVisibility::from_overrides(&overrides);
    let fallback_windows =
        compact_provider_windows(&provider, CliAgentUsageProvider::Claude, &visibility);
    assert_eq!(
        fallback_windows[0],
        (CliAgentUsageMetric::FiveHour, Some(window(12.)))
    );
}

#[test]
fn compact_reset_ignores_hidden_exhausted_windows() {
    let now = Utc.with_ymd_and_hms(2026, 7, 2, 12, 0, 0).unwrap();
    let provider = Provider {
        plan: Some(PlanLimits {
            session: Some(LimitWindow {
                percent: 100.,
                resets_at: Some(now + Duration::hours(2)),
                severity: Severity::Critical,
            }),
            weekly: Some(LimitWindow {
                percent: 100.,
                resets_at: Some(now + Duration::days(2)),
                severity: Severity::Critical,
            }),
            fable_weekly: None,
        }),
        ..Provider::default()
    };
    let overrides = [("claude.weekly".to_string(), false)].into_iter().collect();
    let visibility = CliAgentUsageHeaderVisibility::from_overrides(&overrides);

    assert_eq!(
        visible_exhausted_until(&provider, CliAgentUsageProvider::Claude, &visibility),
        Some(now + Duration::hours(2))
    );
}

#[test]
fn token_windows_are_opt_in_and_format_io_tokens() {
    let provider = Provider {
        today: WindowTotals {
            tokens: TokenCounts {
                input: 1_000,
                output: 500,
                cache_read: 9_000,
                cache_write: 0,
            },
            cost_usd: 0.,
        },
        ..Provider::default()
    };
    assert!(token_windows(
        &provider,
        CliAgentUsageProvider::Claude,
        &CliAgentUsageHeaderVisibility::default()
    )
    .is_empty());

    let overrides = [("claude.tokens.today".to_string(), true)]
        .into_iter()
        .collect();
    let visibility = CliAgentUsageHeaderVisibility::from_overrides(&overrides);
    let windows = token_windows(&provider, CliAgentUsageProvider::Claude, &visibility);
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].0, CliAgentUsageMetric::TodayTokens);
    assert_eq!(token_text(windows[0].1), "1.5k");
}
