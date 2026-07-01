# Fork & Compact buttons for CLI agents — design

- **Date:** 2026-06-30
- **Status:** Design — pending review
- **Scope:** Clinch (the local-only, skip_login OSS build of this Warp fork), macOS. Agents: Claude Code + Codex.
- **Related:**
  - `docs/superpowers/specs/2026-06-20-warp-agent-session-resume-design.md` (the registry + replay machinery this reuses)
  - `docs/superpowers/specs/2026-06-30-agent-attention-badges-design.md` (per-pane CLI-agent detection)
  - `~/.claude/commands/split.md` + `split-launch.sh` (the AppleScript prototype this supersedes for in-app use; **kept**, not removed)

## Summary

Add two buttons to the per-pane CLI-agent footer that Clinch already shows under a pane running
Claude Code or Codex:

- **Fork** — opens a **new tab** whose agent is a fork of *this* pane's session, leaving the
  current session running untouched. Claude → `claude --resume <id> --fork-session`;
  Codex → `codex fork <id>`.
- **Compact** — types `/compact` into the *live* agent in this pane (both agents support it).

The decisive finding: **the machinery already exists and is compiled into Clinch.** The
agent-resume feature gives us per-pane agent detection and a registry of each pane's session
identity; the footer already has a "write to the agent's PTY" path; and `root_view.rs` already
has a native "open a new tab, run a command, bootstrap it" action. This feature is mostly
**wire two buttons to existing primitives + add one registry field**, not build-from-scratch.

This replaces the user's `/split` AppleScript hack (Cmd+T keystroke automation requiring macOS
Accessibility permission and timing `delay`s) with a native in-app button — though `/split`
itself is retained as a typed alias (user decision).

## Goals

- A pane running Claude or Codex shows **Fork** and **Compact** buttons in its CLI-agent footer.
- **Fork** branches the current session into a new tab in the same `cwd`; the original session
  keeps running, untouched. Works for **both** Claude and Codex.
- **Compact** sends `/compact` to the live agent in **this pane** (the footer's own pane, scoped
  by `WriteToPty`). Works for **both** agents.
- Buttons appear **only** when the pane has a detected CLI agent; otherwise absent.
- Fork command derivation lives entirely in Rust: `agent_resume::derive_fork_command` transforms
  the stored resume `command` (the same string the capture scripts already write) into a fork
  command via two hardcoded per-agent rules. No capture-script changes were needed.

## Non-goals (scope guard)

- **Not** removing the `/split` slash command or `split-launch.sh` (user chose to keep them).
- **Not** a global window-level status bar — buttons are **per-pane**, hosted in the existing
  CLI-agent footer (user-selected host). Per-pane removes "which pane?" ambiguity and, for the
  common single-pane window, looks identical to a global bottom bar.
- **Not** supporting agents other than Claude + Codex (no Gemini/Amp/etc.). Adding one later
  requires a small Rust change (a new rule in `derive_fork_command`), not a capture-script change —
  the registry itself stays agent-agnostic, but the fork-command *derivation* is not.
- **Not** preserving in-flight (un-flushed) conversation turns when forking — `--fork-session` /
  `codex fork` read the **on-disk** transcript, so the most recent turn may lag. Inherent; same
  caveat the `/split` script already documents.
- **Not** a confirmation dialog for Compact (user chose "Claude + Codex", no confirm).

## Background: the live machinery (with anchors)

```
detect agent ──────────────► CLIAgentSessionsModel::session(terminal_view_id) -> Option<CLIAgent>
                              (app/src/ai/.../cli_agent_sessions/, CLIAgent = claude | codex)

pane session identity ─────► ~/.warp/agent-resume/<pane_uuid_hex>.json  (one file per pane)
                              read by app/src/agent_resume.rs::read_on_restore_command(uuid)
                              written by tools/agent-resume/{claude.zsh, codex hook}

CLI-agent footer (host) ───► app/src/terminal/view/use_agent_footer/mod.rs
                              UseAgentToolbar renders AgentInputFooter when cli_agent(app).is_some()
                              (mod.rs:1321); chips emit AgentInputFooterEvent -> UseAgentToolbarEvent

write to live agent PTY ───► UseAgentToolbarEvent::WriteToPty(String)  (mod.rs:1165, 1274)
                              already wired from footer chip -> agent stdin

new tab + run command ─────► root_view.rs:1134
                              open_new_tab_insert_subshell_command_and_bootstrap_if_supported(
                                arg: &SubshellCommandArg, ctx)
                              -> workspace.add_terminal_tab(..) + insert command + bootstrap
```

Fork commands (verified against the installed CLIs, codex-cli 0.142.2):

| Agent | Resume (stored today) | Fork (new) |
|---|---|---|
| Claude | `claude --resume <id>` | `claude --resume <id> --fork-session` |
| Codex  | `codex resume <id>`    | `codex fork <id>` (first-class subcommand) |

## Architecture

Four small units, each with one responsibility:

