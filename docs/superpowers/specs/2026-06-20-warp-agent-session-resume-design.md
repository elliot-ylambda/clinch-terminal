# Auto-resume agent sessions on pane restore

**Status:** Design — pending review
**Date:** 2026-06-20
**Author:** Elliot (personal build)
**Scope:** macOS only, personal local Warp build. Agents: Claude Code + Codex CLI.

## Problem

When Warp restores windows/tabs/panes on relaunch (after `cmd-Q`, an update-and-restart, or a
crash), it restores layout, working directory, and scrollback — but the long-lived CLI agent
processes running in those panes (Claude Code, Codex) are killed and **not** brought back. Today
you must manually re-run `claude --resume …` / `codex resume …` in each pane.

Common workflow this targets: **multiple tabs, multiple agents, all in the same repo directory**,
each driving a different feature. This rules out any directory-based heuristic for identifying
"which session belonged to which tab" — the working directory is not unique per tab.

## Goal

On restore, each pane that had a live agent session at the time of the last snapshot should
**automatically re-run the exact resume command** for *that pane's* session, immediately after the
restored shell finishes bootstrapping. No clicks.

### Non-goals

- Keeping the agent **process** alive across a Warp quit (impossible without a separate daemon;
  out of scope — we resume the *conversation*, we do not preserve the *process*).
- Surviving a full machine reboot beyond what `--resume` already gives (the conversation is on
  disk; the process is gone — same as our restore path).
