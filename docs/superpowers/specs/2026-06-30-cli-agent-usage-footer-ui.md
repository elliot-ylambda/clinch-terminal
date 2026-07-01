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
  cli_agent_usage_model.rs   # NEW: CliAgentUsageModel singleton
                             #   - owns an mpsc::Receiver<UsageSnapshot>
                             #   - spawns ONE producer std::thread (the blocking work)
                             #   - UI-thread drain timer: try_recv -> store -> cx.notify()
                             #   - holds `latest: UsageSnapshot`
  cli_agent_usage_format.rs  # NEW: pure formatting/label fns + unit tests
                             #   - fmt_tokens(u64) -> "8.3B" / "1.2M" / "947k" / "512"
                             #   - fmt_pct(f64) -> "47%"
                             #   - fmt_reset(Option<DateTime<Utc>>, now) -> "in 3h" / "—"
                             #   - chip_halves(&UsageSnapshot) -> the two rendered halves
                             #       (text + Severity per provider), or a "hidden" signal
                             #       when both providers are empty
                             #   - severity->color mapping helper (theme token per Severity)

app/src/lib.rs (~1382)                       # register CliAgentUsageModel singleton
app/src/ai/blocklist/agent_view/agent_input_footer/
  toolbar_item.rs (~48)                      # + AgentToolbarItemKind::CliAgentUsage
  mod.rs render_cli_mode_footer() (~1493)    # render chip
  mod.rs (new fn)                            # render expanded panel popover
  mod.rs AgentInputFooter::new() (~258)      # observe model + ctx.notify()

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
Tokio/async runtime**. Clinch (a Warp fork) runs gpui atop async machinery, so we must not
assume any gpui executor thread is Tokio-free.

**Robust design: one dedicated `std::thread` producer + an `mpsc` channel + a UI-thread
drain timer.** A raw OS thread is guaranteed to have no Tokio runtime context, so
`reqwest::blocking` cannot panic there. This is deliberately independent of whether gpui's
background executor happens to be Tokio-backed.

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

    if tx.send(snap).is_err() {
        break;                              // Receiver dropped (model gone) => exit cleanly
    }
    tick = tick.wrapping_add(1);
    std::thread::sleep(FILE_POLL);          // ~5s; blocking sleep is fine on a real thread
}
```

- `fetch_claude_plan` = `keychain::read_claude_token` → expiry check → `fetch.fetch` →
  `http::parse_plan_limits`, returning `Option<PlanLimits>` (all already public). It is the
  Claude-plan half of the crate's `refresh()`, called on its own slow cadence.
- **Last-known-good:** `last_plan` is overwritten only when a fetch succeeds; a transient
  429/timeout/expiry leaves the previous value in place, so the chip does not flicker to `—`.
  If Claude was never logged in, `last_plan` stays `None` and plan-% shows `—` (correct).
- **Codex plan-%** needs no special handling — it is local, populated fresh by `scan_local`
  every ~5s.
- **Clean shutdown:** on model `Drop` the `Receiver` drops; the next `tx.send` returns `Err`
  and the loop breaks. No join handle, no shutdown flag needed. (Singletons live for the app
  lifetime, so in practice the thread runs until exit.)

UI side (`CliAgentUsageModel`, on the main thread):

- On construction: `Paths::detect()`; create `mpsc::channel::<UsageSnapshot>()`; spawn the
  producer thread with the `Sender`, `Paths`, a fresh `Caches::new()`, `MacKeychain`, and
  `ReqwestUsage`. Store the `Receiver` and `latest: UsageSnapshot::default()`.
- A recurring UI-thread timer (gpui `Timer::after` re-armed each fire, ~1 s) calls
  `receiver.try_recv()` in a loop; on the newest `Ok(snap)` it sets `latest = snap` and
  `cx.notify()`. `try_recv` never blocks the UI. (Exact timer idiom taken from an existing
  model in the codebase during planning.)

Cadence constants (module consts, easily tuned): `FILE_POLL = 5s`, `ENDPOINT_EVERY = 12`
(⇒ ~60 s), UI drain `~1s`.

## 5. Chip (in `render_cli_mode_footer`)

- Content: a small gauge/clock icon + two halves — `cc {weekly}%w · cx {weekly}%w` — where
  each `%` is `PlanLimits.weekly.percent` rounded to an integer.
- Color: each half is colored by its own `weekly.severity` (`Normal` → muted/neutral text,
  `Warning` → amber, `Critical` → red), using the same theme tokens the footer's existing
  `icon_for_context_window_usage` (`app/src/ai/blocklist/usage/mod.rs:8`) uses at its
  warning/critical thresholds. We map the crate `Severity` **enum** (authoritative — Claude's
  comes straight from the endpoint's `severity`, Codex's from `severity_from_percent`) rather
  than re-deriving a color from the percent.
- Missing data: a provider with no `weekly` limit shows `— ` for its half (stable width, no
  layout jitter). The **whole chip is hidden** only when *both* providers are entirely empty
  (no token windows and no plan) — i.e. neither tool has ever run.
- **Click** toggles the panel popover (not hover — a detailed panel on hover is janky).

## 6. Panel (popover)

Two columns, **Claude Code | Codex**, using the same popover/overlay pattern an existing
footer chip uses (identified during planning). Rows top-to-bottom:

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

- `app/src/ai/blocklist/agent_view/agent_input_footer/toolbar_item.rs:48` —
  `AgentToolbarItemKind`; add `CliAgentUsage` and handle it everywhere the enum is matched.
- `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs:1493` —
  `render_cli_mode_footer()` (chip) and a new panel-popover fn.
- `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs:258` —
  `AgentInputFooter::new()` (observe the model, `ctx.notify()` on change).
- `app/src/lib.rs:1382` — register the `CliAgentUsageModel` singleton (mirror the
  `AIRequestUsageModel` registration at `app/src/ai/request_usage_model.rs:184`, **without**
  the `is_logged_in()` gate that makes the stock usage models inert in this fork).
- `app/src/ai/blocklist/usage/mod.rs:8` — `icon_for_context_window_usage` color source to
  reuse for the `Severity` → color mapping.

The exact warpui builder chains (chip layout, popover/overlay creation, the timer idiom,
the singleton-entity registration call, and the theme color-token accessors) are pulled
from these files during plan-writing and pinned into the plan as concrete code.

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