```
 capture (user-space)          read+derive (Rust)          act (Rust)                 UI (Rust)
 ┌───────────────────┐   ┌───────────────────────┐   ┌──────────────────────┐   ┌──────────────────┐
 claude.zsh / codex   ─► agent_resume::           ─► Fork: open_new_tab_…    ◄─ Fork button  ─┐
 hook write             read_fork_launch(uuid)        (SubshellCommandArg)       Compact button │
 command + cwd into       (derive_fork_command)      Compact: WriteToPty         in CLI-agent   │
 registry json                                        ("/compact\r")             footer ────────┘
```

### Component 1 — Fork command derivation (`agent_resume.rs`, Rust-only)

**Capture side** (`tools/agent-resume/`): unchanged by this feature. The scripts already write the
resume `command` (e.g. `claude --resume <id>` / `codex resume <id>`) and `cwd` to the per-pane
registry file; no new field was added here.

Registry entry (unchanged shape):
```json
{ "command": "claude --resume <id>", "cwd": "/Users/elliot/projects/clinch-terminal" }
```

**Read + derive side** (`app/src/agent_resume.rs`): `derive_fork_command` is the **sole** source of
the fork command — a two-rule transform of the stored resume `command`, with no separate
capture-script-supplied field:
- `claude --resume <id>` → append ` --fork-session`
- `codex resume <id>` → rewrite leading `codex resume` → `codex fork`
- anything else (or an empty id after the prefix) → `None` (no Fork offered).

`pub fn read_fork_launch(uuid: &[u8]) -> Option<ForkLaunch>` reads the registry entry, runs
`derive_fork_command` on its `command`, and pairs the result with the entry's `cwd` — returning
`ForkLaunch { command: String, cwd: Option<String> }`. This is Rust's only fork-command reader;
there is no opaque `fork_command` field to prefer over derivation, so Fork works uniformly for
every captured session (no "pre-upgrade" special case).

### Component 2 — Fork action (Rust)

A new typed action (e.g. `UseAgentToolbarAction::ForkSession`, mirroring the existing
`Dismiss` action at `use_agent_footer/mod.rs:1102`) handled up the chain where pane context lives:

1. Resolve **this pane's** **uuid** (the registry key) from the footer's
   `terminal_view_id`. *(Plumbing to confirm in planning — see Open items; the pane uuid is the
   same `WARP_TERMINAL_SESSION_UUID` already stored on the terminal pane,
   `app/src/pane_group/pane/terminal_pane.rs:592`.)*
2. `agent_resume::read_fork_command(uuid)` → `ForkLaunch { command, cwd }`. If `None`, the Fork
   button is not shown (see Component 4), so this path only runs when a command exists.
3. Build a `SubshellCommandArg` from `command` (+ `cwd` if present) and dispatch
   `open_new_tab_insert_subshell_command_and_bootstrap_if_supported` (root_view.rs:1134).
4. The original pane is never touched.

### Component 3 — Compact action (Rust)

A new typed action (e.g. `UseAgentToolbarAction::Compact`) whose handler emits
`UseAgentToolbarEvent::WriteToPty("/compact\r")` — the existing footer→PTY path (mod.rs:1165).
This delivers `/compact` + Enter to the live agent's stdin. Agent-agnostic: both Claude and Codex
interpret `/compact`.

### Component 4 — Buttons in the CLI-agent footer (Rust, UI)

In the CLI-agent branch of the footer (`AgentInputFooter`, rendered from `UseAgentToolbar`
when `cli_agent(app).is_some()`, mod.rs:1321):

- Add a **Compact** button (always shown in the CLI-agent footer — both agents support it).
- Add a **Fork** button, shown when `agent_resume::read_fork_command(uuid).is_some()` (true for
  any captured Claude/Codex session; absent only for an agent started outside the capture path).
- Reuse the footer's existing `ActionButton` + `AgentFooterButtonTheme` styling (mod.rs:1419) so
  they match the other footer buttons (`Dismiss`, `Use agent`, …). Icons + tooltips:
  Fork = "Fork this session into a new tab"; Compact = "Compact this agent's context (/compact)".

## Data flow

**Fork:** click → `ForkSession` action → resolve pane uuid → `read_fork_command(uuid)` →
`SubshellCommandArg{command, cwd}` → `open_new_tab_insert_subshell_command_and_bootstrap_if_supported`
→ new tab boots in `cwd`, runs `claude --resume <id> --fork-session` / `codex fork <id>` →
original pane unchanged.

**Compact:** click → `Compact` action → `WriteToPty("/compact\r")` → this pane's agent stdin →
agent runs `/compact`.

## skip_login gating — verify, with fallback (the one structural risk)

The host is Warp's CLI-agent footer. Several Warp AI surfaces are gated by `is_any_ai_enabled`
(false in skip_login). **Planning must confirm the CLI-agent footer (`AgentInputFooter` CLI
branch) actually renders in the logged-out Clinch build.**

