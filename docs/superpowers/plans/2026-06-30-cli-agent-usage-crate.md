# CLI Agent Usage Crate — Implementation Plan (Part A of 2)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a UI-free `cli_agent_usage` Rust crate that reads Claude Code and Codex usage from local files + Claude's `/api/oauth/usage` endpoint and produces one `UsageSnapshot` (token windows, est. cost, real 5h/weekly plan-%).

**Architecture:** Pure, isolated crate under `crates/cli_agent_usage`. Parsers turn raw JSONL into timestamped `Entry`s; an aggregator buckets them into today/week/month windows against `now`; a pricing table computes est. cost; Keychain + HTTP sit behind small traits so they're mockable. `refresh()` composes everything fail-soft. An `examples/print_usage.rs` binary proves it end-to-end. Part B (separate plan) wires the snapshot into the footer.

**Tech Stack:** Rust, `serde`/`serde_json`, `chrono`, `walkdir`, `reqwest` (blocking), `security-framework` (macOS Keychain). All already in the workspace.

**Spec:** `docs/superpowers/specs/2026-06-30-cli-agent-usage-footer-design.md`

## Global Constraints

- Crate is **UI-free**: no `warpui`, no `gpui`, no app deps. Depends only on the libs above.
- **Fail-soft everywhere:** any missing dir / malformed line / absent-or-expired token / HTTP error yields `None`/empty for that slice and never panics. A parser must tolerate a corrupt line by skipping it.
- **No network except** `GET https://api.anthropic.com/api/oauth/usage`. Token is held in memory only, never logged, never written to disk.
- Parsers expose a `*_str(content: &str)` core (tested with literals) plus a thin `*_file(path)` wrapper, so tests need no fixture files.
- Token windows: `today` = since local midnight (`chrono::Local`); `week` = now−7d; `month` = now−30d. Plan-% windows come verbatim from providers, never recomputed.
- Cost is **estimate only** (`est.`), from the pricing table in Task 2; unknown model → that slice contributes `0` cost (tokens still counted) + one `eprintln!` warn.
- Edition 2021. Run all commands from repo root `/Users/ellioteckholm/projects/clinch-terminal`.

---

### Task 1: Scaffold crate + core types

**Files:**
- Create: `crates/cli_agent_usage/Cargo.toml`
- Create: `crates/cli_agent_usage/src/lib.rs`
- Modify: `Cargo.toml` (root) — add path alias under the existing alias block (near `ai = { path = "crates/ai" }`, ~line 32)

**Interfaces:**
- Produces (used by every later task):
  - `struct TokenCounts { input, output, cache_read, cache_write: u64 }` with `fn total(&self)->u64`, `fn add(&mut self,&TokenCounts)`
  - `struct WindowTotals { tokens: TokenCounts, cost_usd: f64 }` with `fn add_entry(&mut self, &Entry)` (prices via `crate::pricing::cost`)
  - `enum Severity { Normal, Warning, Critical }` (`Default`=Normal)
  - `struct LimitWindow { percent: f64, resets_at: Option<DateTime<Utc>>, severity: Severity }`
  - `struct PlanLimits { session: Option<LimitWindow>, weekly: Option<LimitWindow> }`
  - `struct Provider { session, today, week, month: WindowTotals, plan: Option<PlanLimits> }`
  - `struct UsageSnapshot { claude: Provider, codex: Provider }`
  - `struct Entry { ts: DateTime<Utc>, model: String, tokens: TokenCounts, dedup: String }`

- [ ] **Step 1: Create the crate manifest**

