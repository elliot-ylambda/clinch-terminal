//! Codex's current limits, straight from Codex. Its rollout logs only change when a turn runs, so
//! after buying a reset the gauges would otherwise stay wrong until the next turn. `codex
//! app-server` answers over stdio using the user's own Codex login; Clinch never sees a credential.

use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::mpsc;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, TimeZone, Utc};
use command::blocking::Command;
use instant::Instant;
use serde_json::{json, Value};

use crate::codex::severity_from_percent;
use crate::{retain_current_windows, LimitWindow, PlanLimits};

/// How long Codex gets to answer. It normally takes under a second.
const READ_TIMEOUT: StdDuration = StdDuration::from_secs(5);
const BACKGROUND_INTERVAL_SECS: i64 = 5 * 60;
/// The floor when the user just looked at Clinch or the usage panel.
const LOOKING_INTERVAL_SECS: i64 = 30;
/// Beyond this a live read is no longer live, for instance after signing out of Codex.
const MAX_AGE_SECS: i64 = 60 * 60;
const SESSION_WINDOW_MINS: i64 = 300;
const WEEKLY_WINDOW_MINS: i64 = 10_080;

/// Where Codex usually installs. A GUI app's `PATH` rarely includes any of them.
const INSTALL_DIRS: &[&str] = &[".local/bin", ".bun/bin", ".npm-global/bin", ".volta/bin"];
const SYSTEM_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin"];

/// Absolute paths only, so a `codex` in some working directory can never be picked up.
pub fn find_codex_binary(home: &Path, path_env: Option<&OsStr>) -> Option<PathBuf> {
    let on_path = path_env
        .into_iter()
        .flat_map(std::env::split_paths)
        .filter(|dir| dir.is_absolute());
    let installs = INSTALL_DIRS.iter().map(|dir| home.join(dir));
    let system = SYSTEM_DIRS.iter().map(PathBuf::from);
    on_path
        .chain(installs)
        .chain(system)
        .map(|dir| dir.join("codex"))
        .find(|candidate| candidate.is_file())
}

fn window_from(value: &Value) -> Option<(i64, LimitWindow)> {
    let percent = value.get("usedPercent")?.as_f64()?;
    let duration = value.get("windowDurationMins")?.as_i64()?;
    let resets_at = value
        .get("resetsAt")
        .and_then(Value::as_i64)
        .and_then(|seconds| Utc.timestamp_opt(seconds, 0).single());
    Some((
        duration,
        LimitWindow {
            percent,
            resets_at,
            severity: severity_from_percent(percent),
        },
    ))
}

/// The plan in an `account/rateLimits/read` result. Codex decides which window is `primary`, so
/// each one is identified by its length.
fn plan_from_rate_limits(result: &Value) -> Option<PlanLimits> {
    let limits = result.get("rateLimits")?;
    let mut plan = PlanLimits::default();
    for key in ["primary", "secondary"] {
        match limits.get(key).and_then(window_from) {
            Some((SESSION_WINDOW_MINS, window)) => plan.session = Some(window),
            Some((WEEKLY_WINDOW_MINS, window)) => plan.weekly = Some(window),
            _ => {}
        }
    }
    (plan.session.is_some() || plan.weekly.is_some()).then_some(plan)
}

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn request(stdin: &mut impl Write, message: &Value) -> Option<()> {
    writeln!(stdin, "{message}").ok()?;
    stdin.flush().ok()
}

/// **Blocking**, for up to [`READ_TIMEOUT`]. Call only from a dedicated thread. Any failure, from a
/// missing login to an older Codex without this request, is `None`: the rollout logs still work.
pub fn read_limits(binary: &Path, home: &Path) -> Option<PlanLimits> {
    let mut child = KillOnDrop(
        Command::new(binary)
            .arg("app-server")
            .current_dir(home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?,
    );
    let mut stdin = child.0.stdin.take()?;
    let stdout = child.0.stdout.take()?;
    let (lines, responses) = mpsc::channel();
    std::thread::Builder::new()
        .name("codex-limits-reader".to_owned())
        .spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if lines.send(line).is_err() {
                    break;
                }
            }
        })
        .ok()?;

    request(
        &mut stdin,
        &json!({
            "method": "initialize",
            "id": 1,
            "params": {"clientInfo": {
                "name": "clinch",
                "title": "Clinch",
                "version": env!("CARGO_PKG_VERSION"),
            }},
        }),
    )?;
    let deadline = Instant::now() + READ_TIMEOUT;
    loop {
        let line = responses
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .ok()?;
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        match message.get("id").and_then(Value::as_i64) {
            Some(1) => {
                request(&mut stdin, &json!({"method": "initialized"}))?;
                request(
                    &mut stdin,
                    &json!({"method": "account/rateLimits/read", "id": 2}),
                )?;
            }
            Some(2) => return plan_from_rate_limits(message.get("result")?),
            _ => {}
        }
    }
}

