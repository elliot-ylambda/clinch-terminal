# Agent attention badges + notifications — design

- **Date:** 2026-06-30
- **Status:** Approved (pending implementation plan)
- **Scope:** Clinch (the local-only, skip_login OSS build of this Warp fork), macOS
- **Related:** `tools/agent-resume/` (capture/replay hooks), `docs/superpowers/specs/2026-06-20-warp-agent-session-resume-design.md`

## Summary

When Claude Code or Codex running in a Clinch pane **finishes a turn** or **blocks asking
for input**, surface that on the tab and (for "asking") push a macOS notification — with
settings to control it.

The decisive finding from codebase exploration: **the consuming pipeline already exists and
is compiled into the Clinch build.** A per-pane CLI-agent state machine, a status badge over
the agent's brand logo (sidebar tabs + pane header), an in-app notification mailbox/toast,
and a desktop-notification-on-status-change all ship today and are **not** gated behind login.

They have been dark only because **nothing emits the events that drive them.** Those events
are OSC-777 escape sequences (`warp://cli-agent`) normally produced by a Warp-authored
*marketplace plugin* installed into the agent — which a vanilla Clinch install never installs.

Therefore this feature is mostly **enable + one new render surface**, not build-from-scratch:

1. Install the emitter plugins from `agent-resume/install.sh`.
2. Enable the `codex_plugin` build feature so Codex's rich (Blocked/asking) events aren't dropped.
3. Add a CLI-agent status indicator to the **horizontal tab strip** (the only surface with no
   existing path).
4. Relax the desktop-notification focus gate so an agent asking on a *background tab* still pushes.
5. Add one settings toggle; reuse the three that already exist.

## Goals

- A Claude/Codex pane shows **done** (✓) and **asking/needs-you** (❗) status on **all three**
  surfaces: horizontal tab strip, sidebar tabs, pane header. ("Working" spinner already exists
  via `InProgress`.)
- When an agent **asks a question / requests permission**, push a macOS notification — for
  **both** Claude and Codex — whenever the asking pane is not the one the user is looking at.
- Granular settings: show-status-on-tabs, notify-on-done, push-on-asking, sound.
- No regression for the existing agent-resume feature; idempotent install.

## Non-goals (explicit scope guard)

- **Not** writing our own OSC-777 emitter hooks. Decision: use Warp's marketplace plugins.
- **Not** relaxing the `is_any_ai_enabled` AI gate to surface the in-app "Enable notifications"
  chip. We install via the script instead (less invasive, fits the existing agent-resume flow).
- **Not** supporting agents other than Claude + Codex (no Gemini/Amp/etc.).
- **Not** per-agent settings — toggles are global.
- **Not** removing the Codex OSC-9 fallback — it is retained as the no-plugin coverage path.

## Background: how the live pipeline works

Emit → consume → render/notify, with exact anchors:

```
agent + Warp marketplace plugin (claude-code-warp / codex-warp)
  └─ plugin hook prints OSC-777:  ESC ] 777 ; notify ; warp://cli-agent ; <json> BEL   (to the pty)
       └─ ANSI parser  app/src/terminal/model/ansi/mod.rs:996 (b"777" => …)
            └─ gated by FeatureFlag::PluggableNotifications (ON)  terminal_model.rs:3378
                 └─ TerminalView ModelEvent::PluggableNotification { title, body }  view.rs:12670
                      └─ handle_cli_agent_notification(title, body)  view.rs:13050
                           ├─ [Codex only] dropped unless FeatureFlag::CodexPlugin  view.rs:13060-13066
                           ├─ parse_event (v1)  cli_agent_sessions/event/v1.rs
                           ├─ register_cli_agent_listener_from_event  view.rs:13087
                           └─ CLIAgentSessionsModel::update_from_event → emits StatusChanged
                                ├─ (render) terminal_view_agent_icon_variant
                                │            → IconWithStatusVariant::CLIAgent { agent, status }
                                │            → sidebar (vertical_tabs.rs), pane header (pane_impl.rs:298)
                                ├─ (desktop) handle_cli_agent_sessions_event  view.rs:13219
                                │            → send_agent_desktop_notification_or_show_banner  view.rs:15846
                                │            → Workspace SendNotification  workspace/view.rs:15589
                                │            → ctx.send_desktop_notification(UserNotification…)
                                └─ (in-app) AgentNotificationsModel  agent_management_model.rs:125
                                             → mailbox/toast + per-tab unread dot
```

