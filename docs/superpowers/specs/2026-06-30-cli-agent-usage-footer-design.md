# CLI Agent Usage in the Clinch Footer — Design

**Date:** 2026-06-30
**Status:** Approved design; data-layer crate implemented (Plan A). Footer UI now specified
separately.
**Author:** brainstorming session

> **Superseded (footer UI):** the crate (§4–§5, §12) is built. The **threading** (§5–§6,
> which assumed `async fn refresh` — it is actually blocking and panics inside a Tokio
> runtime), the **cost display** (§8–§9 — cost is now omitted from the UI), and the **panel
> metric** (§9 — now input+output headline with cache-read dimmed) are finalized in
> `2026-06-30-cli-agent-usage-footer-ui.md`, which is authoritative for the footer. This
> document remains the record of the overall feature and the data-source research.

## 1. Goal

Show live Claude Code and Codex usage in the Clinch CLI-agent footer: token totals
across several time windows, an estimated cost, and the **real plan-limit percentages**
(5-hour "session" window and 7-day "weekly" window) for both tools.

Surface it as a **compact chip** in the footer that expands into a **panel** with the
full breakdown, because "session / today / week / month for two tools" is far more text
than a footer bar can hold inline.

This is a no-login fork: the user is already authenticated to Claude Code and Codex on
the machine, so all data is sourced from files those tools already write, plus the same
authenticated endpoint Claude Code's own `/usage` command calls. Clinch adds **no new
login of its own**.

## 2. Non-goals (YAGNI)

- No historical charts, no per-project breakdown, no CSV export. Just current windows.
- No persistence of usage history by Clinch — we read the source-of-truth files each poll.
- No support for other agents (Gemini, opencode) in v1. The crate is structured so they
  can be added later, but they are out of scope now.
- No editing/refreshing of provider OAuth tokens. We read the current token; if Claude's
  is expired we skip that poll and keep the last value (Claude Code refreshes it on next
  use because Clinch launches it).

## 3. Data sources (verified on the dev machine)

### 3.1 Claude — token totals (local, robust)

`~/.claude/projects/<encoded-cwd>/<sessionId>.jsonl`. Each assistant line:

```jsonc
{
  "type": "assistant",
  "requestId": "req_011C...",
  "sessionId": "110d3578-...",
  "timestamp": "2026-06-30T21:16:28.384Z",
  "message": {
    "model": "claude-opus-4-7",
    "usage": {
      "input_tokens": 6,
      "output_tokens": 218,
      "cache_creation_input_tokens": 29086,
      "cache_read_input_tokens": 0
    }
  }
}
```

- `costUSD` is **null** in the transcript → cost is computed by us (see §8).
- One file = one session. Daily/weekly/monthly = aggregate all files by `timestamp`.
- **Dedup:** retries and sidechains can repeat a logical message. Dedup by
  `requestId` + message `id` (mirrors how `ccusage` dedupes) so tokens aren't double-counted.

### 3.2 Claude — real plan-% (Keychain token + endpoint)

- Token: macOS Keychain, generic password, service `Claude Code-credentials`,
  account = the OS user. Read natively via `security-framework` (no shelling out). The
  stored blob is JSON shaped `{ "claudeAiOauth": { "accessToken", "refreshToken",
  "expiresAt", "scopes", "subscriptionType" } }`.
- Endpoint (extracted from the Claude Code 2.1.197 binary):
  **`GET https://api.anthropic.com/api/oauth/usage`**, `Authorization: Bearer <accessToken>`,
  plus the OAuth beta + version headers Claude Code sends.
- **Real response shape (captured live, 2026-06-30):**

```jsonc
{
  "five_hour": { "utilization": 78.0, "resets_at": "2026-07-01T02:30:00.49+00:00",
                 "limit_dollars": null, "used_dollars": null, "remaining_dollars": null },
  "seven_day": { "utilization": 43.0, "resets_at": "2026-07-04T15:00:00.49+00:00", ... },
  "seven_day_opus": null, "seven_day_sonnet": null, /* per-model windows, often null */
  "extra_usage": { "is_enabled": true, "used_credits": 35010.0, "currency": "USD", ... },
  "limits": [
    { "kind": "session",    "group": "session", "percent": 78, "severity": "warning",
      "resets_at": "2026-07-01T02:30:00.49+00:00", "is_active": true },
    { "kind": "weekly_all",  "group": "weekly",  "percent": 43, "severity": "normal",
      "resets_at": "2026-07-04T15:00:00.49+00:00", "is_active": false }
  ],
  "spend": { "used": { "amount_minor": 35010, "currency": "USD", "exponent": 2 }, ... }
}
```

- **Parsing rule:** prefer the normalized `limits[]` array — match `group == "session"`
  (5h) and `group == "weekly"` (7d); use `percent` for the bar and `severity`
  (`normal`/`warning`/`critical`…) directly for the chip color. Fall back to the
  `five_hour.utilization` / `seven_day.utilization` + `resets_at` objects if `limits`
  is absent. `resets_at` is ISO-8601 with offset; parse with `chrono`.
