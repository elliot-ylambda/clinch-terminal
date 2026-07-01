//! Claude usage endpoint client + response parsing.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::codex::severity_from_percent;
use crate::{LimitWindow, PlanLimits, Severity};

pub const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

pub trait FetchUsage {
    fn fetch(&self, access_token: &str) -> Result<String, String>;
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
    group: String,
    #[serde(default)]
    percent: f64,
    #[serde(default)]
    severity: String,
    #[serde(default)]
    resets_at: Option<String>,
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
        let pick = |group: &str| -> Option<LimitWindow> {
            resp.limits
                .iter()
                .find(|l| l.group == group)
                .map(|l| LimitWindow {
                    percent: l.percent,
                    resets_at: parse_ts(&l.resets_at),
                    severity: severity_str(&l.severity),
                })
        };
        let session = pick("session");
        let weekly = pick("weekly");
        if session.is_some() || weekly.is_some() {
            return Some(PlanLimits { session, weekly });
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
    Some(PlanLimits { session, weekly })
}

pub struct ReqwestUsage;

impl FetchUsage for ReqwestUsage {
    fn fetch(&self, access_token: &str) -> Result<String, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|e| e.to_string())?;
        let resp = client
            .get(USAGE_URL)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("anthropic-beta", "oauth-2025-04-20")
            .header("anthropic-version", "2023-06-01")
            .header("Accept", "application/json")
            .header("User-Agent", "clinch-usage/0.1")
            .send()
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("usage HTTP {}", resp.status()));
        }
        resp.text().map_err(|e| e.to_string())
    }
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
        {"kind":"weekly_all","group":"weekly","percent":43,"severity":"normal","resets_at":"2026-07-04T15:00:00.49+00:00","is_active":false}
      ]
    }"#;

    #[test]
    fn parses_limits_array_preferred() {
        let p = parse_plan_limits(RESP).unwrap();
        assert_eq!(p.session.unwrap().percent, 78.0);
        assert_eq!(p.session.unwrap().severity, Severity::Warning);
        assert_eq!(p.weekly.unwrap().percent, 43.0);
        assert!(p.session.unwrap().resets_at.is_some());
    }

    #[test]
    fn falls_back_to_five_hour_seven_day() {
        let resp = r#"{"five_hour":{"utilization":12.0,"resets_at":"2026-07-01T02:30:00+00:00"},"seven_day":{"utilization":34.0,"resets_at":"2026-07-04T15:00:00+00:00"}}"#;
        let p = parse_plan_limits(resp).unwrap();
        assert_eq!(p.session.unwrap().percent, 12.0);
        assert_eq!(p.weekly.unwrap().percent, 34.0);
        // severity derived from percent when not provided
        assert_eq!(p.session.unwrap().severity, Severity::Normal);
    }

    #[test]
    fn garbage_is_none() {
        assert!(parse_plan_limits("nope").is_none());
    }
}
