# CLI Agent Usage Footer — UI & Wiring Design (Plan B)

**Date:** 2026-06-30
**Status:** Approved design, pending implementation plan
**Supersedes:** the UI (§9), cost (§8), and threading (§5–§6) portions of
`2026-06-30-cli-agent-usage-footer-design.md`. That document remains the record of
the overall feature and the data-source research; **this document is authoritative for
the footer wiring, threading, and display.**

## 1. Context & scope

The data layer is **done**: the `cli_agent_usage` workspace crate (26 lib tests, clippy
clean) sources and aggregates all Claude Code + Codex usage. This spec covers only **Plan
B: wiring that crate into the Clinch CLI-agent footer** — a compact chip that expands into
a panel — plus the two small crate additions Plan B needs.

Nothing here changes the crate's parsing/aggregation logic. The only crate edits are two
additive helpers (§7).

## 2. Finalized display decisions

Three decisions settled in brainstorming, and they drive the whole UI:

1. **Chip headlines plan-% for both tools** — the one actionable number (how close to your
   caps), not token counts. Weekly window. E.g. `◷ cc 47%w · cx 55%w`.
2. **Panel token metric = input+output headline, cache-read dimmed.** The "work" tokens
   lead; cache-read (which dominates raw totals — month ≈ 8.3 B tok) is shown small/dimmed
   so it informs without misleading.
3. **No cost anywhere.** The crate still computes `cost_usd`, but the footer ignores it —
   for subscription users a dollar figure reads as a bill it is not. `WindowTotals.cost_usd`
   is simply never rendered.

## 3. Architecture

```
crates/cli_agent_usage/src/lib.rs          # + scan_local(); + TokenCounts::io()  (§7)

app/src/ai/blocklist/usage/
  cli_agent_usage_model.rs   # NEW: CliAgentUsageModel singleton (Entity + SingletonEntity)
                             #   - spawns ONE producer std::thread (blocking file IO + HTTP)
                             #   - producer sends UsageSnapshot over an async_channel
                             #   - ctx.spawn_stream_local(rx, on_item, ..) delivers each
                             #     snapshot on the MAIN thread -> store latest + ctx.notify()
                             #   - holds `latest: UsageSnapshot`
  cli_agent_usage_format.rs  # NEW: pure formatting/label fns + unit tests
                             #   - fmt_tokens(u64) -> "8.3B" / "1.2M" / "947k" / "512"
                             #   - fmt_pct(f64) -> "47%"
                             #   - fmt_reset(Option<DateTime<Utc>>, now) -> "in 3h" / "—"
                             #   - chip_halves(&UsageSnapshot) -> the two rendered halves
                             #       (text + Severity per provider), or a "hidden" signal
                             #       when both providers are empty
                             #   - severity->color mapping helper (theme token per Severity)

app/src/lib.rs (~1362)                       # add_singleton_model(CliAgentUsageModel)
app/src/ai/blocklist/agent_view/agent_input_footer/
  toolbar_item.rs (~48)                      # + AgentToolbarItemKind::CliAgentUsage (+ ~9 match arms, default lists)
  mod.rs render_cli_toolbar_item() (~1442)   # render chip (+ new panel-popover fn)
  mod.rs render_toolbar_item() (~2055)       # exhaustive: add arm (returns None; agent view)
  mod.rs AgentInputFooter::new() (~258)      # subscribe_to_model + open-state field

app/Cargo.toml                               # cli_agent_usage = { path = "crates/cli_agent_usage" }  (if not already present)
```

Split of responsibility:

- **Producer thread** owns everything blocking and stateful: the `Caches`, the poll
  cadence, and the last-known-good `PlanLimits`. It emits a *complete* `UsageSnapshot` each
  cycle.
- **`CliAgentUsageModel`** is a thin latest-value holder on the UI thread: it drains the
  channel on a light timer, stores the newest snapshot, and notifies observers. It contains
  no IO and no aggregation logic.
- **`cli_agent_usage_format`** is pure (snapshot → strings + colors), so it is unit-tested
  without a UI or a running app.
- **Footer** renders the chip + panel from `model.latest` via the format helpers.

## 4. Threading model (the crux — corrects the original spec)

The original spec assumed `async fn refresh` on gpui timers. The final crate review proved
`refresh()` is **blocking** and that `reqwest::blocking` **panics if constructed inside a
Tokio/async runtime**. This is not hypothetical here: `ctx.background_executor()` returns a
`tokio::runtime::Runtime` (`warpui_core/src/async/native/executor.rs:129`), and both
`ctx.spawn(future, cb)` and `background_executor().spawn(future)` run their future *on that
Tokio runtime*. So the blocking work must **not** go through any gpui executor slot.