- Working headers (200 OK): `Authorization: Bearer <accessToken>`,
  `anthropic-beta: oauth-2025-04-20`, `anthropic-version: 2023-06-01`.
- Keychain blob top-level keys: `["mcpOAuth", "claudeAiOauth"]`; read
  `claudeAiOauth.accessToken`; `claudeAiOauth.expiresAt` is **epoch milliseconds**.

### 3.3 Codex — token totals + real plan-% (all local)

`~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`. `token_count` events carry both:

```jsonc
{
  "type": "event_msg",
  "payload": {
    "type": "token_count",
    "info": {
      "total_token_usage": {
        "input_tokens": 2348805, "cached_input_tokens": 2214272,
        "output_tokens": 14432, "reasoning_output_tokens": 7418,
        "total_tokens": 2363237
      }
    },
    "rate_limits": {
      "primary":   { "used_percent": 9.0,  "window_minutes": 300,   "resets_at": 1782425344 },
      "secondary": { "used_percent": 18.0, "window_minutes": 10080, "resets_at": 1782421135 },
      "plan_type": "prolite"
    }
  }
}
```

- `total_token_usage` is **cumulative for that session**. The last `token_count` event in a
  file = that session's final total. Sum the last event across files in a date range = the
  period total.
- `rate_limits.primary` = 5h window, `secondary` = 10080 min = weekly. These are the exact
  percentages the Codex TUI shows — no endpoint needed. They refresh whenever Codex runs;
  if Codex hasn't been used recently the % is stale (shown as last-known with its `resets_at`).

### 3.4 Source map

| Metric | Claude | Codex |
|---|---|---|
| session/today/week/month tokens | scan `~/.claude/projects/**/*.jsonl` | scan `~/.codex/sessions/**/rollout-*.jsonl` |
| est. cost | tokens × pricing table | tokens × pricing table |
| 5h + weekly plan-% | `GET /api/oauth/usage` (Keychain token) | newest session file `rate_limits` |

## 4. Architecture

A new **UI-free, unit-tested workspace crate** does all sourcing/aggregation; a thin
singleton model in `/app` polls it on timers and notifies; the footer renders a snapshot.
This keeps file IO + HTTP + pricing out of the 3000-line footer file and testable in
isolation.

```
crates/cli_agent_usage/                 # new, no UI deps
  src/
    lib.rs        # UsageSnapshot, public refresh() entry, Provider/Window types
    claude.rs     # transcript scan + dedup; keychain read; GET /api/oauth/usage
    codex.rs      # rollout scan; latest rate_limits
    pricing.rs    # per-model $/Mtok (Claude + Codex); est. cost
    cache.rs      # path -> (mtime,size) -> parsed totals; only re-parse changed files
    keychain.rs   # security-framework read, behind a trait for tests
    http.rs       # reqwest call, behind a trait for tests

app/src/ai/blocklist/usage/cli_agent_usage_model.rs   # SingletonEntity wrapper + timers
app/src/lib.rs (~1382)                                # register singleton
app/src/ai/blocklist/agent_view/agent_input_footer/
  toolbar_item.rs:48          # new AgentToolbarItemKind::CliAgentUsage variant
  mod.rs render_cli_mode_footer() (~1493)   # render chip
  mod.rs (new) usage panel popover          # render expanded panel
  mod.rs AgentInputFooter::new() (~258)     # subscribe_to_model + ctx.notify()
```

`members = ["crates/*", ...]` already globs the new crate in; add a path alias
`cli_agent_usage = { path = "crates/cli_agent_usage" }` and depend on it from `app`.

## 5. Components

Each unit has one purpose, a narrow interface, and is testable alone.

- **`cache`** — given a directory + glob, returns parsed per-file totals, re-parsing only
  files whose `(mtime, size)` changed since last call. Purpose: avoid re-reading a large
  transcript tree every tick. Depends on: `walkdir`, `glob`, fs.
- **`claude`** — `fn scan(dir) -> ClaudeTotals` (uses `cache`, dedups by `requestId`+id,
  buckets by window); `fn plan_pct(token, http) -> Option<PlanLimits>`. Depends on: `cache`,
  `keychain`, `http`, `serde_json`, `chrono`.
- **`codex`** — `fn scan(dir) -> CodexTotals` + latest `rate_limits`. Depends on: `cache`,
  `serde_json`, `chrono`.
- **`pricing`** — `fn cost(model, tokens) -> f64` over an embedded table. Pure, no IO.
- **`keychain` / `http`** — tiny traits (`ReadSecret`, `FetchUsage`) with real impls
  (`security-framework`, `reqwest`) and test fakes.
- **`lib`** — `UsageSnapshot { claude: Provider, codex: Provider }`,
  `Provider { session, today, week, month: WindowTotals, cost_est, plan: Option<PlanLimits> }`,
  `PlanLimits { five_hour: Pct+reset, seven_day: Pct+reset }`. `async fn refresh(prev) ->
  UsageSnapshot` composes the above; each provider/source is independent and fail-soft.

