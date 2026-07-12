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
    /// Latest canonical observation for each independently-updated window.
    pub session_rate_limit: Option<TimedLimitWindow>,
    pub weekly_rate_limit: Option<TimedLimitWindow>,
    /// Session uuid from the `session_meta` line, when present.
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimedLimitWindow {
    pub window: LimitWindow,
    pub updated_at: DateTime<Utc>,
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
    used_percent: Option<f64>,
    #[serde(default)]
    window_minutes: Option<u64>,
    #[serde(default)]
    resets_at: Option<i64>,
}

#[derive(Deserialize)]
struct RateLimits {
    #[serde(default)]
    limit_id: Option<String>,
    primary: Option<Window>,
    secondary: Option<Window>,
}

impl RateLimits {
    fn is_canonical_codex(&self) -> bool {
        // Older rollout records predate `limit_id`; continue accepting them.
        // Newer model-scoped limits (for example `codex_bengalfox`) must not
        // replace the account's canonical Codex windows.
        matches!(self.limit_id.as_deref(), None | Some("codex"))
    }
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

fn window_to_limit(w: &Window) -> Option<LimitWindow> {
    let percent = w.used_percent?;
    Some(LimitWindow {
        percent,
        resets_at: w.resets_at.and_then(|s| Utc.timestamp_opt(s, 0).single()),
        severity: severity_from_percent(percent),
    })
}

#[derive(Clone, Copy)]
enum WindowKind {
    Session,
    Weekly,
}

fn window_kind(window: &Window, legacy_position: WindowKind) -> Option<WindowKind> {
    match window.window_minutes {
        Some(300) => Some(WindowKind::Session),
        Some(10_080) => Some(WindowKind::Weekly),
        Some(_) => None,
        // Legacy records omitted the duration and consistently used
        // primary=session, secondary=weekly.
        None => Some(legacy_position),
    }
}

fn update_limit(
    slot: &mut Option<TimedLimitWindow>,
    window: LimitWindow,
    updated_at: DateTime<Utc>,
) {
    if slot
        .as_ref()
        .is_none_or(|current| updated_at >= current.updated_at)
    {
        *slot = Some(TimedLimitWindow { window, updated_at });
    }
}

fn merge_limit(slot: &mut Option<TimedLimitWindow>, candidate: Option<TimedLimitWindow>) {
    let Some(candidate) = candidate else {
        return;
    };
    if slot
        .as_ref()
        .is_none_or(|current| candidate.updated_at > current.updated_at)
    {
        *slot = Some(candidate);
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

        // The session_meta line's payload carries the session uuid.
        if out.session_id.is_none() {
            if let Some(sid) = payload.get("session_id").and_then(|v| v.as_str()) {
                out.session_id = Some(sid.to_string());
            }
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
            if rl.is_canonical_codex() {
                for (raw_window, legacy_position) in [
                    (rl.primary.as_ref(), WindowKind::Session),
                    (rl.secondary.as_ref(), WindowKind::Weekly),
                ] {
                    let Some(raw_window) = raw_window else {
                        continue;
                    };
                    let (Some(kind), Some(window)) = (
                        window_kind(raw_window, legacy_position),
                        window_to_limit(raw_window),
                    ) else {
                        continue;
                    };
                    match kind {
                        WindowKind::Session => {
                            update_limit(&mut out.session_rate_limit, window, ts)
                        }
                        WindowKind::Weekly => update_limit(&mut out.weekly_rate_limit, window, ts),
                    }
                }
            }
        }
    }
    let session = out.session_rate_limit.map(|limit| limit.window);
    let weekly = out.weekly_rate_limit.map(|limit| limit.window);
    if session.is_some() || weekly.is_some() {
        out.rate_limits = Some(PlanLimits {
            session,
            weekly,
            fable_weekly: None,
        });
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
    models: &mut std::collections::HashMap<String, String>,
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
    let mut latest_session_limit = None;
    let mut latest_weekly_limit = None;

    for (path, mtime, size) in &files {
        let parsed = cache.get_or_parse(path, *mtime, *size, parse_rollout_file);
        let entries = parsed.entries.clone();
        let is_latest = Some(path) == latest.as_ref();
        let last_total = parsed.last_total;

        merge_limit(&mut latest_session_limit, parsed.session_rate_limit);
        merge_limit(&mut latest_weekly_limit, parsed.weekly_rate_limit);

        // Index this rollout's latest model under its session id (from meta).
        if let Some(session_id) = parsed.session_id.clone() {
            if let Some(model) = parsed.entries.last().map(|e| e.model.clone()) {
                models.insert(session_id, model);
            }
        }

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
            // Session cost prices the whole-session cumulative at the last-seen model's
            // rate; if the session switched models this is approximate (per-window
            // deltas above are priced per-event and remain exact).
            provider.session = WindowTotals {
                tokens: last_total,
                cost_usd: crate::pricing::cost(&model, &last_total),
            };
        }
    }
    let session = latest_session_limit.map(|limit| limit.window);
    let weekly = latest_weekly_limit.map(|limit| limit.window);
    if session.is_some() || weekly.is_some() {
        provider.plan = Some(PlanLimits {
            session,
            weekly,
            fable_weekly: None,
        });
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
        assert_eq!(plan.fable_weekly, None);
        assert_eq!(plan.session.unwrap().severity, Severity::Normal);
        assert_eq!(
            r.session_rate_limit.map(|limit| limit.updated_at),
            Some(
                DateTime::parse_from_rfc3339("2026-06-30T10:00:00.000Z")
                    .unwrap()
                    .with_timezone(&Utc)
            )
        );
    }

    #[test]
    fn model_scoped_rate_limit_does_not_replace_canonical_codex_limit() {
        let canonical = r#"{"timestamp":"2026-07-12T18:20:00Z","payload":{"type":"token_count","rate_limits":{"limit_id":"codex","primary":{"used_percent":68},"secondary":{"used_percent":42}}}}"#;
        let spark = r#"{"timestamp":"2026-07-12T18:21:00Z","payload":{"type":"token_count","rate_limits":{"limit_id":"codex_bengalfox","limit_name":"GPT-5.3-Codex-Spark","primary":{"used_percent":0},"secondary":{"used_percent":0}}}}"#;

        let parsed = parse_rollout_str(&format!("{canonical}\n{spark}"));
        let plan = parsed.rate_limits.expect("canonical Codex limit");
        assert_eq!(plan.session.unwrap().percent, 68.0);
        assert_eq!(plan.weekly.unwrap().percent, 42.0);

        let scoped_only = parse_rollout_str(spark);
        assert_eq!(scoped_only.rate_limits, None);
        assert_eq!(scoped_only.session_rate_limit, None);
        assert_eq!(scoped_only.weekly_rate_limit, None);
    }

    #[test]
    fn classifies_partial_windows_by_duration_and_preserves_other_window() {
        let session = r#"{"timestamp":"2026-07-12T18:20:00Z","payload":{"type":"token_count","rate_limits":{"limit_id":"codex","primary":{"used_percent":78,"window_minutes":300},"secondary":null}}}"#;
        // A current Codex payload can put the weekly-only window in `primary`.
        let weekly = r#"{"timestamp":"2026-07-12T18:21:00Z","payload":{"type":"token_count","rate_limits":{"limit_id":"codex","primary":{"used_percent":46,"window_minutes":10080},"secondary":null}}}"#;

        let parsed = parse_rollout_str(&format!("{session}\n{weekly}"));
        let plan = parsed.rate_limits.expect("merged partial limits");
        assert_eq!(plan.session.unwrap().percent, 78.0);
        assert_eq!(plan.weekly.unwrap().percent, 46.0);
        assert_eq!(
            parsed
                .session_rate_limit
                .expect("session observation")
                .updated_at,
            DateTime::parse_from_rfc3339("2026-07-12T18:20:00Z")
                .unwrap()
                .with_timezone(&Utc)
        );
        assert_eq!(
            parsed
                .weekly_rate_limit
                .expect("weekly observation")
                .updated_at,
            DateTime::parse_from_rfc3339("2026-07-12T18:21:00Z")
                .unwrap()
                .with_timezone(&Utc)
        );
    }

    #[test]
    fn missing_used_percent_does_not_fabricate_zero_percent() {
        let valid = r#"{"timestamp":"2026-07-12T18:20:00Z","payload":{"type":"token_count","rate_limits":{"limit_id":"codex","primary":{"used_percent":78,"window_minutes":300}}}}"#;
        let missing = r#"{"timestamp":"2026-07-12T18:21:00Z","payload":{"type":"token_count","rate_limits":{"limit_id":"codex","primary":{"window_minutes":300}}}}"#;

        let parsed = parse_rollout_str(&format!("{valid}\n{missing}"));
        assert_eq!(
            parsed
                .rate_limits
                .expect("valid limit retained")
                .session
                .unwrap()
                .percent,
            78.0
        );
        assert_eq!(
            parse_rollout_str(missing).rate_limits,
            None,
            "a missing percentage must not deserialize as 0%"
        );
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
        let p = scan(
            &dir,
            &mut cache,
            chrono::Utc::now(),
            &mut std::collections::HashMap::new(),
        );

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

    #[test]
    fn scan_uses_latest_canonical_event_instead_of_latest_file() {
        use std::fs;
        use std::time::{Duration, SystemTime};

        // These files have the newest canonical observations, despite having
        // older mtimes. Each current event contains only one window.
        let current_session = r#"{"timestamp":"2026-07-12T18:20:00Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"output_tokens":10}},"rate_limits":{"limit_id":"codex","primary":{"used_percent":68,"window_minutes":300},"secondary":null}}}"#;
        let current_weekly = r#"{"timestamp":"2026-07-12T18:19:00Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":120,"output_tokens":12}},"rate_limits":{"limit_id":"codex","primary":{"used_percent":42,"window_minutes":10080},"secondary":null}}}"#;

        // An actively-written Spark rollout has the newest mtime. It contains
        // a stale canonical event followed by a model-scoped 0/0 event; neither
        // should displace the fresher canonical event above.
        let stale_canonical = r#"{"timestamp":"2026-07-10T18:20:00Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":200,"output_tokens":20}},"rate_limits":{"limit_id":"codex","primary":{"used_percent":50,"window_minutes":300},"secondary":{"used_percent":40,"window_minutes":10080}}}}"#;
        let spark = r#"{"timestamp":"2026-07-12T18:21:00Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":300,"output_tokens":30}},"rate_limits":{"limit_id":"codex_bengalfox","primary":{"used_percent":0,"window_minutes":10080},"secondary":null}}}"#;

        let dir =
            std::env::temp_dir().join(format!("cau_codex_canonical_scan_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let session_file = dir.join("rollout-session.jsonl");
        let weekly_file = dir.join("rollout-weekly.jsonl");
        let spark_file = dir.join("rollout-spark.jsonl");
        fs::write(&session_file, current_session).unwrap();
        fs::write(&weekly_file, current_weekly).unwrap();
        fs::write(&spark_file, format!("{stale_canonical}\n{spark}")).unwrap();

        let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        fs::File::open(&session_file)
            .unwrap()
            .set_modified(base)
            .unwrap();
        fs::File::open(&weekly_file)
            .unwrap()
            .set_modified(base + Duration::from_secs(30))
            .unwrap();
        fs::File::open(&spark_file)
            .unwrap()
            .set_modified(base + Duration::from_secs(60))
            .unwrap();

        let mut cache = crate::cache::ScanCache::new();
        let provider = scan(
            &dir,
            &mut cache,
            chrono::Utc::now(),
            &mut std::collections::HashMap::new(),
        );

        let plan = provider.plan.expect("latest canonical plan");
        assert_eq!(plan.session.unwrap().percent, 68.0);
        assert_eq!(plan.weekly.unwrap().percent, 42.0);
        // Session totals still come from the newest-mtime active rollout.
        assert_eq!(provider.session.tokens.input, 300);
        assert_eq!(provider.session.tokens.output, 30);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_rollout_captures_session_id_from_meta() {
        const META: &str = r#"{"timestamp":"2026-06-25T22:51:38.016Z","type":"session_meta","payload":{"session_id":"019f00fb-40b5-7192-9b79-aa6d1034fe1b","id":"019f00fb-40b5-7192-9b79-aa6d1034fe1b","model":"gpt-5-codex"}}"#;
        let r = parse_rollout_str(META);
        assert_eq!(
            r.session_id.as_deref(),
            Some("019f00fb-40b5-7192-9b79-aa6d1034fe1b")
        );
    }

    #[test]
    fn scan_indexes_codex_model_by_session_id() {
        let dir = std::env::temp_dir().join(format!("cau_codex_modelidx_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path =
            dir.join("rollout-2026-06-25T15-51-33-019f00fb-40b5-7192-9b79-aa6d1034fe1b.jsonl");
        let meta = r#"{"timestamp":"2026-06-25T22:51:38.016Z","type":"session_meta","payload":{"session_id":"019f00fb-40b5-7192-9b79-aa6d1034fe1b","model":"gpt-5-codex"}}"#;
        let older = r#"{"timestamp":"2026-06-25T22:52:00.000Z","type":"event_msg","payload":{"type":"token_count","model":"gpt-5-codex","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":0,"output_tokens":5,"total_tokens":15}}}}"#;
        let newer = r#"{"timestamp":"2026-06-25T23:10:00.000Z","type":"event_msg","payload":{"type":"token_count","model":"gpt-5.1-codex","info":{"total_token_usage":{"input_tokens":20,"cached_input_tokens":0,"output_tokens":8,"total_tokens":28}}}}"#;
        std::fs::write(&path, format!("{meta}\n{older}\n{newer}")).unwrap();

        let mut cache = crate::cache::ScanCache::new();
        let mut models = std::collections::HashMap::new();
        let _ = scan(&dir, &mut cache, chrono::Utc::now(), &mut models);

        assert_eq!(
            models
                .get("019f00fb-40b5-7192-9b79-aa6d1034fe1b")
                .map(String::as_str),
            Some("gpt-5.1-codex")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