`crates/cli_agent_usage/Cargo.toml`:
```toml
[package]
name = "cli_agent_usage"
version = "0.1.0"
edition = "2021"
publish = false

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["clock"] }
walkdir = "2"
reqwest = { version = "0.13", features = ["blocking", "json"] }

[target.'cfg(target_os = "macos")'.dependencies]
security-framework = "3"

[dev-dependencies]
```
(If `cargo` reports a different resolved major for `security-framework`/`walkdir`/`chrono` already in the lock, match the lock's major to avoid a second copy.)

- [ ] **Step 2: Register the crate alias in the root manifest**

In root `Cargo.toml`, in the alias block that starts around line 32 (`ai = { path = "crates/ai" }`), add alphabetically:
```toml
cli_agent_usage = { path = "crates/cli_agent_usage" }
```
(`members = ["crates/*", ...]` already includes the crate; this only adds the convenient path alias.)

- [ ] **Step 3: Write the failing test for core types**

`crates/cli_agent_usage/src/lib.rs` (append at bottom):
```rust
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
```

- [ ] **Step 4: Implement the types**

`crates/cli_agent_usage/src/lib.rs` (top of file, above the test module):
```rust
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
```

> Note: this references the `pricing`, `cache`, `claude`, `codex`, `http`, `keychain`
> modules created in later tasks, so all six `src/<name>.rs` files must exist now or the
> crate won't compile. Create them as part of this step:
> - `src/pricing.rs` needs a **callable stub** because `WindowTotals::add_entry` calls it:
>   ```rust
>   //! stub — real table in Task 2
>   use crate::TokenCounts;
>   pub fn cost(_model: &str, _t: &TokenCounts) -> f64 { 0.0 }
>   ```
>   (Returning `0.0` makes Task 2's pricing test fail for the right reason, then pass.)
> - `src/{cache,claude,codex,http,keychain}.rs` are unused by `lib.rs` types, so each may
>   contain only a `//! stub` line for now; later tasks fill them.

- [ ] **Step 5: Run tests — verify pass**

Run: `cargo test -p cli_agent_usage --lib`
Expected: PASS (`token_counts_total_and_add`, `severity_default_is_normal`).

- [ ] **Step 6: Commit**
```bash
git add crates/cli_agent_usage Cargo.toml Cargo.lock
git commit -m "feat(usage): scaffold cli_agent_usage crate with core types"
```

---

### Task 2: Pricing table

**Files:**
- Modify: `crates/cli_agent_usage/src/pricing.rs`

**Interfaces:**
- Produces: `pub fn cost(model: &str, t: &TokenCounts) -> f64` (USD estimate).

- [ ] **Step 1: Write the failing test**

`crates/cli_agent_usage/src/pricing.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::cost;
    use crate::TokenCounts;

    #[test]
    fn opus_priced_per_mtok() {
        // 1M input @ $15, 1M output @ $75, 1M cache_read @ $1.50, 1M cache_write @ $18.75
        let t = TokenCounts { input: 1_000_000, output: 1_000_000, cache_read: 1_000_000, cache_write: 1_000_000 };
        let c = cost("claude-opus-4-8", &t);
        assert!((c - (15.0 + 75.0 + 1.50 + 18.75)).abs() < 1e-6, "got {c}");
    }

    #[test]
    fn codex_gpt5_priced() {
        // 1M input @ $1.25, 1M output @ $10, 1M cache_read @ $0.125
        let t = TokenCounts { input: 1_000_000, output: 1_000_000, cache_read: 1_000_000, cache_write: 0 };
        let c = cost("gpt-5.5", &t);
        assert!((c - (1.25 + 10.0 + 0.125)).abs() < 1e-6, "got {c}");
    }

    #[test]
    fn unknown_model_is_zero_cost() {
        let t = TokenCounts { input: 1_000_000, output: 0, cache_read: 0, cache_write: 0 };
        assert_eq!(cost("totally-unknown-model", &t), 0.0);
    }
}
```

- [ ] **Step 2: Run test — verify it fails**

Run: `cargo test -p cli_agent_usage --lib pricing`
Expected: FAIL — `cost` not found.

- [ ] **Step 3: Implement**

Replace the stub `cost` in `pricing.rs` (above the test module) with the real table:
```rust
//! Estimated USD pricing per model. Rates are USD per 1,000,000 tokens.
//! ESTIMATE ONLY — maintain against published pricing. Matching is by substring so
//! version suffixes (claude-opus-4-7 / -4-8, gpt-5 / gpt-5.5 / gpt-5-codex) all resolve.

use crate::TokenCounts;

#[derive(Clone, Copy)]
struct Rates {
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: f64,
}

fn rates(model: &str) -> Option<Rates> {
    let m = model.to_ascii_lowercase();
    // Claude
    if m.contains("opus") {
        return Some(Rates { input: 15.0, output: 75.0, cache_read: 1.50, cache_write: 18.75 });
    }
    if m.contains("sonnet") {
        return Some(Rates { input: 3.0, output: 15.0, cache_read: 0.30, cache_write: 3.75 });
    }
    if m.contains("haiku") {
        return Some(Rates { input: 0.80, output: 4.0, cache_read: 0.08, cache_write: 1.0 });
    }
    if m.contains("fable") {
        // Placeholder until Fable 5 pricing is published; sonnet-class estimate.
        return Some(Rates { input: 3.0, output: 15.0, cache_read: 0.30, cache_write: 3.75 });
    }
    // Codex / OpenAI (gpt-5 family, incl. gpt-5-codex, gpt-5.5)
    if m.contains("gpt-5") || m.contains("gpt5") || m.contains("codex") {
        return Some(Rates { input: 1.25, output: 10.0, cache_read: 0.125, cache_write: 0.0 });
    }
    None
}

pub fn cost(model: &str, t: &TokenCounts) -> f64 {
    let Some(r) = rates(model) else {
        eprintln!("[cli_agent_usage] no pricing for model '{model}'; counting 0 cost");
        return 0.0;
    };
    let per = 1_000_000.0;
    (t.input as f64) / per * r.input
        + (t.output as f64) / per * r.output
        + (t.cache_read as f64) / per * r.cache_read
        + (t.cache_write as f64) / per * r.cache_write
}
```

- [ ] **Step 4: Run tests — verify pass**

Run: `cargo test -p cli_agent_usage --lib pricing`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**
```bash
git add crates/cli_agent_usage/src/pricing.rs
git commit -m "feat(usage): per-model estimated pricing table"
```

---

### Task 3: Incremental file scan cache

**Files:**
- Modify: `crates/cli_agent_usage/src/cache.rs`

**Interfaces:**
- Produces:
  - `struct ScanCache<T> { ... }` with `fn new()`, `fn get_or_parse(&mut self, path:&Path, mtime:SystemTime, size:u64, parse: impl FnOnce(&Path)->T) -> &T`
  - `fn scan_dir(root:&Path, ext:&str) -> Vec<(PathBuf, SystemTime, u64)>` (recursive, files ending in `ext`, skips unreadable/missing dir → empty Vec)

- [ ] **Step 1: Write the failing tests**

`crates/cli_agent_usage/src/cache.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::SystemTime;

    fn tmp() -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("cau_cache_{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn scan_dir_lists_only_matching_ext() {
        let d = tmp();
        fs::write(d.join("a.jsonl"), "x").unwrap();
        fs::write(d.join("b.txt"), "x").unwrap();
        fs::create_dir_all(d.join("sub")).unwrap();
        fs::write(d.join("sub/c.jsonl"), "x").unwrap();
        let mut found: Vec<_> = scan_dir(&d, ".jsonl").into_iter().map(|(p, _, _)| p).collect();
        found.sort();
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|p| p.extension().unwrap() == "jsonl"));
    }

    #[test]
    fn scan_dir_missing_is_empty() {
        assert!(scan_dir(std::path::Path::new("/no/such/dir/xyz"), ".jsonl").is_empty());
    }

    #[test]
    fn cache_reparses_only_on_change() {
        let mut c: ScanCache<u32> = ScanCache::new();
        let p = std::path::Path::new("/fake/x.jsonl");
        let calls = std::cell::Cell::new(0u32);
        let m1 = SystemTime::UNIX_EPOCH;
        let v = *c.get_or_parse(p, m1, 10, |_| { calls.set(calls.get() + 1); 42 });
        assert_eq!(v, 42);
        // same mtime+size -> no re-parse
        let _ = c.get_or_parse(p, m1, 10, |_| { calls.set(calls.get() + 1); 99 });
        assert_eq!(calls.get(), 1);
        // changed size -> re-parse
        let v2 = *c.get_or_parse(p, m1, 11, |_| { calls.set(calls.get() + 1); 7 });
        assert_eq!(v2, 7);
        assert_eq!(calls.get(), 2);
    }
}
```

- [ ] **Step 2: Run — verify fail**

Run: `cargo test -p cli_agent_usage --lib cache`
Expected: FAIL — `ScanCache`/`scan_dir` not found.

- [ ] **Step 3: Implement**

Replace `//! stub` in `cache.rs` with:
```rust
//! Incremental file cache: parse a file only when its (mtime, size) changed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

struct Entry<T> {
    mtime: SystemTime,
    size: u64,
    value: T,
}

pub struct ScanCache<T> {
    entries: HashMap<PathBuf, Entry<T>>,
}

impl<T> ScanCache<T> {
    pub fn new() -> Self {
        ScanCache { entries: HashMap::new() }
    }

    pub fn get_or_parse(
        &mut self,
        path: &Path,
        mtime: SystemTime,
        size: u64,
        parse: impl FnOnce(&Path) -> T,
    ) -> &T {
        let fresh = match self.entries.get(path) {
            Some(e) => e.mtime == mtime && e.size == size,
            None => false,
        };
        if !fresh {
            let value = parse(path);
            self.entries.insert(path.to_path_buf(), Entry { mtime, size, value });
        }
        &self.entries.get(path).expect("just inserted").value
    }
}

impl<T> Default for ScanCache<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Recursively list files under `root` whose name ends with `ext` (e.g. ".jsonl").
/// Missing/unreadable dir → empty vec (fail-soft).
pub fn scan_dir(root: &Path, ext: &str) -> Vec<(PathBuf, SystemTime, u64)> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if !path.to_string_lossy().ends_with(ext) {
            continue;
        }
        if let Ok(md) = entry.metadata() {
            let mtime = md.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            out.push((path.to_path_buf(), mtime, md.len()));
        }
    }
    out
}
```

- [ ] **Step 4: Run — verify pass**

Run: `cargo test -p cli_agent_usage --lib cache`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**
```bash
git add crates/cli_agent_usage/src/cache.rs
git commit -m "feat(usage): incremental (mtime,size) scan cache"
```

---

### Task 4: Claude transcript parser + scan

**Files:**
- Modify: `crates/cli_agent_usage/src/claude.rs`

**Interfaces:**
- Consumes: `Entry`, `TokenCounts`, `Provider`, `WindowTotals`, `aggregate_windows`, `cache::{ScanCache, scan_dir}`.
- Produces:
  - `pub fn parse_transcript_str(content: &str) -> Vec<Entry>`
  - `pub fn parse_transcript_file(path: &Path) -> Vec<Entry>`
  - `pub fn scan(projects_dir: &Path, cache: &mut ScanCache<Vec<Entry>>, now: DateTime<Utc>) -> Provider` (fills session/today/week/month; `plan` left `None` — set in Task 7)

- [ ] **Step 1: Write the failing tests**

`crates/cli_agent_usage/src/claude.rs`:
```rust
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
        assert_eq!(e.tokens, TokenCounts { input: 6, output: 218, cache_write: 29086, cache_read: 4 });
        assert_eq!(e.dedup, "req_1:msg_1");
        assert_eq!(e.ts, Utc.with_ymd_and_hms(2026, 6, 30, 21, 16, 28).unwrap() + chrono::Duration::milliseconds(384));
    }

    #[test]
    fn skips_non_assistant_and_malformed() {
        let content = format!(
            "{}\n{}\n{}",
            r#"{"type":"user","message":{"role":"user"}}"#,
            "not json at all",
            LINE
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
```

- [ ] **Step 2: Run — verify fail**

Run: `cargo test -p cli_agent_usage --lib claude`
Expected: FAIL — functions not found.

- [ ] **Step 3: Implement**

Replace `//! stub` in `claude.rs` with:
```rust
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
        let Some(ts_str) = line.timestamp.as_deref() else { continue };
        let Ok(ts) = DateTime::parse_from_rfc3339(ts_str) else { continue };
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

pub fn scan(projects_dir: &Path, cache: &mut ScanCache<Vec<Entry>>, now: DateTime<Utc>) -> Provider {
    let mut provider = Provider::default();
    let mut seen = std::collections::HashSet::new();

    let files = scan_dir(projects_dir, ".jsonl");
    // session = the most-recently-modified transcript
    let latest = files.iter().max_by_key(|(_, mtime, _)| *mtime).map(|(p, _, _)| p.clone());

    for (path, mtime, size) in &files {
        let entries = cache.get_or_parse(path, *mtime, *size, |p| parse_transcript_file(p)).clone();
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
```

> `get_or_parse(...).clone()` returns an owned `Vec<Entry>` so the borrow on `cache` ends
> before the next loop iteration. Acceptable: entries are small and this runs off the UI thread.

- [ ] **Step 4: Run — verify pass**

Run: `cargo test -p cli_agent_usage --lib claude`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**
```bash
git add crates/cli_agent_usage/src/claude.rs
git commit -m "feat(usage): Claude transcript parser + window scan"
```

---

### Task 5: Codex rollout parser + scan (incl. local rate-limits)

**Files:**
- Modify: `crates/cli_agent_usage/src/codex.rs`

**Interfaces:**
- Consumes: `Entry`, `TokenCounts`, `Provider`, `WindowTotals`, `LimitWindow`, `PlanLimits`, `Severity`, `aggregate_windows`, `cache::{ScanCache, scan_dir}`.
- Produces:
  - `pub struct RollupFile { pub entries: Vec<Entry>, pub last_total: TokenCounts, pub rate_limits: Option<PlanLimits> }`
  - `pub fn parse_rollout_str(content: &str) -> RollupFile`
  - `pub fn parse_rollout_file(path: &Path) -> RollupFile`
  - `pub fn scan(sessions_dir: &Path, cache: &mut ScanCache<RollupFile>, now: DateTime<Utc>) -> Provider`
  - `pub fn severity_from_percent(p: f64) -> Severity` (shared with Claude fallback): `<75 Normal`, `<90 Warning`, else `Critical`.

- [ ] **Step 1: Write the failing tests**

`crates/cli_agent_usage/src/codex.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::Severity;

    // Two token_count events: deltas are (2000 input/100 out) then (+500 input/+50 out).
    const A: &str = r#"{"timestamp":"2026-06-30T10:00:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":2000,"cached_input_tokens":1000,"output_tokens":100,"reasoning_output_tokens":10,"total_tokens":2100}},"rate_limits":{"primary":{"used_percent":9.0,"window_minutes":300,"resets_at":1782425344},"secondary":{"used_percent":18.0,"window_minutes":10080,"resets_at":1782421135},"plan_type":"prolite"}}}"#;
    const B: &str = r#"{"timestamp":"2026-06-30T11:00:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":2500,"cached_input_tokens":1200,"output_tokens":150,"reasoning_output_tokens":20,"total_tokens":2650}},"rate_limits":{"primary":{"used_percent":20.0,"window_minutes":300,"resets_at":1782461677},"secondary":{"used_percent":19.0,"window_minutes":10080,"resets_at":1783028371},"plan_type":"prolite"}}}"#;
    const META: &str = r#"{"timestamp":"2026-06-30T09:59:00.000Z","type":"session_meta","payload":{"model":"gpt-5.5"}}"#;

    #[test]
    fn deltas_and_uncached_split() {
        let r = parse_rollout_str(&format!("{META}\n{A}\n{B}"));
        assert_eq!(r.entries.len(), 2);
        // first event: uncached = 2000-1000=1000 input, cache_read=1000, output=100
        assert_eq!(r.entries[0].tokens, crate::TokenCounts { input: 1000, output: 100, cache_read: 1000, cache_write: 0 });
        assert_eq!(r.entries[0].model, "gpt-5.5");
        // second event delta: total input 2500-2000=500 of which cached 1200-1000=200 => uncached 300
        assert_eq!(r.entries[1].tokens, crate::TokenCounts { input: 300, output: 50, cache_read: 200, cache_write: 0 });
        // session total = last event cumulative, split uncached
        assert_eq!(r.last_total, crate::TokenCounts { input: 1300, output: 150, cache_read: 1200, cache_write: 0 });
    }

    #[test]
    fn rate_limits_map_to_session_and_weekly() {
        let r = parse_rollout_str(A);
        let plan = r.rate_limits.unwrap();
        assert_eq!(plan.session.unwrap().percent, 9.0);
        assert_eq!(plan.weekly.unwrap().percent, 18.0);
        assert_eq!(plan.session.unwrap().severity, Severity::Normal);
    }

    #[test]
    fn empty_or_malformed_is_empty() {
        let r = parse_rollout_str("garbage\n\n{}");
        assert!(r.entries.is_empty());
        assert!(r.rate_limits.is_none());
    }
}
```

- [ ] **Step 2: Run — verify fail**

Run: `cargo test -p cli_agent_usage --lib codex`
Expected: FAIL — symbols not found.

- [ ] **Step 3: Implement**

Replace `//! stub` in `codex.rs` with:
```rust
//! Parse Codex rollout sessions (~/.codex/sessions/**/rollout-*.jsonl).

use std::path::Path;

use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;

use crate::cache::{scan_dir, ScanCache};
use crate::{aggregate_windows, Entry, LimitWindow, PlanLimits, Provider, Severity, TokenCounts, WindowTotals};

#[derive(Default)]
pub struct RollupFile {
    pub entries: Vec<Entry>,
    pub last_total: TokenCounts,
    pub rate_limits: Option<PlanLimits>,
}

#[derive(Deserialize)]
struct Line {
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    payload: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct TotalUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    cached_input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

#[derive(Deserialize)]
struct Window {
    #[serde(default)]
    used_percent: f64,
    #[serde(default)]
    resets_at: Option<i64>,
}

#[derive(Deserialize)]
struct RateLimits {
    primary: Option<Window>,
    secondary: Option<Window>,
}

pub fn severity_from_percent(p: f64) -> Severity {
    if p < 75.0 {
        Severity::Normal
    } else if p < 90.0 {
        Severity::Warning
    } else {
        Severity::Critical
    }
}

fn window_to_limit(w: &Window) -> LimitWindow {
    LimitWindow {
        percent: w.used_percent,
        resets_at: w.resets_at.and_then(|s| Utc.timestamp_opt(s, 0).single()),
        severity: severity_from_percent(w.used_percent),
    }
}

/// `total_token_usage` is cumulative per session: split into uncached input vs cache_read.
fn split(total: &TotalUsage) -> TokenCounts {
    TokenCounts {
        input: total.input_tokens.saturating_sub(total.cached_input_tokens),
        output: total.output_tokens,
        cache_read: total.cached_input_tokens,
        cache_write: 0,
    }
}

pub fn parse_rollout_str(content: &str) -> RollupFile {
    let mut out = RollupFile::default();
    let mut model = "gpt-5-codex".to_string();
    let mut prev_cumulative: Option<TokenCounts> = None;

    for raw in content.lines() {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let Ok(line) = serde_json::from_str::<Line>(raw) else {
            continue;
        };
        let Some(payload) = line.payload.as_ref() else { continue };

        // Track the latest model id seen anywhere in the file.
        if let Some(m) = payload.get("model").and_then(|v| v.as_str()) {
            model = m.to_string();
        }

        if payload.get("type").and_then(|v| v.as_str()) != Some("token_count") {
            continue;
        }
        let Some(ts_str) = line.timestamp.as_deref() else { continue };
        let Ok(ts) = DateTime::parse_from_rfc3339(ts_str) else { continue };
        let ts = ts.with_timezone(&Utc);

        if let Some(info_total) = payload
            .get("info")
            .and_then(|i| i.get("total_token_usage"))
            .and_then(|t| serde_json::from_value::<TotalUsage>(t.clone()).ok())
        {
            let cumulative = split(&info_total);
            // delta vs previous cumulative (clamped at 0 in case of resets)
            let delta = match prev_cumulative {
                Some(p) => TokenCounts {
                    input: cumulative.input.saturating_sub(p.input),
                    output: cumulative.output.saturating_sub(p.output),
                    cache_read: cumulative.cache_read.saturating_sub(p.cache_read),
                    cache_write: 0,
                },
                None => cumulative,
            };
            prev_cumulative = Some(cumulative);
            out.last_total = cumulative;
            out.entries.push(Entry {
                ts,
                model: model.clone(),
                tokens: delta,
                dedup: String::new(),
            });
        }

        if let Some(rl) = payload
            .get("rate_limits")
            .and_then(|v| serde_json::from_value::<RateLimits>(v.clone()).ok())
        {
            out.rate_limits = Some(PlanLimits {
                session: rl.primary.as_ref().map(window_to_limit),
                weekly: rl.secondary.as_ref().map(window_to_limit),
            });
        }
    }
    out
}

pub fn parse_rollout_file(path: &Path) -> RollupFile {
    match std::fs::read_to_string(path) {
        Ok(s) => parse_rollout_str(&s),
        Err(_) => RollupFile::default(),
    }
}

pub fn scan(sessions_dir: &Path, cache: &mut ScanCache<RollupFile>, now: DateTime<Utc>) -> Provider {
    let mut provider = Provider::default();
    let mut seen = std::collections::HashSet::new();

    let mut files = scan_dir(sessions_dir, ".jsonl");
    files.retain(|(p, _, _)| {
        p.file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with("rollout-"))
            .unwrap_or(false)
    });
    let latest = files.iter().max_by_key(|(_, mtime, _)| *mtime).map(|(p, _, _)| p.clone());

    for (path, mtime, size) in &files {
        let parsed = cache.get_or_parse(path, *mtime, *size, |p| parse_rollout_file(p));
        let entries = parsed.entries.clone();
        let is_latest = Some(path) == latest.as_ref();
        let last_total = parsed.last_total;
        let rate_limits = if is_latest { parsed.rate_limits } else { None };

        aggregate_windows(
            &entries,
            now,
            &mut seen,
            &mut provider.today,
            &mut provider.week,
            &mut provider.month,
        );
        if is_latest {
            let model = entries.last().map(|e| e.model.clone()).unwrap_or_default();
            let mut s = WindowTotals::default();
            s.tokens = last_total;
            s.cost_usd = crate::pricing::cost(&model, &last_total);
            provider.session = s;
            provider.plan = rate_limits;
        }
    }
    provider
}
```

> `parsed.rate_limits` is `Option<PlanLimits>` and `PlanLimits` is `Copy`, so `parsed.rate_limits`
> copies out without moving from the cache borrow.

- [ ] **Step 4: Run — verify pass**

Run: `cargo test -p cli_agent_usage --lib codex`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**
```bash
git add crates/cli_agent_usage/src/codex.rs
git commit -m "feat(usage): Codex rollout parser, window deltas, local rate-limits"
```

---

### Task 6: Keychain reader

**Files:**
- Modify: `crates/cli_agent_usage/src/keychain.rs`

**Interfaces:**
- Produces:
  - `pub trait ReadSecret { fn read(&self, service: &str, account: &str) -> Option<String>; }`
  - `pub struct ClaudeToken { pub access_token: String, pub expires_at_ms: Option<i64> }` with `pub fn is_expired(&self, now_ms: i64) -> bool`
  - `pub fn parse_claude_token(blob: &str) -> Option<ClaudeToken>`
  - `pub fn read_claude_token(reader: &dyn ReadSecret, account: &str) -> Option<ClaudeToken>`
  - `pub struct MacKeychain;` impl `ReadSecret` (macOS, via `security-framework`); non-macOS impl returns `None`.

- [ ] **Step 1: Write the failing tests**

`crates/cli_agent_usage/src/keychain.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(&'static str);
    impl ReadSecret for Fake {
        fn read(&self, _s: &str, _a: &str) -> Option<String> {
            Some(self.0.to_string())
        }
    }

    const BLOB: &str = r#"{"mcpOAuth":{},"claudeAiOauth":{"accessToken":"tok_abc","refreshToken":"r","expiresAt":1782879812921,"scopes":["user:inference"],"subscriptionType":"max"}}"#;

    #[test]
    fn parses_access_token_and_expiry() {
        let t = parse_claude_token(BLOB).unwrap();
        assert_eq!(t.access_token, "tok_abc");
        assert_eq!(t.expires_at_ms, Some(1782879812921));
        assert!(!t.is_expired(1782879812921 - 1000));
        assert!(t.is_expired(1782879812921 + 1000));
    }

    #[test]
    fn reads_via_provider() {
        let t = read_claude_token(&Fake(BLOB), "anyuser").unwrap();
        assert_eq!(t.access_token, "tok_abc");
    }

    #[test]
    fn garbage_blob_is_none() {
        assert!(parse_claude_token("not json").is_none());
    }
}
```

- [ ] **Step 2: Run — verify fail**

Run: `cargo test -p cli_agent_usage --lib keychain`
Expected: FAIL — symbols not found.

- [ ] **Step 3: Implement**

Replace `//! stub` in `keychain.rs` with:
```rust
//! Read Claude Code's OAuth token from the OS secret store (macOS Keychain).

use serde::Deserialize;

pub const CLAUDE_SERVICE: &str = "Claude Code-credentials";

pub trait ReadSecret {
    /// Return the stored secret string for (service, account), or None.
    fn read(&self, service: &str, account: &str) -> Option<String>;
}

#[derive(Debug, Clone)]
pub struct ClaudeToken {
    pub access_token: String,
    pub expires_at_ms: Option<i64>,
}

impl ClaudeToken {
    pub fn is_expired(&self, now_ms: i64) -> bool {
        match self.expires_at_ms {
            Some(exp) => now_ms >= exp,
            None => false,
        }
    }
}

#[derive(Deserialize)]
struct Blob {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: Option<OAuth>,
}

#[derive(Deserialize)]
struct OAuth {
    #[serde(rename = "accessToken")]
    access_token: Option<String>,
    #[serde(rename = "expiresAt")]
    expires_at: Option<i64>,
}

pub fn parse_claude_token(blob: &str) -> Option<ClaudeToken> {
    let parsed: Blob = serde_json::from_str(blob).ok()?;
    let oauth = parsed.claude_ai_oauth?;
    let access_token = oauth.access_token?;
    if access_token.is_empty() {
        return None;
    }
    Some(ClaudeToken { access_token, expires_at_ms: oauth.expires_at })
}

pub fn read_claude_token(reader: &dyn ReadSecret, account: &str) -> Option<ClaudeToken> {
    let blob = reader.read(CLAUDE_SERVICE, account)?;
    parse_claude_token(&blob)
}

pub struct MacKeychain;

#[cfg(target_os = "macos")]
impl ReadSecret for MacKeychain {
    fn read(&self, service: &str, account: &str) -> Option<String> {
        let pw = security_framework::passwords::get_generic_password(service, account).ok()?;
        String::from_utf8(pw).ok()
    }
}

#[cfg(not(target_os = "macos"))]
impl ReadSecret for MacKeychain {
    fn read(&self, _service: &str, _account: &str) -> Option<String> {
        None
    }
}
```

> If `security-framework` 3.x exposes the password API at a different path, adjust the call;
> the function returns the raw bytes for the generic password matching (service, account).
> The OS account name is the login user — obtain at the call site via `std::env::var("USER")`.

- [ ] **Step 4: Run — verify pass**

Run: `cargo test -p cli_agent_usage --lib keychain`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**
```bash
git add crates/cli_agent_usage/src/keychain.rs
git commit -m "feat(usage): Keychain reader + Claude token parsing"
```

---

### Task 7: Claude usage HTTP client + plan-limit parser

**Files:**
- Modify: `crates/cli_agent_usage/src/http.rs`

**Interfaces:**
- Consumes: `PlanLimits`, `LimitWindow`, `Severity`, `codex::severity_from_percent`.
- Produces:
  - `pub trait FetchUsage { fn fetch(&self, access_token: &str) -> Result<String, String>; }`
  - `pub fn parse_plan_limits(json: &str) -> Option<PlanLimits>`
  - `pub struct ReqwestUsage;` impl `FetchUsage` (real GET).
  - `pub const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";`

- [ ] **Step 1: Write the failing tests**

`crates/cli_agent_usage/src/http.rs`:
```rust
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
```

- [ ] **Step 2: Run — verify fail**

Run: `cargo test -p cli_agent_usage --lib http`
Expected: FAIL — symbols not found.

- [ ] **Step 3: Implement**

Replace `//! stub` in `http.rs` with:
```rust
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
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc))
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
            resp.limits.iter().find(|l| l.group == group).map(|l| LimitWindow {
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
```

- [ ] **Step 4: Run — verify pass**

Run: `cargo test -p cli_agent_usage --lib http`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**
```bash
git add crates/cli_agent_usage/src/http.rs
git commit -m "feat(usage): Claude usage endpoint client + plan-limit parser"
```

---

### Task 8: Compose `refresh()` + end-to-end example

**Files:**
- Modify: `crates/cli_agent_usage/src/lib.rs` (add `refresh` + `Paths`)
- Create: `crates/cli_agent_usage/examples/print_usage.rs`

**Interfaces:**
- Consumes: everything above.
- Produces:
  - `pub struct Paths { pub claude_projects: PathBuf, pub codex_sessions: PathBuf, pub os_account: String }` with `pub fn detect() -> Option<Paths>` (uses `$HOME` + `$USER`)
  - `pub struct Caches { claude: ScanCache<Vec<Entry>>, codex: ScanCache<codex::RollupFile> }` with `pub fn new()`
  - `pub fn refresh(paths:&Paths, caches:&mut Caches, now: DateTime<Utc>, secret:&dyn keychain::ReadSecret, fetch:&dyn http::FetchUsage) -> UsageSnapshot`

- [ ] **Step 1: Write the failing test**

Append to the `tests` module in `crates/cli_agent_usage/src/lib.rs`:
```rust
    #[test]
    fn refresh_is_fail_soft_with_no_files_and_no_token() {
        struct NoSecret;
        impl crate::keychain::ReadSecret for NoSecret {
            fn read(&self, _: &str, _: &str) -> Option<String> { None }
        }
        struct NoFetch;
        impl crate::http::FetchUsage for NoFetch {
            fn fetch(&self, _: &str) -> Result<String, String> { Err("no".into()) }
        }
        let paths = Paths {
            claude_projects: "/no/such/claude".into(),
            codex_sessions: "/no/such/codex".into(),
            os_account: "nobody".into(),
        };
        let mut caches = Caches::new();
        let snap = refresh(&paths, &mut caches, chrono::Utc::now(), &NoSecret, &NoFetch);
        assert_eq!(snap.claude.month.tokens.total(), 0);
        assert!(snap.claude.plan.is_none());
        assert_eq!(snap.codex.month.tokens.total(), 0);
    }
```

- [ ] **Step 2: Run — verify fail**

Run: `cargo test -p cli_agent_usage --lib refresh_is_fail_soft`
Expected: FAIL — `Paths`/`Caches`/`refresh` not found.

- [ ] **Step 3: Implement**

Add to `crates/cli_agent_usage/src/lib.rs` (above the test module, after the types):
```rust
use std::path::PathBuf;

use crate::cache::ScanCache;

pub struct Paths {
    pub claude_projects: PathBuf,
    pub codex_sessions: PathBuf,
    pub os_account: String,
}

impl Paths {
    pub fn detect() -> Option<Paths> {
        let home = std::env::var("HOME").ok()?;
        let os_account = std::env::var("USER").unwrap_or_default();
        Some(Paths {
            claude_projects: PathBuf::from(&home).join(".claude/projects"),
            codex_sessions: PathBuf::from(&home).join(".codex/sessions"),
            os_account,
        })
    }
}

pub struct Caches {
    claude: ScanCache<Vec<Entry>>,
    codex: ScanCache<codex::RollupFile>,
}

impl Caches {
    pub fn new() -> Self {
        Caches { claude: ScanCache::new(), codex: ScanCache::new() }
    }
}

impl Default for Caches {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a full snapshot. Every source is independent and fail-soft.
pub fn refresh(
    paths: &Paths,
    caches: &mut Caches,
    now: DateTime<Utc>,
    secret: &dyn keychain::ReadSecret,
    fetch: &dyn http::FetchUsage,
) -> UsageSnapshot {
    let mut claude = claude::scan(&paths.claude_projects, &mut caches.claude, now);
    let codex = codex::scan(&paths.codex_sessions, &mut caches.codex, now);

    // Claude plan-% via Keychain token + endpoint (best-effort).
    claude.plan = (|| {
        let token = keychain::read_claude_token(secret, &paths.os_account)?;
        if token.is_expired(now.timestamp_millis()) {
            return None;
        }
        let body = fetch.fetch(&token.access_token).ok()?;
        http::parse_plan_limits(&body)
    })();

    UsageSnapshot { claude, codex }
}
```

- [ ] **Step 4: Run — verify pass**

Run: `cargo test -p cli_agent_usage --lib`
Expected: PASS (all lib tests including `refresh_is_fail_soft_with_no_files_and_no_token`).

- [ ] **Step 5: Write the end-to-end example**

`crates/cli_agent_usage/examples/print_usage.rs`:
```rust
//! Manual end-to-end check: prints real usage for the current machine.
//! Run: cargo run -p cli_agent_usage --example print_usage

use cli_agent_usage::{http::ReqwestUsage, keychain::MacKeychain, refresh, Caches, Paths, Provider};

fn fmt_provider(name: &str, p: &Provider) {
    println!("== {name} ==");
    let tok = |w: &cli_agent_usage::WindowTotals| {
        format!("{} tok  ~${:.2}", w.tokens.total(), w.cost_usd)
    };
    println!("  session: {}", tok(&p.session));
    println!("  today:   {}", tok(&p.today));
    println!("  week:    {}", tok(&p.week));
    println!("  month:   {}", tok(&p.month));
    match &p.plan {
        Some(pl) => {
            if let Some(s) = pl.session {
                println!("  5h limit:   {:.0}%  (resets {:?})", s.percent, s.resets_at);
            }
            if let Some(w) = pl.weekly {
                println!("  weekly lim: {:.0}%  (resets {:?})", w.percent, w.resets_at);
            }
        }
        None => println!("  plan-%: (unavailable)"),
    }
}

fn main() {
    let paths = Paths::detect().expect("HOME set");
    let mut caches = Caches::new();
    let now = chrono::Utc::now();
    let snap = refresh(&paths, &mut caches, now, &MacKeychain, &ReqwestUsage);
    fmt_provider("Claude Code", &snap.claude);
    fmt_provider("Codex", &snap.codex);
}
```

- [ ] **Step 6: Run the example — verify real output**

Run: `cargo run -p cli_agent_usage --example print_usage`
Expected: prints non-zero `month` token totals for both providers; Claude `5h limit`/`weekly lim` percentages appear (the live endpoint call); Codex percentages appear from the local rate-limits. If Claude plan-% shows `(unavailable)`, confirm the Keychain entry exists and the token is unexpired — this is the one networked path.

- [ ] **Step 7: Commit**
```bash
git add crates/cli_agent_usage/src/lib.rs crates/cli_agent_usage/examples/print_usage.rs
git commit -m "feat(usage): compose refresh() + print_usage example"
```

---

## Self-Review

**Spec coverage:**
- §3.1 Claude tokens → Task 4. §3.2 Claude plan-% (Keychain + endpoint) → Tasks 6, 7. §3.3 Codex tokens + rate-limits → Task 5. §3.4 source map → Task 8 `refresh`. §7 windows → `aggregate_windows` (Task 1) + per-provider scan. §8 pricing → Task 2. §10 fail-soft → every parser returns empty on error + Task 8 `refresh` best-effort closure + Task 8 test. §12 testing → unit tests each task + `print_usage` for end-to-end. §13 deps → Task 1 manifest. **UI (§9) and the app singleton (§4 `/app` parts) are intentionally deferred to Plan B** — this plan delivers the data layer only.
- Gap check: none for the data layer. Plan B will cover the singleton model, toolbar item, chip, and panel.

**Placeholder scan:** No "TBD"/"add error handling"/"similar to Task N". Every code step shows complete code. The one external-reality caveat (security-framework 3.x API path, Task 6) names the exact function and contract to match.

**Type consistency:** `TokenCounts`/`WindowTotals`/`Provider`/`PlanLimits`/`LimitWindow`/`Severity`/`Entry` defined once in Task 1 and used verbatim after. `aggregate_windows` signature (Task 1) matches every call (Tasks 4, 5). `ScanCache::get_or_parse` signature (Task 3) matches calls in Tasks 4, 5. `ReadSecret`/`FetchUsage` traits (Tasks 6, 7) match `refresh` params (Task 8). `severity_from_percent` defined in Task 5, reused in Task 7.

---

## Follow-up: Plan B (separate document, written after Plan A lands)

Plan B wires `UsageSnapshot` into the footer, against warpui's live APIs (read at execution time):
1. `app/src/ai/blocklist/usage/cli_agent_usage_model.rs` — `SingletonEntity` running `refresh` on a background thread (reqwest is blocking) via two cadences (files ~5s, endpoint ~60s), storing the latest `UsageSnapshot`, emitting a change event. Modeled on `AIRequestUsageModel` (`app/src/ai/request_usage_model.rs:184`) **minus** the `is_logged_in()` gate.
2. Register the singleton in `app/src/lib.rs` (~line 1382).
3. `toolbar_item.rs:48` — add `AgentToolbarItemKind::CliAgentUsage`; render the chip in `render_cli_mode_footer()` (`agent_input_footer/mod.rs` ~1493), color from `Severity` reusing `icon_for_context_window_usage` color logic (`app/src/ai/blocklist/usage/mod.rs:8`).
4. The expandable panel (2-col Claude|Codex grid) as a popover following the existing chip+popover pattern.
5. Subscribe to the singleton in `AgentInputFooter::new()` (~line 258) and `ctx.notify()` on change.