- Cross-platform support (macOS only).
- Upstreaming / feature-flagging (it's a personal build; see "Future" for the flag-able seam).

## Key facts this design relies on (verified)

1. **Pane UUID is stable across quit→restore.** A pane's persisted `uuid` is read back unchanged
   and becomes its `WARP_TERMINAL_SESSION_UUID` env var.
   - `app/src/persistence/sqlite.rs:1177` (save), `:2163` (read)
   - `app/src/pane_group/mod.rs:1579` (restore), `:5890` (`add_session_focus_env_vars`)
2. **Child processes inherit pane env vars.** A `claude`/`codex` launched in a pane sees
   `WARP_TERMINAL_SESSION_UUID`. — `app/src/terminal/local_tty/unix.rs:381`
3. **A command can be injected after the restored shell is ready** by subscribing to the
   `Bootstrapped` event and calling `write_command`.
   - `app/src/terminal/writeable_pty/pty_controller.rs:120` (Bootstrapped), `:511` (`write_command`)
4. **Agent capture asymmetry** (drives the per-agent capture choice):
   - **Claude Code** hooks expose `session_id`/`cwd` but **cannot** read arbitrary parent env vars
     (only `CLAUDE_*`). Claude **can** force the id at launch: `claude --session-id <uuid>`; resume
     with `claude --resume <id>` (scoped to the original cwd).
   - **Codex** **cannot** force an id at launch, but its `SessionStart` hook **inherits the shell
     env** (so it can read `WARP_TERMINAL_SESSION_UUID`) and receives `session_id` on stdin; resume
     with `codex resume <id>`.

## Architecture

Warp stays **agent-agnostic**. The Rust side persists and replays a single opaque string per pane:
*"on restore, run this command."* All agent-specific knowledge lives in small user-space scripts.
Adding another agent later (e.g. Gemini CLI) needs **zero** Rust changes.

```
 capture (user-space)            persist+replay (Rust)
 ┌─────────────────────┐        ┌──────────────────────────────┐
 claude wrapper  ─┐              snapshot: read registry[uuid]
 codex hooks     ─┼─► registry ─►   → freeze command into
                  │   ~/.warp/        TerminalPaneSnapshot
                  │   agent-resume/                │
                  │   <uuid>.json              SQLite column
                  │                                │
                  └───────────────────────► restore: after Bootstrapped,
                                              write_command(stored command)
```

### Component 1 — The registry (shared contract)

A directory of **one file per pane** (one-file-per-pane avoids concurrent-append races that a
single shared JSONL would have):

```
~/.warp/agent-resume/<pane_uuid_hex>.json
```

```json
{ "command": "claude --resume 550e8400-e29b-41d4-a716-446655440000", "cwd": "/Users/elliot/projects/warp" }
```

- Key = `WARP_TERMINAL_SESSION_UUID` (hex), the only per-tab-unique identifier.
- Value = a **ready-to-run resume command** plus the cwd it belongs to. Warp never parses it; it
  just stores and replays it. This is what keeps Warp agent-agnostic.
- Writes are **atomic**: write to `<uuid>.json.tmp` then `rename` into place.
- Lifecycle invariant: at any instant the registry contains exactly the agent sessions **currently
  live**, each tagged to its pane. A live session has a file; a gracefully-exited session does not.

### Component 2 — Claude capture (shell wrapper)

A `claude` shell function (installed in `~/.zshrc`, shadows the binary; reversible by deleting it):

- **Fresh start** (no session-controlling flag present): generate a UUID, write the registry file
  (`command = "claude --resume <uuid>"`, `cwd = $PWD`), then `exec` the real binary with
  `--session-id <uuid>` and the user's args. On exit, remove the registry file.
- **Explicit `--resume <id>` / `-r <id>`**: record *that* id instead, then exec unchanged.
- **`--continue` / interactive picker / `--print` (headless)**: pass through untouched, **do not**
  record. Documented limitation (see Edge cases). `--print` is excluded because it is a one-shot
  non-interactive invocation, not a resumable interactive session.
- Locating the real binary: resolve via `command -v` / a saved absolute path so the function does
  not recurse into itself.

### Component 3 — Codex capture (config hooks)

`SessionStart` and `SessionEnd` command hooks in `~/.codex/config.toml`:

- **SessionStart** (`matcher = "startup|resume"`): read `session_id` from stdin JSON and
  `WARP_TERMINAL_SESSION_UUID` from the inherited env; write the registry file
  (`command = "codex resume <session_id>"`, `cwd` from stdin). No-op if the env var is absent
  (i.e. codex launched outside a Warp pane).
- **SessionEnd**: remove the registry file for that pane.

### Component 4 — Persist (Rust, the core feature)

- Add `on_restore_command: Option<String>` to `TerminalPaneSnapshot`
  (`app/src/app_state.rs:201`).
- **At snapshot construction** (where `cwd` is already captured via `view.pwd_if_local`,
  `app/src/pane_group/pane/terminal_pane.rs:~581`): read `~/.warp/agent-resume/<uuid_hex>.json`;
  if present, set `on_restore_command = Some(entry.command)`. Freezing the command **at snapshot
  time** (while the agent is still alive) is deliberately more robust than re-reading the registry
  at restore, where a hard process kill on quit might have already removed the file.
- Persist via a new nullable `on_restore_command` TEXT column in the `terminal_panes` table:
  - write in `save_pane_state` (`app/src/persistence/sqlite.rs:~1175`)
  - read in `read_node` (`app/src/persistence/sqlite.rs:~2162`)
- This rides on Warp's **existing** pane-snapshot persistence (the same mechanism that already
  restores cwd). No new snapshot scheduling is introduced.
  - **To confirm during planning:** the exact snapshot *cadence*. `cmd-Q` and update-and-restart
    snapshot on shutdown (agents still alive → command frozen → fully covered). The **crash** case
    only recovers the command if persistence is periodic/continuous, not shutdown-only; if it is
    shutdown-only, a crash falls back to no auto-resume for that pane (no worse than today). Verify
    before committing to the crash guarantee.

### Component 5 — Replay (Rust)

- During restore, in the pane-creation path (`app/src/pane_group/mod.rs:~1655`, after
  `create_session`), if `on_restore_command` is `Some`, subscribe to the pane's model events and on
  the first `Bootstrapped` event call `write_command(cmd, …)`, then unsubscribe (fire once).
