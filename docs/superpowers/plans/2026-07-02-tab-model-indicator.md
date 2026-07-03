# Tab Model Indicator Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show which model (e.g. `Opus 4.8`, `GPT-5 Codex`) each CLI-agent tab is running, as a dim chip next to the tab's existing agent icon.

**Architecture:** The `cli_agent_usage` crate already parses the model out of Claude transcripts and Codex rollouts on a 5s background scan. Extend that scan to also build a `session_id → latest model` map on `UsageSnapshot`. The vertical tab row looks up its session's model by `session_id` and renders a friendly-named chip. Purely additive; fail-soft (unknown/missing → no chip, never a wrong model).

**Tech Stack:** Rust, WarpUI (Entity/Handle + Element tree), `cli_agent_usage` crate, `serde_json`, `chrono`.

## Global Constraints

- **Exhaustive matching:** never add wildcard `_` match arms; add explicit arms (project rule).
- **Inline format args:** `format!("{x}")` not `format!("{}", x)` (Clippy `uninlined_format_args`).
- **Context param:** any `AppContext`/`ViewContext`/`ModelContext` is named `ctx` and goes last.
- **No `_`-prefixed unused params:** remove them and update call sites.
- **Tests:** co-locate as `${file}_tests.rs` (or an inline `mod tests` where the file already uses one — `claude.rs`/`codex.rs` do) and wire with `#[cfg(test)] #[path=...] mod tests;`.
- **Fail-soft:** a missing dir / corrupt line / absent session_id yields empty/None for that slice; never panic.
- **Model label is empty-string-means-hidden:** `friendly_model_name` returns `""` for unknown/`<synthetic>`/`unknown`; callers treat `""` as "no chip".
- **Before any PR:** `./script/format` and the presubmit `cargo clippy` must pass.
- **Crate package:** `cli_agent_usage`. **App package:** `warp`.

---

### Task 1: `friendly_model_name` formatter

Maps a raw model id to a short tab label. Pure function, no toolkit deps — lives beside the other display helpers.

**Files:**
- Modify: `crates/cli_agent_usage/src/format.rs` (append the function + a private `version_from` helper, and add cases to the existing `#[cfg(test)] mod tests` if present, else add one)

**Interfaces:**
- Consumes: nothing.
- Produces: `pub fn friendly_model_name(model: &str) -> String` — `""` means "no displayable model".

- [ ] **Step 1: Write the failing tests**

Add to `format.rs` (inside its test module; if `format.rs` has no test module yet, add `#[cfg(test)] mod tests { use super::*; ... }` at the end):

```rust
#[test]
fn friendly_model_name_maps_known_ids() {
    assert_eq!(friendly_model_name("claude-opus-4-8"), "Opus 4.8");
    assert_eq!(friendly_model_name("claude-sonnet-5"), "Sonnet 5");
    assert_eq!(friendly_model_name("claude-haiku-4-5-20251001"), "Haiku 4.5");
    assert_eq!(friendly_model_name("claude-haiku"), "Haiku");
    assert_eq!(friendly_model_name("gpt-5-codex"), "GPT-5 Codex");
    assert_eq!(friendly_model_name("gpt-5.5"), "GPT-5.5");
    assert_eq!(friendly_model_name("gpt-5"), "GPT-5");
}

#[test]
fn friendly_model_name_hides_noise_and_titlecases_unknown() {
    assert_eq!(friendly_model_name("<synthetic>"), "");
    assert_eq!(friendly_model_name("unknown"), "");
    assert_eq!(friendly_model_name("   "), "");
    assert_eq!(friendly_model_name("some-new-model"), "Some New Model");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p cli_agent_usage friendly_model_name`
Expected: FAIL — `cannot find function 'friendly_model_name'`.

- [ ] **Step 3: Implement the function**

Append to `crates/cli_agent_usage/src/format.rs` (module scope, not inside `mod tests`):