**Robust design: one dedicated `std::thread` producer + an `async_channel` + the toolkit's
`ctx.spawn_stream_local` to consume it on the main thread.** A raw OS thread is guaranteed to
have no Tokio runtime context, so `reqwest::blocking` cannot panic there — independent of the
(confirmed) fact that gpui's background executor is Tokio-backed.

Producer loop (pseudocode; runs on the dedicated thread):

```rust
// owns: paths: Paths, caches: Caches, keychain: MacKeychain, fetch: ReqwestUsage
let mut last_plan: Option<PlanLimits> = None;   // last-known-good, PlanLimits is Copy
let mut tick: u64 = 0;
loop {
    let now = Utc::now();
    // ~5s: always cheap (incremental cache re-parses only changed files)
    let mut snap = cli_agent_usage::scan_local(&paths, &mut caches, now);   // §7

    // ~60s: slow, network — Claude plan-% only
    if tick % ENDPOINT_EVERY == 0 {
        if let Some(fresh) = fetch_claude_plan(&keychain, &fetch, &paths, now) {
            last_plan = Some(fresh);        // overwrite only on success => last-good retained
        }
    }
    snap.claude.plan = last_plan;           // apply last-good (Codex plan is already local)

    if tx.send_blocking(snap).is_err() {
        break;                              // Receiver dropped (model gone) => exit cleanly
    }
    tick = tick.wrapping_add(1);
    std::thread::sleep(FILE_POLL);          // ~5s; blocking sleep is fine on a real thread
}
```

`tx`/`rx` are an `async_channel` pair (small bounded capacity). `send_blocking` is the
sync-callable send for a non-async thread; it returns `Err` once the `Receiver` is gone,
which is our shutdown signal.

- `fetch_claude_plan` = `keychain::read_claude_token` → expiry check → `fetch.fetch` →
  `http::parse_plan_limits`, returning `Option<PlanLimits>` (all already public). It is the
  Claude-plan half of the crate's `refresh()`, called on its own slow cadence.
- **Last-known-good:** `last_plan` is overwritten only when a fetch succeeds; a transient
  429/timeout/expiry leaves the previous value in place, so the chip does not flicker to `—`.
  If Claude was never logged in, `last_plan` stays `None` and plan-% shows `—` (correct).
- **Codex plan-%** needs no special handling — it is local, populated fresh by `scan_local`
  every ~5s.
- **Clean shutdown:** when the consuming stream task is dropped (model gone) the `Receiver`
  drops; the next `tx.send_blocking` returns `Err` and the loop breaks. No join handle, no
  shutdown flag needed. (Singletons live for the app lifetime, so in practice the thread runs
  until exit.)

UI side (`CliAgentUsageModel`, on the main thread — an `Entity` + `SingletonEntity`):

- On construction (`new(ctx: &mut ModelContext<Self>)`): `Paths::detect()`; create an
  `async_channel` pair; spawn the producer `std::thread` with the `Sender`, `Paths`, a fresh
  `Caches::new()`, `MacKeychain`, and `ReqwestUsage`. Hold `latest: UsageSnapshot::default()`.
- Consume with **`ctx.spawn_stream_local(rx, |model, snap, ctx| { model.latest = snap;
  ctx.notify(); }, |_, _| {})`** — the toolkit's documented "receive on the main thread from
  another thread" primitive (`warpui_core` model/view `context.rs`; used by
  `terminal/model_events.rs`, `input_suggestions.rs`). `on_item` runs on the main thread with
  `&mut Self`, so storing the snapshot and calling `ctx.notify()` (and/or `ctx.emit(...)` for
  the footer subscription) is correct there. No hand-rolled timer, no `try_recv` polling.
- The footer observes via `ctx.subscribe_to_model(&CliAgentUsageModel::handle(ctx), |_, _, _,
  ctx| ctx.notify())`, exactly as it already does for `AIRequestUsageModel`.

Cadence constants (module consts, easily tuned): `FILE_POLL = 5s`, `ENDPOINT_EVERY = 12`
(⇒ ~60 s). Delivery is push (stream), not poll.

## 5. Chip

- Content: a small gauge/clock icon + two halves — `cc {weekly}%w · cx {weekly}%w` — where
  each `%` is `PlanLimits.weekly.percent` rounded to an integer.