/// The most recent successful live read, and when the next attempt is due.
#[derive(Default)]
pub struct LiveCodexLimits {
    last_attempt: Option<DateTime<Utc>>,
    live: Option<LivePlan>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LivePlan {
    pub plan: PlanLimits,
    pub fetched_at: DateTime<Utc>,
}

impl LiveCodexLimits {
    pub fn is_due(&self, now: DateTime<Utc>, user_looking: bool) -> bool {
        let interval = if user_looking {
            LOOKING_INTERVAL_SECS
        } else {
            BACKGROUND_INTERVAL_SECS
        };
        self.last_attempt
            .is_none_or(|last| now.signed_duration_since(last) >= Duration::seconds(interval))
    }

    /// Records an attempt. A failed one keeps the last good read until that goes stale.
    pub fn record_attempt(&mut self, plan: Option<PlanLimits>, now: DateTime<Utc>) {
        self.last_attempt = Some(now);
        if let Some(plan) = plan {
            self.live = Some(LivePlan {
                plan,
                fetched_at: now,
            });
        }
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// The live read, while it's recent and still describes a current limit period.
    pub fn current(&self, now: DateTime<Utc>) -> Option<LivePlan> {
        let live = self.live.filter(|live| {
            now.signed_duration_since(live.fetched_at) <= Duration::seconds(MAX_AGE_SECS)
        })?;
        retain_current_windows(live.plan, now).map(|plan| LivePlan {
            plan,
            fetched_at: live.fetched_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_800_000_000 + seconds, 0).unwrap()
    }

    fn plan(percent: f64) -> PlanLimits {
        PlanLimits {
            weekly: Some(LimitWindow {
                percent,
                resets_at: None,
                severity: severity_from_percent(percent),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn identifies_windows_by_length_not_position() {
        // A plan with only a weekly limit reports it as `primary`.
        let weekly_only = json!({"rateLimits": {
            "primary": {"usedPercent": 100, "windowDurationMins": 10080, "resetsAt": 1793824982},
            "secondary": null,
            "planType": "prolite",
        }});
        let parsed = plan_from_rate_limits(&weekly_only).unwrap();
        assert_eq!(parsed.session, None);
        let weekly = parsed.weekly.unwrap();
        assert_eq!(weekly.percent, 100.0);
        assert_eq!(weekly.severity, crate::Severity::Critical);
        assert_eq!(weekly.resets_at.unwrap().timestamp(), 1793824982);

        let both = json!({"rateLimits": {
            "primary": {"usedPercent": 12.5, "windowDurationMins": 300},
            "secondary": {"usedPercent": 40, "windowDurationMins": 10080},
        }});
        let parsed = plan_from_rate_limits(&both).unwrap();
        assert_eq!(parsed.session.unwrap().percent, 12.5);
        assert_eq!(parsed.weekly.unwrap().percent, 40.0);
    }

    #[test]
    fn a_response_without_a_known_window_is_no_plan() {
        assert_eq!(plan_from_rate_limits(&json!({})), None);
        assert_eq!(
            plan_from_rate_limits(&json!({"rateLimits": {"primary": null, "secondary": null}})),
            None
        );
        assert_eq!(
            plan_from_rate_limits(&json!({"rateLimits": {
                "primary": {"usedPercent": 5, "windowDurationMins": 60},
            }})),
            None
        );
    }

    #[test]
    fn asks_often_only_while_the_user_is_looking() {
        let mut live = LiveCodexLimits::default();
        assert!(live.is_due(at(0), false));
        live.record_attempt(Some(plan(10.0)), at(0));
        assert!(!live.is_due(at(29), true));
        assert!(live.is_due(at(30), true));
        assert!(!live.is_due(at(299), false));
        assert!(live.is_due(at(300), false));
    }

    #[test]
    fn a_failed_read_keeps_the_last_one_until_it_goes_stale() {
        let mut live = LiveCodexLimits::default();
        assert_eq!(live.current(at(0)), None);
        live.record_attempt(Some(plan(10.0)), at(0));
        live.record_attempt(None, at(300));
        let current = live.current(at(300)).unwrap();
        assert_eq!(current.plan, plan(10.0));
        assert_eq!(current.fetched_at, at(0));
        assert_eq!(live.current(at(MAX_AGE_SECS + 1)), None);
    }

    #[test]
    fn finds_codex_on_path_before_the_usual_install_dirs() {
        let root = std::env::temp_dir().join(format!("clinch-codex-find-{}", std::process::id()));
        let home = root.join("home");
        let elsewhere = root.join("elsewhere");
        std::fs::create_dir_all(home.join(".local/bin")).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(home.join(".local/bin/codex"), "").unwrap();
        assert_eq!(
            find_codex_binary(&home, None),
            Some(home.join(".local/bin/codex"))
        );
        std::fs::write(elsewhere.join("codex"), "").unwrap();
        let path = std::env::join_paths([&elsewhere]).unwrap();
        assert_eq!(
            find_codex_binary(&home, Some(&path)),
            Some(elsewhere.join("codex"))
        );
        // A relative PATH entry is never searched.
        assert_eq!(
            find_codex_binary(&home, Some(OsStr::new("elsewhere"))),
            Some(home.join(".local/bin/codex"))
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
