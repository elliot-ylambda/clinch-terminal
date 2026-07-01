//! Parse Codex rollout sessions (~/.codex/sessions/**/rollout-*.jsonl).

use std::path::Path;

use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;

use crate::cache::{scan_dir, ScanCache};
use crate::{
    aggregate_windows, Entry, LimitWindow, PlanLimits, Provider, Severity, TokenCounts,
    WindowTotals,
};

#[derive(Default)]
pub struct RollupFile {
    pub entries: Vec<Entry>,
    pub last_total: TokenCounts,
    pub rate_limits: Option<PlanLimits>,
}

#[derive(Deserialize)]
struct Line {
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    payload: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct TotalUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    cached_input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

#[derive(Deserialize)]
struct Window {
    #[serde(default)]
    used_percent: f64,
    #[serde(default)]
    resets_at: Option<i64>,
}

#[derive(Deserialize)]
struct RateLimits {
    primary: Option<Window>,
    secondary: Option<Window>,
}

pub fn severity_from_percent(p: f64) -> Severity {
    if p < 75.0 {
        Severity::Normal
    } else if p < 90.0 {
        Severity::Warning
    } else {
        Severity::Critical
    }
}

fn window_to_limit(w: &Window) -> LimitWindow {
    LimitWindow {
        percent: w.used_percent,
        resets_at: w.resets_at.and_then(|s| Utc.timestamp_opt(s, 0).single()),
        severity: severity_from_percent(w.used_percent),
    }
}

/// `total_token_usage` is cumulative per session: split into uncached input vs cache_read.
fn split(total: &TotalUsage) -> TokenCounts {
    TokenCounts {
        input: total.input_tokens.saturating_sub(total.cached_input_tokens),
        output: total.output_tokens,
        cache_read: total.cached_input_tokens,
        cache_write: 0,
    }
}

pub fn parse_rollout_str(content: &str) -> RollupFile {
    let mut out = RollupFile::default();
    let mut model = "gpt-5-codex".to_string();
    let mut prev_cumulative: Option<TokenCounts> = None;

    for raw in content.lines() {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let Ok(line) = serde_json::from_str::<Line>(raw) else {
            continue;
        };
        let Some(payload) = line.payload.as_ref() else {
            continue;
        };

        // Track the latest model id seen anywhere in the file.
        if let Some(m) = payload.get("model").and_then(|v| v.as_str()) {
            model = m.to_string();
        }

        if payload.get("type").and_then(|v| v.as_str()) != Some("token_count") {
            continue;
        }
        let Some(ts_str) = line.timestamp.as_deref() else {
            continue;
        };
        let Ok(ts) = DateTime::parse_from_rfc3339(ts_str) else {
            continue;
        };
        let ts = ts.with_timezone(&Utc);

        if let Some(info_total) = payload
            .get("info")
            .and_then(|i| i.get("total_token_usage"))
            .and_then(|t| serde_json::from_value::<TotalUsage>(t.clone()).ok())
        {
            let cumulative = split(&info_total);
            // delta vs previous cumulative (clamped at 0 in case of resets)
            let delta = match prev_cumulative {
                Some(p) => TokenCounts {
                    input: cumulative.input.saturating_sub(p.input),
                    output: cumulative.output.saturating_sub(p.output),
                    cache_read: cumulative.cache_read.saturating_sub(p.cache_read),
                    cache_write: 0,
                },
                None => cumulative,
            };
            prev_cumulative = Some(cumulative);
            out.last_total = cumulative;
            out.entries.push(Entry {
                ts,
                model: model.clone(),
                tokens: delta,
                dedup: String::new(),
            });
        }

        if let Some(rl) = payload
            .get("rate_limits")
            .and_then(|v| serde_json::from_value::<RateLimits>(v.clone()).ok())
        {
            out.rate_limits = Some(PlanLimits {
                session: rl.primary.as_ref().map(window_to_limit),
                weekly: rl.secondary.as_ref().map(window_to_limit),
            });
        }
    }
    out
}

pub fn parse_rollout_file(path: &Path) -> RollupFile {
    match std::fs::read_to_string(path) {
        Ok(s) => parse_rollout_str(&s),
        Err(_) => RollupFile::default(),
    }
}

pub fn scan(
    sessions_dir: &Path,
    cache: &mut ScanCache<RollupFile>,
    now: DateTime<Utc>,
) -> Provider {
    let mut provider = Provider::default();
    let mut seen = std::collections::HashSet::new();

    let mut files = scan_dir(sessions_dir, ".jsonl");
    files.retain(|(p, _, _)| {
        p.file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with("rollout-"))
            .unwrap_or(false)
    });
    let latest = files
        .iter()
        .max_by_key(|(_, mtime, _)| *mtime)
        .map(|(p, _, _)| p.clone());

    for (path, mtime, size) in &files {
        let parsed = cache.get_or_parse(path, *mtime, *size, parse_rollout_file);
        let entries = parsed.entries.clone();
        let is_latest = Some(path) == latest.as_ref();
        let last_total = parsed.last_total;
        let rate_limits = if is_latest { parsed.rate_limits } else { None };

        aggregate_windows(
            &entries,
            now,
            &mut seen,
            &mut provider.today,
            &mut provider.week,
            &mut provider.month,
        );
        if is_latest {
            let model = entries.last().map(|e| e.model.clone()).unwrap_or_default();
            provider.session = WindowTotals {
                tokens: last_total,
                cost_usd: crate::pricing::cost(&model, &last_total),
            };
            provider.plan = rate_limits;
        }
    }
    provider
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Severity;

