//! Cross-process cache and throttle for Claude's live plan limits.
//!
//! Stable Clinch and ClinchDev can run at the same time. Without coordination,
//! each process polls Anthropic independently and one can remain permanently
//! rate-limited. A small sibling cache, guarded by `flock`, makes the machine's
//! Clinch processes share one request cadence and one last-known-good plan.

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::{retain_current_windows, PlanFetchOutcome, PlanLimits};

const VERSION: u32 = 1;

/// Minimum cadence for fresh requests to Claude's usage endpoint. Consumers
/// that wait for a post-stop usage confirmation must allow at least this long.
pub const MIN_ATTEMPT_INTERVAL_SECS: i64 = 5 * 60;
/// The floor when the user just opened the usage panel.
const LOOKING_MIN_ATTEMPT_INTERVAL_SECS: i64 = 60;

/// Plan percentages move much faster than local token totals. Keep a recent
/// result across transient failures, but hide it after an hour rather than
/// presenting an old percentage as live data.
fn max_plan_age() -> Duration {
    Duration::hours(1)
}

#[derive(Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    attempted_at: DateTime<Utc>,
    #[serde(default)]
    retry_at: Option<DateTime<Utc>>,
    fetched_at: Option<DateTime<Utc>>,
    plan: Option<PlanLimits>,
    /// The account this describes. Another account's limits and rate limiting don't apply.
    #[serde(default)]
    account: Option<String>,
}

#[derive(Clone, Copy)]
struct CachedPlan {
    attempted_at: DateTime<Utc>,
    retry_at: Option<DateTime<Utc>>,
    fetched_at: Option<DateTime<Utc>>,
    plan: Option<PlanLimits>,
}

/// What this refresh knows beyond the clock.
#[derive(Clone, Copy, Default)]
pub struct RefreshPolicy<'a> {
    /// The signed-in account, when known. A cache entry from a different account is ignored.
    pub account: Option<&'a str>,
    /// An explicit Retry just got a token. A prior token-less or unauthorized attempt may have left
    /// an empty entry inside the normal throttle; retry now in that one case, so clicking Retry
    /// doesn't appear to do nothing.
    pub retry_empty: bool,
    /// The user just opened the usage panel, so a shorter throttle applies.
    pub user_looking: bool,
}

fn cache_paths(snapshot_cache: &Path) -> Option<(PathBuf, PathBuf)> {
    let dir = snapshot_cache.parent()?;
    Some((
        dir.join("cli-agent-usage-claude-plan.json"),
        dir.join(".cli-agent-usage-claude-plan.lock"),
    ))
}

fn load(path: &Path, now: DateTime<Utc>, account: Option<&str>) -> Option<CachedPlan> {
    let file = serde_json::from_str::<CacheFile>(&fs::read_to_string(path).ok()?).ok()?;
    if file.version != VERSION
        || file.attempted_at > now
        || account.is_some_and(|account| file.account.as_deref() != Some(account))
    {
        return None;
    }

    let plan_is_fresh = file.fetched_at.is_some_and(|fetched_at| {
        let age = now.signed_duration_since(fetched_at);
        age >= Duration::zero() && age <= max_plan_age()
    });

    Some(CachedPlan {
        attempted_at: file.attempted_at,
        retry_at: file.retry_at.filter(|retry_at| *retry_at > now),
        fetched_at: if plan_is_fresh { file.fetched_at } else { None },
        plan: if plan_is_fresh {
            file.plan.and_then(|plan| retain_current_windows(plan, now))
        } else {
            None
        },
    })
}

