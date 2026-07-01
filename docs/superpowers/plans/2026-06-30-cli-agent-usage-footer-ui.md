# CLI Agent Usage Footer (Plan B) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the finished `cli_agent_usage` crate into the Clinch CLI-agent footer as a compact plan-% chip that expands into a token/plan panel.

**Architecture:** A new `CliAgentUsageModel` singleton owns a dedicated `std::thread` that runs the crate's blocking file-scan + HTTP refresh on a split cadence (files ~5s, Claude endpoint ~60s, last-known-good retained) and pushes each `UsageSnapshot` over an `async_channel`; `ctx.spawn_stream_local` delivers snapshots on the main thread and `ctx.emit`s. The footer renders a custom two-colored chip (per-provider weekly plan-%) that toggles a positioned-overlay panel.

**Tech Stack:** Rust, `cli_agent_usage` workspace crate (chrono, reqwest::blocking, security-framework), Warp's gpui-derived `warpui` toolkit, `async_channel`.

**Spec:** `docs/superpowers/specs/2026-06-30-cli-agent-usage-footer-ui.md`

## Global Constraints

Every task's requirements implicitly include these (verbatim from the spec):

- **Blocking work runs ONLY on a dedicated `std::thread`** — never the gpui `background_executor` or `ctx.spawn` future slot (both are Tokio; `reqwest::blocking` panics under Tokio).
- **Fail-soft everywhere:** any missing dir / malformed line / absent-or-expired token / HTTP error → that cell renders `—`; the footer never blocks or panics.
- **Last-known-good plan-%:** the Claude `PlanLimits` is overwritten only on a successful fetch, so transient failures don't blank the chip.
- **The only outbound network call is `GET https://api.anthropic.com/api/oauth/usage`** (already inside the crate). The OAuth token stays in memory, is never logged or written to disk (crate's `ClaudeToken` has a redacting `Debug`).
- **No cost in the UI.** `WindowTotals.cost_usd` is never rendered.
- **Panel token metric = input+output headline (`TokenCounts::io()`), cache-read dimmed.** Never surface `TokenCounts::total()` (cache-dominated, misleading).
- **Chip = weekly plan-% per provider**, labels `cc` (Claude Code) / `cx` (Codex), color by crate `Severity` enum. Missing metric → `—` half; chip hidden only when *both* providers are entirely empty.
- **Provider `Severity` → theme color:** `Normal → theme.main_text_color(bg)`, `Warning → Fill::Solid(theme.ui_warning_color())`, `Critical → Fill::Solid(theme.ui_error_color())`.

---

## File Structure

- `crates/cli_agent_usage/src/lib.rs` — **modify**: add `TokenCounts::io()`, `scan_local()`, `fetch_claude_plan()`; refactor `refresh()` to compose them (removes inline duplication).
- `crates/cli_agent_usage/src/format.rs` — **create**: pure display helpers (`fmt_tokens`, `fmt_pct`, `fmt_reset`, `chip_halves`, `ChipHalf`).
- `app/Cargo.toml` — **modify**: depend on `cli_agent_usage`.
- `app/src/ai/blocklist/usage/cli_agent_usage_model.rs` — **create**: `CliAgentUsageModel` singleton + producer thread.
- `app/src/ai/blocklist/usage/cli_agent_usage_chip.rs` — **create**: chip + panel element builders + color/text helpers.
- `app/src/ai/blocklist/usage/mod.rs` — **modify**: declare + re-export the two new modules.
- `app/src/lib.rs` — **modify** (~1362): register the singleton.
- `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs` — **modify**: render the chip directly in `render_cli_mode_footer` (always-on, not via the persisted `AgentToolbarItemKind` selection — see Task 5 rationale), plus panel overlay, footer state field, toggle action + handler, model subscription.

> **Design note (principal-engineer revision):** the chip is rendered **directly** in `render_cli_mode_footer`, NOT added as an `AgentToolbarItemKind` variant. `AgentToolbarItemKind` is a *persisted settings value*; the CLI footer renders from each user's saved `cli_agent_footer_chip_selection`, so a new default would be invisible to any existing user without a settings migration. Direct rendering makes the always-on usage chip guaranteed-visible and touches one fewer file. This supersedes the `AgentToolbarItemKind` path sketched in spec §10.

---

## Task 1: Crate — `io()`, `scan_local()`, `fetch_claude_plan()` (refactor `refresh`)

**Files:**
- Modify: `crates/cli_agent_usage/src/lib.rs`
- Test: inline `#[cfg(test)] mod tests` in `crates/cli_agent_usage/src/lib.rs`

**Interfaces:**
- Consumes (existing, verified): `TokenCounts { input, output, cache_read, cache_write: u64 }`; `Provider { session, today, week, month: WindowTotals, plan: Option<PlanLimits> }`; `UsageSnapshot { claude, codex: Provider }`; `Paths { claude_projects, codex_sessions: PathBuf, os_account: String }`; `Caches { claude, codex }` (private fields — new fns live in `lib.rs`, same module) with `Caches::new()`; `claude::scan(&Path, &mut ScanCache<Vec<Entry>>, DateTime<Utc>) -> Provider`; `codex::scan(&Path, &mut ScanCache<RollupFile>, DateTime<Utc>) -> Provider`; `keychain::read_claude_token(&dyn ReadSecret, &str) -> Option<ClaudeToken>`; `ClaudeToken::is_expired(&self, now_ms: i64) -> bool`; `ClaudeToken.access_token: String`; `http::FetchUsage::fetch(&self, &str) -> Result<String, String>`; `http::parse_plan_limits(&str) -> Option<PlanLimits>`; `keychain::ReadSecret`; `PlanLimits` (Copy).
- Produces: `TokenCounts::io(&self) -> u64`; `pub fn scan_local(paths: &Paths, caches: &mut Caches, now: DateTime<Utc>) -> UsageSnapshot`; `pub fn fetch_claude_plan(secret: &dyn keychain::ReadSecret, fetch: &dyn http::FetchUsage, paths: &Paths, now: DateTime<Utc>) -> Option<PlanLimits>`. `refresh` keeps its exact existing signature.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `crates/cli_agent_usage/src/lib.rs`:

```rust
#[test]
fn token_counts_io_is_input_plus_output_only() {
    let t = TokenCounts {
        input: 10,
        output: 5,
        cache_read: 100,
        cache_write: 7,
    };
    assert_eq!(t.io(), 15);
    // io() must NOT include cache traffic (unlike total()).
    assert_eq!(t.total(), 122);
}

#[test]
fn scan_local_is_fail_soft_and_leaves_claude_plan_none() {
    let paths = Paths {
        claude_projects: "/no/such/claude".into(),
        codex_sessions: "/no/such/codex".into(),
        os_account: "nobody".into(),
    };
    let mut caches = Caches::new();
    let snap = scan_local(&paths, &mut caches, chrono::Utc::now());
    assert_eq!(snap.claude.month.tokens.total(), 0);
    assert_eq!(snap.codex.month.tokens.total(), 0);
    // scan_local never touches Keychain/HTTP, so Claude plan is always None here.
    assert!(snap.claude.plan.is_none());
}

#[test]
fn fetch_claude_plan_none_without_token_and_some_with_valid_body() {
    use crate::http::FetchUsage;
    use crate::keychain::ReadSecret;

    struct NoSecret;
    impl ReadSecret for NoSecret {
        fn read(&self, _: &str, _: &str) -> Option<String> {
            None
        }
    }
    struct Secret;
    impl ReadSecret for Secret {
        fn read(&self, _: &str, _: &str) -> Option<String> {
            // Non-expired token blob (expiresAt far in the future).
            Some(
                r#"{"claudeAiOauth":{"accessToken":"tok","expiresAt":95617584000000}}"#
                    .to_string(),
            )
        }
    }
    struct Fetch;
    impl FetchUsage for Fetch {
        fn fetch(&self, _: &str) -> Result<String, String> {
            Ok(r#"{"limits":[{"kind":"weekly_all","group":"weekly","percent":47,"severity":"normal","resets_at":"2026-07-04T15:00:00+00:00","is_active":true}]}"#.to_string())
        }
    }
    let paths = Paths {
        claude_projects: "/x".into(),
        codex_sessions: "/x".into(),
        os_account: "me".into(),
    };
    let now = chrono::Utc::now();
    assert!(fetch_claude_plan(&NoSecret, &Fetch, &paths, now).is_none());
    let plan = fetch_claude_plan(&Secret, &Fetch, &paths, now).expect("valid plan");
    assert!(plan.weekly.is_some());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cli_agent_usage io_is_input_plus_output scan_local fetch_claude_plan`
Expected: FAIL — `no method named io`, `cannot find function scan_local`, `cannot find function fetch_claude_plan`.

- [ ] **Step 3: Add `io()` to `impl TokenCounts`**

In `crates/cli_agent_usage/src/lib.rs`, inside `impl TokenCounts` (right after `total`):

```rust
    /// Input + output tokens — the "work" total, excluding cache traffic.
    /// This is the headline metric for the footer (cache-read dominates
    /// `total()` and would mislead).
    pub fn io(&self) -> u64 {
        self.input + self.output
    }
```

- [ ] **Step 4: Add `scan_local` + `fetch_claude_plan` and refactor `refresh`**

Replace the existing `refresh` function body (currently `let mut claude = claude::scan(...); let codex = codex::scan(...); claude.plan = (|| { ... })(); UsageSnapshot { claude, codex }`) with these three functions. Keep `refresh`'s exact signature and doc comment:

```rust
/// Scan both providers' local files into a snapshot. No network, no Keychain:
/// `claude.plan` is always `None` (fetch it separately via [`fetch_claude_plan`]);
/// `codex.plan` is populated from local rate-limit events. Fail-soft.
pub fn scan_local(paths: &Paths, caches: &mut Caches, now: DateTime<Utc>) -> UsageSnapshot {
    let claude = claude::scan(&paths.claude_projects, &mut caches.claude, now);
    let codex = codex::scan(&paths.codex_sessions, &mut caches.codex, now);
    UsageSnapshot { claude, codex }
}

/// The Claude plan-% half of a refresh: read the Keychain token, and if present
/// and unexpired, fetch and parse `/api/oauth/usage`. Best-effort — any failure
/// (no token, expired, network, parse) yields `None`.
///
/// **Blocking** (Keychain + a blocking HTTP call). Call only from a dedicated
/// thread, never a Tokio/async runtime.
pub fn fetch_claude_plan(
    secret: &dyn keychain::ReadSecret,
    fetch: &dyn http::FetchUsage,
    paths: &Paths,
    now: DateTime<Utc>,
) -> Option<PlanLimits> {
    let token = keychain::read_claude_token(secret, &paths.os_account)?;
    if token.is_expired(now.timestamp_millis()) {
        return None;
    }
    let body = fetch.fetch(&token.access_token).ok()?;
    http::parse_plan_limits(&body)
}
```

Then change `refresh`'s body (keeping the signature and the existing `/// **Blocking and NOT async-safe.** ...` doc) to:

```rust
pub fn refresh(
    paths: &Paths,
    caches: &mut Caches,
    now: DateTime<Utc>,
    secret: &dyn keychain::ReadSecret,
    fetch: &dyn http::FetchUsage,
) -> UsageSnapshot {
    let mut snap = scan_local(paths, caches, now);
    snap.claude.plan = fetch_claude_plan(secret, fetch, paths, now);
    snap
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p cli_agent_usage`
Expected: PASS — all existing tests (26) plus the 3 new ones. The refactor is behavior-preserving, so `refresh_*` tests stay green.

- [ ] **Step 6: Lint + example still builds**

Run: `cargo clippy -p cli_agent_usage --all-targets -- -D warnings && cargo build -p cli_agent_usage --examples`
Expected: no warnings; `print_usage` example compiles (refresh signature unchanged).

- [ ] **Step 7: Commit**

```bash
git add crates/cli_agent_usage/src/lib.rs
git commit -m "feat(usage): add TokenCounts::io, scan_local, fetch_claude_plan; refactor refresh"
```

---

## Task 2: Crate — pure `format` module

**Files:**
- Create: `crates/cli_agent_usage/src/format.rs`
- Modify: `crates/cli_agent_usage/src/lib.rs` (add `pub mod format;`)

**Interfaces:**
- Consumes: `Severity` (enum `Normal|Warning|Critical`, derives `Default`+`PartialEq`); `UsageSnapshot`; `Provider`; `PlanLimits { session, weekly: Option<LimitWindow> }` (Copy); `LimitWindow { percent: f64, resets_at: Option<DateTime<Utc>>, severity: Severity }` (Copy); `WindowTotals { tokens: TokenCounts, cost_usd: f64 }`; `TokenCounts::total()`.
- Produces: `fmt_tokens(u64) -> String`; `fmt_pct(f64) -> String`; `fmt_reset(Option<DateTime<Utc>>, DateTime<Utc>) -> String`; `struct ChipHalf { label: &'static str, pct: String, severity: Severity }`; `chip_halves(&UsageSnapshot) -> Option<[ChipHalf; 2]>`.

- [ ] **Step 1: Add the module declaration**

In `crates/cli_agent_usage/src/lib.rs`, next to the other `pub mod` lines (after `pub mod pricing;`):

```rust
pub mod format;
```

- [ ] **Step 2: Write the failing tests (create the file with tests first)**

Create `crates/cli_agent_usage/src/format.rs` with ONLY the test module to start (implementation added in Step 4):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LimitWindow, PlanLimits, Severity, TokenCounts, UsageSnapshot};
    use chrono::{Duration, TimeZone, Utc};

    #[test]
    fn fmt_tokens_boundaries() {
        assert_eq!(fmt_tokens(0), "0");
        assert_eq!(fmt_tokens(999), "999");
        assert_eq!(fmt_tokens(1_000), "1.0k");
        assert_eq!(fmt_tokens(1_500), "1.5k");
        assert_eq!(fmt_tokens(999_999), "1000.0k");
        assert_eq!(fmt_tokens(1_500_000), "1.5M");
        assert_eq!(fmt_tokens(8_316_864_043), "8.3B");
    }

    #[test]
    fn fmt_pct_rounds_to_integer() {
        assert_eq!(fmt_pct(47.0), "47%");
        assert_eq!(fmt_pct(54.6), "55%");
    }

    #[test]
    fn fmt_reset_none_and_relative() {
        let now = Utc.with_ymd_and_hms(2026, 6, 30, 12, 0, 0).unwrap();
        assert_eq!(fmt_reset(None, now), "—");
        assert_eq!(fmt_reset(Some(now + Duration::minutes(12)), now), "in 12m");
        assert_eq!(fmt_reset(Some(now + Duration::hours(3)), now), "in 3h");
        assert_eq!(fmt_reset(Some(now + Duration::days(2)), now), "in 2d");
        assert_eq!(fmt_reset(Some(now - Duration::hours(1)), now), "now");
    }

    #[test]
    fn chip_halves_hidden_when_snapshot_empty() {
        assert!(chip_halves(&UsageSnapshot::default()).is_none());
    }

    #[test]
    fn chip_halves_present_half_and_missing_half() {
        let mut snap = UsageSnapshot::default();
        // Claude: has a weekly plan -> "47%w", Warning.
        snap.claude.plan = Some(PlanLimits {
            session: None,
            weekly: Some(LimitWindow {
                percent: 47.0,
                resets_at: None,
                severity: Severity::Warning,
            }),
        });
        // Codex: token data but no plan -> "—" half, still shown.
        snap.codex.month.tokens = TokenCounts {
            input: 5,
            output: 5,
            cache_read: 0,
            cache_write: 0,
        };
        let [cc, cx] = chip_halves(&snap).expect("has data");
        assert_eq!(cc.label, "cc");
        assert_eq!(cc.pct, "47%w");
        assert_eq!(cc.severity, Severity::Warning);
        assert_eq!(cx.label, "cx");
        assert_eq!(cx.pct, "—");
        assert_eq!(cx.severity, Severity::Normal);
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p cli_agent_usage --lib format::`
Expected: FAIL to compile — `cannot find function fmt_tokens` etc.

- [ ] **Step 4: Write the implementation (above the test module)**

Prepend to `crates/cli_agent_usage/src/format.rs`:

```rust
//! Pure, UI-free display helpers for footer rendering. No toolkit/theme deps.

use chrono::{DateTime, Utc};

use crate::{Provider, Severity, UsageSnapshot};

/// Humanize a token count: `0..=999` verbatim, then `k`/`M`/`B` with one decimal.
pub fn fmt_tokens(n: u64) -> String {
    if n < 1_000 {
        return n.to_string();
    }
    let (value, suffix) = if n < 1_000_000 {
        (n as f64 / 1_000.0, "k")
    } else if n < 1_000_000_000 {
        (n as f64 / 1_000_000.0, "M")
    } else {
        (n as f64 / 1_000_000_000.0, "B")
    };
    format!("{value:.1}{suffix}")
}

/// Round a percentage to an integer, e.g. `54.6 -> "55%"`.
pub fn fmt_pct(percent: f64) -> String {
    format!("{}%", percent.round() as i64)
}

/// Relative "resets" label: `"in 12m"` / `"in 3h"` / `"in 2d"`; past-or-now -> `"now"`;
/// `None` -> `"—"`.
pub fn fmt_reset(resets_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> String {
    let Some(target) = resets_at else {
        return "—".to_string();
    };
    let secs = (target - now).num_seconds();
    if secs <= 0 {
        "now".to_string()
    } else if secs < 3_600 {
        format!("in {}m", secs / 60)
    } else if secs < 86_400 {
        format!("in {}h", secs / 3_600)
    } else {
        format!("in {}d", secs / 86_400)
    }
}

/// One half of the footer chip.
pub struct ChipHalf {
    /// `"cc"` (Claude Code) or `"cx"` (Codex).
    pub label: &'static str,
    /// Weekly plan-% like `"47%w"`, or `"—"` when unknown.
    pub pct: String,
    /// Drives the half's color; `Normal` when `pct == "—"`.
    pub severity: Severity,
}

/// The two chip halves `[claude, codex]`. Returns `None` when NEITHER provider has
/// any data (the chip is hidden). A provider that has token data but no weekly plan
/// yields a `"—"` half but still counts as "has data".
pub fn chip_halves(snap: &UsageSnapshot) -> Option<[ChipHalf; 2]> {
    fn half(label: &'static str, p: &Provider) -> (ChipHalf, bool) {
        let has_tokens = p.month.tokens.total() > 0;
        match p.plan.and_then(|pl| pl.weekly) {
            Some(w) => (
                ChipHalf {
                    label,
                    pct: format!("{}w", fmt_pct(w.percent)),
                    severity: w.severity,
                },
                true,
            ),
            None => (
                ChipHalf {
                    label,
                    pct: "—".to_string(),
                    severity: Severity::Normal,
                },
                has_tokens,
            ),
        }
    }
    let (cc, cc_has) = half("cc", &snap.claude);
    let (cx, cx_has) = half("cx", &snap.codex);
    if !cc_has && !cx_has {
        return None;
    }
    Some([cc, cx])
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p cli_agent_usage --lib format::`
Expected: PASS (5 tests).

- [ ] **Step 6: Lint**

Run: `cargo clippy -p cli_agent_usage --all-targets -- -D warnings && cargo fmt -p cli_agent_usage`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/cli_agent_usage/src/lib.rs crates/cli_agent_usage/src/format.rs
git commit -m "feat(usage): pure format module (fmt_tokens/pct/reset, chip_halves)"
```

---

## Task 3: App — `CliAgentUsageModel` singleton + registration

**Files:**
- Modify: `app/Cargo.toml` (add dependency)
- Create: `app/src/ai/blocklist/usage/cli_agent_usage_model.rs`
- Modify: `app/src/ai/blocklist/usage/mod.rs` (declare + re-export module)
- Modify: `app/src/lib.rs` (~1362, register singleton)

**Interfaces:**
- Consumes: `cli_agent_usage::{scan_local, fetch_claude_plan, Paths, Caches, UsageSnapshot, PlanLimits}`; `cli_agent_usage::keychain::{MacKeychain, ReadSecret}` (`MacKeychain` is a unit struct); `cli_agent_usage::http::{ReqwestUsage, FetchUsage}` (`ReqwestUsage` is a unit struct); `async_channel::unbounded` (already an `app` dependency — used in `app/src/input_suggestions.rs:308`); warpui `Entity`/`SingletonEntity`/`ModelContext`/`spawn_stream_local`/`add_singleton_model` (mirror `app/src/ai/request_usage_model.rs` for exact `use` paths).
- Produces: `pub struct CliAgentUsageModel` (`Entity<Event = CliAgentUsageModelEvent>` + `SingletonEntity`) with `pub fn new(&mut ModelContext<Self>) -> Self` and `pub fn latest(&self) -> &UsageSnapshot`; `pub enum CliAgentUsageModelEvent { Updated }`.

> **NOTE for implementer:** copy the exact `use` lines for `Entity`, `SingletonEntity`, `ModelContext` from the top of `app/src/ai/request_usage_model.rs` (they resolve through `warpui`). Do the same for the `add_singleton_model` call site style in `app/src/lib.rs` (see the `AIRequestUsageModel::new` registration near line 1362).

- [ ] **Step 1: Add the dependency**

In `app/Cargo.toml`, under `[dependencies]`, add (match the repo's convention — if other in-repo crates use `{ workspace = true }` add `cli_agent_usage` to root `[workspace.dependencies]` as `cli_agent_usage = { path = "crates/cli_agent_usage" }` and reference it here as `cli_agent_usage = { workspace = true }`; otherwise a direct path dep):

```toml
cli_agent_usage = { path = "../crates/cli_agent_usage" }
```

- [ ] **Step 2: Create the model file**

Create `app/src/ai/blocklist/usage/cli_agent_usage_model.rs`:

```rust
//! Singleton that keeps the latest CLI-agent (Claude Code + Codex) usage snapshot
//! fresh for the footer. All blocking work (file IO + the Claude usage HTTP call)
//! runs on ONE dedicated `std::thread` — never the gpui background executor, which
//! is Tokio-backed and would make `reqwest::blocking` panic.

use std::time::Duration;

use chrono::Utc;
use cli_agent_usage::http::{FetchUsage, ReqwestUsage};
use cli_agent_usage::keychain::{MacKeychain, ReadSecret};
use cli_agent_usage::{fetch_claude_plan, scan_local, Caches, Paths, PlanLimits, UsageSnapshot};
// Match the imports used by app/src/ai/request_usage_model.rs:
use warpui::{Entity, ModelContext, SingletonEntity};

/// How often the producer thread re-scans local files.
const FILE_POLL: Duration = Duration::from_secs(5);
/// Fetch the Claude usage endpoint every Nth tick (~60s at FILE_POLL = 5s).
const ENDPOINT_EVERY: u64 = 12;

pub enum CliAgentUsageModelEvent {
    Updated,
}

pub struct CliAgentUsageModel {
    latest: UsageSnapshot,
}

impl Entity for CliAgentUsageModel {
    type Event = CliAgentUsageModelEvent;
}

impl SingletonEntity for CliAgentUsageModel {}

impl CliAgentUsageModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let (tx, rx) = async_channel::unbounded::<UsageSnapshot>();
        if let Some(paths) = Paths::detect() {
            // Dedicated OS thread => guaranteed no Tokio runtime context.
            let _ = std::thread::Builder::new()
                .name("cli-agent-usage".to_string())
                .spawn(move || producer_loop(paths, tx));
        }
        // Deliver each snapshot on the main thread; store it and notify observers.
        ctx.spawn_stream_local(rx, Self::on_snapshot, |_, _| {});
        Self {
            latest: UsageSnapshot::default(),
        }
    }

    pub fn latest(&self) -> &UsageSnapshot {
        &self.latest
    }

    fn on_snapshot(&mut self, snap: UsageSnapshot, ctx: &mut ModelContext<Self>) {
        self.latest = snap;
        ctx.emit(CliAgentUsageModelEvent::Updated);
        ctx.notify();
    }
}

/// Runs on the dedicated thread. Split cadence: local scans every `FILE_POLL`,
/// the Claude usage endpoint every `ENDPOINT_EVERY` ticks, retaining the last good
/// `PlanLimits` across transient failures. Exits when the receiver is dropped.
fn producer_loop(paths: Paths, tx: async_channel::Sender<UsageSnapshot>) {
    let mut caches = Caches::new();
    let keychain = MacKeychain;
    let fetch = ReqwestUsage;
    let mut last_plan: Option<PlanLimits> = None;
    let mut tick: u64 = 0;
    loop {
        let now = Utc::now();
        let mut snap = scan_local(&paths, &mut caches, now);
        if tick % ENDPOINT_EVERY == 0 {
            if let Some(fresh) = fetch_claude_plan(
                &keychain as &dyn ReadSecret,
                &fetch as &dyn FetchUsage,
                &paths,
                now,
            ) {
                last_plan = Some(fresh); // overwrite only on success => last-good retained
            }
        }
        snap.claude.plan = last_plan;
        if tx.send_blocking(snap).is_err() {
            break; // receiver dropped (model gone) => exit cleanly
        }
        tick = tick.wrapping_add(1);
        std::thread::sleep(FILE_POLL);
    }
}
```

- [ ] **Step 3: Declare + re-export the module**

In `app/src/ai/blocklist/usage/mod.rs`, add alongside the existing submodule declarations:

```rust
mod cli_agent_usage_model;
pub use cli_agent_usage_model::{CliAgentUsageModel, CliAgentUsageModelEvent};
```

- [ ] **Step 4: Register the singleton**

In `app/src/lib.rs`, immediately after the `AIRequestUsageModel` registration (near line 1362, `ctx.add_singleton_model(|ctx| AIRequestUsageModel::new(ai_client, ctx));`), add:

```rust
        ctx.add_singleton_model(|ctx| {
            crate::ai::blocklist::usage::CliAgentUsageModel::new(ctx)
        });
```

(If `lib.rs` imports models by `use` at the top, add `use crate::ai::blocklist::usage::CliAgentUsageModel;` there instead and call `CliAgentUsageModel::new(ctx)`.)

- [ ] **Step 5: Build to verify it compiles**

Run: `cargo build -p cli_agent_usage && cargo build`
Expected: PASS. (The full `app` build may take several minutes.) Warnings about `CliAgentUsageModel::latest`/`CliAgentUsageModelEvent` being unused are acceptable at this stage — they are consumed in Task 5. If the build denies warnings, add a temporary `#[allow(dead_code)]` on `latest` and remove it in Task 5.

- [ ] **Step 6: Commit**

```bash
git add app/Cargo.toml app/src/ai/blocklist/usage/cli_agent_usage_model.rs app/src/ai/blocklist/usage/mod.rs app/src/lib.rs
git commit -m "feat(usage): CliAgentUsageModel singleton + producer thread + registration"
```

---

## Task 4: App — chip + panel element builders

**Files:**
- Create: `app/src/ai/blocklist/usage/cli_agent_usage_chip.rs`
- Modify: `app/src/ai/blocklist/usage/mod.rs` (declare + re-export module)

**Interfaces:**
- Consumes: `cli_agent_usage::format::{chip_halves, fmt_pct, fmt_reset, fmt_tokens, ChipHalf}`; `cli_agent_usage::{Severity, UsageSnapshot, Provider, WindowTotals}`; warpui elements/theme — mirror the imports in `app/src/context_chips/display_chip.rs` and `.../agent_input_footer/mod.rs`: `Flex, Text, Container, ConstrainedBox, Empty, Icon, Fill, CrossAxisAlignment, Border, CornerRadius, Radius`, `warpui::Appearance`, `warp_core::ui::theme::{Fill, WarpTheme}`, `internal_colors`. (`Text::new_inline(text, font_family, font_size).with_color(fill).with_line_height_ratio(r).finish()` per `display_chip.rs:107`; container styling per `render_ftu_callout`, `mod.rs:2312-2347`.)
- Produces: `pub fn render_cli_agent_usage_chip(&UsageSnapshot, &Appearance, Fill) -> Option<Box<dyn Element>>`; `pub fn render_cli_agent_usage_panel(&UsageSnapshot, &Appearance) -> Box<dyn Element>`.

> **NOTE for implementer:** this task's functions are not yet called (Task 5 wires them), so annotate the module with `#![allow(dead_code)]` at the top of the new file; Task 5 removes it. There is no unit test — the deliverable is that `cargo build` compiles. Use `app/src/context_chips/display_chip.rs:103-161` (the multi-colored git-diff-stats row) as the exact template for `Flex::row()` + per-span `Text::new_inline(...).with_color(...)`, and `render_ftu_callout` (`mod.rs:2312-2347`) as the template for the panel's bordered box.

- [ ] **Step 1: Declare the module**

In `app/src/ai/blocklist/usage/mod.rs`:

```rust
mod cli_agent_usage_chip;
pub use cli_agent_usage_chip::{render_cli_agent_usage_chip, render_cli_agent_usage_panel};
```

- [ ] **Step 2: Create the chip/panel builders**

Create `app/src/ai/blocklist/usage/cli_agent_usage_chip.rs`. Adjust the `use` paths to match the two reference files above if they differ; the compiler will guide exact module paths.

```rust
#![allow(dead_code)] // wired up in Task 5; remove this attribute there.

use chrono::Utc;
use cli_agent_usage::format::{chip_halves, fmt_pct, fmt_reset, fmt_tokens};
use cli_agent_usage::{Provider, Severity, UsageSnapshot, WindowTotals};

// Element + theme imports — mirror app/src/context_chips/display_chip.rs.
use warp_core::ui::theme::{Fill, WarpTheme};
use warpui::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Element, Empty, Flex,
    Icon, Radius, Text,
};
use warpui::Appearance;

/// Map a crate `Severity` to a fill against `bg` (the surface the text sits on).
fn severity_fill(severity: Severity, theme: &WarpTheme, bg: Fill) -> Fill {
    match severity {
        Severity::Normal => theme.main_text_color(bg),
        Severity::Warning => Fill::Solid(theme.ui_warning_color()),
        Severity::Critical => Fill::Solid(theme.ui_error_color()),
    }
}

/// A monospace text span in a given color.
fn span(text: impl Into<String>, color: Fill, appearance: &Appearance) -> Box<dyn Element> {
    Text::new_inline(
        text.into(),
        appearance.monospace_font_family(),
        appearance.monospace_font_size(),
    )
    .with_color(color)
    .with_line_height_ratio(appearance.line_height_ratio())
    .finish()
}

/// The footer chip: `[clock] cc 47%w · cx 55%w`, each %-half colored by its severity.
/// `None` when neither tool has data (chip hidden).
pub fn render_cli_agent_usage_chip(
    snapshot: &UsageSnapshot,
    appearance: &Appearance,
    bg: Fill,
) -> Option<Box<dyn Element>> {
    let halves = chip_halves(snapshot)?;
    let theme = appearance.theme();
    let neutral = theme.sub_text_color(bg);
    let icon_size = appearance.monospace_font_size();

    let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
    row.add_child(
        Container::new(
            ConstrainedBox::new(Icon::Clock.to_warpui_icon(theme.main_text_color(bg)).finish())
                .with_width(icon_size)
                .with_height(icon_size)
                .finish(),
        )
        .with_margin_right(4.)
        .finish(),
    );

    for (i, half) in halves.iter().enumerate() {
        if i > 0 {
            row.add_child(span(" · ", neutral, appearance));
        }
        row.add_child(span(format!("{} ", half.label), neutral, appearance));
        row.add_child(span(
            half.pct.clone(),
            severity_fill(half.severity, theme, bg),
            appearance,
        ));
    }

    Some(Container::new(row.finish()).with_vertical_padding(4.).finish())
}

/// The expanded panel: two columns (Claude | Codex) — 5h %, weekly %, then
/// session/today/week/month input+output tokens (cache-read dimmed). No cost.
pub fn render_cli_agent_usage_panel(
    snapshot: &UsageSnapshot,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let bg = theme.surface_2().into_solid();
    let main = theme.main_text_color(bg);
    let sub = theme.sub_text_color(bg);
    let now = Utc::now();

    // Header row.
    let mut col = Flex::column().with_spacing(4.);
    col.add_child(panel_row(
        span("", sub, appearance),
        span("Claude Code", main, appearance),
        span("Codex", main, appearance),
    ));

    // Plan-% rows.
    col.add_child(panel_row(
        span("5h", sub, appearance),
        plan_cell(snapshot.claude.plan.and_then(|p| p.session), now, appearance, bg),
        plan_cell(snapshot.codex.plan.and_then(|p| p.session), now, appearance, bg),
    ));
    col.add_child(panel_row(
        span("Weekly", sub, appearance),
        plan_cell(snapshot.claude.plan.and_then(|p| p.weekly), now, appearance, bg),
        plan_cell(snapshot.codex.plan.and_then(|p| p.weekly), now, appearance, bg),
    ));

    // Token rows.
    for (label, pick) in [
        ("Session", 0u8),
        ("Today", 1),
        ("This week", 2),
        ("This month", 3),
    ] {
        col.add_child(panel_row(
            span(label, sub, appearance),
            token_cell(window(&snapshot.claude, pick), appearance, main, sub),
            token_cell(window(&snapshot.codex, pick), appearance, main, sub),
        ));
    }

    ConstrainedBox::new(
        Container::new(col.finish())
            .with_vertical_padding(12.)
            .with_horizontal_padding(16.)
            .with_background(bg)
            .with_border(Border::all(1.).with_border_fill(theme.accent()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .finish(),
    )
    .with_width(320.)
    .finish()
}

fn window(p: &Provider, pick: u8) -> &WindowTotals {
    match pick {
        0 => &p.session,
        1 => &p.today,
        2 => &p.week,
        _ => &p.month,
    }
}

/// A three-cell row: fixed-width label, then two equal provider columns.
fn panel_row(
    label: Box<dyn Element>,
    claude: Box<dyn Element>,
    codex: Box<dyn Element>,
) -> Box<dyn Element> {
    let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
    row.add_child(ConstrainedBox::new(label).with_width(84.).finish());
    row.add_child(ConstrainedBox::new(claude).with_width(108.).finish());
    row.add_child(ConstrainedBox::new(codex).with_width(108.).finish());
    row.finish()
}

/// `{pct}% · resets {when}` colored by severity, or `—` when absent.
fn plan_cell(
    limit: Option<cli_agent_usage::LimitWindow>,
    now: chrono::DateTime<Utc>,
    appearance: &Appearance,
    bg: Fill,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let sub = theme.sub_text_color(bg);
    match limit {
        None => span("—", sub, appearance),
        Some(w) => {
            let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
            row.add_child(span(fmt_pct(w.percent), severity_fill(w.severity, theme, bg), appearance));
            row.add_child(span(
                format!(" · {}", fmt_reset(w.resets_at, now)),
                sub,
                appearance,
            ));
            row.finish()
        }
    }
}

/// `{io} · {cache} cache` — headline io in main color, cache-read dimmed.
fn token_cell(
    totals: &WindowTotals,
    appearance: &Appearance,
    main: Fill,
    sub: Fill,
) -> Box<dyn Element> {
    if totals.tokens.total() == 0 {
        return span("—", sub, appearance);
    }
    let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
    row.add_child(span(fmt_tokens(totals.tokens.io()), main, appearance));
    if totals.tokens.cache_read > 0 {
        row.add_child(span(
            format!(" · {} cache", fmt_tokens(totals.tokens.cache_read)),
            sub,
            appearance,
        ));
    }
    row.finish()
}
```

- [ ] **Step 3: Build to verify it compiles**

Run: `cargo build`
Expected: PASS. If any element method name differs (e.g. `with_background`/`with_border`/`with_corner_radius`/`to_warpui_icon`/`with_color` coercion), reconcile against the cited reference lines (`mod.rs:2312-2347`, `display_chip.rs:107-161`, `display_chip.rs:1927`) — those exact calls compile in this codebase. `LimitWindow` is re-exported at the crate root (`cli_agent_usage::LimitWindow`).

- [ ] **Step 4: Commit**

```bash
git add app/src/ai/blocklist/usage/cli_agent_usage_chip.rs app/src/ai/blocklist/usage/mod.rs
git commit -m "feat(usage): footer chip + panel element builders"
```

---

## Task 5: App — render the chip directly in the footer (panel toggle, subscription)

**Files:**
- Modify: `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs` (state field, action + handler, subscription, chip render method + call site)
- Modify: `app/src/ai/blocklist/usage/cli_agent_usage_chip.rs` (remove `#![allow(dead_code)]`)

> **Why direct render, not `AgentToolbarItemKind`:** `AgentToolbarItemKind` is a persisted settings value (`Serialize`/`Deserialize`/`SettingsValue`). The CLI footer renders from each user's *saved* `cli_agent_footer_chip_selection`, so adding a new default variant would be invisible to any existing user until they reset or re-add it — a silent visibility bug for an always-on usage chip. Rendering directly in `render_cli_mode_footer` makes it guaranteed-visible and touches one fewer file. (Trade-off: not hideable/reorderable via the footer configurator — acceptable and intended for an always-on indicator.)

**Interfaces:**
- Consumes: `CliAgentUsageModel::{handle, as_ref}` + `latest()`; `render_cli_agent_usage_chip`, `render_cli_agent_usage_panel`; `cli_agent_usage::format::chip_halves`; warpui `Hoverable`, `MouseStateHandle`, `Stack`, `OffsetPositioning::offset_from_parent`, `ParentOffsetBounds::WindowByPosition`, `ParentAnchor::TopLeft`, `ChildAnchor::BottomLeft`, `Empty`, `Cursor::PointingHand`, `vec2f`; `SharedSessionStatus::is_viewer`; `ctx.dispatch_typed_action` / `ctx.subscribe_to_model`.
- Produces: an always-on footer chip + click-to-toggle panel.

> **NOTE for implementer:** the `Hoverable` + `Stack` overlay is copied from `display_chip.rs:1103-1157` (`git_branch_chip`). The panel toggle mirrors `AgentInputFooterAction::ToggleFileExplorer` (`mod.rs:2394` variant, `:2465` handler). The subscription mirrors the `AIRequestUsageModel` one (`mod.rs:~709`). Import paths: `Hoverable, MouseStateHandle, Stack, OffsetPositioning, ParentAnchor, ChildAnchor, ParentOffsetBounds, Empty` from `warpui`; `Cursor` from `warpui::platform`; `vec2f` from `pathfinder_geometry::vector` (see `display_chip.rs:6,12,17`).

- [ ] **Step 1: Add the footer state fields (`mod.rs`)**

Add to the `AgentInputFooter` struct definition (near the other footer state fields):
```rust
    cli_agent_usage_panel_open: bool,
    cli_agent_usage_mouse_state: MouseStateHandle,
```

Initialize both in `AgentInputFooter::new()` (~line 258, in the returned struct literal):
```rust
            cli_agent_usage_panel_open: false,
            cli_agent_usage_mouse_state: Default::default(),
```

- [ ] **Step 2: Add the toggle action + handler (`mod.rs`)**

Add the variant to `enum AgentInputFooterAction` (near `ToggleFileExplorer`, ~line 2394):
```rust
    ToggleCliAgentUsagePanel,
```

Handle it in the footer action `match` (next to the `ToggleFileExplorer` arm, ~line 2465):
```rust
            AgentInputFooterAction::ToggleCliAgentUsagePanel => {
                self.cli_agent_usage_panel_open = !self.cli_agent_usage_panel_open;
                ctx.notify();
            }
```

- [ ] **Step 3: Subscribe to the model (`mod.rs`, in `new()`)**

Next to the existing `ctx.subscribe_to_model(&AIRequestUsageModel::handle(ctx), ...)` (~line 709):
```rust
        ctx.subscribe_to_model(&CliAgentUsageModel::handle(ctx), |_, _, _, ctx| {
            ctx.notify()
        });
```

- [ ] **Step 4: Add the chip render method (`mod.rs`, in `impl AgentInputFooter`)**

Add this method to the same `impl AgentInputFooter` block that contains `render_cli_mode_footer`:
```rust
    /// The always-on CLI-agent usage chip (Claude Code + Codex plan-%), which
    /// expands into a panel on click. `None` when there is no data, or in
    /// viewer/transcript contexts (the host's usage is private).
    fn render_cli_agent_usage_chip_item(
        &self,
        shared_status: &SharedSessionStatus,
        is_conversation_transcript_context: bool,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        if shared_status.is_viewer() || is_conversation_transcript_context {
            return None;
        }
        let appearance = Appearance::as_ref(app);
        let bg = appearance.theme().surface_1().into_solid();
        let snapshot = CliAgentUsageModel::as_ref(app).latest().clone();
        // Hidden when neither tool has data.
        if cli_agent_usage::format::chip_halves(&snapshot).is_none() {
            return None;
        }
        let panel_open = self.cli_agent_usage_panel_open;
        let snapshot_for_panel = snapshot.clone();

        let hover = Hoverable::new(self.cli_agent_usage_mouse_state.clone(), move |_state| {
            render_cli_agent_usage_chip(&snapshot, appearance, bg)
                .unwrap_or_else(|| Empty::new().finish())
        })
        .on_click(|ctx, _app, _position| {
            ctx.dispatch_typed_action(AgentInputFooterAction::ToggleCliAgentUsagePanel);
        })
        .with_cursor(Cursor::PointingHand)
        .finish();

        let mut stack = Stack::new().with_child(hover);
        if panel_open {
            let panel = render_cli_agent_usage_panel(&snapshot_for_panel, appearance);
            stack.add_positioned_overlay_child(
                panel,
                OffsetPositioning::offset_from_parent(
                    vec2f(0., -4.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::TopLeft,
                    ChildAnchor::BottomLeft,
                ),
            );
        }
        Some(stack.finish())
    }
```

- [ ] **Step 5: Render it in `render_cli_mode_footer` (`mod.rs`)**

Immediately after the `for item in &right_items { ... right_buttons.add_child(element); }` loop (~line 1602) and before `let content = Wrap::row()...`:
```rust
        if let Some(chip) = self.render_cli_agent_usage_chip_item(
            &shared_status,
            is_conversation_transcript_context,
            app,
        ) {
            right_buttons.add_child(chip);
        }
```

Add the missing imports at the top of `mod.rs` (some already present): `MouseStateHandle`, `Hoverable`, `Stack`, `OffsetPositioning`, `ParentAnchor`, `ChildAnchor`, `ParentOffsetBounds`, `Empty` (from `warpui`); `Cursor` (from `warpui::platform`); `vec2f` (from `pathfinder_geometry::vector`); and `use crate::ai::blocklist::usage::{CliAgentUsageModel, render_cli_agent_usage_chip, render_cli_agent_usage_panel};`.

- [ ] **Step 6: Remove the temporary dead-code allow**

Delete the `#![allow(dead_code)]` line at the top of `app/src/ai/blocklist/usage/cli_agent_usage_chip.rs` (the functions are now used). If Task 3 added a temporary `#[allow(dead_code)]` on `CliAgentUsageModel::latest`, remove that too.

- [ ] **Step 7: Build**

Run: `cargo build`
Expected: PASS (no dead-code warnings remain). Reconcile any `Hoverable`/`Stack`/`OffsetPositioning` signature mismatch against `display_chip.rs:1103-1157` (the exact working precedent).

- [ ] **Step 8: Manual verification**

Run the app (`cargo run` or the project's usual launch) with a CLI agent (Claude Code or Codex) session and confirm:
- The footer's right group shows `[clock] cc NN%w · cx NN%w` (or `—` for a provider without a weekly plan).
- Half colors track severity (neutral → amber near warning → red near critical).
- Clicking the chip opens a panel above it with two columns (Claude | Codex): 5h %, Weekly %, then Session/Today/This week/This month token rows (io headline, dimmed cache); clicking again closes it.
- With neither tool ever run, the chip is absent (not an empty box).
- The panel does not clip the right window edge. `ParentOffsetBounds::WindowByPosition` should keep it on-screen; if it still clips, switch the anchors to `ParentAnchor::TopRight`/`ChildAnchor::BottomRight` (confirm those variants exist in the toolkit first — the verified precedent uses the `*Left` variants).

- [ ] **Step 9: Commit**

```bash
git add app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs \
        app/src/ai/blocklist/usage/cli_agent_usage_chip.rs
git commit -m "feat(usage): render always-on CLI agent usage chip + panel in the footer"
```

---

## Self-Review

**1. Spec coverage** (checked against `2026-06-30-cli-agent-usage-footer-ui.md`):

- §2 chip = weekly plan-% both tools → Task 2 `chip_halves`, Task 4 chip, Task 5 render. ✓
- §2 panel io headline + cache dimmed, no cost → Task 4 `token_cell` (uses `io()`, dims `cache_read`, never reads `cost_usd`). ✓
- §4 threading (std::thread + async_channel + spawn_stream_local, split cadence, last-known-good) → Task 3 `producer_loop` + `on_snapshot`. ✓
- §5 chip color from `Severity` enum; hidden when both empty; click-to-open → Task 4 `severity_fill`, Task 2 `chip_halves` (None), Task 5 `Hoverable` on_click. ✓
- §6 popover via `Stack::add_positioned_overlay_child` + `OffsetPositioning` → Task 5. ✓
- §7 crate additions (`io`, `scan_local`; plus `fetch_claude_plan` for DRY) → Task 1. ✓
- §8 fail-soft + last-known-good → crate (fail-soft) + Task 3 (`last_plan` retained). ✓
- §9 testing (pure fns unit-tested; render manual) → Tasks 1–2 TDD, Tasks 3–5 build+manual. ✓
- §10 integration points (registration + subscription + chip render) → Tasks 3, 5. **Deviation:** the spec sketched an `AgentToolbarItemKind::CliAgentUsage` variant; the plan renders the chip directly in `render_cli_mode_footer` instead (principal-engineer revision — the enum is a persisted settings value and a new default would be invisible to existing users). Same UX, simpler + guaranteed-visible. ✓
- §11 security (one outbound call, token in memory) → unchanged; all network stays inside the crate. ✓

**2. Placeholder scan:** No `TBD`/`TODO`/"handle edge cases"/"similar to". Two intentional "mirror the cited reference" notes (Tasks 3–5) point at exact `file:line` precedents for warpui plumbing that cannot be unit-verified from a plan; each names the specific lines and the exact call to copy — not vague deferral.

**3. Type consistency:** `TokenCounts::io()` (T1) used in `token_cell` (T4). `scan_local`/`fetch_claude_plan` (T1) consumed in `producer_loop` (T3). `chip_halves`/`ChipHalf`/`fmt_*` (T2) consumed in T4/T5. `CliAgentUsageModel::{new,latest,handle,as_ref}` + `CliAgentUsageModelEvent` (T3) consumed in T5. `render_cli_agent_usage_chip`/`render_cli_agent_usage_panel` (T4) consumed by `render_cli_agent_usage_chip_item` (T5). `cli_agent_usage_panel_open`/`cli_agent_usage_mouse_state`/`ToggleCliAgentUsagePanel` all defined and used within T5. Signatures match across tasks. ✓

**Dead-code check (per repo review bar):** the `#![allow(dead_code)]` bridge in T4 is explicitly removed in T5 Step 6; `refresh()` is retained (public one-shot entry used by the `print_usage` example) and now composes `scan_local`+`fetch_claude_plan` (removing its former inline-closure duplication). No code is left unreferenced after Task 5.