```rust
/// Map a raw model id to a short human label for the tab chip.
/// Returns `""` for unknown / synthetic ids so callers can hide the chip.
///
/// Examples: `claude-opus-4-8` → `Opus 4.8`, `claude-haiku-4-5-20251001` →
/// `Haiku 4.5`, `gpt-5-codex` → `GPT-5 Codex`, `gpt-5.5` → `GPT-5.5`.
pub fn friendly_model_name(model: &str) -> String {
    let m = model.trim().to_ascii_lowercase();
    if m.is_empty() || m == "<synthetic>" || m == "unknown" {
        return String::new();
    }

    // Anthropic families.
    for (needle, label) in [("opus", "Opus"), ("sonnet", "Sonnet"), ("haiku", "Haiku")] {
        if let Some(idx) = m.find(needle) {
            let ver = version_from(&m[idx + needle.len()..]);
            return if ver.is_empty() {
                label.to_string()
            } else {
                format!("{label} {ver}")
            };
        }
    }

    // OpenAI / Codex.
    if m.starts_with("gpt") || m.contains("codex") {
        let core = m.trim_start_matches("gpt-").trim_start_matches("gpt");
        let ver = version_from(core);
        let mut label = String::from("GPT");
        if !ver.is_empty() {
            label.push('-');
            label.push_str(&ver);
        }
        if m.contains("codex") {
            label.push_str(" Codex");
        }
        return label;
    }

    // Fallback: title-case the dashed id.
    m.split(['-', '_'])
        .filter(|s| !s.is_empty())
        .map(|s| {
            let mut chars = s.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Read a leading `major[.minor]` version from `s` (after any leading `-`/`.`),
/// stopping at the first non-numeric token, a date-like token (len ≥ 5), or two
/// components. `"-4-8"` → `"4.8"`, `"-4-5-20251001"` → `"4.5"`, `"5-codex"` →
/// `"5"`, `""` → `""`.
fn version_from(s: &str) -> String {
    let s = s.trim_start_matches(['-', '.']);
    let mut parts = Vec::new();
    for token in s.split(['-', '.']) {
        if token.is_empty() || !token.chars().all(|c| c.is_ascii_digit()) || token.len() >= 5 {
            break;
        }
        parts.push(token);
        if parts.len() == 2 {
            break;
        }
    }
    parts.join(".")
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p cli_agent_usage friendly_model_name`
Expected: PASS (both tests).

- [ ] **Step 5: Commit**

```bash
git add crates/cli_agent_usage/src/format.rs
git commit -m "feat(usage): friendly_model_name for tab model chip"
```

---

### Task 2: Codex rollout exposes `session_id`

The Codex model index is keyed by session id. Capture it from the rollout's `session_meta` line.

**Files:**
- Modify: `crates/cli_agent_usage/src/codex.rs` (`RollupFile` struct ~L14, `parse_rollout_str` ~L81, tests ~L215)

**Interfaces:**
- Consumes: nothing.
- Produces: `RollupFile { session_id: Option<String>, .. }` — the session uuid from the meta line, or `None` if absent.

- [ ] **Step 1: Write the failing test**

Add to the existing `#[cfg(test)] mod tests` in `codex.rs`:

```rust
#[test]
fn parse_rollout_captures_session_id_from_meta() {
    const META: &str = r#"{"timestamp":"2026-06-25T22:51:38.016Z","type":"session_meta","payload":{"session_id":"019f00fb-40b5-7192-9b79-aa6d1034fe1b","id":"019f00fb-40b5-7192-9b79-aa6d1034fe1b","model":"gpt-5-codex"}}"#;
    let r = parse_rollout_str(META);
    assert_eq!(
        r.session_id.as_deref(),
        Some("019f00fb-40b5-7192-9b79-aa6d1034fe1b")
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p cli_agent_usage parse_rollout_captures_session_id`
Expected: FAIL — `no field 'session_id' on type RollupFile`.

- [ ] **Step 3: Add the field and parse it**

In `codex.rs`, add the field to `RollupFile` (~L14):

