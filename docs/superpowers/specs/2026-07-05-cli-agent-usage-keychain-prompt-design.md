# Stop the recurring "Allow Clinch to read Claude Code credentials" prompt

**Date:** 2026-07-05
**Branch:** `cli-agent-usage-keychain-prompt`
**Status:** Design → implementation

## Problem

Every ~60 seconds, macOS pops up **"Clinch wants to use your confidential
information stored in 'Claude Code-credentials' in your keychain"** and demands
the login-keychain (computer) password. It is relentless and interrupts work.

## Root cause

The CLI-agent usage widget (Claude Code + Codex status in the tab bar) fetches
Claude's *live plan-limit* gauges (the 5-hour and weekly rate-limit %) from
`https://api.anthropic.com/api/oauth/usage`. To authorize that call it needs
Claude Code's OAuth access token, which on macOS lives **only** in the login
Keychain under service `Claude Code-credentials` (verified: there is no
`~/.claude/.credentials.json` on macOS).

Two facts combine to make this a recurring prompt:

1. **Cross-app Keychain ACL.** The `Claude Code-credentials` item was created by
   Claude Code, so its access-control list trusts Claude Code — not Clinch. Any
   read by Clinch triggers the OS authorization prompt.
   (`crates/cli_agent_usage/src/keychain.rs:73`,
   `security_framework::passwords::get_generic_password`.)

2. **The read is on a tight loop and "Always Allow" never sticks.** The producer
   thread reads the Keychain **every `ENDPOINT_EVERY` (12) ticks × `FILE_POLL`
   (5 s) = ~60 s**, forever
   (`app/src/ai/blocklist/usage/cli_agent_usage_model.rs:90`). And because the
   `--selfsign` build signs with `script/Debug-Entitlements.plist`, which sets
   `com.apple.security.get-task-allow=true` (the *debuggable* flag), macOS
   refuses to persist a durable "Always Allow" grant for the app. So even
   clicking "Always Allow" would not silence it, and the every-minute cadence
   re-asks constantly.

Crucially, **the Keychain token powers only the live plan-% gauges.** Every
other number in the widget (token counts, $ cost, per-window totals, per-session
model) is derived from scanning `~/.claude/projects/**/*.jsonl` and
`~/.codex/sessions/**` locally, with zero Keychain access
(`crates/cli_agent_usage/src/lib.rs` `scan_local`).

## Goals / non-goals

**Goals**
- Reduce the prompt from "every 60 s" to **at most once per app launch** (and
  again only when the cached token expires), by caching the token in memory.
- Give the user an **off switch** that eliminates the Keychain read (and the
  prompt) entirely, at the cost of the live plan-% gauges. Default **on** so
  existing behavior is preserved, just far quieter.
- Follow repo convention for a toggleable setting: Settings UI row **and** a
  Command Palette Enable/Disable pair gated by a context flag
  (`CLAUDE.md`/`WARP.md`).

**Non-goals**
- Persisting the token across app restarts or refreshing it ourselves via the
  OAuth refresh token (the user explicitly chose in-memory "ask once", not
  cross-restart persistence). One prompt per launch is acceptable.
- Changing the app's signing/entitlements. `get-task-allow` stays; we stop
  relying on a durable Keychain grant instead of trying to earn one.
- Touching the Codex path (local-file only; never hits the Keychain).

## Design

### Part 1 — In-memory token cache (the "ask once on startup" fix)

Split the Keychain read from the HTTP fetch so the producer can cache the token
and only re-read the Keychain when it has no valid (unexpired) token.

**`crates/cli_agent_usage/src/lib.rs`**
- Add `fetch_plan_for_token(fetch: &dyn FetchUsage, token: &ClaudeToken, now_ms: i64) -> Option<PlanLimits>`
  — the HTTP-only half: expiry guard → `fetch.fetch(access_token)` →
  `http::parse_plan_limits`.