- Color: each half is colored by its own `weekly.severity`, mapping the crate `Severity`
  **enum** (authoritative — Claude's comes straight from the endpoint's `severity`, Codex's
  from `severity_from_percent`) to theme tokens: `Normal` → `theme.main_text_color(bg)`,
  `Warning` → `Fill::Solid(theme.ui_warning_color())` (amber), `Critical` →
  `Fill::Solid(theme.ui_error_color())` (red). (The existing context-window chip's color lives
  in `render_context_window_usage_icon`, `app/src/ai/blocklist/usage/mod.rs:35`, which is a
  two-stop red-at-≥0.8 ramp; we use the three-stop semantic tokens to honor `Warning`.)
- Because the two halves carry different colors, the chip is a small **custom element**
  (`Flex::row` of an icon + two colored text spans) rather than a single-color stock
  `ActionButton`; it is wrapped in a `Hoverable` (cursor + click) following the `DisplayChip`
  interaction pattern (`app/src/context_chips/display_chip.rs`).
- Missing data: a provider with no `weekly` limit shows `— ` for its half (stable width, no
  layout jitter). The **whole chip is hidden** only when *both* providers are entirely empty
  (no token windows and no plan) — i.e. neither tool has ever run.
- **Click** toggles the panel popover (not hover — a detailed panel on hover is janky).

## 6. Panel (popover)

Two columns, **Claude Code | Codex**. The popover uses the codebase's overlay mechanism —
**`Stack::add_positioned_overlay_child(panel_element, OffsetPositioning::offset_from_parent(
offset, ParentOffsetBounds::WindowByPosition, parent_anchor, child_anchor))`**, added only
when an open-state bool is set — the same pattern `DisplayChip` and the footer's FTU callout
use (there is no gpui `PopoverMenu` in this toolkit). The open/close bool lives on
`AgentInputFooter` (mirroring its existing `has_open_chip_menu`), toggled by a footer action
dispatched from the chip's `on_click`. Rows top-to-bottom:

| Row | Cell content | Color |
|---|---|---|
| *(header)* | `Claude Code` \| `Codex` | default |
| 5-hour | `{session.percent}%` · `resets {fmt_reset}` | `session.severity` |
| Weekly | `{weekly.percent}%` · `resets {fmt_reset}` | `weekly.severity` |
| Session | `{io} tok` **·** dimmed `{cache_read} cache` | default / muted |
| Today | `{io} tok` · dimmed `{cache_read} cache` | default / muted |
| This week | `{io} tok` · dimmed `{cache_read} cache` | default / muted |
| This month | `{io} tok` · dimmed `{cache_read} cache` | default / muted |

- `{io}` = `WindowTotals.tokens.io()` (input+output, §7); `{cache_read}` =
  `WindowTotals.tokens.cache_read`, rendered with the theme's muted/secondary text token.
- **No cost row.** `cost_usd` is not read here.
- Any missing cell (absent provider/window/limit) → `—`.
- Numbers humanized via `fmt_tokens` (`8.3B`, `1.2M`, `947k`, `512`); percents via
  `fmt_pct`; resets via `fmt_reset` (relative, e.g. `in 3h` / `in 2d`, `—` when `None`).

## 7. Crate additions (additive, small)

Both are pure additions to `cli_agent_usage` — no behavior change to existing functions.

1. **`TokenCounts::io(&self) -> u64`** → `self.input + self.output`. The chosen headline
   metric. One method + one unit test.
2. **`scan_local(paths: &Paths, caches: &mut Caches, now: DateTime<Utc>) -> UsageSnapshot`**
   → exactly `refresh()` minus the Claude Keychain+HTTP block: `claude::scan` +
   `codex::scan`, returning a snapshot whose `claude.plan` is `None` and whose `codex.plan`
   is populated locally. This is what lets the producer poll files frequently without
   touching the network, so cadence can be split. One fail-soft unit test (no dirs →
   all-zero snapshot, no panic).

`refresh()` is **retained** as the simple one-shot public entry (used by the `print_usage`
example and a future headless `clinch usage`); Plan B deliberately uses the finer-grained
pieces (`scan_local` + `read_claude_token`/`fetch`/`parse_plan_limits`) to split cadence.
It is not dead code.

## 8. Error handling (fail-soft + last-known-good)

- Every cell is independent: an absent dir, malformed line, missing/expired token, or HTTP
  error yields `—` for that cell and never blocks or panics — the crate already guarantees
  this, and the producer thread cannot crash the UI regardless.
- Plan-% uses last-known-good (§4) so transient endpoint failures don't blank the chip.
- The footer must render correctly when neither tool has ever run (chip hidden; panel all
  `—` if opened via any residual state).
