//! Claude usage endpoint client + response parsing.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::fmt;
use std::time::Duration;

use crate::codex::severity_from_percent;
use crate::{LimitWindow, PlanLimits, Severity};

pub const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchErrorKind {
    Unauthorized,
    RateLimited,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchError {
    kind: FetchErrorKind,
    retry_after: Option<Duration>,
    message: String,
}

impl FetchError {
    pub fn other(message: impl Into<String>) -> Self {
        Self {
            kind: FetchErrorKind::Other,
            retry_after: None,
            message: message.into(),
        }
    }

    pub fn unauthorized() -> Self {
        Self {
            kind: FetchErrorKind::Unauthorized,
            retry_after: None,
            message: "usage HTTP 401 Unauthorized".to_string(),
        }
    }

    pub fn rate_limited(retry_after: Option<Duration>) -> Self {
        Self {
            kind: FetchErrorKind::RateLimited,
            retry_after,
            message: "usage HTTP 429 Too Many Requests".to_string(),
        }
    }

    fn http(status: reqwest::StatusCode, retry_after: Option<Duration>) -> Self {
        let kind = match status {
            reqwest::StatusCode::UNAUTHORIZED => FetchErrorKind::Unauthorized,
            reqwest::StatusCode::TOO_MANY_REQUESTS => FetchErrorKind::RateLimited,
            _ => FetchErrorKind::Other,
        };
        Self {
            kind,
            retry_after,
            message: format!("usage HTTP {status}"),
        }
    }

    pub fn kind(&self) -> FetchErrorKind {
        self.kind
    }

    pub fn retry_after(&self) -> Option<Duration> {
        self.retry_after
    }
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for FetchError {}

pub trait FetchUsage {
    fn fetch(&self, access_token: &str) -> Result<String, FetchError>;
}

#[derive(Deserialize)]
struct Resp {
    five_hour: Option<RawWindow>,
    seven_day: Option<RawWindow>,
    #[serde(default)]
    limits: Vec<RawLimit>,
}

#[derive(Deserialize)]
struct RawWindow {
    #[serde(default)]
    utilization: f64,
    #[serde(default)]
    resets_at: Option<String>,
}

#[derive(Deserialize)]
struct RawLimit {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    group: String,
    #[serde(default)]
    percent: f64,
    #[serde(default)]
    severity: String,
    #[serde(default)]
    resets_at: Option<String>,
    #[serde(default)]
    scope: Option<RawScope>,
}

#[derive(Deserialize)]
struct RawScope {
    #[serde(default)]
    model: Option<RawModelScope>,
}

#[derive(Deserialize)]
struct RawModelScope {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
}

fn is_fable_model(model: &RawModelScope) -> bool {
    [model.id.as_deref(), model.display_name.as_deref()]
        .into_iter()
        .flatten()
        .any(|name| {
            let name = name.trim().to_ascii_lowercase();
            name == "fable" || name.starts_with("fable ") || name.starts_with("fable-")
        })
}

fn parse_ts(s: &Option<String>) -> Option<DateTime<Utc>> {
    let s = s.as_deref()?;
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

fn severity_str(s: &str) -> Severity {
    match s {
        "warning" => Severity::Warning,
        "critical" | "blocked" | "exceeded" => Severity::Critical,
        _ => Severity::Normal,
    }
}

pub fn parse_plan_limits(json: &str) -> Option<PlanLimits> {
    let resp: Resp = serde_json::from_str(json).ok()?;

    // Prefer the normalized limits[] array.
    if !resp.limits.is_empty() {
        let to_window = |limit: &RawLimit| LimitWindow {
            percent: limit.percent,
            resets_at: parse_ts(&limit.resets_at),
            severity: severity_str(&limit.severity),
        };

        // `group` is not unique: both the account-wide and model-scoped
        // windows use "weekly". Prefer the explicit kinds and only use a
        // group-only entry as a compatibility fallback for older payloads.
        let pick = |kind: &str, legacy_group: &str| {
            resp.limits
                .iter()
                .find(|limit| limit.kind == kind)
                .or_else(|| {
                    resp.limits
                        .iter()
                        .find(|limit| limit.kind.is_empty() && limit.group == legacy_group)
                })
                .map(to_window)
        };
        let session = pick("session", "session");
        let weekly = pick("weekly_all", "weekly");
        let fable_weekly = resp
            .limits
            .iter()
            .find(|limit| {
                limit.kind == "weekly_scoped"
                    && limit
                        .scope
                        .as_ref()
                        .and_then(|scope| scope.model.as_ref())
                        .is_some_and(is_fable_model)
            })
            .map(to_window);
        if session.is_some() || weekly.is_some() || fable_weekly.is_some() {
            return Some(PlanLimits {
                session,
                weekly,
                fable_weekly,
            });
        }
    }

    // Fallback: five_hour / seven_day objects.
    let to_win = |w: &RawWindow| LimitWindow {
        percent: w.utilization,
        resets_at: parse_ts(&w.resets_at),
        severity: severity_from_percent(w.utilization),
    };
    let session = resp.five_hour.as_ref().map(to_win);
    let weekly = resp.seven_day.as_ref().map(to_win);
    if session.is_none() && weekly.is_none() {
        return None;
    }
    Some(PlanLimits {
        session,
        weekly,
        fable_weekly: None,
    })
}

/// Blocking client for the usage endpoint. Uses `reqwest::blocking`, which
/// **panics if constructed inside an async runtime** — call only from a
/// synchronous/dedicated thread, never from within Tokio.
pub struct ReqwestUsage;

impl FetchUsage for ReqwestUsage {
    fn fetch(&self, access_token: &str) -> Result<String, FetchError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|e| FetchError::other(e.to_string()))?;
        let resp = client
            .get(USAGE_URL)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("anthropic-beta", "oauth-2025-04-20")
            .header("anthropic-version", "2023-06-01")
            .header("Accept", "application/json")
            .header("User-Agent", "clinch-usage/0.1")
            .send()
            .map_err(|e| FetchError::other(e.to_string()))?;
        if !resp.status().is_success() {
            let retry_after = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(parse_retry_after);
            return Err(FetchError::http(resp.status(), retry_after));
        }
        resp.text().map_err(|e| FetchError::other(e.to_string()))
    }
}

fn parse_retry_after(value: &str) -> Option<Duration> {
    value.trim().parse::<u64>().ok().map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Severity;

    // Real captured shape (2026-06-30), trimmed.
    const RESP: &str = r#"{
      "five_hour": {"utilization": 78.0, "resets_at": "2026-07-01T02:30:00.49+00:00"},
      "seven_day": {"utilization": 43.0, "resets_at": "2026-07-04T15:00:00.49+00:00"},
      "limits": [
        {"kind":"session","group":"session","percent":78,"severity":"warning","resets_at":"2026-07-01T02:30:00.49+00:00","is_active":true},
        {"kind":"weekly_scoped","group":"weekly","percent":61,"severity":"warning","resets_at":"2026-07-05T15:00:00.49+00:00","scope":{"model":{"id":null,"display_name":"Fable"},"surface":null},"is_active":true},
        {"kind":"weekly_all","group":"weekly","percent":43,"severity":"normal","resets_at":"2026-07-04T15:00:00.49+00:00","is_active":false}
      ]
    }"#;

    #[test]
    fn parses_limits_array_preferred() {
        let p = parse_plan_limits(RESP).unwrap();
        assert_eq!(p.session.unwrap().percent, 78.0);
        assert_eq!(p.session.unwrap().severity, Severity::Warning);
        assert_eq!(p.weekly.unwrap().percent, 43.0);
        assert_eq!(p.fable_weekly.unwrap().percent, 61.0);
        assert_eq!(p.fable_weekly.unwrap().severity, Severity::Warning);
        assert!(p.session.unwrap().resets_at.is_some());
    }

    #[test]
    fn weekly_all_is_not_confused_with_other_scoped_limits() {
        let resp = r#"{"limits":[
          {"kind":"weekly_scoped","group":"weekly","percent":88,"scope":{"model":{"display_name":"Other"}}},
          {"kind":"weekly_all","group":"weekly","percent":34},
          {"kind":"weekly_scoped","group":"weekly","percent":55,"scope":{"model":{"display_name":"Fable 5"}}}
        ]}"#;
        let p = parse_plan_limits(resp).unwrap();
        assert_eq!(p.weekly.unwrap().percent, 34.0);
        assert_eq!(p.fable_weekly.unwrap().percent, 55.0);
    }

    #[test]
    fn legacy_group_only_limits_are_still_supported() {
        let resp = r#"{"limits":[
          {"group":"session","percent":12},
          {"group":"weekly","percent":34}
        ]}"#;
        let p = parse_plan_limits(resp).unwrap();
        assert_eq!(p.session.unwrap().percent, 12.0);
        assert_eq!(p.weekly.unwrap().percent, 34.0);
        assert_eq!(p.fable_weekly, None);
    }

    #[test]
    fn falls_back_to_five_hour_seven_day() {
        let resp = r#"{"five_hour":{"utilization":12.0,"resets_at":"2026-07-01T02:30:00+00:00"},"seven_day":{"utilization":34.0,"resets_at":"2026-07-04T15:00:00+00:00"}}"#;
        let p = parse_plan_limits(resp).unwrap();
        assert_eq!(p.session.unwrap().percent, 12.0);
        assert_eq!(p.weekly.unwrap().percent, 34.0);
        assert_eq!(p.fable_weekly, None);
        // severity derived from percent when not provided
        assert_eq!(p.session.unwrap().severity, Severity::Normal);
    }

    #[test]
    fn garbage_is_none() {
        assert!(parse_plan_limits("nope").is_none());
    }

    #[test]
    fn parses_retry_after_delta_seconds() {
        assert_eq!(parse_retry_after("2037"), Some(Duration::from_secs(2037)));
        assert_eq!(parse_retry_after(" 60 "), Some(Duration::from_secs(60)));
        assert_eq!(parse_retry_after("not-a-delay"), None);
    }

    #[test]
    fn classifies_http_failures_without_losing_retry_delay() {
        let retry = Duration::from_secs(2100);
        let limited = FetchError::http(reqwest::StatusCode::TOO_MANY_REQUESTS, Some(retry));
        assert_eq!(limited.kind(), FetchErrorKind::RateLimited);
        assert_eq!(limited.retry_after(), Some(retry));

        let unauthorized = FetchError::http(reqwest::StatusCode::UNAUTHORIZED, None);
        assert_eq!(unauthorized.kind(), FetchErrorKind::Unauthorized);
        assert_eq!(unauthorized.retry_after(), None);
    }
}