    // Two token_count events: deltas are (2000 input/100 out) then (+500 input/+50 out).
    const A: &str = r#"{"timestamp":"2026-06-30T10:00:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":2000,"cached_input_tokens":1000,"output_tokens":100,"reasoning_output_tokens":10,"total_tokens":2100}},"rate_limits":{"primary":{"used_percent":9.0,"window_minutes":300,"resets_at":1782425344},"secondary":{"used_percent":18.0,"window_minutes":10080,"resets_at":1782421135},"plan_type":"prolite"}}}"#;
    const B: &str = r#"{"timestamp":"2026-06-30T11:00:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":2500,"cached_input_tokens":1200,"output_tokens":150,"reasoning_output_tokens":20,"total_tokens":2650}},"rate_limits":{"primary":{"used_percent":20.0,"window_minutes":300,"resets_at":1782461677},"secondary":{"used_percent":19.0,"window_minutes":10080,"resets_at":1783028371},"plan_type":"prolite"}}}"#;
    const META: &str = r#"{"timestamp":"2026-06-30T09:59:00.000Z","type":"session_meta","payload":{"model":"gpt-5.5"}}"#;

    #[test]
    fn deltas_and_uncached_split() {
        let r = parse_rollout_str(&format!("{META}\n{A}\n{B}"));
        assert_eq!(r.entries.len(), 2);
        // first event: uncached = 2000-1000=1000 input, cache_read=1000, output=100
        assert_eq!(
            r.entries[0].tokens,
            crate::TokenCounts {
                input: 1000,
                output: 100,
                cache_read: 1000,
                cache_write: 0
            }
        );
        assert_eq!(r.entries[0].model, "gpt-5.5");
        // second event delta: total input 2500-2000=500 of which cached 1200-1000=200 => uncached 300
        assert_eq!(
            r.entries[1].tokens,
            crate::TokenCounts {
                input: 300,
                output: 50,
                cache_read: 200,
                cache_write: 0
            }
        );
        // session total = last event cumulative, split uncached
        assert_eq!(
            r.last_total,
            crate::TokenCounts {
                input: 1300,
                output: 150,
                cache_read: 1200,
                cache_write: 0
            }
        );
    }

    #[test]
    fn rate_limits_map_to_session_and_weekly() {
        let r = parse_rollout_str(A);
        let plan = r.rate_limits.unwrap();
        assert_eq!(plan.session.unwrap().percent, 9.0);
        assert_eq!(plan.weekly.unwrap().percent, 18.0);
        assert_eq!(plan.session.unwrap().severity, Severity::Normal);
    }

    #[test]
    fn empty_or_malformed_is_empty() {
        let r = parse_rollout_str("garbage\n\n{}");
        assert!(r.entries.is_empty());
        assert!(r.rate_limits.is_none());
    }

    #[test]
    fn scan_uses_latest_rollout_for_session_and_plan() {
        use std::fs;
        use std::time::{Duration, SystemTime};

        let older = r#"{"timestamp":"2026-06-29T10:00:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":500,"cached_input_tokens":0,"output_tokens":10,"reasoning_output_tokens":0,"total_tokens":510}},"rate_limits":{"primary":{"used_percent":5.0,"window_minutes":300,"resets_at":1782425344},"secondary":{"used_percent":6.0,"window_minutes":10080,"resets_at":1782421135}}}}"#;
        let newer = r#"{"timestamp":"2026-06-30T10:00:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":2000,"cached_input_tokens":800,"output_tokens":100,"reasoning_output_tokens":10,"total_tokens":2100}},"rate_limits":{"primary":{"used_percent":42.0,"window_minutes":300,"resets_at":1782461677},"secondary":{"used_percent":19.0,"window_minutes":10080,"resets_at":1783028371}}}}"#;

        let dir = std::env::temp_dir().join(format!("cau_codex_scan_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let of = dir.join("rollout-older.jsonl");
        let nf = dir.join("rollout-newer.jsonl");
        fs::write(&of, older).unwrap();
        fs::write(&nf, newer).unwrap();

        let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        fs::File::open(&of).unwrap().set_modified(base).unwrap();
        fs::File::open(&nf)
            .unwrap()
            .set_modified(base + Duration::from_secs(60))
            .unwrap();

        let mut cache = crate::cache::ScanCache::new();
        let p = scan(&dir, &mut cache, chrono::Utc::now());

        // plan comes from the NEWER rollout's rate_limits
        let plan = p.plan.expect("plan from latest rollout");
        assert_eq!(plan.session.unwrap().percent, 42.0);
        assert_eq!(plan.weekly.unwrap().percent, 19.0);
        // session = newer file's last cumulative total, uncached-split: input 2000-800=1200, cache_read 800, output 100
        assert_eq!(
            p.session.tokens,
            crate::TokenCounts {
                input: 1200,
                output: 100,
                cache_read: 800,
                cache_write: 0
            }
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