- Re-implement the existing `fetch_claude_plan` as a thin wrapper over
  `read_claude_token` + `fetch_plan_for_token` (no behavior change, keeps its
  test coverage, avoids duplication). It remains the uncached convenience API.
- Re-export `ClaudeToken` at the crate root for the producer.

**`app/src/ai/blocklist/usage/cli_agent_usage_model.rs` — `producer_loop`**
- Hold `let mut cached_token: Option<ClaudeToken> = None;` and
  `let mut last_read_ms: Option<i64> = None;`
- On each endpoint tick, obtain a token via a small local step:
  - If `cached_token` is `Some(t)` and `!t.is_expired(now_ms)` → reuse it (no
    Keychain read, no prompt).
  - Else (missing/expired), read the Keychain **only if** we've never read or
    `now_ms - last_read_ms >= REREAD_BACKOFF_MS` — then `read_claude_token`
    (the one read that can prompt), set `last_read_ms = now_ms`, store into
    `cached_token`.
- With a valid token in hand, call `fetch_plan_for_token(&fetch, &token,
  now_ms)`; keep the last-good `PlanLimits` on transient HTTP failure
  (unchanged).

**Why the backoff.** Without it, a *stored token that is itself expired*
(Claude Code hasn't refreshed lately) would satisfy "missing/expired" on every
endpoint tick, re-reading — and re-prompting — every 60 s, reintroducing the
exact bug. `REREAD_BACKOFF_MS` (≈5 min) caps re-reads in that pathological case
to once per 5 min. In the normal case the first read returns a token valid for
hours, so there is exactly one read per launch until it expires.

Result: the Keychain is read once at launch (tick 0), then only after the
access token's `expiresAt` passes (and at most once per `REREAD_BACKOFF_MS`
while a fresh valid token is unavailable) — i.e. "ask once on startup," matching
the user's choice.

### Part 2 — Off switch (setting + Command Palette + context flag)

A new machine-local boolean setting, default on. When off, the producer never
reads the Keychain and never fetches the plan endpoint; the plan-% gauges simply
don't render (local token/cost stats are unaffected).

**New setting — `app/src/settings/cli_agent_usage.rs`** (new file, one-field
`define_settings_group!`, modeled on `app/src/settings/code.rs`):
```rust
define_settings_group!(CliAgentUsageSettings, settings: [
    show_plan_limits: ShowCliAgentPlanLimits {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::MAC, // Keychain read is macOS-only
        sync_to_cloud: SyncToCloud::Never,            // machine-local (per-Keychain)
        private: false,
        toml_path: "ai.cli_agent_usage.show_plan_limits",
        description: "Show Claude Code's live plan-limit gauges in the usage \
                      widget. Reads the 'Claude Code-credentials' item from your \
                      macOS Keychain, which prompts for your password once per \
                      launch. Turn off to stop the prompt (local token/cost \
                      stats are unaffected).",
    }
]);
```
- Register it in `app/src/settings/init.rs` (`CliAgentUsageSettings::register(ctx);`).
- Declare the module in `app/src/settings/mod.rs`.
- If `SupportedPlatforms::MAC` does not exist, fall back to `ALL` (the read is a
  no-op off macOS regardless) and gate the Command Palette pair with
  `.is_supported_on_current_platform(...)`.

**Off-thread bridge — `cli_agent_usage_model.rs`**
Settings are main-thread-only (`as_ref`/`handle` require `AppContext`), but the
producer runs on a bare `std::thread`. Bridge with an `Arc<AtomicBool>`, the
same lock-free pattern `FeatureFlag` uses:
- In `CliAgentUsageModel::new(ctx)`: read the initial value
  `CliAgentUsageSettings::as_ref(ctx).show_plan_limits.value()`, store into
  `Arc<AtomicBool>`, and `move` a clone into `producer_loop`.
- Keep it live: `ctx.subscribe_to_model(&CliAgentUsageSettings::handle(ctx), …)`
  (groups emit `CliAgentUsageSettingsChangedEvent`); on change, `store` the new
  value into the Arc.