State machine — `CLIAgentSession::apply_event` (`cli_agent_sessions/mod.rs:177`):

| Event (`event` JSON field) | Status | Meaning / surface |
|---|---|---|
| `prompt_submit`, `tool_complete` | `InProgress` | working — spinner (exists today) |
| `stop` | `Success` | **done** — ✓ badge |
| `permission_request`, `question_asked` | `Blocked { message }` | **asking** — ❗ badge + push |
| `permission_replied` | `InProgress` | back to working |
| `idle_prompt` | (no change) | — |

Because our events arrive as `CLIAgentEventSource::RichPlugin`, they set `received_rich_notification`
and so `supports_rich_status()` becomes true — Claude gets the full Blocked/asking state (the
Codex OSC-9 fallback alone only yields coarse `Stop`→`Success`, never Blocked).

### Wire format (OSC-777 reference)

- Framing (urxvt/foot style), confirmed by `ansi/mod_tests.rs:952`:
  `\x1b]777;notify;<title>;<body>\x07` — title/body split on the first two `;`, body may contain `;`.
- Title (sentinel): `warp://cli-agent` (`event/mod.rs:12`, `CLI_AGENT_NOTIFICATION_SENTINEL`).
- Body (JSON, parsed by `event/v1.rs`):
  ```json
  { "v": 1, "agent": "claude", "event": "stop", "session_id": "…", "cwd": "…" }
  ```
  `agent` must equal `CLIAgent::command_prefix()` — exactly `"claude"` / `"codex"` (`cli_agent.rs:152`).
  The Warp plugins emit this; we don't author it. Listed here only so the consumer path is documented.
- Gate: the consumer requires `FeatureFlag::PluggableNotifications` (ON in this build).

## Component 1 — Install the emitter plugins (tooling)

**Where:** `tools/agent-resume/install.sh` (+ README, + a test in `tools/agent-resume/tests/`).

**What:** add an idempotent install step that runs the agents' own plugin CLIs. Commands taken
from the in-app installer (`plugin_manager/claude.rs:98`, `plugin_manager/codex.rs:153`):

- Claude:
  ```
  claude plugin marketplace add warpdotdev/claude-code-warp
  claude plugin install warp@claude-code-warp
  ```
- Codex:
  ```
  codex plugin marketplace add warpdotdev/codex-warp
  codex plugin add warp@codex-warp
  ```
  (Constants from `plugin_manager/codex.rs:18-20`: `MARKETPLACE_REPO=warpdotdev/codex-warp`,
  `PLUGIN_KEY=warp@codex-warp`. We install **only** the notification plugin `warp@codex-warp`,
  **not** the `orchestration@codex-warp` platform plugin — that one is for the cloud
  orchestration harness and is irrelevant to status/notifications.)

**Constraints:**
- Idempotent / safe to re-run (the install commands are themselves idempotent; re-adding a
  marketplace + reinstall is harmless).
- Guard on tool presence: skip Claude block if `command -v claude` fails; same for `codex`.
- Tolerate offline / failure: warn and continue (never abort install.sh — agent-resume must
  still install). The feature simply stays dormant until the plugin is present.
- Print a clear note that the agent must be **restarted** for the plugin to load (consistent
  with the existing "restart your shell" guidance).
- This path is intentionally independent of `is_any_ai_enabled`: the in-app auto-install and
  the "Enable notifications" footer chip are gated behind it (false when logged out), but the
  **consumer** of the plugin's OSC-777 events is not, so an out-of-band install works fully.

**Why not the in-app chip:** every `install()` caller is behind `is_any_ai_enabled`
(`settings/ai.rs:1657`), which is false in skip_login (no credentials), so the chip never
renders. Relaxing that gate touches a broad AI surface; the script is the smaller change.

## Component 2 — Enable structured Codex events (Rust, 1 line)

**Where:** `app/Cargo.toml`, the `default` feature aggregate (around line 502–691).

**What:** add `"codex_plugin"` next to the existing `"codex_notifications"`.

**Why:** the consumer drops Codex's structured OSC-777 unless `FeatureFlag::CodexPlugin` is on
(`view.rs:13060-13066`; listener `listener/mod.rs:136`). Without it, Codex can only reach
`Success` via OSC-9 — never `Blocked` — so "push when **Codex** asks a question" is impossible.
The compile-time bridge already exists (`features.rs:486` `#[cfg(feature = "codex_plugin")]`)
and the cargo feature is empty (`codex_plugin = []`, `Cargo.toml:1010`) — no new deps, no other
code paths beyond the Codex plugin-manager methods that are already correctly flagged.

