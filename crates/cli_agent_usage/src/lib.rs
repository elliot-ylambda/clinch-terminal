use chrono::{DateTime, Utc};

pub mod cache;
pub mod claude;
pub mod codex;
pub mod format;
pub mod http;
pub mod keychain;
pub mod pricing;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenCounts {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl TokenCounts {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }
    /// Input + output tokens — the "work" total, excluding cache traffic.
    /// This is the headline metric for the footer (cache-read dominates
    /// `total()` and would mislead).
    pub fn io(&self) -> u64 {
        self.input + self.output
    }
    pub fn add(&mut self, o: &TokenCounts) {
        self.input += o.input;
        self.output += o.output;
        self.cache_read += o.cache_read;
        self.cache_write += o.cache_write;
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct WindowTotals {
    pub tokens: TokenCounts,
    pub cost_usd: f64,
}

impl WindowTotals {
    pub fn add_entry(&mut self, e: &Entry) {
        self.tokens.add(&e.tokens);
        self.cost_usd += crate::pricing::cost(&e.model, &e.tokens);
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Severity {
    #[default]
    Normal,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LimitWindow {
    pub percent: f64,
    pub resets_at: Option<DateTime<Utc>>,
    pub severity: Severity,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PlanLimits {
    pub session: Option<LimitWindow>,
    pub weekly: Option<LimitWindow>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Provider {
    pub session: WindowTotals,
    pub today: WindowTotals,
    pub week: WindowTotals,
    pub month: WindowTotals,
    pub plan: Option<PlanLimits>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageSnapshot {
    pub claude: Provider,
    pub codex: Provider,
}

/// One billable event extracted from a transcript/rollout, timezone-normalized to UTC.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub ts: DateTime<Utc>,
    pub model: String,
    pub tokens: TokenCounts,
    pub dedup: String,
}

/// Bucket entries into (today, week, month) against `now`, deduping by `Entry::dedup`.
pub fn aggregate_windows(
    entries: &[Entry],
    now: DateTime<Utc>,
    seen: &mut std::collections::HashSet<String>,
    today: &mut WindowTotals,
    week: &mut WindowTotals,
    month: &mut WindowTotals,
) {
    use chrono::{Local, TimeZone};
    let midnight_local = Local
        .from_local_datetime(
            &now.with_timezone(&Local)
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .expect("valid midnight"),
        )
        .single()
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or(now);
    let week_ago = now - chrono::Duration::days(7);
    let month_ago = now - chrono::Duration::days(30);

    for e in entries {
        if !e.dedup.is_empty() && !seen.insert(e.dedup.clone()) {
            continue;
        }
        if e.ts >= midnight_local {
            today.add_entry(e);
        }
        if e.ts >= week_ago {
            week.add_entry(e);
        }
        if e.ts >= month_ago {
            month.add_entry(e);
        }
    }
}

use std::path::PathBuf;

use crate::cache::ScanCache;

pub struct Paths {
    pub claude_projects: PathBuf,
    pub codex_sessions: PathBuf,
    pub os_account: String,
}

impl Paths {
    pub fn detect() -> Option<Paths> {
        let home = std::env::var("HOME").ok()?;
        let os_account = std::env::var("USER").unwrap_or_default();
        Some(Paths {
            claude_projects: PathBuf::from(&home).join(".claude/projects"),
            codex_sessions: PathBuf::from(&home).join(".codex/sessions"),
            os_account,
        })
    }
}

pub struct Caches {
    claude: ScanCache<Vec<Entry>>,
    codex: ScanCache<codex::RollupFile>,
}

impl Caches {
    pub fn new() -> Self {
        Caches {
            claude: ScanCache::new(),
            codex: ScanCache::new(),
        }
    }
}

impl Default for Caches {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a full [`UsageSnapshot`]. Every source is independent and fail-soft:
/// a missing dir, corrupt line, absent/expired token, or HTTP error yields
/// empty/`None` for that slice and never panics or aborts the others.
///
/// **Blocking and NOT async-safe.** This does synchronous file IO and, via
/// [`http::ReqwestUsage`], a *blocking* HTTP call. `reqwest::blocking` panics if
/// constructed inside a Tokio/async runtime, so a footer/poller (Plan B) MUST run
/// `refresh` on a dedicated thread (e.g. `spawn_blocking`), never on the async
/// runtime or the UI thread. Recommended: poll local scans frequently, the usage
/// endpoint slowly, and retain the last good `PlanLimits` across transient fetch
/// failures so plan-% does not flicker to `None`.
pub fn refresh(
    paths: &Paths,
    caches: &mut Caches,
    now: DateTime<Utc>,
    secret: &dyn keychain::ReadSecret,
    fetch: &dyn http::FetchUsage,
) -> UsageSnapshot {
    let mut snap = scan_local(paths, caches, now);
    snap.claude.plan = fetch_claude_plan(secret, fetch, paths, now);
    snap
}

/// Scan both providers' local files into a snapshot. No network, no Keychain:
/// `claude.plan` is always `None` (fetch it separately via [`fetch_claude_plan`]);
/// `codex.plan` is populated from local rate-limit events. Fail-soft.
pub fn scan_local(paths: &Paths, caches: &mut Caches, now: DateTime<Utc>) -> UsageSnapshot {
    let claude = claude::scan(&paths.claude_projects, &mut caches.claude, now);
    let codex = codex::scan(&paths.codex_sessions, &mut caches.codex, now);
    UsageSnapshot { claude, codex }
}

/// The Claude plan-% half of a refresh: read the Keychain token, and if present
/// and unexpired, fetch and parse `/api/oauth/usage`. Best-effort — any failure
/// (no token, expired, network, parse) yields `None`.
///
/// **Blocking** (Keychain + a blocking HTTP call). Call only from a dedicated
/// thread, never a Tokio/async runtime.
pub fn fetch_claude_plan(
    secret: &dyn keychain::ReadSecret,
    fetch: &dyn http::FetchUsage,
    paths: &Paths,
    now: DateTime<Utc>,
) -> Option<PlanLimits> {
    let token = keychain::read_claude_token(secret, &paths.os_account)?;
    if token.is_expired(now.timestamp_millis()) {
        return None;
    }
    let body = fetch.fetch(&token.access_token).ok()?;
    http::parse_plan_limits(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_counts_total_and_add() {
        let mut a = TokenCounts {
            input: 10,
            output: 5,
            cache_read: 1,
            cache_write: 2,
        };
        assert_eq!(a.total(), 18);
        a.add(&TokenCounts {
            input: 1,
            output: 1,
            cache_read: 0,
            cache_write: 0,
        });
        assert_eq!(a.total(), 20);
    }

    #[test]
    fn severity_default_is_normal() {
        assert_eq!(Severity::default(), Severity::Normal);
    }

    #[test]
    fn refresh_is_fail_soft_with_no_files_and_no_token() {
        struct NoSecret;
        impl crate::keychain::ReadSecret for NoSecret {
            fn read(&self, _: &str, _: &str) -> Option<String> {
                None
            }
        }
        struct NoFetch;
        impl crate::http::FetchUsage for NoFetch {
            fn fetch(&self, _: &str) -> Result<String, String> {
                Err("no".into())
            }
        }
        let paths = Paths {
            claude_projects: "/no/such/claude".into(),
            codex_sessions: "/no/such/codex".into(),
            os_account: "nobody".into(),
        };
        let mut caches = Caches::new();
        let snap = refresh(&paths, &mut caches, chrono::Utc::now(), &NoSecret, &NoFetch);
        assert_eq!(snap.claude.month.tokens.total(), 0);
        assert!(snap.claude.plan.is_none());
        assert_eq!(snap.codex.month.tokens.total(), 0);
    }

    #[test]
    fn refresh_claude_plan_success_and_failure_branches() {
        use crate::http::FetchUsage;
        use crate::keychain::ReadSecret;

        struct Secret(&'static str);
        impl ReadSecret for Secret {
            fn read(&self, _: &str, _: &str) -> Option<String> {
                Some(self.0.to_string())
            }
        }
        struct Fetch(Result<&'static str, &'static str>);
        impl FetchUsage for Fetch {
            fn fetch(&self, _: &str) -> Result<String, String> {
                self.0.map(|s| s.to_string()).map_err(|e| e.to_string())
            }
        }

        // never-expiring token blob; valid usage JSON (limits[] preferred path)
        let blob = r#"{"claudeAiOauth":{"accessToken":"tok","expiresAt":99999999999999}}"#;
        let usage = r#"{"limits":[{"group":"session","percent":78,"severity":"warning","resets_at":"2026-07-01T02:30:00+00:00","is_active":true},{"group":"weekly","percent":43,"severity":"normal","resets_at":"2026-07-04T15:00:00+00:00","is_active":false}]}"#;
        let paths = Paths {
            claude_projects: "/no/such/claude".into(),
            codex_sessions: "/no/such/codex".into(),
            os_account: "u".into(),
        };
        let now = chrono::Utc::now();

        // success: valid token + valid usage -> plan populated with expected percentages
        let mut caches = Caches::new();
        let snap = refresh(&paths, &mut caches, now, &Secret(blob), &Fetch(Ok(usage)));
        let plan = snap.claude.plan.expect("plan populated on success");
        assert_eq!(plan.session.unwrap().percent, 78.0);
        assert_eq!(plan.weekly.unwrap().percent, 43.0);

        // fetch error -> plan None (fail-soft)
        let mut caches = Caches::new();
        let snap = refresh(&paths, &mut caches, now, &Secret(blob), &Fetch(Err("boom")));
        assert!(snap.claude.plan.is_none());

        // malformed body -> plan None (fail-soft)
        let mut caches = Caches::new();
        let snap = refresh(
            &paths,
            &mut caches,
            now,
            &Secret(blob),
            &Fetch(Ok("garbage")),
        );
        assert!(snap.claude.plan.is_none());

        // expired token -> plan None (short-circuits before fetch)
        let expired = r#"{"claudeAiOauth":{"accessToken":"tok","expiresAt":1}}"#;
        let mut caches = Caches::new();
        let snap = refresh(
            &paths,
            &mut caches,
            now,
            &Secret(expired),
            &Fetch(Ok(usage)),
        );
        assert!(snap.claude.plan.is_none());
    }

    #[test]
    fn token_counts_io_is_input_plus_output_only() {
        let t = TokenCounts {
            input: 10,
            output: 5,
            cache_read: 100,
            cache_write: 7,
        };
        assert_eq!(t.io(), 15);
        // io() must NOT include cache traffic (unlike total()).
        assert_eq!(t.total(), 122);
    }

    #[test]
    fn scan_local_is_fail_soft_and_leaves_claude_plan_none() {
        let paths = Paths {
            claude_projects: "/no/such/claude".into(),
            codex_sessions: "/no/such/codex".into(),
            os_account: "nobody".into(),
        };
        let mut caches = Caches::new();
        let snap = scan_local(&paths, &mut caches, chrono::Utc::now());
        assert_eq!(snap.claude.month.tokens.total(), 0);
        assert_eq!(snap.codex.month.tokens.total(), 0);
        // scan_local never touches Keychain/HTTP, so Claude plan is always None here.
        assert!(snap.claude.plan.is_none());
    }

    #[test]
    fn fetch_claude_plan_none_without_token_and_some_with_valid_body() {
        use crate::http::FetchUsage;
        use crate::keychain::ReadSecret;

        struct NoSecret;
        impl ReadSecret for NoSecret {
            fn read(&self, _: &str, _: &str) -> Option<String> {
                None
            }
        }
        struct Secret;
        impl ReadSecret for Secret {
            fn read(&self, _: &str, _: &str) -> Option<String> {
                // Non-expired token blob (expiresAt far in the future).
                Some(
                    r#"{"claudeAiOauth":{"accessToken":"tok","expiresAt":95617584000000}}"#
                        .to_string(),
                )
            }
        }
        struct Fetch;
        impl FetchUsage for Fetch {
            fn fetch(&self, _: &str) -> Result<String, String> {
                Ok(r#"{"limits":[{"kind":"weekly_all","group":"weekly","percent":47,"severity":"normal","resets_at":"2026-07-04T15:00:00+00:00","is_active":true}]}"#.to_string())
            }
        }
        let paths = Paths {
            claude_projects: "/x".into(),
            codex_sessions: "/x".into(),
            os_account: "me".into(),
        };
        let now = chrono::Utc::now();
        assert!(fetch_claude_plan(&NoSecret, &Fetch, &paths, now).is_none());
        let plan = fetch_claude_plan(&Secret, &Fetch, &paths, now).expect("valid plan");
        assert!(plan.weekly.is_some());
    }

    #[test]
    fn aggregate_windows_week_month_boundaries_and_dedup() {
        use chrono::{Duration, Utc};
        let now = Utc::now();
        let mk = |ts, tag: &str, out: u64| Entry {
            ts,
            model: "claude-haiku".to_string(),
            tokens: TokenCounts {
                input: 0,
                output: out,
                cache_read: 0,
                cache_write: 0,
            },
            dedup: tag.to_string(),
        };
        let entries = vec![
            mk(now - Duration::days(3), "3d", 20), // in week & month
            mk(now - Duration::days(7) + Duration::minutes(1), "wk_in", 5), // just inside 7d
            mk(now - Duration::days(8), "8d", 40), // month only (outside week)
            mk(now - Duration::days(40), "40d", 80), // outside all windows
            mk(now - Duration::days(3), "3d", 20), // duplicate dedup key -> ignored
        ];
        let mut seen = std::collections::HashSet::new();
        let (mut today, mut week, mut month) = Default::default();
        aggregate_windows(&entries, now, &mut seen, &mut today, &mut week, &mut month);
        // dup "3d" counted once; "wk_in" inside 7d; "8d"/"40d" outside week
        assert_eq!(week.tokens.output, 20 + 5);
        // + "8d" inside 30d; "40d" still excluded
        assert_eq!(month.tokens.output, 20 + 5 + 40);
        let _ = today; // `today` uses LOCAL midnight (tz-dependent) — covered via scan tests, not asserted here
    }
}
