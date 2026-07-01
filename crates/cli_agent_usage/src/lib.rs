use chrono::{DateTime, Utc};

pub mod cache;
pub mod claude;
pub mod codex;
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

#[derive(Debug, Clone, Default)]
pub struct Provider {
    pub session: WindowTotals,
    pub today: WindowTotals,
    pub week: WindowTotals,
    pub month: WindowTotals,
    pub plan: Option<PlanLimits>,
}

#[derive(Debug, Clone, Default)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_counts_total_and_add() {
        let mut a = TokenCounts { input: 10, output: 5, cache_read: 1, cache_write: 2 };
        assert_eq!(a.total(), 18);
        a.add(&TokenCounts { input: 1, output: 1, cache_read: 0, cache_write: 0 });
        assert_eq!(a.total(), 20);
    }

    #[test]
    fn severity_default_is_normal() {
        assert_eq!(Severity::default(), Severity::Normal);
    }
}