- The producer thread never calls `panic!`; the one theoretical panic source
  (`reqwest::blocking` in an async context) is structurally excluded by running on a raw
  `std::thread`.

## 9. Testing

- **`cli_agent_usage_format` (pure):** unit tests for `fmt_tokens` boundaries
  (999 → `999`, 1_000 → `1.0k`, 1_500_000 → `1.5M`, 8_316_864_043 → `8.3B`, 0 → `0`),
  `fmt_pct`, `fmt_reset` (`None` → `—`; a fixed future instant → `in Nh`/`in Nd`), and
  `chip_halves` (both providers present; one missing → `—` half; both empty → hidden signal).
- **Crate additions:** `TokenCounts::io()` and `scan_local()` (fail-soft, no dirs) unit
  tests as in §7.
- **Threading/render:** not unit-tested (gpui UI + real thread). Verified by running the app
  against the live machine (the crate's `print_usage` example already proves the data path
  end-to-end). The pure seams above carry the automated coverage.

## 10. Integration points (verified file\:line)

`AgentToolbarItemKind` (`toolbar_item.rs:48`) is a **serialized settings enum** (derives
`Serialize`/`Deserialize`/`JsonSchema`/`SettingsValue`), so a new `CliAgentUsage` variant is a
schema addition and the compiler forces it into ~9 exhaustive `match self` arms. Mirror the
existing `ContextWindowUsage` variant (the closest analog — also a live-state usage chip):

- `toolbar_item.rs` — add the variant, then handle it in `available_in()` (:78),
  `available_to_session_viewer()` (:97), `display_label()` (:117), `icon()` (:134),
  `is_available_during_handoff_compose()` (:155); and add it to the CLI default/available
  lists `cli_default_right()` (:280) + `all_available_for_cli_input()` (:289). (`is_available()`
  has a `_ => true` catch-all.) For an always-on fork chip, availability returns CLI-only/true
  rather than a `FeatureFlag` gate.
- `.../agent_input_footer/mod.rs` — the two **exhaustive** render matches: add an arm to
  `render_toolbar_item()` (:2055, agent view — returns `None`) and the real chip render to
  `render_cli_toolbar_item()` (:1442). The chip render reads model state via
  `CliAgentUsageModel::as_ref(app)` and returns `Some(element)` (or `None` to hide). The panel
  overlay is attached here via `Stack::add_positioned_overlay_child` gated on the footer's
  open-state bool (§6).
- `.../agent_input_footer/mod.rs:258` — `AgentInputFooter::new()`: add the open-state field and
  `ctx.subscribe_to_model(&CliAgentUsageModel::handle(ctx), |_, _, _, ctx| ctx.notify())`
  (alongside the existing `AIRequestUsageModel` subscription at ~:709).
- `app/src/lib.rs:1362` — register the singleton with
  `ctx.add_singleton_model(|ctx| CliAgentUsageModel::new(ctx))` (next to the
  `AIRequestUsageModel` registration). Model uses `impl Entity { type Event }` +
  `impl SingletonEntity {}`, mirroring `request_usage_model.rs:182,659` — **without** the
  `is_logged_in()` gate that makes the stock usage models inert in this fork.
- Color/text tokens: `render_context_window_usage_icon` (`app/src/ai/blocklist/usage/mod.rs:35`)
  for the reference ramp; `theme.main_text_color(bg)` / `theme.sub_text_color(bg)` /
  `theme.ui_warning_color()` / `theme.ui_error_color()` for the Severity + dimmed mapping;
  theme obtained in the footer via `Appearance::as_ref(app).theme()`.

Exact builder chains (chip `Flex`/`Hoverable` layout, `OffsetPositioning` anchors, the
`spawn_stream_local` call, `async_channel` construction) are pinned into the plan as concrete
code from these files.

## 11. Security & privacy (unchanged, restated)

- The one outbound call remains `GET https://api.anthropic.com/api/oauth/usage` with the
  Keychain OAuth token — the same call Claude Code's own `/usage` makes. No third party, no
  telemetry.
- The token is held in memory only for the request (crate's `ClaudeToken` has a redacting
  `Debug`), never logged, never written to disk. Running the fetch on the producer thread
  does not change this — the token never leaves the crate's fetch path.

## 12. Out of scope (unchanged)

Gemini/opencode agents; a Claude refresh-token flow; historical/charted usage; per-project
drill-down; showing cost. The crate boundaries already leave room for these without
touching the footer.