`GeminiNotifications` stays OFF (unrelated; only the Gemini plugin-manager arm reads it).

## Component 3 — Horizontal tab strip indicator (Rust, net-new)

**Where:** `app/src/tab.rs` (the horizontal tab bar). Today its `enum Indicator`
(`tab.rs:749`) has no CLI-agent variant and `agent_indicator()` (`tab.rs:1019`) sources only
Warp's own Oz conversation — there is no per-pane CLI status on this surface.

**What:**
- Add a CLI-agent indicator (e.g. `Indicator::CLIAgent { agent, status }`) computed from
  `CLIAgentSessionsModel` for the tab's pane(s).
- Render it with the shared `render_icon_with_status` / `IconWithStatusVariant::CLIAgent` so it
  is visually identical to the sidebar/pane-header badge (brand circle + status overlay).
- **Per-tab aggregation** (a tab = a pane_group that may hold several terminal panes / splits):
  pick the most attention-worthy CLI-agent session among the tab's panes by priority
  **`Blocked` > `InProgress` > `Success`**; show that agent's brand + status. If a tab has no
  CLI-agent session, fall back to the existing indicator behavior unchanged.
- **Precedence vs. existing indicators:** when a CLI-agent session is present in the tab, the
  CLI-agent indicator takes precedence over the plain `Shell` indicator. (Decision: the user's
  active CLI agent is the most relevant thing to surface. Oz/`Agent` co-occurrence is rare in
  the local build; treat CLI-agent as winning when present.)
- Gated by the new "show agent status on tabs" setting (Component 5); when off, behavior is
  exactly as today.

**Reuse, don't fork:** the variant derivation already lives in
`ui_components/agent_icon.rs::terminal_view_agent_icon_variant`; expose/reuse it per pane and
aggregate, rather than recomputing brand/status logic in `tab.rs`.

## Component 4 — Smarter push timing (Rust, focus gate)

**Where:** the CLI-agent notification path in `app/src/terminal/view.rs`
(`handle_cli_agent_sessions_event`, view.rs:13219/13324-13355).

**What:** replace the window-level suppression `is_navigated_away_from_window`
(`view.rs:21055`, `Some(ctx.window_id()) != active_window`) — *for the CLI-agent path only* —
with a pane-level test:

```
active_focused_terminal_id(ctx) != Some(self.view_id)   // "I am not the focused pane"
```

`active_focused_terminal_id` already exists privately in
`agent_management_model.rs:590` (active window → its `Workspace` →
`Workspace::active_terminal_id`, `workspace/view.rs:7678`). Promote it to a shared/`pub`
helper (or add a thin equivalent on `TerminalView`) built on the public primitives
`Workspace::active_terminal_id` + `AppContext::windows().active_window()`.

**Result:** an agent that finishes or asks on a background tab/pane notifies even while Clinch
is focused on another tab; the pane the user is actively viewing stays quiet. Applies to both
"done" and "asking" CLI-agent notifications. Each remains gated by its settings toggle (below).

**Leave untouched:** `is_navigated_away_from_window` continues to serve the non-CLI paths
(long-running command, Oz agent-mode, password-prompt). No dead code.

## Component 5 — Settings (Rust, mostly reuse)

**Already present** (backing field in `NotificationsSettings`, `session_settings.rs:70`, all
default `true`; UI rows in `settings_view/features_page.rs:322-393`) — verify they take effect
in the logged-out build, no new work expected:

| User-facing toggle | Backing field | Drives |
|---|---|---|
| "agent task completion notifications" | `is_agent_task_completed_enabled` | desktop notify on **done** |
| "needs-attention notifications" | `is_needs_attention_enabled` | push on **asking** |
| "notification sounds" (macOS) | `play_notification_sound` | sound |

**New** — one toggle, "Show agent status on tabs":
- Add a `bool` field to `NotificationsSettings` (e.g. `show_agent_status_on_tabs`), under the
  existing `#[serde(default)]` so old configs stay backward-compatible; default `true`.
- Add a `FeaturesPageAction::ToggleAgentStatusOnTabs` + handler + a `ToggleSettingActionPair`
  row + context flag, mirroring the existing notification rows in `features_page.rs`.