- Auto-runs immediately (chosen behavior). A stale/invalid id simply produces a harmless shell
  error ("No conversation found …") in an otherwise-usable shell.

## Data flow

**Capture (steady state):** user runs `claude`/`codex` in a pane → wrapper/hook writes
`registry/<uuid>.json` → on graceful exit the file is removed.

**Snapshot (periodic + on quit):** Warp builds each `TerminalPaneSnapshot` → reads
`registry/<uuid>.json` → freezes `command` into the snapshot → writes to SQLite.

**Restore (relaunch):** Warp reads snapshots → recreates pane with the same `uuid` (→ same
`WARP_TERMINAL_SESSION_UUID`) and cwd → shell boots **in the restored cwd** (so `claude --resume`'s
cwd-scoping is satisfied) → `Bootstrapped` fires → Warp runs the stored `claude --resume <id>` /
`codex resume <id>`.

## Edge cases & failure handling

- **Agent gracefully exited before quit:** registry file already removed → no `on_restore_command`
  frozen → pane restores to a normal shell. Correct.
- **Hard kill at quit (SessionEnd may not fire):** command was already frozen at the last snapshot
  → resumes correctly. The snapshot-time freeze is what makes this robust.
- **Multiple agents, same directory:** disambiguated by pane UUID, not cwd. The whole point.
- **Resume target no longer exists** (session deleted, agent changed): command errors harmlessly;
  shell remains usable. No special handling.
- **`claude --continue` / picker:** not recorded → not auto-resumed. Documented limitation.
- **Non-Warp shells / agent launched outside a pane:** `WARP_TERMINAL_SESSION_UUID` absent →
  wrapper/hook no-op. Safe.
- **Wrapper recursion:** the `claude` function must invoke the real binary by absolute path, not by
  name, to avoid calling itself.
- **Stale registry files** (e.g. wrapper killed before cleanup): harmless — only ever read for a
  pane that is being restored *right now* with that exact UUID; a UUID is never reused. Optional
  best-effort sweep of files older than N days can be added later (not required).

## Security / privacy

- The registry stores a resume command + cwd in `~/.warp/agent-resume/` (user-owned, `0700` dir /
  `0600` files). It contains session UUIDs and paths, not conversation content.
- No network. Local files only.

## Testing

- **Rust unit/integration:** extend the existing session-restoration integration coverage
  (`crates/integration/src/test/session_restoration.rs`) to assert that a pane with a populated
  registry file round-trips `on_restore_command` through save→read and that, on restore, the command
  is written to the PTY after `Bootstrapped`. Use a fake registry dir.
- **Scripts:** unit-test the `claude` wrapper's arg parsing (fresh vs `--resume <id>` vs
  `--continue`/`--print` passthrough) and the codex hook's JSON+env handling, with a temp
  `HOME`/registry dir.
- **Manual:** open 3 tabs in the same repo (claude, claude, codex), `cmd-Q`, relaunch, confirm each
  tab resumes its own session.

## Code to remove / avoid (no dead code)

- No pre-existing resume/auto-run code exists to delete (confirmed: there is no
  `initial_command`/`startup_command` field today).
- **Do not** add agent-name parsing, a cwd-based "most recent session" fallback, or a
  `WARP_RESTORED` marker env var — the agent-agnostic registry + frozen command makes all of these
  unnecessary. Keeping them out is what prevents future dead code.
- Single source of truth for the registry path (one constant in Rust, one in the shell scripts) so
  the contract can't drift.

## Future (explicitly deferred — YAGNI)

- Feature flag / upstreaming seam (gate replay behind a flag).
- Gemini CLI and other agents (just another capture script writing the same registry format).
- Handling `claude --continue` / picker via a Claude `SessionStart` hook + PID-ancestor correlation.
- Registry GC sweep.
