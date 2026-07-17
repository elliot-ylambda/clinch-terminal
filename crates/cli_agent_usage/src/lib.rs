use std::collections::HashMap;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub mod cache;
pub mod claude;
pub mod claude_plan_cache;
pub mod codex;
pub mod format;
pub mod http;
pub mod keychain;
pub mod pricing;
pub mod snapshot_cache;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    #[default]
    Normal,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LimitWindow {
    pub percent: f64,
    pub resets_at: Option<DateTime<Utc>>,
    pub severity: Severity,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct PlanLimits {
    pub session: Option<LimitWindow>,
    pub weekly: Option<LimitWindow>,
    /// Fable's model-scoped weekly limit, when reported by Claude's usage API.
    pub fable_weekly: Option<LimitWindow>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlanFetchOutcome {
    Success(PlanLimits),
    RateLimited(StdDuration),
    Unauthorized,
    Unavailable,
}

/// A usage window counts as exhausted only at (or above) full utilization.
/// Near-limit states (severity `Warning`/`Critical`) intentionally do NOT
/// qualify: exhaustion drives the chip's reset countdown and the
/// auto-continue scheduler, both of which must react only to a hard stop.
const EXHAUSTED_PERCENT: f64 = 100.0;

impl PlanLimits {
    /// When at least one provider-wide window is exhausted (percent >= 100),
    /// returns the instant at which *every* exhausted window will have reset
    /// (the latest of their reset times). Returns `None` when no window is
    /// exhausted, or when any exhausted window's reset time is unknown —
    /// callers must treat "don't know when" as "not schedulable" rather than
    /// guessing. Model-scoped limits such as Fable are intentionally excluded:
    /// exhausting one must not pause sessions using another Claude model.
    pub fn exhausted_until(&self) -> Option<DateTime<Utc>> {
        let mut latest: Option<DateTime<Utc>> = None;
        for window in [self.session, self.weekly].into_iter().flatten() {
            if window.percent >= EXHAUSTED_PERCENT {
                let resets_at = window.resets_at?;
                latest = Some(latest.map_or(resets_at, |current| current.max(resets_at)));
            }
        }
        latest
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Provider {
    pub session: WindowTotals,
    pub today: WindowTotals,
    pub week: WindowTotals,
    pub month: WindowTotals,
    pub plan: Option<PlanLimits>,
    /// Claude only: the plan gauges are enabled, but reading the Keychain
    /// token would raise the macOS credential prompt (the item's ACL no
    /// longer trusts a silent read). The poller sets this instead of reading;
    /// the UI offers an explicit Authorize gesture that sanctions the prompt.
    #[serde(default)]
    pub plan_needs_authorization: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageSnapshot {
    pub claude: Provider,
    pub codex: Provider,
    /// `session_id` → latest model id seen for that session. Built during the
    /// same local file walk that produces `claude`/`codex`. Empty when neither
    /// provider has local data.
    pub models_by_session: HashMap<String, String>,
}

impl UsageSnapshot {
    /// The latest model id recorded for `session_id`, if any.
    pub fn model_for_session(&self, session_id: &str) -> Option<&str> {
        self.models_by_session.get(session_id).map(String::as_str)
    }
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
    /// Where the last emitted snapshot is cached across launches (see
    /// [`snapshot_cache`]). Deliberately channel-agnostic (`~/.warp`): the
    /// scanned transcript dirs are per-`$HOME`, not per-channel, so a dev
    /// build's first launch can reuse the prod build's cache and vice versa.
    pub snapshot_cache: PathBuf,
}

impl Paths {
    pub fn detect() -> Option<Paths> {
        let home = std::env::var("HOME").ok()?;
        let os_account = std::env::var("USER").unwrap_or_default();
        Some(Paths {
            claude_projects: PathBuf::from(&home).join(".claude/projects"),
            codex_sessions: PathBuf::from(&home).join(".codex/sessions"),
            os_account,
            snapshot_cache: PathBuf::from(&home).join(".warp/cli-agent-usage-snapshot.json"),
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
    let mut models_by_session = HashMap::new();
    let claude = claude::scan(
        &paths.claude_projects,
        &mut caches.claude,
        now,
        &mut models_by_session,
    );
    let codex = codex::scan(
        &paths.codex_sessions,
        &mut caches.codex,
        now,
        &mut models_by_session,
    );
    UsageSnapshot {
        claude,
        codex,
        models_by_session,
    }
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
    fetch_plan_for_token(fetch, &token, now.timestamp_millis())
}

/// The HTTP half of a Claude plan fetch: given an already-obtained token, fetch
/// and parse `/api/oauth/usage`. Returns `None` if the token is expired or any
/// step fails.
///
/// Split out from [`fetch_claude_plan`] so a poller can read the Keychain token
/// **once**, cache it, and reuse it across many fetches — the Keychain read is
/// what triggers the macOS "allow access" prompt, so re-reading it on every poll
/// pesters the user. See `CliAgentUsageModel::producer_loop`.
///
/// **Blocking** (a blocking HTTP call). Call only from a dedicated thread, never
/// a Tokio/async runtime.
pub fn fetch_plan_for_token(
    fetch: &dyn http::FetchUsage,
    token: &keychain::ClaudeToken,
    now_ms: i64,
) -> Option<PlanLimits> {
    match fetch_plan_for_token_outcome(fetch, token, now_ms) {
        PlanFetchOutcome::Success(plan) => Some(plan),
        PlanFetchOutcome::RateLimited(_)
        | PlanFetchOutcome::Unauthorized
        | PlanFetchOutcome::Unavailable => None,
    }
}

/// Detailed form of [`fetch_plan_for_token`] used by the live poller. In
/// particular, it preserves the endpoint's rate-limit delay so every Clinch
/// process can share and honor the same retry deadline.
pub fn fetch_plan_for_token_outcome(
    fetch: &dyn http::FetchUsage,
    token: &keychain::ClaudeToken,
    now_ms: i64,
) -> PlanFetchOutcome {
    if token.is_expired(now_ms) {
        return PlanFetchOutcome::Unavailable;
    }
    let body = match fetch.fetch(&token.access_token) {
        Ok(body) => body,
        Err(error) => {
            return match error.kind() {
                http::FetchErrorKind::Unauthorized => PlanFetchOutcome::Unauthorized,
                http::FetchErrorKind::RateLimited => error
                    .retry_after()
                    .map(PlanFetchOutcome::RateLimited)
                    .unwrap_or(PlanFetchOutcome::Unavailable),
                http::FetchErrorKind::Other => PlanFetchOutcome::Unavailable,
            };
        }
    };
    http::parse_plan_limits(&body)
        .map(PlanFetchOutcome::Success)
        .unwrap_or(PlanFetchOutcome::Unavailable)
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
    fn exhausted_until_none_when_no_window_is_full() {
        use chrono::{Duration, Utc};
        let reset = Utc::now() + Duration::hours(1);
        let window = |percent| LimitWindow {
            percent,
            resets_at: Some(reset),
            severity: Severity::Critical,
        };
        // Near-limit (even Critical severity) is NOT exhausted.
        let plan = PlanLimits {
            session: Some(window(99.9)),
            weekly: Some(window(95.0)),
            fable_weekly: None,
        };
        assert_eq!(plan.exhausted_until(), None);
        assert_eq!(PlanLimits::default().exhausted_until(), None);
    }

    #[test]
    fn exhausted_until_uses_the_latest_reset_of_all_exhausted_windows() {
        use chrono::{Duration, Utc};
        let now = Utc::now();
        let session_reset = now + Duration::hours(2);
        let weekly_reset = now + Duration::days(3);
        let window = |percent, resets_at| LimitWindow {
            percent,
            resets_at,
            severity: Severity::Critical,
        };

        // Only the session window exhausted -> its reset.
        let plan = PlanLimits {
            session: Some(window(100.0, Some(session_reset))),
            weekly: Some(window(60.0, Some(weekly_reset))),
            fable_weekly: None,
        };
        assert_eq!(plan.exhausted_until(), Some(session_reset));

        // Both exhausted -> the later (weekly) reset: continuing before it
        // would still be blocked.
        let plan = PlanLimits {
            session: Some(window(101.0, Some(session_reset))),
            weekly: Some(window(100.0, Some(weekly_reset))),
            fable_weekly: None,
        };
        assert_eq!(plan.exhausted_until(), Some(weekly_reset));
    }

    #[test]
    fn exhausted_until_none_when_any_exhausted_window_lacks_a_reset_time() {
        use chrono::{Duration, Utc};
        let reset = Utc::now() + Duration::hours(2);
        let window = |percent, resets_at| LimitWindow {
            percent,
            resets_at,
            severity: Severity::Critical,
        };

        // The only exhausted window has no reset time -> unknowable.
        let plan = PlanLimits {
            session: Some(window(100.0, None)),
            weekly: Some(window(40.0, Some(reset))),
            fable_weekly: None,
        };
        assert_eq!(plan.exhausted_until(), None);

        // One exhausted window is known but another is not -> still None
        // (we cannot know when usage actually becomes available again).
        let plan = PlanLimits {
            session: Some(window(100.0, Some(reset))),
            weekly: Some(window(100.0, None)),
            fable_weekly: None,
        };
        assert_eq!(plan.exhausted_until(), None);
    }

    #[test]
    fn exhausted_until_ignores_model_scoped_fable_limit() {
        use chrono::{Duration, Utc};
        let reset = Utc::now() + Duration::days(3);
        let plan = PlanLimits {
            session: None,
            weekly: None,
            fable_weekly: Some(LimitWindow {
                percent: 100.0,
                resets_at: Some(reset),
                severity: Severity::Critical,
            }),
        };

        assert_eq!(plan.exhausted_until(), None);
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
            fn fetch(&self, _: &str) -> Result<String, crate::http::FetchError> {
                Err(crate::http::FetchError::other("no"))
            }
        }
        let paths = Paths {
            claude_projects: "/no/such/claude".into(),
            codex_sessions: "/no/such/codex".into(),
            os_account: "nobody".into(),
            snapshot_cache: "/no/such/snapshot.json".into(),
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
            fn fetch(&self, _: &str) -> Result<String, crate::http::FetchError> {
                self.0
                    .map(|s| s.to_string())
                    .map_err(crate::http::FetchError::other)
            }
        }

        // never-expiring token blob; valid usage JSON (limits[] preferred path)
        let blob = r#"{"claudeAiOauth":{"accessToken":"tok","expiresAt":99999999999999}}"#;
        let usage = r#"{"limits":[{"group":"session","percent":78,"severity":"warning","resets_at":"2026-07-01T02:30:00+00:00","is_active":true},{"group":"weekly","percent":43,"severity":"normal","resets_at":"2026-07-04T15:00:00+00:00","is_active":false}]}"#;
        let paths = Paths {
            claude_projects: "/no/such/claude".into(),
            codex_sessions: "/no/such/codex".into(),
            os_account: "u".into(),
            snapshot_cache: "/no/such/snapshot.json".into(),
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
    fn fetch_plan_for_token_uses_cached_token_and_guards_expiry() {
        use crate::http::FetchUsage;
        use crate::keychain::ClaudeToken;

        struct Fetch(Result<&'static str, &'static str>);
        impl FetchUsage for Fetch {
            fn fetch(&self, _: &str) -> Result<String, crate::http::FetchError> {
                self.0
                    .map(|s| s.to_string())
                    .map_err(crate::http::FetchError::other)
            }
        }
        let usage = r#"{"limits":[{"group":"session","percent":78,"severity":"warning","resets_at":"2026-07-01T02:30:00+00:00","is_active":true}]}"#;

        let valid = ClaudeToken {
            access_token: "tok".to_string(),
            expires_at_ms: Some(10_000),
        };
        // Valid token + good body -> plan (no Keychain read involved).
        let plan = fetch_plan_for_token(&Fetch(Ok(usage)), &valid, 5_000).expect("plan");
        assert_eq!(plan.session.unwrap().percent, 78.0);

        // Expired token short-circuits before the fetch.
        assert!(fetch_plan_for_token(&Fetch(Ok(usage)), &valid, 10_000).is_none());

        // Fetch error / garbage body -> None (fail-soft).
        assert!(fetch_plan_for_token(&Fetch(Err("boom")), &valid, 5_000).is_none());
        assert!(fetch_plan_for_token(&Fetch(Ok("garbage")), &valid, 5_000).is_none());
    }

    #[test]
    fn detailed_plan_fetch_preserves_rate_limit_and_auth_failures() {
        use crate::http::{FetchError, FetchUsage};
        use crate::keychain::ClaudeToken;

        struct Limited;
        impl FetchUsage for Limited {
            fn fetch(&self, _: &str) -> Result<String, FetchError> {
                Err(FetchError::rate_limited(Some(StdDuration::from_secs(
                    2_100,
                ))))
            }
        }
        struct Unauthorized;
        impl FetchUsage for Unauthorized {
            fn fetch(&self, _: &str) -> Result<String, FetchError> {
                Err(FetchError::unauthorized())
            }
        }

        let token = ClaudeToken {
            access_token: "tok".to_string(),
            expires_at_ms: Some(10_000),
        };
        assert_eq!(
            fetch_plan_for_token_outcome(&Limited, &token, 5_000),
            PlanFetchOutcome::RateLimited(StdDuration::from_secs(2_100))
        );
        assert_eq!(
            fetch_plan_for_token_outcome(&Unauthorized, &token, 5_000),
            PlanFetchOutcome::Unauthorized
        );
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
            snapshot_cache: "/no/such/snapshot.json".into(),
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
            fn fetch(&self, _: &str) -> Result<String, crate::http::FetchError> {
                Ok(r#"{"limits":[{"kind":"weekly_all","group":"weekly","percent":47,"severity":"normal","resets_at":"2026-07-04T15:00:00+00:00","is_active":true}]}"#.to_string())
            }
        }
        let paths = Paths {
            claude_projects: "/x".into(),
            codex_sessions: "/x".into(),
            os_account: "me".into(),
            snapshot_cache: "/no/such/snapshot.json".into(),
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
