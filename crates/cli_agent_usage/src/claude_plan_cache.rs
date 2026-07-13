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

use crate::PlanLimits;

const VERSION: u32 = 1;

/// Anthropic's usage endpoint is rate-limited. This is a wall-clock throttle,
/// independent of how long transcript scans take.
fn min_attempt_interval() -> Duration {
    Duration::minutes(1)
}

/// Match the snapshot cache's stale-while-revalidate horizon. A failed refresh
/// can retain a recent plan, but not one old enough to be actively misleading.
fn max_plan_age() -> Duration {
    Duration::hours(48)
}

#[derive(Clone, Copy, Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    attempted_at: DateTime<Utc>,
    fetched_at: Option<DateTime<Utc>>,
    plan: Option<PlanLimits>,
}

#[derive(Clone, Copy)]
struct CachedPlan {
    attempted_at: DateTime<Utc>,
    fetched_at: Option<DateTime<Utc>>,
    plan: Option<PlanLimits>,
}

fn cache_paths(snapshot_cache: &Path) -> Option<(PathBuf, PathBuf)> {
    let dir = snapshot_cache.parent()?;
    Some((
        dir.join("cli-agent-usage-claude-plan.json"),
        dir.join(".cli-agent-usage-claude-plan.lock"),
    ))
}

fn load(path: &Path, now: DateTime<Utc>) -> Option<CachedPlan> {
    let file = serde_json::from_str::<CacheFile>(&fs::read_to_string(path).ok()?).ok()?;
    if file.version != VERSION || file.attempted_at > now {
        return None;
    }

    let plan_is_fresh = file.fetched_at.is_some_and(|fetched_at| {
        let age = now.signed_duration_since(fetched_at);
        age >= Duration::zero() && age <= max_plan_age()
    });

    Some(CachedPlan {
        attempted_at: file.attempted_at,
        fetched_at: if plan_is_fresh { file.fetched_at } else { None },
        plan: if plan_is_fresh { file.plan } else { None },
    })
}

fn store(path: &Path, cached: CachedPlan) {
    let file = CacheFile {
        version: VERSION,
        attempted_at: cached.attempted_at,
        fetched_at: cached.fetched_at,
        plan: cached.plan,
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

/// Return a shared recent plan, fetching at most once per minute across every
/// Clinch process using this snapshot-cache directory.
///
/// `fetch` runs while the interprocess lock is held, so a second process waits
/// for the first request and then consumes its result instead of making a
/// duplicate request. A failed request records the attempt and retains a recent
/// last-known-good plan.
pub fn refresh_shared(
    snapshot_cache: &Path,
    now: DateTime<Utc>,
    fetch: impl FnOnce() -> Option<PlanLimits>,
) -> Option<PlanLimits> {
    let (cache_path, lock_path) = cache_paths(snapshot_cache)?;
    let Some(_lock) = CacheLock::acquire(&lock_path) else {
        return load(&cache_path, now).and_then(|cached| cached.plan);
    };

    let previous = load(&cache_path, now);
    if previous.is_some_and(|cached| {
        now.signed_duration_since(cached.attempted_at) < min_attempt_interval()
    }) {
        return previous.and_then(|cached| cached.plan);
    }

    let fresh = fetch();
    let cached = match fresh {
        Some(plan) => CachedPlan {
            attempted_at: now,
            fetched_at: Some(now),
            plan: Some(plan),
        },
        None => CachedPlan {
            attempted_at: now,
            fetched_at: previous.and_then(|cached| cached.fetched_at),
            plan: previous.and_then(|cached| cached.plan),
        },
    };
    store(&cache_path, cached);
    cached.plan
}

#[cfg(test)]
#[path = "claude_plan_cache_tests.rs"]
mod tests;