- Read it where the badge renders (Component 3, and optionally the sidebar/pane-header variant
  derivation) so turning it off hides the CLI-agent status everywhere.

## Data ownership / boundaries

- **Emitter:** the Warp marketplace plugin (external, in `~/.claude` / `~/.codex`). We only
  *install* it; we do not author or maintain the wire format.
- **Transport:** OSC-777 over the pty → existing ANSI parser. Unchanged.
- **State:** `CLIAgentSessionsModel` (singleton, keyed by terminal-view `EntityId`). Unchanged.
- **Render:** `IconWithStatusVariant::CLIAgent` shared component — reused on a new surface
  (`tab.rs`), no duplicate rendering logic.
- **Notify:** existing desktop-notification path with a narrower focus gate.
- **Settings:** `NotificationsSettings` + `features_page.rs` — one field/row added.

Each unit keeps its current responsibility; the change adds one render surface, one feature
flag, one settings field, one gate swap, and one install step.

## Edge cases & error handling

- **Plugin not installed / offline at install time:** install.sh warns and continues; feature
  stays dormant; zero regression. Existing agent-resume still installs.
- **Plugin installed but agent not restarted:** no events until restart; documented in install
  output.
- **Codex without `codex_plugin` enabled (e.g. a stale build):** Codex falls back to OSC-9
  (done-only). With the flag on + plugin installed, structured OSC-777 supersedes OSC-9
  (`plugin_already_active`).
- **Multiple agents in one tab (splits):** aggregation rule `Blocked > InProgress > Success`.
- **Agent exits:** `Ended` clears the session → badge clears. (Note: the in-app mailbox unread
  dot clears on focus per existing behavior; the status badge tracks live session state.)
- **"Done" badge persistence:** `Success` persists until the next `prompt_submit` (→InProgress)
  or `Ended`. Intended — it tells you the pane finished while you were away.
- **skip_login:** consumer, desktop notifications, and mailbox all function without login;
  only the (bypassed) in-app install chip is AI-gated.
- **Double-emit safety:** the state machine is idempotent, so coexisting with any other emitter
  is harmless.

## Testing

- **Shell:** extend `tools/agent-resume/tests/` — install.sh runs the plugin commands when
  `claude`/`codex` exist, skips cleanly when absent, and never aborts on plugin failure;
  idempotent re-run.
- **Rust unit:** `tab.rs` aggregation (Blocked > InProgress > Success; CLI-agent precedence
  over Shell; no CLI session → unchanged). Existing `cli_agent` detect tests stay green.
- **Rust unit:** the new focus-gate helper (`active_focused_terminal_id != self` ⇒ notify).
- **Manual (build-app.sh):** install plugins, restart agent, run `claude` and `codex` in a
  background tab; observe spinner→✓ on done and ❗ + macOS push on a permission prompt, on all
  three surfaces; toggle each setting and confirm effect; confirm the focused pane stays quiet.

## Risks / open trade-offs

- **External dependency vs. Clinch's local-only ethos:** the emitter is Warp-authored code
  downloaded from GitHub into the agents. Accepted by the user; flagged here. (The alternative
  — our own emitter hooks — was considered and declined.)
- **Plugin protocol drift:** if Warp changes the plugin's payload, the versioned parser
  (`event/mod.rs` `VERSIONED_PARSERS`) logs and ignores unknown versions — degrade, not crash.
- **Horizontal-tab precedence** (CLI-agent over Oz/Shell) is a UX judgment; revisit if it hides
  something users expect.

## Decision log

- Push applies to **both** Claude and Codex on "asking". (User.)
- Emitter = **Warp marketplace plugins**, installed via `install.sh`, not our own hooks. (User.)
- Push fires when the asking pane **is not the active/focused pane** (not merely
  "window unfocused"). (User.)
- Settings = **granular**, reusing the three existing toggles + one new "show status on tabs". (User.)
- Badge surfaces = **all three**: horizontal tab strip (net-new) + sidebar + pane header. (User.)
- Enabling `codex_plugin` is **required** for Codex's asking/Blocked state and is part of scope.

## No-dead-code audit

- No code is removed; nothing becomes unreachable. `is_navigated_away_from_window` stays for
  non-CLI paths; the Codex OSC-9 fallback stays for the no-plugin case; the in-app install chip
  and AI gate are untouched. The only additions are one feature in `default`, one settings
  field + row, one tab indicator variant, one focus-gate helper, and install-script lines.
