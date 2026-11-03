//! Reset instants stated in provider usage-limit messages.

use std::sync::LazyLock;

use chrono::{DateTime, Duration, Utc};
use regex::{Captures, Regex};

/// A stated reset at most this far behind `now` is reported as-is (the
/// message was written just before the reset and observed just after), so
/// callers' "never arm a past reset" rule applies instead of rolling the
/// reset forward by a whole day or year.
const PAST_RESET_GRACE: Duration = Duration::minutes(10);

static CLAUDE_RESET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?ix)
        \bresets\s+
        (?:(?P<month>jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*\.?\s+
           (?P<day>\d{1,2})(?:st|nd|rd|th)?,?\s+(?:at\s+)?)?
        (?P<hour>\d{1,2})(?::(?P<minute>\d{2}))?\s*(?P<meridiem>am|pm)\s*
        \((?P<tz>[A-Za-z][A-Za-z0-9_+\-]*(?:/[A-Za-z0-9_+\-]+)*)\)",
    )
    .expect("valid Claude reset pattern")
});

/// Claude Code states the reset in its limit message, e.g.
/// `You've hit your session limit · resets 6:50pm (America/Los_Angeles)` or
/// `You've hit your weekly limit · resets Oct 28 at 4pm (America/Los_Angeles)`.
/// Spend-limit messages can append `your weekly limit resets …`; when several
/// resets are stated, the latest is the earliest moment a continue can work.
pub fn claude_limit_reset(message: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    CLAUDE_RESET
        .captures_iter(message)
        .filter_map(|captures| resolve_claude_reset(&captures, now))
        .max()
}

fn resolve_claude_reset(captures: &Captures<'_>, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let tz = jiff::tz::TimeZone::get(captures.name("tz")?.as_str()).ok()?;
    let hour12: i8 = captures.name("hour")?.as_str().parse().ok()?;
    if !(1..=12).contains(&hour12) {
        return None;
    }
    let minute: i8 = match captures.name("minute") {
        Some(minute) => minute.as_str().parse().ok()?,
        None => 0,
    };
    let is_pm = captures
        .name("meridiem")?
        .as_str()
        .eq_ignore_ascii_case("pm");
    let hour = (hour12 % 12) + if is_pm { 12 } else { 0 };
    let time = jiff::civil::Time::new(hour, minute, 0, 0).ok()?;

    let local_now = jiff::Timestamp::from_second(now.timestamp())
        .ok()?
        .to_zoned(tz.clone());
    let earliest = now - PAST_RESET_GRACE;
    let instant = |date: jiff::civil::Date| -> Option<DateTime<Utc>> {
        let zoned = jiff::civil::DateTime::from_parts(date, time)
            .to_zoned(tz.clone())
            .ok()?;
        DateTime::from_timestamp(zoned.timestamp().as_second(), 0)
    };

    match (captures.name("month"), captures.name("day")) {
        (Some(month), Some(day)) => {
            let month = month_number(month.as_str())?;
            let day: i8 = day.as_str().parse().ok()?;
            let this_year = jiff::civil::Date::new(local_now.year(), month, day).ok()?;
            let candidate = instant(this_year)?;
            if candidate >= earliest {
                return Some(candidate);
            }
            // "resets Jan 2" stated in late December.
            instant(jiff::civil::Date::new(local_now.year() + 1, month, day).ok()?)
        }
        _ => {
            let today = local_now.date();
            let candidate = instant(today)?;
            if candidate >= earliest {
                return Some(candidate);
            }
            instant(today.tomorrow().ok()?)
        }
    }
}

