# Design: Show the running model in CLI-agent tabs

**Date:** 2026-07-02
**Status:** Draft — pending user approval of the display form (see Open question)

## Problem

Tabs running a CLI agent (Claude Code or Codex) show a per-agent icon in the
vertical tab panel, so you can tell *which agent* is running — but not *which
model*. When several tabs each have an agent going, there's no way to glance at
the tab strip and see "this one is on Opus 4.8, that one is on GPT-5-Codex."

## Goal

For each tab with a live Claude/Codex session, surface the model that session is
running, directly in the tab, updating as the session runs (and if the user
switches models mid-session).

## Non-goals

- No changes to the OSC plugin wire protocol (`event/v1.rs`) or the external
  `warpdotdev/claude-code-warp` plugin. The model is derived client-side.
- No new user setting. Follows the existing "show when there is data" convention
  (same as the usage chip). Hidden automatically when no model is known.
- No changes to how usage tokens are aggregated or displayed.

## Background: relevant code

- **Per-tab agent state:** `app/src/terminal/cli_agent_sessions/mod.rs`
  - `CLIAgentSessionsModel` (singleton) keys `CLIAgentSession` by
    `EntityId` (terminal-view id). `.session(view_id)` returns the tab's session.
  - `CLIAgentSession { agent, status, session_context, .. }`;
    `CLIAgentSessionContext { session_id, cwd, project, .. }` — already carries
    `session_id`, populated in `apply_event` from every event.
- **Tab render (vertical panel):** `app/src/workspace/view/vertical_tabs.rs`
  - `render_detail_kind_badge_icon` (~L1415): the `TypedPane::Terminal` arm
    returns `session.agent.icon()` tinted by `session.agent.brand_color()` — the
    existing agent glyph.
  - Tab content is a column (~L3261): a `title_row` (title + optional status
    indicator) with an optional **subtitle line** below (`effective_subtitle`,
    ~L3267). Two natural spots for a model label.
- **Model already parsed (client-side):** `crates/cli_agent_usage/`
  - `claude::parse_transcript_str(content) -> Vec<Entry>`; each `Entry` has
    `model` and `ts`. Claude transcript files live at
    `~/.claude/projects/**/<session_id>.jsonl` — **filename stem = session_id.**
  - `codex::parse_rollout_str(content) -> RollupFile`; tracks the latest `model`
    seen. Codex files live at `~/.codex/sessions/**/rollout-<ts>-<uuid>.jsonl` —
    **filename embeds the session uuid.**
  - `scan_local(paths, caches, now) -> UsageSnapshot` walks both dirs via
    `cache::scan_dir` (which yields each file's `PathBuf`) and parses each file
    through an `mtime`/size-keyed `ScanCache`. Exposed as the `CliAgentUsageModel`
    singleton; `::as_ref(app).latest()` returns the snapshot and `::handle(ctx)`
    is subscribable. The tab bar's usage header already subscribes to it (see
    `2026-07-02-usage-header-footer-design.md`).

## Design

### A. Data: derive `session_id → model` from the existing scan

Extend `cli_agent_usage`'s scan (which already opens and parses every transcript
/ rollout file) to also record, per file, the **latest-by-timestamp model** and
the **session id derived from the file path**. Aggregate into a lookup on the
snapshot:

```rust
// lib.rs
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageSnapshot {
    pub claude: Provider,
    pub codex: Provider,
    /// session_id -> latest model id seen for that session. Populated during the
    /// same file walk that builds `claude`/`codex`.
    pub models_by_session: HashMap<String, String>,
}
```

- **Claude** (`claude::scan`): for each file, `session_id` = the path's file stem
  (`path.file_stem()`); model = the `model` of the newest `Entry` in that file.
  This relies on Claude Code naming transcripts `<session_id>.jsonl` and that
  same id being what the OSC plugin reports as `session_id`. **Verify against a
  real transcript during implementation**; if they ever diverge, fall back to
  keying by `transcript_path` (which the event already carries — store it in
  `CLIAgentSessionContext` and match on it). Fail-soft either way: a miss shows
  no chip, never a wrong model.
- **Codex** (`codex::scan`): for each file, `session_id` = the uuid parsed from
  the `rollout-<ts>-<uuid>.jsonl` stem; model = `RollupFile`'s last-seen `model`.
  (If a rollout also records a `session_id`/`conversation_id` in a meta line, use
  that when present and fall back to the filename uuid — pinned during
  implementation against a real rollout file.)

