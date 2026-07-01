//! Pure, UI-free display helpers for footer rendering. No toolkit/theme deps.

use chrono::{DateTime, Utc};

use crate::{Provider, Severity, UsageSnapshot};

/// Humanize a token count: `0..=999` verbatim, then `k`/`M`/`B` with one decimal.
pub fn fmt_tokens(n: u64) -> String {
    if n < 1_000 {
        return n.to_string();
    }
    let (value, suffix) = if n < 1_000_000 {
        (n as f64 / 1_000.0, "k")
    } else if n < 1_000_000_000 {
        (n as f64 / 1_000_000.0, "M")
    } else {
        (n as f64 / 1_000_000_000.0, "B")
    };
    format!("{value:.1}{suffix}")
}

/// Round a percentage to an integer, e.g. `54.6 -> "55%"`.
pub fn fmt_pct(percent: f64) -> String {
    format!("{}%", percent.round() as i64)
}

/// Relative "resets" label: `"in 12m"` / `"in 3h"` / `"in 2d"`; past-or-now -> `"now"`;
/// `None` -> `"—"`.
pub fn fmt_reset(resets_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> String {
    let Some(target) = resets_at else {
        return "—".to_string();
    };
    let secs = (target - now).num_seconds();
    if secs <= 0 {
        "now".to_string()
    } else if secs < 3_600 {
        format!("in {}m", secs / 60)
    } else if secs < 86_400 {
        format!("in {}h", secs / 3_600)
    } else {
        format!("in {}d", secs / 86_400)
    }
}

/// One half of the footer chip.
pub struct ChipHalf {
    /// `"cc"` (Claude Code) or `"cx"` (Codex).
    pub label: &'static str,
    /// Weekly plan-% like `"47%w"`, or `"—"` when unknown.
    pub pct: String,
    /// Drives the half's color; `Normal` when `pct == "—"`.
    pub severity: Severity,
}

/// The two chip halves `[claude, codex]`. Returns `None` when NEITHER provider has
/// any data (the chip is hidden). A provider that has token data but no weekly plan
/// yields a `"—"` half but still counts as "has data".
pub fn chip_halves(snap: &UsageSnapshot) -> Option<[ChipHalf; 2]> {
    fn half(label: &'static str, p: &Provider) -> (ChipHalf, bool) {
        let has_tokens = p.month.tokens.total() > 0;
        match p.plan.and_then(|pl| pl.weekly) {
            Some(w) => (
                ChipHalf {
                    label,
                    pct: format!("{}w", fmt_pct(w.percent)),
                    severity: w.severity,
                },
                true,
            ),
            None => (
                ChipHalf {
                    label,
                    pct: "—".to_string(),
                    severity: Severity::Normal,
                },
                has_tokens,
            ),
        }
    }
    let (cc, cc_has) = half("cc", &snap.claude);
    let (cx, cx_has) = half("cx", &snap.codex);
    if !cc_has && !cx_has {
        return None;
    }
    Some([cc, cx])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LimitWindow, PlanLimits, Severity, TokenCounts, UsageSnapshot};
    use chrono::{Duration, TimeZone, Utc};

    #[test]
    fn fmt_tokens_boundaries() {
        assert_eq!(fmt_tokens(0), "0");
        assert_eq!(fmt_tokens(999), "999");
        assert_eq!(fmt_tokens(1_000), "1.0k");
        assert_eq!(fmt_tokens(1_500), "1.5k");
        assert_eq!(fmt_tokens(999_999), "1000.0k");
        assert_eq!(fmt_tokens(1_500_000), "1.5M");
        assert_eq!(fmt_tokens(8_316_864_043), "8.3B");
    }

    #[test]
    fn fmt_pct_rounds_to_integer() {
        assert_eq!(fmt_pct(47.0), "47%");
        assert_eq!(fmt_pct(54.6), "55%");
    }

    #[test]
    fn fmt_reset_none_and_relative() {
        let now = Utc.with_ymd_and_hms(2026, 6, 30, 12, 0, 0).unwrap();
        assert_eq!(fmt_reset(None, now), "—");
        assert_eq!(fmt_reset(Some(now + Duration::minutes(12)), now), "in 12m");
        assert_eq!(fmt_reset(Some(now + Duration::hours(3)), now), "in 3h");
        assert_eq!(fmt_reset(Some(now + Duration::days(2)), now), "in 2d");
        assert_eq!(fmt_reset(Some(now - Duration::hours(1)), now), "now");
    }

    #[test]
    fn chip_halves_hidden_when_snapshot_empty() {
        assert!(chip_halves(&UsageSnapshot::default()).is_none());
    }

    #[test]
    fn chip_halves_present_half_and_missing_half() {
        let mut snap = UsageSnapshot::default();
        // Claude: has a weekly plan -> "47%w", Warning.
        snap.claude.plan = Some(PlanLimits {
            session: None,
            weekly: Some(LimitWindow {
                percent: 47.0,
                resets_at: None,
                severity: Severity::Warning,
            }),
        });
        // Codex: token data but no plan -> "—" half, still shown.
        snap.codex.month.tokens = TokenCounts {
            input: 5,
            output: 5,
            cache_read: 0,
            cache_write: 0,
        };
        let [cc, cx] = chip_halves(&snap).expect("has data");
        assert_eq!(cc.label, "cc");
        assert_eq!(cc.pct, "47%w");
        assert_eq!(cc.severity, Severity::Warning);
        assert_eq!(cx.label, "cx");
        assert_eq!(cx.pct, "—");
        assert_eq!(cx.severity, Severity::Normal);
    }
}