fn month_number(name: &str) -> Option<i8> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let prefix = name.get(..3)?.to_ascii_lowercase();
    MONTHS
        .iter()
        .position(|month| *month == prefix)
        .map(|index| index as i8 + 1)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    use super::*;

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    // Each row: (message captured from a real Claude Code transcript, the
    // transcript's own timestamp, expected reset instant).
    #[test]
    fn parses_real_claude_code_limit_messages() {
        let cases = [
            // 2.1.305, error=rate_limit
            ("You've hit your session limit · resets 6:50pm (America/Los_Angeles)",
             "2026-10-23T01:13:33.418Z", "2026-10-23T01:50:00Z"),
            ("You've hit your session limit · resets 5pm (America/Los_Angeles)",
             "2026-10-20T21:20:54.228Z", "2026-10-21T00:00:00Z"),
            // crosses local midnight
            ("You've hit your session limit · resets 12:20am (America/Los_Angeles)",
             "2026-10-21T05:57:46.849Z", "2026-10-21T07:20:00Z"),
            ("You've hit your session limit · resets 10:20pm (America/Los_Angeles)",
             "2026-07-10T02:39:33.570Z", "2026-07-10T05:20:00Z"),
            // 2.1.310 weekly with a date
            ("You've hit your weekly limit · resets Oct 28 at 4pm (America/Los_Angeles)",
             "2026-10-24T00:56:44.124Z", "2026-10-28T23:00:00Z"),
            // weekly within a day: time-only form
            ("You've hit your weekly limit · resets 8am (America/Los_Angeles)",
             "2026-06-26T22:09:02.889Z", "2026-06-27T15:00:00Z"),
            // spend limit with an appended window reset
            ("You've hit your monthly spend limit · raise it at https://claude.ai/settings/usage?from=cc_cli_limit_message · your weekly limit resets Oct 24 at 8am (America/Los_Angeles)",
             "2026-10-21T19:30:41.399Z", "2026-10-24T15:00:00Z"),
            ("You've hit your monthly spend limit · raise it at claude.ai/settings/usage?from=cc_cli_limit_message · your session limit resets 1pm (America/Los_Angeles)",
             "2026-08-31T19:00:49.892Z", "2026-08-31T20:00:00Z"),
        ];
        for (message, at, expected) in cases {
            assert_eq!(
                claude_limit_reset(message, utc(at)),
                Some(utc(expected)),
                "{message}"
            );
        }
    }

    #[test]
    fn messages_without_a_stated_reset_yield_none() {
        let now = utc("2026-10-21T19:30:41Z");
        for message in [
            "You've hit your monthly spend limit · raise it at claude.ai/settings/usage",
            "You've reached your Fable limit. Run /usage-credits to continue or switch models with /model.",
            "API Error: Server is temporarily limiting requests (not your usage limit) · Rate limited",
            "API Error: Request rejected (429) · This request would exceed your account's rate limit. Please try again later.",
            "resets 6:50pm (Not/AZone)",
            "resets 13pm (America/Los_Angeles)",
        ] {
            assert_eq!(claude_limit_reset(message, now), None, "{message}");
        }
    }

    #[test]
    fn a_reset_just_behind_now_is_not_rolled_a_day_forward() {
        let message = "You've hit your session limit · resets 7pm (America/Los_Angeles)";
        // 7:02pm PDT: reported as the (past) 7pm instant, not tomorrow.
        let now = utc("2026-10-21T02:02:00Z");
        assert_eq!(
            claude_limit_reset(message, now),
            Some(utc("2026-10-21T02:00:00Z"))
        );
    }

    #[test]
    fn a_january_reset_stated_in_december_lands_next_year() {
        let message = "You've hit your weekly limit · resets Jan 2 at 9am (America/Los_Angeles)";
        let now = Utc.with_ymd_and_hms(2026, 12, 30, 20, 0, 0).unwrap();
        assert_eq!(
            claude_limit_reset(message, now),
            Some(utc("2027-01-02T17:00:00Z"))
        );
    }

    #[test]
    fn honours_the_stated_zone_rather_than_the_local_zone() {
        let message = "You've hit your session limit · resets 3pm (Europe/Berlin)";
        let now = utc("2026-10-21T10:00:00Z");
        assert_eq!(
            claude_limit_reset(message, now),
            Some(utc("2026-10-21T13:00:00Z"))
        );
    }
}
