//! Disk cache of the last emitted [`UsageSnapshot`], so a fresh app launch
//! shows the usage widget immediately (stale-while-revalidate) instead of
//! hiding it until the first cold scan of the transcript dirs — which can take
//! tens of seconds — completes. Everything here is best-effort: a missing,
//! unreadable, or incompatible cache simply yields `None`/no-op, and the live
//! scan overwrites the preview within one poll tick.

use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::UsageSnapshot;

/// Bump on any incompatible change to [`UsageSnapshot`]'s persisted shape;
/// mismatched files are ignored rather than migrated.
const VERSION: u32 = 1;

/// Snapshots older than this are ignored at load: numbers that stale are more
/// misleading than a briefly absent widget.
fn max_age() -> Duration {
    Duration::hours(48)
}

#[derive(Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    saved_at: DateTime<Utc>,
    snapshot: UsageSnapshot,
}

/// Read the cached snapshot, or `None` when absent, unreadable, from another
/// cache version, from the future (clock skew), or older than [`max_age`].
pub fn load(path: &Path, now: DateTime<Utc>) -> Option<UsageSnapshot> {
    parse(&std::fs::read_to_string(path).ok()?, now)
}

fn parse(contents: &str, now: DateTime<Utc>) -> Option<UsageSnapshot> {
    let file = serde_json::from_str::<CacheFile>(contents).ok()?;
    if file.version != VERSION {
        return None;
    }
    let age = now.signed_duration_since(file.saved_at);
    if age < Duration::zero() || age > max_age() {
        return None;
    }
    Some(file.snapshot)
}

/// Atomically persist `snapshot` (temp file + rename, mode 0600), creating the
/// parent directory if needed. Failures are swallowed — this is only a cache.
pub fn store(path: &Path, snapshot: &UsageSnapshot, now: DateTime<Utc>) {
    let file = CacheFile {
        version: VERSION,
        saved_at: now,
        snapshot: snapshot.clone(),
    };
    let Ok(contents) = serde_json::to_string(&file) else {
        return;
    };
    let Some(dir) = path.parent() else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let temp = dir.join(format!(
        ".{}.tmp.{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("snapshot"),
        std::process::id()
    ));
    if std::fs::write(&temp, contents).is_err() {
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600));
    }
    if std::fs::rename(&temp, path).is_err() {
        let _ = std::fs::remove_file(&temp);
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::{Provider, TokenCounts, WindowTotals};

    fn snapshot_with_tokens(input: u64) -> UsageSnapshot {
        UsageSnapshot {
            claude: Provider {
                month: WindowTotals {
                    tokens: TokenCounts {
                        input,
                        ..TokenCounts::default()
                    },
                    cost_usd: 1.25,
                },
                ..Provider::default()
            },
            ..UsageSnapshot::default()
        }
    }

    fn serialized(saved_at: DateTime<Utc>, version: u32) -> String {
        serde_json::to_string(&CacheFile {
            version,
            saved_at,
            snapshot: snapshot_with_tokens(42),
        })
        .unwrap()
    }

    #[test]
    fn fresh_cache_round_trips() {
        let now = Utc.with_ymd_and_hms(2026, 7, 13, 12, 0, 0).unwrap();
        let loaded = parse(&serialized(now - Duration::minutes(5), VERSION), now).unwrap();
        assert_eq!(loaded, snapshot_with_tokens(42));
    }

    #[test]
    fn stale_future_and_mismatched_caches_are_ignored() {
        let now = Utc.with_ymd_and_hms(2026, 7, 13, 12, 0, 0).unwrap();
        assert!(parse(&serialized(now - Duration::hours(49), VERSION), now).is_none());
        assert!(parse(&serialized(now + Duration::minutes(1), VERSION), now).is_none());
        assert!(parse(&serialized(now, VERSION + 1), now).is_none());
        assert!(parse("not json", now).is_none());
    }

    #[test]
    fn store_then_load_round_trips_on_disk() {
        let now = Utc.with_ymd_and_hms(2026, 7, 13, 12, 0, 0).unwrap();
        let dir = std::env::temp_dir().join(format!(
            "cli-agent-usage-snapshot-test-{}",
            std::process::id()
        ));
        let path = dir.join("nested").join("snapshot.json");
        let snapshot = snapshot_with_tokens(7);

        store(&path, &snapshot, now);
        assert_eq!(load(&path, now), Some(snapshot));
        assert_eq!(load(&path, now + Duration::hours(72)), None);

        let _ = std::fs::remove_dir_all(dir);
    }
}
