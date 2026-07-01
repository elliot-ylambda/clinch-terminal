//! Parse Claude Code transcripts (~/.claude/projects/**/*.jsonl).

use std::path::Path;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::cache::{scan_dir, ScanCache};
use crate::{aggregate_windows, Entry, Provider, TokenCounts, WindowTotals};

#[derive(Deserialize)]
struct Line {
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(rename = "requestId", default)]
    request_id: Option<String>,
    #[serde(default)]
    uuid: Option<String>,
    #[serde(default)]
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Usage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
}

pub fn parse_transcript_str(content: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    for raw in content.lines() {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let Ok(line) = serde_json::from_str::<Line>(raw) else {
            continue;
        };
        if line.r#type != "assistant" {
            continue;
        }
        let Some(msg) = line.message else { continue };
        let Some(usage) = msg.usage else { continue };
        let Some(ts_str) = line.timestamp.as_deref() else {
            continue;
        };
        let Ok(ts) = DateTime::parse_from_rfc3339(ts_str) else {
            continue;
        };
        let ts = ts.with_timezone(&Utc);

        let model = msg.model.unwrap_or_else(|| "unknown".to_string());
        let req = line.request_id.or(line.uuid).unwrap_or_default();
        let mid = msg.id.unwrap_or_default();
        let dedup = if req.is_empty() && mid.is_empty() {
            String::new()
        } else {
            format!("{req}:{mid}")
        };

        out.push(Entry {
            ts,
            model,
            tokens: TokenCounts {
                input: usage.input_tokens,
                output: usage.output_tokens,
                cache_read: usage.cache_read_input_tokens,
                cache_write: usage.cache_creation_input_tokens,
            },
            dedup,
        });
    }
    out
}

pub fn parse_transcript_file(path: &Path) -> Vec<Entry> {
    match std::fs::read_to_string(path) {
        Ok(s) => parse_transcript_str(&s),
        Err(_) => Vec::new(),
    }
}

pub fn scan(
    projects_dir: &Path,
    cache: &mut ScanCache<Vec<Entry>>,
    now: DateTime<Utc>,
) -> Provider {
    let mut provider = Provider::default();
    let mut seen = std::collections::HashSet::new();

    let files = scan_dir(projects_dir, ".jsonl");
    // session = the most-recently-modified transcript
    let latest = files
        .iter()
        .max_by_key(|(_, mtime, _)| *mtime)
        .map(|(p, _, _)| p.clone());

    for (path, mtime, size) in &files {
        let entries = cache
            .get_or_parse(path, *mtime, *size, |p| parse_transcript_file(p))
            .clone();
        aggregate_windows(
            &entries,
            now,
            &mut seen,
            &mut provider.today,
            &mut provider.week,
            &mut provider.month,
        );
        if Some(path) == latest.as_ref() {
            let mut s = WindowTotals::default();
            for e in &entries {
                s.add_entry(e);
            }
            provider.session = s;
        }
    }
    provider
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    const LINE: &str = r#"{"type":"assistant","requestId":"req_1","timestamp":"2026-06-30T21:16:28.384Z","message":{"id":"msg_1","model":"claude-opus-4-8","usage":{"input_tokens":6,"output_tokens":218,"cache_creation_input_tokens":29086,"cache_read_input_tokens":4}}}"#;

    #[test]
    fn parses_one_assistant_line() {
        let v = parse_transcript_str(LINE);
        assert_eq!(v.len(), 1);
        let e = &v[0];
        assert_eq!(e.model, "claude-opus-4-8");
        assert_eq!(
            e.tokens,
            TokenCounts {
                input: 6,
                output: 218,
                cache_write: 29086,
                cache_read: 4
            }
        );
        assert_eq!(e.dedup, "req_1:msg_1");
        assert_eq!(
            e.ts,
            Utc.with_ymd_and_hms(2026, 6, 30, 21, 16, 28).unwrap()
                + chrono::Duration::milliseconds(384)
        );
    }

    #[test]
    fn skips_non_assistant_and_malformed() {
        let content = format!(
            "{}\n{}\n{}",
            r#"{"type":"user","message":{"role":"user"}}"#, "not json at all", LINE
        );
        let v = parse_transcript_str(&content);
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn dedup_within_aggregate() {
        // two identical dedup keys count once
        let entries = parse_transcript_str(&format!("{LINE}\n{LINE}"));
        assert_eq!(entries.len(), 2);
        let mut seen = std::collections::HashSet::new();
        let (mut t, mut w, mut m) = Default::default();
        crate::aggregate_windows(&entries, Utc::now(), &mut seen, &mut t, &mut w, &mut m);
        assert_eq!(m.tokens.total(), 6 + 218 + 29086 + 4);
    }
}