fn store(path: &Path, cached: CachedPlan, account: Option<&str>) {
    let file = CacheFile {
        version: VERSION,
        attempted_at: cached.attempted_at,
        retry_at: cached.retry_at,
        fetched_at: cached.fetched_at,
        plan: cached.plan,
        account: account.map(str::to_owned),
    };
    let Ok(contents) = serde_json::to_string(&file) else {
        return;
    };
    let Some(dir) = path.parent() else { return };
    if fs::create_dir_all(dir).is_err() {
        return;
    }
    let temp = dir.join(format!(
        ".{}.tmp.{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("claude-plan"),
        std::process::id()
    ));
    if fs::write(&temp, contents).is_err() {
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&temp, fs::Permissions::from_mode(0o600));
    }
    if fs::rename(&temp, path).is_err() {
        let _ = fs::remove_file(&temp);
    }
}

struct CacheLock {
    #[cfg(unix)]
    file: File,
}

impl CacheLock {
    fn acquire(path: &Path) -> Option<Self> {
        let dir = path.parent()?;
        fs::create_dir_all(dir).ok()?;

        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            use std::os::unix::fs::OpenOptionsExt;

            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .open(path)
                .ok()?;
            // SAFETY: `file` owns a valid descriptor for the lifetime of the
            // guard, and `LOCK_EX` is a valid flock operation.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
                return None;
            }
            Some(Self { file })
        }

        #[cfg(not(unix))]
        {
            let _ = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .open(path)
                .ok()?;
            Some(Self {})
        }
    }
}

#[cfg(unix)]
impl Drop for CacheLock {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;

        // SAFETY: the descriptor remains valid until after `drop` returns.
        let _ = unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
    }
}

/// Return a shared recent plan, fetching at most once per five minutes, or per
/// minute when the user just opened the usage panel, across every Clinch
/// process using this snapshot-cache directory.
///
/// `fetch` runs while the interprocess lock is held, so a second process waits
/// for the first request and then consumes its result instead of making a
/// duplicate request. A rate-limited response persists the server's retry
/// deadline, and a failed request retains only a recent, not-yet-reset plan.
pub fn refresh_shared(
    snapshot_cache: &Path,
    now: DateTime<Utc>,
    policy: RefreshPolicy<'_>,
    fetch: impl FnOnce() -> PlanFetchOutcome,
) -> Option<PlanLimits> {
    let (cache_path, lock_path) = cache_paths(snapshot_cache)?;
    let Some(_lock) = CacheLock::acquire(&lock_path) else {
        return load(&cache_path, now, policy.account).and_then(|cached| cached.plan);
    };

    let previous = load(&cache_path, now, policy.account);
    let min_interval = Duration::seconds(if policy.user_looking {
        LOOKING_MIN_ATTEMPT_INTERVAL_SECS
    } else {
        MIN_ATTEMPT_INTERVAL_SECS
    });
    if previous.is_some_and(|cached| {
        cached.retry_at.is_some_and(|retry_at| retry_at > now)
            || (now.signed_duration_since(cached.attempted_at) < min_interval
                && !(policy.retry_empty && cached.retry_at.is_none() && cached.plan.is_none()))
    }) {
        return previous.and_then(|cached| cached.plan);
    }

    let fresh = fetch();
    let cached = match fresh {
        PlanFetchOutcome::Success(plan) => CachedPlan {
            attempted_at: now,
            retry_at: None,
            fetched_at: Some(now),
            plan: retain_current_windows(plan, now),
        },
        PlanFetchOutcome::RateLimited(retry_after) => CachedPlan {
            attempted_at: now,
            retry_at: Duration::from_std(retry_after)
                .ok()
                .and_then(|delay| now.checked_add_signed(delay)),
            fetched_at: previous.and_then(|cached| cached.fetched_at),
            plan: previous.and_then(|cached| cached.plan),
        },
        PlanFetchOutcome::Unauthorized | PlanFetchOutcome::Unavailable => CachedPlan {
            attempted_at: now,
            retry_at: None,
            fetched_at: previous.and_then(|cached| cached.fetched_at),
            plan: previous.and_then(|cached| cached.plan),
        },
    };
    store(&cache_path, cached, policy.account);
    cached.plan
}

#[cfg(test)]
#[path = "claude_plan_cache_tests.rs"]
mod tests;
