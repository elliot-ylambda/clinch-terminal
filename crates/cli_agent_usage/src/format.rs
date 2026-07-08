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

/// Relative "resets" label: `"in 12m"` / `"in 1h 45m"` / `"in 3h"` / `"in 2d"`;
/// past-or-now -> `"now"`; `None` -> `"—"`. Under a day the remaining minutes are
/// shown alongside the hours (dropped only when they are exactly zero) so a window
/// that resets in 1h 45m never reads as a flat "in 1h".
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
        let hours = secs / 3_600;
        let mins = (secs % 3_600) / 60;
        if mins == 0 {
            format!("in {hours}h")
        } else {
            format!("in {hours}h {mins}m")
        }
    } else {
        format!("in {}d", secs / 86_400)
    }
}

/// Map a raw model id to a short human label for the tab chip.
/// Returns `""` for unknown / synthetic ids so callers can hide the chip.
///
/// Examples: `claude-opus-4-8` → `Opus 4.8`, `claude-haiku-4-5-20251001` →
/// `Haiku 4.5`, `gpt-5-codex` → `GPT-5 Codex`, `gpt-5.5` → `GPT-5.5`.
pub fn friendly_model_name(model: &str) -> String {
    let m = model.trim().to_ascii_lowercase();
    if m.is_empty() || m == "<synthetic>" || m == "unknown" {
        return String::new();
    }

    // Anthropic families.
    for (needle, label) in [("opus", "Opus"), ("sonnet", "Sonnet"), ("haiku", "Haiku")] {
        if let Some(idx) = m.find(needle) {
            let ver = version_from(&m[idx + needle.len()..]);
            return if ver.is_empty() {
                label.to_string()
            } else {
                format!("{label} {ver}")
            };
        }
    }

    // OpenAI / Codex.
    if m.starts_with("gpt") || m.contains("codex") {
        let core = m.trim_start_matches("gpt-").trim_start_matches("gpt");
        let ver = version_from(core);
        let mut label = String::from("GPT");
        if !ver.is_empty() {
            label.push('-');
            label.push_str(&ver);
        }
        if m.contains("codex") {
            label.push_str(" Codex");
        }
        return label;
    }

    // Fallback: title-case the dashed id.
    m.split(['-', '_'])
        .filter(|s| !s.is_empty())
        .map(|s| {
            let mut chars = s.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Read a leading `major[.minor]` version from `s` (after any leading `-`/`.`),
/// stopping at the first non-numeric token, a date-like token (len ≥ 5), or two
/// components. `"-4-8"` → `"4.8"`, `"-4-5-20251001"` → `"4.5"`, `"5-codex"` →
/// `"5"`, `""` → `""`.
fn version_from(s: &str) -> String {
    let s = s.trim_start_matches(['-', '.']);
    let mut parts = Vec::new();
    for token in s.split(['-', '.']) {
        if token.is_empty() || !token.chars().all(|c| c.is_ascii_digit()) || token.len() >= 5 {
            break;
        }
        parts.push(token);
        if parts.len() == 2 {
            break;
        }
    }
    parts.join(".")
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
    use chrono::{Duration, TimeZone, Utc};

    use super::*;
    use crate::{LimitWindow, PlanLimits, Severity, TokenCounts, UsageSnapshot};

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
        assert_eq!(
            fmt_reset(Some(now + Duration::hours(1) + Duration::minutes(45)), now),
            "in 1h 45m"
        );
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

    #[test]
    fn chip_halves_shown_when_only_tokens_no_plan() {
        // Regression guard for the "tokens-but-no-plan still counts as data" rule:
        // neither provider has a plan; ONLY Codex has token data. The chip must still
        // show. If `has_tokens` were ignored, this would wrongly return None and the
        // `.expect` below would panic.
        let mut snap = UsageSnapshot::default();
        snap.codex.month.tokens = TokenCounts {
            input: 10,
            output: 0,
            cache_read: 0,
            cache_write: 0,
        };
        let [cc, cx] = chip_halves(&snap).expect("codex has token data => chip shown");
        assert_eq!(cc.pct, "—"); // claude: no plan, no tokens
        assert_eq!(cx.pct, "—"); // codex: no plan (tokens don't set pct)
        assert_eq!(cc.severity, Severity::Normal);
        assert_eq!(cx.severity, Severity::Normal);
    }

    #[test]
    fn friendly_model_name_maps_known_ids() {
        assert_eq!(friendly_model_name("claude-opus-4-8"), "Opus 4.8");
        assert_eq!(friendly_model_name("claude-sonnet-5"), "Sonnet 5");
        assert_eq!(
            friendly_model_name("claude-haiku-4-5-20251001"),
            "Haiku 4.5"
        );
        assert_eq!(friendly_model_name("claude-haiku"), "Haiku");
        assert_eq!(friendly_model_name("gpt-5-codex"), "GPT-5 Codex");
        assert_eq!(friendly_model_name("gpt-5.5"), "GPT-5.5");
        assert_eq!(friendly_model_name("gpt-5"), "GPT-5");
    }

    #[test]
    fn friendly_model_name_hides_noise_and_titlecases_unknown() {
        assert_eq!(friendly_model_name("<synthetic>"), "");
        assert_eq!(friendly_model_name("unknown"), "");
        assert_eq!(friendly_model_name("   "), "");
        assert_eq!(friendly_model_name("some-new-model"), "Some New Model");
    }
}