This reuses **all** existing machinery — file discovery, the `ScanCache`
(re-parses only when a file's mtime/size changes), and the singleton's polling —
so there is no new file I/O, no globbing, and no async work on the render path.
Model switches mid-session are picked up on the next scan (the file's mtime
changes, so the cache re-parses it).

Add a convenience accessor:

```rust
impl UsageSnapshot {
    pub fn model_for_session(&self, session_id: &str) -> Option<&str> {
        self.models_by_session.get(session_id).map(String::as_str)
    }
}
```

### B. Friendly model names

Add a small pure formatter (in `cli_agent_usage/src/format.rs`, where the other
display helpers live), mapping raw ids to short display names:

```rust
/// "claude-opus-4-8" -> "Opus 4.8", "gpt-5-codex" -> "GPT-5 Codex", etc.
/// Unknown ids fall back to a lightly title-cased form of the raw id.
pub fn friendly_model_name(model: &str) -> String
```

Rules (unit-tested):
- Strip a leading `claude-` / `anthropic.` / `gpt-`-family prefix where it maps
  cleanly; title-case the family (`opus`/`sonnet`/`haiku` → `Opus`/`Sonnet`/
  `Haiku`); turn a trailing `-4-8` → `4.8`.
- Codex: `gpt-5-codex` → `GPT-5 Codex`, `gpt-5.5` → `GPT-5.5`.
- Anything unrecognized (e.g. Claude's `<synthetic>`): return `None`-equivalent by
  yielding an empty string so callers hide the chip rather than show noise.
  (Signature may become `-> Option<String>` — decided during implementation; the
  caller treats empty/None as "no model to show".)

### C. Render: model label in the tab

**Primary (recommended) — inline chip in `title_row`.** In the vertical tab's
content builder (`vertical_tabs.rs` ~L3229), after the title `Shrinkable` and
before/after the status indicator, add a dim monospace model chip when the tab
has a CLI-agent session with a known model:

- `let session = CLIAgentSessionsModel::as_ref(app).session(view_id);`
- `let model = session.and_then(|s| s.session_context.session_id.as_deref())
     .and_then(|id| CliAgentUsageModel::as_ref(app).latest().model_for_session(id).map(str::to_owned));`
  - **Perf:** read the snapshot by reference (`::as_ref(app).latest()`) and copy
    only the single model `String` via `model_for_session` — do **not**
    `.clone()` the whole `UsageSnapshot` (and its `models_by_session` map) per
    tab per frame. `model_for_session` returns `Option<&str>`; clone just that.
- If `Some(m)` and `friendly_model_name(&m)` is non-empty, render a
  `Text::new_inline(friendly, mono_font, ~11.)` colored `sub_text_color`, wrapped
  so it yields space to the title (`Shrinkable` / lower flex weight) and clips
  with ellipsis. Never let the model chip push the title off-screen.
- **Graceful degradation:** wrap the chip in a `SizeConstraintSwitch`
  (`crates/warpui_core/.../size_constraint_switch.rs`, already used by the usage
  header) so at narrow tab widths the chip drops out and the model is available
  via the tab tooltip instead (see below).

**Tooltip fallback (fast-follow, not v1).** The tab row has no existing tooltip
hook, so adding tooltip infra is disproportionate for v1. Instead, v1 bounds the
chip with a max-width + ellipsis so it can never push the title off-screen; at
extreme narrow widths it clips to nothing. A hover tooltip carrying the full
model is a small follow-on once the chip lands.

**Alternative render (Option B), if preferred over A:** put
`"{agent} · {friendly_model}"` into the existing subtitle slot
(`effective_subtitle`, ~L3267) instead of an inline chip. Lower-friction (the
slot already exists) but competes with the running-command subtitle. Chosen vs A
by the Open question below.

### D. Re-render on updates

The tab strip must re-render when the scan produces a new model. If the
vertical-tabs view already re-renders on `CliAgentUsageModel` changes (the
usage-header work subscribes the workspace/tab-bar to it), reuse that
subscription. Otherwise add a `ctx.subscribe_to_model(&CliAgentUsageModel::handle(ctx), .. ctx.notify())`
in the panel's constructor. Session start/end already notifies via
`CLIAgentSessionsModelEvent`.

**Scan freshness — already guaranteed.** `CliAgentUsageModel::new` starts an
unconditional dedicated-thread producer that re-scans local files every 5s
(`FILE_POLL`, `cli_agent_usage_model.rs`) from app boot when `Paths::detect()`
succeeds — independent of whether any usage UI is visible. So the model map is
always kept current; no extra polling is needed. `on_snapshot` notifies only on
`snap != latest`, so adding `models_by_session` to `UsageSnapshot`'s `PartialEq`
makes a model switch trigger the re-render automatically (max ~5s lag).

### E. Horizontal tab bar

The vertical panel is Clinch's featured layout and is where the agent icon
already lives, so it is the primary target. If the horizontal tab bar
(`app/src/tab.rs`) also renders the agent icon, apply the same tooltip + (space
permitting) inline chip there using the identical helper. If it does not surface
agent state today, horizontal-bar support is a small follow-on, not part of this
change — called out so scope is explicit.

## Edge cases

- **No model known** (no `~/.claude`/`~/.codex` data yet, brand-new session
  before the first assistant turn is written, or unrecognized/`<synthetic>`
  model): `model_for_session` returns `None` / friendly name is empty → no chip,
  tooltip omits the model line. Tab looks exactly as it does today.
- **Codex requires the rich plugin.** Codex's OSC 9 fallback hardcodes
  `session_id: None` (`listener/mod.rs:111`), so those tabs have no key and show
  no model chip — expected degradation. Only plugin-backed Codex (OSC 777,
  `FeatureFlag::CodexPlugin`) reports a `session_id`. Claude's plugin always
  reports one. Command-only-detected sessions (no rich event yet) also lack a
  session_id until the first rich event arrives.
- **session_id mismatch** (any provider): the lookup simply misses → no chip.
  Fail-soft; never shows a wrong model.
- **Model switch mid-session** (`/model`, Codex model change): the file mtime
  changes, `ScanCache` re-parses, the latest-by-ts model updates on the next
  scan; the chip follows.
- **Scan staleness:** the chip can lag a model change by up to one scan interval.
  Acceptable for a model indicator; models rarely change mid-session.
- **Remote/SSH sessions:** the usage scan is local-only, so remote sessions have
  no local file and show no model (tooltip omits it). Consistent with usage.

## Dead code check

No code is obsoleted by this change (it is purely additive: a new snapshot field,
a new pure formatter, and a render branch). Nothing to remove. The new
`models_by_session` map and `friendly_model_name` are both consumed by the tab
render, so neither is dead.

## Testing

- **Unit (`crates/cli_agent_usage`):**
  - `friendly_model_name`: table test over real ids (`claude-opus-4-8`,
    `claude-sonnet-5`, `claude-haiku-4-5`, `gpt-5-codex`, `gpt-5.5`, unknown,
    `<synthetic>`).
  - scan → `models_by_session`: feed a Claude transcript string with two
    assistant lines of differing timestamps/models and assert the *latest* model
    is recorded under the file's session id; same for a Codex rollout string
    (place in the existing `claude`/`codex` test modules).
- **Render logic:** if the tab builder gains any non-trivial pure helper (e.g.
  "compute the chip string for a view"), unit-test that per repo `*_tests.rs`
  convention. `SizeConstraintSwitch` selection is framework-tested.
- **Manual (after `make install-local` / `cargo run`):**
  1. Start Claude Code in one tab and Codex in another; each tab shows its model
     (e.g. `Opus 4.8`, `GPT-5 Codex`) next to the agent icon.
  2. Narrow the vertical panel → chip drops out gracefully, tooltip still shows
     the model; tabs never pushed off-screen.
  3. `/model` in a Claude tab → the chip updates within a scan interval.
  4. A plain terminal tab (no agent) shows no chip and no model tooltip line.

## Rollout

Single additive change, no feature flag (consistent with how the usage chip
shipped). Always on when model data exists.

## Open question (needs user confirmation)

Display form was asked but not answered. This spec assumes **Option A: inline
model chip next to the agent icon, with a tooltip fallback at narrow widths.**
Alternatives, in decreasing recommendation: **B** dim subtitle line
(`"Claude · Opus 4.8"` under the title), **C** tooltip-only (zero space, not
glanceable), **D** model badge replacing the agent glyph (loses the distinct
agent icon). If A is not preferred, the render section swaps to the chosen form;
the data path (A/B sections) is unchanged.