```rust
#[derive(Default)]
pub struct RollupFile {
    pub entries: Vec<Entry>,
    pub last_total: TokenCounts,
    pub rate_limits: Option<PlanLimits>,
    /// Session uuid from the `session_meta` line, when present.
    pub session_id: Option<String>,
}
```

In `parse_rollout_str`, inside the per-line loop, right after the existing
"Track the latest model id" block (the `if let Some(m) = payload.get("model")`
block, ~L99), add:

```rust
        // The session_meta line's payload carries the session uuid.
        if out.session_id.is_none() {
            if let Some(sid) = payload.get("session_id").and_then(|v| v.as_str()) {
                out.session_id = Some(sid.to_string());
            }
        }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p cli_agent_usage parse_rollout_captures_session_id`
Expected: PASS.

- [ ] **Step 5: Run the whole codex module to confirm no regressions**

Run: `cargo nextest run -p cli_agent_usage codex`
Expected: PASS (all existing codex tests still green).

- [ ] **Step 6: Commit**

```bash
git add crates/cli_agent_usage/src/codex.rs
git commit -m "feat(usage): capture codex session_id from rollout meta"
```

---

### Task 3: `models_by_session` index on `UsageSnapshot`

Build the `session_id → latest model` map during the existing scan and expose a lookup. Threads a `&mut HashMap` out-param through both provider scans (matching the crate's existing `&mut` out-param style in `aggregate_windows`).

**Files:**
- Modify: `crates/cli_agent_usage/src/lib.rs` (add `use std::collections::HashMap;`; `UsageSnapshot` ~L80; `scan_local` ~L202; add accessor)
- Modify: `crates/cli_agent_usage/src/claude.rs` (`scan` signature ~L101; test call site ~L226)
- Modify: `crates/cli_agent_usage/src/codex.rs` (`scan` signature ~L166; test call site ~L297)

**Interfaces:**
- Consumes: `RollupFile.session_id` (Task 2); `Entry { ts, model, .. }`.
- Produces:
  - `UsageSnapshot { claude, codex, models_by_session: HashMap<String, String> }`
  - `UsageSnapshot::model_for_session(&self, session_id: &str) -> Option<&str>`
  - `claude::scan(dir, cache, now, models: &mut HashMap<String, String>) -> Provider`
  - `codex::scan(dir, cache, now, models: &mut HashMap<String, String>) -> Provider`

- [ ] **Step 1: Write the failing tests**

Add to `claude.rs`'s `mod tests`:

```rust
#[test]
fn scan_indexes_latest_model_by_session_id() {
    let dir = std::env::temp_dir().join(format!("cau_claude_modelidx_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Filename stem is the session id.
    let path = dir.join("11111111-2222-3333-4444-555555555555.jsonl");
    let older = r#"{"type":"assistant","requestId":"r1","timestamp":"2026-06-30T10:00:00.000Z","message":{"id":"m1","model":"claude-haiku","usage":{"input_tokens":1,"output_tokens":1,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}}"#;
    let newer = r#"{"type":"assistant","requestId":"r2","timestamp":"2026-06-30T12:00:00.000Z","message":{"id":"m2","model":"claude-opus-4-8","usage":{"input_tokens":1,"output_tokens":1,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}}"#;
    std::fs::write(&path, format!("{older}\n{newer}")).unwrap();

    let mut cache = crate::cache::ScanCache::new();
    let mut models = std::collections::HashMap::new();
    let _ = scan(&dir, &mut cache, chrono::Utc::now(), &mut models);

    assert_eq!(
        models.get("11111111-2222-3333-4444-555555555555").map(String::as_str),
        Some("claude-opus-4-8")
    );
    let _ = std::fs::remove_dir_all(&dir);
}
```

Add to `codex.rs`'s `mod tests`:

```rust
#[test]
fn scan_indexes_codex_model_by_session_id() {
    let dir = std::env::temp_dir().join(format!("cau_codex_modelidx_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("rollout-2026-06-25T15-51-33-019f00fb-40b5-7192-9b79-aa6d1034fe1b.jsonl");
    let meta = r#"{"timestamp":"2026-06-25T22:51:38.016Z","type":"session_meta","payload":{"session_id":"019f00fb-40b5-7192-9b79-aa6d1034fe1b","model":"gpt-5-codex"}}"#;
    let tok = r#"{"timestamp":"2026-06-25T22:52:00.000Z","type":"event_msg","payload":{"type":"token_count","model":"gpt-5-codex","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":0,"output_tokens":5,"total_tokens":15}}}}"#;
    std::fs::write(&path, format!("{meta}\n{tok}")).unwrap();

    let mut cache = crate::cache::ScanCache::new();
    let mut models = std::collections::HashMap::new();
    let _ = scan(&dir, &mut cache, chrono::Utc::now(), &mut models);

    assert_eq!(
        models.get("019f00fb-40b5-7192-9b79-aa6d1034fe1b").map(String::as_str),
        Some("gpt-5-codex")
    );
    let _ = std::fs::remove_dir_all(&dir);
}
```

> Both test modules already `use super::*` and construct temp dirs via
> `std::env::temp_dir().join(format!("cau_..._{}", std::process::id()))` and a
> `crate::cache::ScanCache::new()` — the snippets above match that existing
> pattern verbatim (distinct dir suffixes avoid colliding with the current scan
> tests in the same module).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p cli_agent_usage scan_indexes`
Expected: FAIL — `scan` takes 3 arguments but 4 were supplied.

- [ ] **Step 3: Add the field, accessor, and HashMap import in `lib.rs`**

At the top of `lib.rs`, add:

```rust
use std::collections::HashMap;
```

Change `UsageSnapshot` (~L80):

```rust
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageSnapshot {
    pub claude: Provider,
    pub codex: Provider,
    /// `session_id` → latest model id seen for that session. Built during the
    /// same local file walk that produces `claude`/`codex`. Empty when neither
    /// provider has local data.
    pub models_by_session: HashMap<String, String>,
}

impl UsageSnapshot {
    /// The latest model id recorded for `session_id`, if any.
    pub fn model_for_session(&self, session_id: &str) -> Option<&str> {
        self.models_by_session.get(session_id).map(String::as_str)
    }
}
```

Update `scan_local` (~L202) to create the map, pass it to both scans, and store it:

```rust
pub fn scan_local(paths: &Paths, caches: &mut Caches, now: DateTime<Utc>) -> UsageSnapshot {
    let mut models_by_session = HashMap::new();
    let claude = claude::scan(
        &paths.claude_projects,
        &mut caches.claude,
        now,
        &mut models_by_session,
    );
    let codex = codex::scan(
        &paths.codex_sessions,
        &mut caches.codex,
        now,
        &mut models_by_session,
    );
    UsageSnapshot {
        claude,
        codex,
        models_by_session,
    }
}
```

- [ ] **Step 4: Populate the map in `claude::scan`**

Change the signature (`claude.rs` ~L101) to add the out-param, and index each file inside the existing `for (path, mtime, size) in &files` loop:

```rust
pub fn scan(
    projects_dir: &Path,
    cache: &mut ScanCache<Vec<Entry>>,
    now: DateTime<Utc>,
    models: &mut std::collections::HashMap<String, String>,
) -> Provider {
```

Inside that loop, after `entries` is obtained (after the `.clone()` at ~L118) add:

```rust
        // Index this transcript's latest model under its session id (= file stem).
        if let Some(session_id) = path.file_stem().and_then(|s| s.to_str()) {
            if let Some(model) = entries.iter().max_by_key(|e| e.ts).map(|e| e.model.clone()) {
                models.insert(session_id.to_string(), model);
            }
        }
```

Update the existing test call site (`claude.rs` ~L226) from
`let p = scan(&dir, &mut cache, chrono::Utc::now());` to:

```rust
        let p = scan(&dir, &mut cache, chrono::Utc::now(), &mut std::collections::HashMap::new());
```

- [ ] **Step 5: Populate the map in `codex::scan`**

Change the signature (`codex.rs` ~L166) to add the out-param, and index each file inside the existing `for (path, mtime, size) in &files` loop:

```rust
pub fn scan(
    sessions_dir: &Path,
    cache: &mut ScanCache<RollupFile>,
    now: DateTime<Utc>,
    models: &mut std::collections::HashMap<String, String>,
) -> Provider {
```

Inside that loop, after `let parsed = cache.get_or_parse(...)` and before/after the aggregate call, add:

```rust
        // Index this rollout's latest model under its session id (from meta).
        if let Some(session_id) = parsed.session_id.clone() {
            if let Some(model) = parsed.entries.last().map(|e| e.model.clone()) {
                models.insert(session_id, model);
            }
        }
```

> Note `parsed` is a `&RollupFile` here; `.clone()` on `session_id`/model copies only the needed strings, not the whole file. Read these before the existing `let entries = parsed.entries.clone();` moves nothing (it clones); order doesn't matter as long as it's within the loop where `parsed` is borrowed.

Update the existing test call site (`codex.rs` ~L297) from
`let p = scan(&dir, &mut cache, chrono::Utc::now());` to:

```rust
        let p = scan(&dir, &mut cache, chrono::Utc::now(), &mut std::collections::HashMap::new());
```

- [ ] **Step 6: Run the new tests + full crate**

Run: `cargo nextest run -p cli_agent_usage`
Expected: PASS — including `scan_indexes_latest_model_by_session_id`, `scan_indexes_codex_model_by_session_id`, and every pre-existing test (the `UsageSnapshot { claude, codex }` literal at the old L205 is now the updated one; the `scan_local` tests at lib.rs still compile because `scan_local`'s signature is unchanged).

- [ ] **Step 7: Commit**

```bash
git add crates/cli_agent_usage/src/lib.rs crates/cli_agent_usage/src/claude.rs crates/cli_agent_usage/src/codex.rs
git commit -m "feat(usage): index session_id -> model on UsageSnapshot"
```

---

### Task 4: Render the model chip in the tab

Add a pure app-side label helper (unit-tested), subscribe the workspace to usage updates, and render the chip in the vertical tab row.

**Files:**
- Create: `app/src/ai/blocklist/usage/tab_model_label.rs` (+ `#[path]` test file `tab_model_label_tests.rs`)
- Modify: `app/src/ai/blocklist/usage/mod.rs` (declare + re-export)
- Modify: `app/src/workspace/view.rs` (subscribe to `CliAgentUsageModel` near the existing `CLIAgentSessionsModel` subscription ~L3055)
- Modify: `app/src/workspace/view/vertical_tabs.rs` (`render_pane_row` ~L3199; add `tab_agent_model_label` wrapper)

**Interfaces:**
- Consumes: `UsageSnapshot::model_for_session` (Task 3); `friendly_model_name` (Task 1); `CLIAgent`, `CLIAgentSessionsModel`, `CliAgentUsageModel`.
- Produces:
  - `pub fn cli_agent_model_label(snapshot: &UsageSnapshot, agent: CLIAgent, session_id: Option<&str>) -> Option<String>` (pure)
  - `fn tab_agent_model_label(app: &AppContext, view_id: EntityId) -> Option<String>` (private to `vertical_tabs.rs`)

- [ ] **Step 1: Write the failing test for the pure helper**

Create `app/src/ai/blocklist/usage/tab_model_label_tests.rs`:

```rust
use cli_agent_usage::UsageSnapshot;

use super::cli_agent_model_label;
use crate::terminal::CLIAgent;

fn snapshot_with(session_id: &str, model: &str) -> UsageSnapshot {
    let mut snap = UsageSnapshot::default();
    snap.models_by_session
        .insert(session_id.to_string(), model.to_string());
    snap
}

#[test]
fn returns_friendly_label_for_known_session() {
    let snap = snapshot_with("sess-1", "claude-opus-4-8");
    assert_eq!(
        cli_agent_model_label(&snap, CLIAgent::Claude, Some("sess-1")),
        Some("Opus 4.8".to_string())
    );
}

#[test]
fn none_when_no_session_id_or_unknown_agent_or_miss() {
    let snap = snapshot_with("sess-1", "claude-opus-4-8");
    assert_eq!(cli_agent_model_label(&snap, CLIAgent::Claude, None), None);
    assert_eq!(cli_agent_model_label(&snap, CLIAgent::Unknown, Some("sess-1")), None);
    assert_eq!(cli_agent_model_label(&snap, CLIAgent::Claude, Some("nope")), None);
}

#[test]
fn none_when_model_is_noise() {
    let snap = snapshot_with("sess-1", "<synthetic>");
    assert_eq!(cli_agent_model_label(&snap, CLIAgent::Claude, Some("sess-1")), None);
}
```

- [ ] **Step 2: Create the helper and wire the module**

Create `app/src/ai/blocklist/usage/tab_model_label.rs`:

```rust
use cli_agent_usage::format::friendly_model_name;
use cli_agent_usage::UsageSnapshot;

use crate::terminal::CLIAgent;

/// The tab chip label for a CLI-agent session, or `None` when there's nothing to
/// show: unknown agent, no session id, no indexed model, or a noise model id.
pub fn cli_agent_model_label(
    snapshot: &UsageSnapshot,
    agent: CLIAgent,
    session_id: Option<&str>,
) -> Option<String> {
    if matches!(agent, CLIAgent::Unknown) {
        return None;
    }
    let raw = snapshot.model_for_session(session_id?)?;
    let friendly = friendly_model_name(raw);
    if friendly.is_empty() {
        None
    } else {
        Some(friendly)
    }
}

#[cfg(test)]
#[path = "tab_model_label_tests.rs"]
mod tests;
```

In `app/src/ai/blocklist/usage/mod.rs`, add near the other `pub mod`/`pub use` lines (~L7-L11):

```rust
pub mod tab_model_label;
pub use tab_model_label::cli_agent_model_label;
```

- [ ] **Step 3: Run the helper test to verify it passes**

Run: `cargo nextest run -p warp cli_agent_model_label`
Expected: PASS (3 tests). (This also confirms the module wiring compiles.)

- [ ] **Step 4: Commit the helper**

```bash
git add app/src/ai/blocklist/usage/tab_model_label.rs app/src/ai/blocklist/usage/tab_model_label_tests.rs app/src/ai/blocklist/usage/mod.rs
git commit -m "feat(tabs): pure cli_agent_model_label helper"
```

- [ ] **Step 5: Subscribe the workspace to usage updates**

In `app/src/workspace/view.rs`, locate the constructor block that subscribes to
`CLIAgentSessionsModel` (~L3055):

```rust
        ctx.subscribe_to_model(&CLIAgentSessionsModel::handle(ctx), |me, _, event, ctx| {
            me.handle_cli_agent_sessions_event(event, ctx);
        });
```

Immediately after it, add a subscription that re-renders the tab strip when the
model index changes (the model emits only on real change, so this does not wake
every 5s):

```rust
        // Re-render tabs when a CLI-agent session's model changes.
        ctx.subscribe_to_model(&CliAgentUsageModel::handle(ctx), |_, _, _, ctx| {
            ctx.notify();
        });
```

Add the import at the top of `view.rs` (next to the existing `CLIAgentSessionsModel` import ~L377):

```rust
use crate::ai::blocklist::usage::CliAgentUsageModel;
```

> If a later merge of the usage-header work adds the same subscription, keep only one.

- [ ] **Step 6: Add the render wrapper and chip in `vertical_tabs.rs`**

At the top of `vertical_tabs.rs`, ensure these are imported (add any missing):

```rust
use crate::ai::blocklist::usage::{cli_agent_model_label, CliAgentUsageModel};
```

(`CLIAgentSessionsModel` is already imported at L59; `CLIAgent`, `Text`, `Container`, `ConstrainedBox`, `ClipConfig` are already used in this file.)

Add this private helper (near the other free functions, e.g. just above `render_pane_row` at ~L3199):

```rust
/// The model chip label for a terminal tab running a CLI agent, or `None`.
fn tab_agent_model_label(app: &AppContext, view_id: EntityId) -> Option<String> {
    let sessions = CLIAgentSessionsModel::as_ref(app);
    let session = sessions.session(view_id)?;
    let snapshot = CliAgentUsageModel::as_ref(app).latest();
    cli_agent_model_label(
        snapshot,
        session.agent,
        session.session_context.session_id.as_deref(),
    )
}
```

In `render_pane_row` (`~L3199`), after the `title_row` title child is added and
just before the `if has_indicator {` block (~L3253), insert:

```rust
        if let TypedPane::Terminal(terminal_pane) = &props.typed {
            let view_id = terminal_pane.terminal_view(app).as_ref(app).id();
            if let Some(label) = tab_agent_model_label(app, view_id) {
                title_row.add_child(
                    Container::new(
                        ConstrainedBox::new(
                            Text::new_inline(label, font_family, 11.)
                                .with_clip(ClipConfig::ellipsis())
                                .with_color(theme.sub_text_color(theme.background()).into())
                                .finish(),
                        )
                        .with_max_width(96.)
                        .finish(),
                    )
                    .with_margin_left(6.)
                    .finish(),
                );
            }
        }
```

> `font_family` and `theme` are already bound in `render_pane_row` (used by the
> title `Text` at ~L3239 and color at ~L3241). If `Text::new_inline`'s exact
> signature or `ConstrainedBox`/`Container` builder names differ here, mirror the
> title `Text` construction a few lines above — do not invent new APIs.

- [ ] **Step 7: Build to verify it compiles**

Run: `cargo check -p warp`
Expected: compiles cleanly (no unused-import / borrow errors).

- [ ] **Step 8: Manual verification**

Run: `cargo run` (or `make install-local` then launch), then:
1. Start Claude Code in a tab → the tab shows a dim `Opus 4.8` (or current model) chip next to the Claude icon within ~5s.
2. Start Codex (with the rich plugin / `FeatureFlag::CodexPlugin`) in another tab → shows `GPT-5 Codex` (or current). A Codex tab on the OSC 9 fallback shows no chip (expected — no session_id).
3. `/model` in the Claude tab → chip updates within ~5s.
4. A plain terminal tab (no agent) shows no chip.
5. Narrow the vertical panel → the chip clips with ellipsis and never pushes the tab title off-screen.

- [ ] **Step 9: Format, clippy, commit**

```bash
./script/format
cargo clippy -p warp --all-targets -- -D warnings
git add app/src/workspace/view.rs app/src/workspace/view/vertical_tabs.rs
git commit -m "feat(tabs): show running model chip in CLI-agent tabs"
```

---

## Out of scope (v1)

- **Hover tooltip** with the full model at narrow widths — no existing tooltip hook on the tab row; the chip's max-width + ellipsis already prevents overflow. Fast-follow.
- **Horizontal tab bar** (`app/src/tab.rs`) — the vertical panel is the featured Clinch layout and already renders the agent icon; apply the same helper there as a follow-on if desired.
- **OSC 9-fallback Codex** model display — blocked on `session_id: None` upstream; would need a Codex rich-plugin session.

## Self-review notes

- **Spec coverage:** data map (Task 3) ← spec §A; friendly names (Task 1) ← §B; render + subscription (Task 4) ← §C/§D; Codex session_id (Task 2) enables §A for Codex. Edge cases (no data, unknown model, OSC9 Codex, model switch) covered by Task 1/4 tests + manual steps.
- **Type consistency:** `models_by_session: HashMap<String, String>`, `model_for_session(&str) -> Option<&str>`, `scan(.., &mut HashMap<String, String>) -> Provider`, `friendly_model_name(&str) -> String`, `cli_agent_model_label(&UsageSnapshot, CLIAgent, Option<&str>) -> Option<String>` — used identically across tasks.
- **No placeholders:** every code step shows full code; the two "mirror existing API" notes point at concrete adjacent lines rather than deferring design.