- If it **does** render logged-out (expected — CLI-agent detection/badges work without login per
  the attention-badges design): proceed as specified.
- If it is **gated**: host the two buttons in a minimal dedicated "agent actions" mini-bar
  rendered by the same `UseAgentToolbar` when `cli_agent(app).is_some()` — same trigger, same
  per-pane placement, but not behind the AI gate. The action/registry logic (Components 1–3) is
  identical; only Component 4's container changes. This keeps the risk contained to one component.

## Edge cases & failure handling

- **No CLI agent in pane:** neither button shown. (Footer's CLI branch isn't rendered at all.)
- **Agent started outside the capture path** (`claude --continue`, picker, no wrapper): no registry
  file → `read_fork_command` is `None` → **Fork hidden**, **Compact still shown** (Compact needs
  no session id). Correct and graceful.
- **Pre-upgrade live session** (registry has `command` but no `fork_command`): derivation fallback
  produces the fork command. Fork works without restarting the agent.
- **Fork target id stale/invalid:** the new tab's command errors harmlessly in an otherwise-usable
  shell (same failure mode as resume). No special handling.
- **Un-flushed latest turn:** fork reads on-disk transcript; newest turn may lag. Documented
  non-goal; inherent to both CLIs.
- **Compact while agent is mid-turn / not at a prompt:** `/compact\r` is delivered as typed input;
  if the agent isn't accepting input it's handled by the agent exactly as if the user typed it
  (no Clinch-side special-casing). Acceptable.
- **Multiple panes / splits:** each pane has its own footer and uuid, so each Fork/Compact targets
  that pane's agent unambiguously.

## Security / privacy

- No new data leaves the machine. `fork_command` is a local command string (session UUID + cwd),
  stored in the existing user-owned registry dir (`0700`/`0600`), no conversation content.
- No network. Local files + local PTY only.

## Testing

- **Shell (`tools/agent-resume/tests/`):** the claude wrapper and codex hook write a correct
  `fork_command` for fresh/`--resume`/`codex resume` cases; absent for passthrough cases; registry
  write stays atomic and idempotent.
- **Rust unit (`agent_resume.rs`):** `read_fork_command` returns the stored `fork_command` when
  present; derives correctly from `command` when absent (claude → `+ --fork-session`, codex
  `resume` → `fork`); returns `None` for unrecognized/empty.
- **Rust unit (footer):** Fork button hidden when `read_fork_command` is `None`, shown otherwise;
  Compact always shown in the CLI-agent footer; Compact action emits `WriteToPty("/compact\r")`.
- **Rust unit/integration (Fork action):** `ForkSession` builds the right `SubshellCommandArg` and
  invokes the new-tab path with the registry command + cwd (extend the session-restoration
  coverage pattern with a fake registry dir).
- **Manual (`build-app.sh`):** run `claude` and `codex` in panes; click Fork → new tab resumes a
  forked session, original untouched; click Compact → agent compacts. Confirm buttons absent in a
  plain shell pane. Confirm behavior in the logged-out build (skip_login gating check).

## No-dead-code audit

- Purely additive: one registry field (`fork_command`), one `agent_resume.rs` reader, two footer
  actions, two buttons.
- Nothing is removed or orphaned. The resume `command` field, the `WriteToPty` path, the new-tab
  action, and the CLI-agent footer all keep their current uses.
- `/split` + `split-launch.sh` are **retained** by user decision (a typed alias); they do not
  become unreachable and are not duplicated in Rust.
- Single source of truth for the registry shape stays the contract documented in the resume spec;
  `fork_command` is added there too so the scripts and Rust can't drift.

## Decision log

- Host = **existing CLI-agent footer** (`UseAgentToolbar`/`AgentInputFooter`), per-pane. (User.)
- Fork supports **both** Claude and Codex. (User — "/fork also exists in Codex".)
- Compact supports **both** agents, **no** confirm dialog. (User.)
- Keep the `/split` AppleScript command alongside the new button. (User.)
- Fork is **native** (new tab + registry command), not literal `/fork` injection — injecting
  `/fork` into the current pane can't keep the current session in the current tab. (Design.)
- Fork command supplied by capture scripts as opaque `fork_command`, with a Rust derivation
  fallback for pre-upgrade sessions. (Design — preserves agent-agnostic property.)

## Open items to confirm in planning

1. **skip_login gating** of the CLI-agent footer (deciding factor for Component 4 host; fallback
   specified above).
2. **`/compact` PTY delivery:** confirm `WriteToPty("/compact\r")` is received as a submitted
   command by the live agent TUI (vs. needing bracketed-paste/newline handling). Adjust the exact
   bytes if needed.
3. **Focused `TerminalView` → pane uuid** resolution for the registry key (the uuid exists on the
   terminal pane; confirm the accessor path from the footer's `terminal_view_id`).
4. **`SubshellCommandArg` shape** (does it carry cwd, or is cwd inherited from the active tab?) —
   confirm whether we pass cwd explicitly or rely on the new tab inheriting it.