The `/app` **singleton model** owns two timers (files ~5s, endpoints ~60s), calls
`refresh`, stores the latest `UsageSnapshot`, and emits an event → footer `ctx.notify()`.
Modeled on the existing `AIRequestUsageModel` (`app/src/ai/request_usage_model.rs:184`),
but **not** gated on `is_logged_in()` (that gate is why the existing usage models are inert
in this fork).

## 6. Data flow & refresh cadence

1. Singleton spawns on app start (registered in `app/src/lib.rs` near the other singletons).
2. **File timer (~5s):** re-scan both trees via `cache` (incremental); recompute window
   totals + cost. Cheap because only changed files are re-parsed.
3. **Endpoint timer (~60s):** read Keychain token; if present and unexpired, `GET
   /api/oauth/usage`; update Claude `PlanLimits`. Codex `PlanLimits` come from the file
   scan (no network). On any failure, retain the previous value.
4. On any change, emit an event; the footer subscribes and `ctx.notify()`s to repaint.

## 7. Window semantics

- **session** — the most-recently-modified session file per provider (its current totals).
- **today** — since local midnight (`chrono` local tz).
- **week** — rolling last 7 days (token totals). Plan-% "weekly" uses the *provider's own*
  7-day window, which is authoritative and may not align to our rolling sum — that's fine;
  they're labeled distinctly (tokens vs. limit-%).
- **month** — rolling last 30 days (token totals).

All token windows are computed by bucketing line/event timestamps; plan-% windows come
straight from the providers and are never recomputed by us.

## 8. Pricing / cost

- Embedded `$/Mtok` table per model (input, output, cache-read, cache-write for Claude;
  input, cached-input, output for Codex). Seeded from public pricing; borrow `ccusage`'s
  Claude table as a reference.
- Cost is labeled **"est."** everywhere. For subscription users it represents *equivalent
  API cost*, not money actually billed — stated in the panel so it isn't mistaken for a bill.
- Unknown model id → cost contribution `0` for that slice and a one-time warn log; tokens
  still count.

## 9. UI

- **Chip** (in `render_cli_mode_footer`): the two most decision-relevant numbers, e.g.
  `◷ cc 22%w · cx 18%w` (weekly plan-%). Color ramps neutral→amber→red as % climbs, reusing
  the footer's existing context-usage color logic (`icon_for_context_window_usage`,
  `app/src/ai/blocklist/usage/mod.rs:8`). If a provider has no data, omit its half.
- **Panel** (click/hover popover, same toolkit pattern as existing chips): 2 columns
  (Claude | Codex) × rows — session / today / week / month tokens, est. cost, 5h % + reset,
  weekly % + reset. Missing cells show `—`.

## 10. Error handling (fail-soft, always)

Every source is independent. Missing dir, malformed line, expired/absent token, HTTP error,
or unexpected JSON → that cell renders `—` and the footer **never blocks or panics**. A
shape/version mismatch logs **once** (deduped) so format drift is visible without spamming.
The footer must render correctly when *neither* tool has ever been run.

## 11. Security & privacy (explicit)

- Clinch reads the user's **Claude Code Keychain token** and makes a **network call to
  `api.anthropic.com`** — local-only, the same call Claude Code's `/usage` already makes,
  no third party. This is the one outbound call the feature introduces and is called out
  here as a conscious, approved choice.
- The token is held in memory only for the request, never logged, never written to disk.
- All other data is read from the user's own local files. No telemetry is added.

## 12. Testing

- Unit tests in `cli_agent_usage` against fixture JSONL files (Claude + Codex) covering:
  aggregation across windows, dedup-by-`requestId`, today/week/month boundary edges
  (timezone, midnight), pricing math, malformed-line tolerance, and "no files at all".
- `keychain` and `http` behind traits → tested with fakes; the recorded `/api/oauth/usage`
  fixture (the §3.2 live capture) drives the Claude plan-% parser test.
- A `cache` test proves unchanged files are not re-parsed (mtime/size gate).

## 13. Dependencies (all already in the workspace)

`reqwest` 0.13, `security-framework` (native Keychain), `serde_json`, `chrono`/`time`,
`walkdir`, `glob`, `notify` (available; timer-polling is primary, `notify` optional later).
No new third-party crate is required.

## 14. Risks

- **Format drift:** transcript/rollout schemas and the `/api/oauth/usage` shape are
  internal/undocumented; a CLI update could change them. Mitigated by fail-soft `—`,
  one-time drift logging, and isolating each parser.
- **Token expiry:** if Claude's Keychain token is expired and Claude hasn't been run, plan-%
  shows last-known/`—` until next Claude use refreshes it. Acceptable; no refresh flow in v1.
- **Cost accuracy:** estimate only; pricing table must be maintained.
- **Large transcript trees:** mitigated by incremental `(mtime,size)` caching.

## 15. Out of scope / future

Gemini/opencode agents; a refresh-token flow for Claude; historical/charted usage;
per-project drill-down; making cost a real billed figure. The crate boundaries leave room
for all of these without touching the footer.
