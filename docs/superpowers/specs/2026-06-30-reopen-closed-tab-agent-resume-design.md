# Resume agent sessions when reopening a closed tab

**Status:** Design — approved
**Date:** 2026-06-30
**Author:** Elliot (personal build)
**Scope:** macOS only, personal local Clinch build. Agents: Claude Code + Codex CLI.
**Depends on:** [Auto-resume agent sessions on pane restore](./2026-06-20-warp-agent-session-resume-design.md)
(the `on_restore_command` registry + replay machinery this feature reuses).

## Problem

Clinch already has two restore systems:

- **(A) In-memory undo-close** (`app/src/undo_close/`): the recently-closed tab/pane/window is kept
  alive for a grace period (default 60 s, `app/src/undo_close/settings.rs:18`) and re-attached on
  `app:reopen_closed_session`. The retained `TabData` holds a **strong** `ViewHandle<PaneGroup>`
  (`app/src/tab.rs:144`), so the whole view tree survives.
- **(B) On-disk session restoration with agent auto-resume** (`app/src/agent_resume.rs` + the
  snapshot/replay path): survives an app restart and replays `warp_agent_resume_launch <agent> <id>`
  (→ `claude --resume <id>` / `codex resume <id>`) after the restored shell's first `Bootstrapped`
  event.

These two were never connected. When a tab is closed, `remove_tab` shuts down any **long-running**
PTY before detaching (`app/src/workspace/view.rs:11665-11684`):

```rust
if terminal_view.…active_block().is_active_and_long_running() {
    terminal_view.shutdown_pty(ctx);   // kills the process
}
```

A live Claude/Codex agent is a long-running foreground process, so closing its tab **kills its
PTY**. The `TabData` (with its `PaneGroup`) is still retained by the undo stack, but on reopen
`reattach_panes` re-attaches a leaf whose shell process is gone → a **dead tab you can't type
into**. Plain-shell tabs never hit `shutdown_pty`, so they reopen fine — which is why only agent
tabs feel broken.

## Goal

When you reopen a closed tab whose PTY was killed on close, the leaf should come back as a **fresh,
usable shell in its original working directory**, and — if it had an agent — should **replay the
agent's resume command** so the conversation comes back, reusing system (B)'s exact pattern. No
clicks beyond the reopen shortcut.

### Decisions (settled during brainstorming)

- **Resume, don't keep-alive.** Keep today's "kill long-running on close" behavior. The agent must
  not keep running (and editing files / spending tokens) after you close the tab. On reopen we spawn
  a fresh shell and replay the resume command — mirroring the relaunch path, not reattaching a
  still-running process.
- **Respawn *any* dead leaf, not just agents.** A non-agent long-running process (dev server,
  `tail -f`) also gets `shutdown_pty`'d on close and has no resume command. On reopen it should still
  respawn as a plain usable shell (no replay), so "you can't do anything" is fixed universally.

### Non-goals

- Keeping the agent **process** alive across a close (out of scope by the decision above — we resume
  the *conversation*, we do not preserve the *process*).
- Reopen after the grace period (the undo item is already discarded — unchanged from today).
- Cross-platform support (macOS only).
- A new feature flag (rides the existing `undo_closed_panes` flag + `local_tty` cfg).

## Key facts this design relies on (verified)

1. **The kill site at close** — `app/src/workspace/view.rs:11665-11684`. Iterates terminal panes,
   `shutdown_pty()`s the long-running ones, then `detach_panes_for_close`. The agent is still alive
   here, so its registry file `~/.warp/agent-resume/<uuid_hex>.json` still exists at this instant.
2. **`shutdown_pty` only requests shutdown** (sets `manual_pty_shutdown_requested`, emits an event;
   `app/src/terminal/view.rs:8486`) — it does **not** drop the `TerminalPane`, so the dead leaf is
   fully present and replaceable on reopen.
3. **Pane UUID is stable** and becomes `WARP_TERMINAL_SESSION_UUID` — re-creating a session with the
   same UUID re-binds the resume command to the right conversation (`TerminalPane.uuid`,
   `app/src/pane_group/pane/terminal_pane.rs:83`).