- `producer_loop` checks `enabled.load(Relaxed)` before the token/HTTP step.
  When disabled it resets `cached_token = None` and `last_plan = None` and sets
  `snap.claude.plan = None`, so the gauges disappear immediately and re-enabling
  forces a fresh read (one prompt).

**Command Palette Enable/Disable pair + context flag**
- Add context flag `SHOW_CLI_AGENT_PLAN_LIMITS_CONTEXT_FLAG: &str` to the `flags`
  module in `app/src/settings_view/mod.rs`.
- Add `FeaturesPageAction::ToggleCliAgentPlanLimits` variant + handler (handler
  calls `toggle_and_save_value`), modeled on `ToggleMouseReporting`.
- Register a `ToggleSettingActionPair::new("Claude Code live plan limits", …,
  context, flags::SHOW_CLI_AGENT_PLAN_LIMITS_CONTEXT_FLAG)` in
  `features_page.rs::init_actions_from_parent_view`, guarded with
  `.is_supported_on_current_platform(...)`.
- Insert the flag into the Workspace `keymap_context` when the setting is on
  (`app/src/workspace/view.rs`, modeled on `mouse_reporting_enabled`), so the
  palette shows Disable when on / Enable when off.

**Settings UI row**
- Add a switch on the Features page (`features_page.rs` render) dispatching
  `FeaturesPageAction::ToggleCliAgentPlanLimits`, with
  `LocalOnlyIconState::for_setting(...)` to mark it machine-local (it is
  `SyncToCloud::Never`).

## Data flow (after)

```
launch ─▶ producer tick 0 ─▶ enabled? ──no──▶ plan = None (no Keychain, no prompt)
                                │yes
                                ▼
                      cached_token valid? ──yes──▶ fetch_plan_for_token ─▶ gauges
                                │no
                                ▼
                 read_claude_token  ← the ONE prompt (per launch / per expiry)
                                ▼
                       cache it, then fetch_plan_for_token ─▶ gauges
```

## Testing

- **`cli_agent_usage` unit tests** (pure, no Keychain/network via the existing
  `ReadSecret`/`FetchUsage` fakes):
  - `fetch_plan_for_token`: valid token → plan; expired token → `None`; fetch
    error / garbage body → `None`.
  - Cache behavior: a counting `ReadSecret` fake proves the Keychain is read
    once while a token stays unexpired, and re-read after expiry. (Assert the
    read counter, since that read is exactly what triggers the OS prompt.)
  - `fetch_claude_plan` wrapper: existing tests stay green unchanged.
- **Settings**: default is `true`; `toml_path` round-trips; `sync_to_cloud ==
  Never`.
- **Manual (macOS)**: launch → exactly one prompt; confirm no repeat for
  several minutes; toggle off via Settings and via Command Palette (Enable/
  Disable label flips) → gauges vanish, no further prompts; toggle on → one
  prompt, gauges return.
- `./script/format` + `cargo clippy` (presubmit versions) must pass.

## Dead-code / cleanup

- No dead code introduced. `fetch_claude_plan` is retained but reimplemented as
  a wrapper over the two new pieces (still used by its unit tests), so there is
  no duplicated fetch logic.
- The producer stops calling `fetch_claude_plan` in favor of the cache-aware
  path (`read_claude_token` + `fetch_plan_for_token`); no symbol is orphaned.

## Risks / mitigations

- **Access-token lifetime governs prompt frequency.** If Claude Code's access
  token is short-lived, "once per launch" degrades toward "a few per day." Still
  a >50× reduction from every-60 s, and the off switch is the hard escape.
- **Setting-change latency.** The atomic updates on the main thread via the
  group's change event; the producer observes it on its next tick (≤5 s). No
  UI-thread Keychain/HTTP work is introduced (all stays on the dedicated
  thread).
- **`get-task-allow` remains**, so we deliberately do *not* depend on a durable
  Keychain grant. The cache + off switch are the mitigation, not entitlement
  surgery.