4. **`reattach_panes` is the shared reopen path** for both tab-reopen and window-reopen
   (`app/src/pane_group/mod.rs:7461`; window reopen calls it per tab via
   `Workspace::handle_reopen` → `app/src/workspace/view.rs:12001`).
5. **The three primitives needed all exist and are proven:**
   - `PaneGroup::create_session(…)` builds a fresh `(view, manager)` for a UUID
     (`app/src/pane_group/mod.rs:5920`; used by the relaunch path at `:1642`).
   - `PaneGroup::replace_pane(original_id, replacement, is_temporary, ctx)` swaps one leaf's content
     for another in the tree (`app/src/pane_group/mod.rs:4900`; used today for terminal↔code
     conversion at `:4984`/`:5013` and loading→terminal swaps at `:5290`/`:6269`).
   - `manager.set_on_restore_command(cmd, ctx)` queues a one-shot replay after `Bootstrapped`
     (`app/src/terminal/local_tty/terminal_manager.rs:1055` →
     `app/src/terminal/writeable_pty/pty_controller.rs:510-540`).
6. **The stored command is a robust wrapper.** `read_on_restore_command` returns
   `warp_agent_resume_launch <agent> <id> [flags]` (`tools/agent-resume/`), which checks
   resumability and **falls back to a fresh `claude`/`codex`** if the conversation no longer exists.
   So replaying a stale command degrades gracefully.

## Architecture

Clinch stays **agent-agnostic** — the new code never parses the resume command, it only captures and
replays the same opaque string system (B) already uses. The whole change is: *capture a per-leaf
"restart spec" at the moment we kill a PTY, then act on it where panes are re-attached.*

```
 close (capture)                              reopen (act)
 ┌───────────────────────────────┐           ┌──────────────────────────────────────┐
 remove_tab shutdown loop                     reattach_panes (tab + window reopen)
   for each long-running leaf:                  for each leaf carrying a RestartSpec:
     read agent-resume registry[uuid]             create_session(same uuid, spec.cwd)
     RestartSpec{ cwd, on_restore_command } ─┐    → wrap in TerminalPane
     store on the TerminalPane               │    → replace_pane(dead_id, fresh)
     shutdown_pty()                          │    → if cmd: set_on_restore_command(cmd)
                                             └──► (spec rides inside retained TabData) ─► clear spec
```

### Component 1 — `RestartSpec` (new)

A small struct describing how to rebuild one killed leaf:

```rust
struct RestartSpec {
    cwd: Option<PathBuf>,            // original working directory (for the fresh shell + resume scope)
    on_restore_command: Option<String>, // the agent resume command, or None for non-agent processes
}
```

Stored on `TerminalPane` as `Option<RestartSpec>`, set via a method. **Its presence is the single
signal** that "this leaf's PTY was killed on close and must be restarted on reopen." It is only ever
set in the close path, and cleared after a successful restart.

### Component 2 — Capture at close (`app/src/workspace/view.rs`)

In the existing shutdown loop (`:11665-11684`), for each terminal pane about to be `shutdown_pty`'d:

- read `cwd` (same source the snapshot uses, `pwd_if_local`),
- read `on_restore_command = agent_resume::read_on_restore_command(&pane.uuid)` (the registry file is
  still present because the agent is still alive at this instant — this is the same "freeze at
  capture time" robustness system (B) relies on),
- set `RestartSpec { cwd, on_restore_command }` on the `TerminalPane`,
- then `shutdown_pty()` as today.

`on_restore_command` is `None` for non-agent long-running processes; the leaf still gets a
`RestartSpec` so it is respawned as a plain shell.

### Component 3 — Restart on reopen (`app/src/pane_group/mod.rs`)

In `reattach_panes` (`:7461`), for each pane that carries a `RestartSpec` (gated
`#[cfg(feature = "local_tty")]`):

1. `let (view, manager) = create_session(uuid = pane.uuid, cwd = spec.cwd, …)` — same UUID, so
   `WARP_TERMINAL_SESSION_UUID` matches and the shell boots in the original cwd (satisfying
   `claude --resume`'s cwd-scoping).
2. Wrap in a new `TerminalPane`; `replace_pane(dead_pane_id, fresh_pane, false, ctx)` to swap it into
   the dead leaf's tree slot.
3. If `spec.on_restore_command` is `Some`, `manager.set_on_restore_command(cmd, ctx)` — it fires once
   after the fresh shell's first non-subshell `Bootstrapped`, exactly as on relaunch.
4. Clear the spec so a second reopen does not re-restart.

Leaves **without** a `RestartSpec` (plain shells still live within the grace window, live splits) are
re-attached unchanged — no regression to their running processes or scrollback.

## Data flow

**Close (agent tab):** `remove_tab` → capture `RestartSpec { cwd, "warp_agent_resume_launch claude <id>" }`
on the leaf → `shutdown_pty` kills the agent → `TabData` (carrying the spec) pushed onto the undo
stack.

**Reopen:** `app:reopen_closed_session` → `restore_closed_tab` → `reattach_panes` sees the spec →
`create_session(same uuid, same cwd)` + `replace_pane` → shell boots in original cwd → `Bootstrapped`
→ replays the resume command → agent reattaches to its conversation (or the wrapper falls back to a
fresh agent if the conversation is gone).

## Edge cases & failure handling

- **Non-agent long-running pane** (dev server, `tail -f`): `RestartSpec` with
  `on_restore_command: None` → respawns a plain usable shell, no replay. Correct.
- **Mixed tab** (killed agent split + live plain split): only the leaf carrying a `RestartSpec` is
  rebuilt; the live split keeps its process and scrollback. The per-leaf design is what makes this
  safe (a whole-tab snapshot rebuild would not).
- **Reopen after the 60 s grace period:** the undo item was already discarded; nothing to reopen.
  Unchanged from today.
- **Stale / deleted conversation:** `warp_agent_resume_launch` falls back to a fresh `claude`/`codex`
  in an otherwise-usable shell. No special handling.
- **Plain-shell tab** (never long-running): no `shutdown_pty`, no `RestartSpec` → reattaches live,
  exactly as today.
- **Registry file already gone at close** (agent exited cleanly a moment before close):
  `on_restore_command` is `None` → respawns a plain shell. Acceptable (there was no live conversation
  to resume).

## Scope / non-goals (this spec)

- **In scope:** tab reopen and window reopen (both flow through `reattach_panes`).
- **Split-pane reopen** (`restore_closed_pane`, `app/src/pane_group/mod.rs:5443`): apply the same
  capture + restart **only if** single-pane close (`handle_pane_closed_by_id` path,
  `app/src/pane_group/mod.rs:4744`) also `shutdown_pty`s long-running PTYs. Verify in planning; extend
  if so, otherwise defer (YAGNI — do not build a restart path for a close path that doesn't kill).
- macOS / local build; no new feature flag.

## Testing

- **Rust unit/view tests** (`app/src/workspace/view_tests.rs`, alongside the existing
  `restore_closed_tab` / `test_reopen_closed_shared_tab` coverage): with a fake agent-resume registry
  dir, assert that closing a tab whose pane has a populated registry file captures a `RestartSpec`
  with the expected command, and that reopen calls `create_session` with the **original UUID** and
  queues the resume command for replay.
- **Integration** (`crates/integration/src/test/session_restoration.rs` style): close → reopen an
  agent tab and assert the resume command is written to the PTY after `Bootstrapped`.
- **Manual:** open a tab, run `claude`, close it, reopen — confirm the shell comes back in the right
  cwd and `claude --resume` runs. Repeat with a non-agent `npm run dev` (expect a plain shell, no
  replay) and with a plain idle shell (expect today's live reattach).

## Code to remove / avoid (no dead code)

- **No code is removed.** The change is additive: one struct (`RestartSpec`), one setter +
  field on `TerminalPane`, one capture site in `remove_tab`, one restart branch in `reattach_panes`.
- Reuses `create_session`, `replace_pane`, `set_on_restore_command`, and
  `agent_resume::read_on_restore_command` rather than duplicating any of them — so no parallel
  resume/restart logic is introduced that could later rot.
- **Do not** add a separate snapshot representation for closed agent tabs, an agent-name parser, or a
  "keep the agent alive" path — the per-leaf `RestartSpec` + reused replay covers the requirement
  without them.

## Future (explicitly deferred — YAGNI)

- Split-pane reopen parity (pending the close-path verification above).
- Surfacing a subtle hint that a reopened agent tab is "resuming" (the wrapper's own output already
  makes this visible).
