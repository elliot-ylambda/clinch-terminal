---
name: clinch-control
description: Control and inspect the running Clinch app from Claude Code or Codex with its local control CLI. Use when the user asks to manipulate Clinch windows, tabs, panes, sessions, sidebar sections, toolbelts, or UI surfaces; inspect or search rendered terminal contents across tabs; or launch a long-lived, interactive, or user-visible project process such as a dev server, watcher, REPL, or log tail in a new tab. Do not use for tests, lint, builds, Git commands, or other bounded work the agent can run in its own shell.
---

<!-- managed-by: Clinch; version: 1.6.0 -->

# Clinch control

Use Clinch's existing local control CLI. Do not create a separate MCP server,
edit Clinch's SQLite database, or automate UI clicks when a typed CLI action is
available.

## Choose the execution surface

Create a new Clinch terminal tab when any of these are true:

- The user explicitly asks for a new tab or asks to keep a process visible.
- The command is a persistent foreground process: a dev server, file watcher,
  REPL, log tail, emulator, or similar process expected to outlive this tool call.
- The process is interactive or the user is likely to inspect and stop it later.

Keep the command in the agent-owned shell when it is bounded: tests, linters,
formatters, builds, type checks, Git commands, file inspection, and diagnostic
commands. If duration or interactivity is unclear, use the agent-owned shell.
Do not create a tab merely because a command is a subprocess.

## Bind to the current Clinch app

Current Clinch versions inject four values into every local host terminal
shell:

- `CLINCH_CONTROL_COMMAND`: the current channel's command name.
- `CLINCH_CONTROL_WRAPPER`: the exact wrapper inside the app that launched the
  shell.
- `CLINCH_CONTROL_PID`: the process ID of that exact app instance.
- `WARP_TERMINAL_SESSION_UUID`: the durable identity of the terminal and outer
  project tab that launched the agent.

When `CLINCH_CONTROL_WRAPPER` is set and executable, use that exact path as
the executable in the `"$CLINCH_CONTROL_WRAPPER" ctrl` command prefix. Add
`--pid "$CLINCH_CONTROL_PID"` to every command that accepts a target selector.
The wrapper takes precedence over `PATH`; the PID also distinguishes separate
local worktree builds that share one channel. If the wrapper is set but
missing, or its PID is absent, stop and tell the user to relaunch or rebuild
that Clinch app. Do not fall back to another channel or instance. Always quote
the wrapper path and include the `ctrl` subcommand.

Before the first requested control action, verify the bound app responds:

```sh
"$CLINCH_CONTROL_WRAPPER" ctrl --output-format json app ping \
  --pid "$CLINCH_CONTROL_PID"
```

New Clinch installs enable local control by default. If this returns
`no_instance` from a bound terminal, retry once. If it still fails, report that
local control is unavailable or may have been explicitly disabled; point to
**Settings > Local control** only as recovery. Do not re-enable a setting the
user deliberately disabled. The bundled wrapper does not require a global
command installation.

An isolated container cannot receive the host app's control capability. If
`/.dockerenv` exists and the current shell has no complete binding, tell the
user to run the request from a host Clinch terminal; do not diagnose the
container as an outdated Clinch install.

When it is absent and `WARP_FOCUS_URL` begins with `clinch://` or
`clinchdev://`, treat the session as coming from an older Clinch version that
predates current-app binding. Tell the user to update and start a new
terminal/agent session. `TERM_PROGRAM=WarpTerminal` alone is not sufficient to
identify Clinch because Warp uses the same value. Do not use an absolute bundle
path saved in this user-scope skill and do not guess from running processes.

Outside a Clinch terminal, check `clinch` and `clinch-local` on `PATH` and run
`<available-command> ctrl instance list` for each. Use it only if exactly one
live Clinch channel is found; ask the user to select when more than one is
live. If none is available, tell the user to install and run the latest
Clinch. The optional global command for use outside Clinch is available in
**Settings > Local control**. Never create or replace a `/usr/local/bin`
symlink without explicit approval. `warpctrl` is a legacy compatibility alias,
not the user-facing Clinch command.

Run control commands serially because creating or activating a tab changes the
active target. Outside a bound Clinch terminal, if multiple same-channel
instances are running, select the intended instance explicitly with
`--instance <id>`.

In a bound Clinch terminal, keep `WARP_TERMINAL_SESSION_UUID` in the command
environment and omit `--window` when creating a tab unless the user explicitly
chooses another window. The CLI carries that identity automatically so an
implicit `tab create` targets the project tab that launched the agent, even if
another project is active when the request arrives.

## Search terminal contents across tabs

Use the read-only `tab grep` action when the user asks what is running, shown,
or mentioned in other tabs. It searches inactive tabs without activating them
and returns JSON identities for the window, tab, pane, and matching snapshot
line. With no window, tab, or pane selector it searches all terminal panes in
the active project window. Prefer an exact `--window <window-id>` from `window
list` when it is already known:

```sh
"$CLINCH_CONTROL_WRAPPER" ctrl --output-format json tab grep "error|failed" \
  --pid "$CLINCH_CONTROL_PID" --window <window-id> --ignore-case
```

Patterns are regular expressions by default. Use `--fixed-strings` for literal
text and `--max-matches <count>` to lower the default result limit. Add an exact
`--tab <tab-id>` or `--pane <pane-id>` when the user's request is narrower.
Search only terms relevant to the request; do not use an empty or catch-all
pattern to reconstruct every tab's contents.

The searchable text is a live, bounded snapshot: retained plaintext
prompt/output for normal shells and the currently rendered viewport for
full-screen terminal apps. Secret cells remain obfuscated. Check
`content_truncated`, `matches_truncated`, and `skipped_non_terminal_panes`
before claiming the search was exhaustive; individual matches also report
`text_truncated`. This action does not expose hidden conversation history or
non-terminal document contents.

## Launch a persistent project process

1. Resolve the exact absolute directory in which the command should run. Use
   the current project directory unless the user names a subdirectory. Never
   rely on the active tab's inherited working-directory setting.
2. Invoke `tab create` with `--cwd`, put the executable and its arguments after
   `--`, and request JSON output so the returned window and tab IDs are easy to
   reuse:

   ```sh
   "$CLINCH_CONTROL_WRAPPER" ctrl --output-format json tab create \
     --cwd "/absolute/project/path" \
     --pid "$CLINCH_CONTROL_PID" \
     -- npm run dev
   ```

   Outside a bound Clinch terminal, replace the PID selector with the selected
   `--instance <instance-id>`; omit it only when exactly one matching instance
   is running. Pass arguments directly after `--`. To intentionally use shell
   syntax such as a pipeline, make the shell explicit, for example
   `-- sh -lc 'cmd | other'`.
3. Use the returned IDs for supported exact follow-up mutations. For example,
   give the new tab a useful name with an exact selector:

   ```sh
   "$CLINCH_CONTROL_WRAPPER" ctrl tab rename "Dev server" \
     --pid "$CLINCH_CONTROL_PID" --window <window-id> --tab <tab-id>
   ```
4. Do not wait for the persistent process to exit. Report which tab was
   created, the directory and command it owns, and that the user can stop it in
   the tab with normal terminal controls.

If launch fails, report the control error. Do not silently fall back to an
untracked background process in the agent shell.

## Handle other Clinch changes

- Discover the installed surface with `"$CLINCH_CONTROL_WRAPPER" ctrl help`
  and `"$CLINCH_CONTROL_WRAPPER" ctrl <group> --help`; do not invent actions.
- Inspect targets before mutating them and reuse opaque IDs from CLI results.
- Use `section list` to inspect project sidebar sections. A section ID is scoped
  to its window and must be reused exactly.
- Create a named section from an existing tab with
  `section create "Backend" --window <window-id> --tab <tab-id>`. Empty sections
  are not supported.
- Manage a section with `section update <section-id> --name "API"`,
  `--collapsed true|false`, or `--color red|default`; reorder it one slot with
  `section move <section-id> --direction up|down`.
- Add and remove tabs with `section tab add <section-id> --tab <tab-id>` and
  `section tab remove --tab <tab-id>`.
- `section delete <section-id>` removes only the section container. Its tabs and
  running sessions remain open and become unsectioned.
- Before deleting, resolve the intended window and section with `window list`
  and `section list --window <window-id>`. Delete with exact selectors:

  ```sh
  "$CLINCH_CONTROL_WRAPPER" ctrl section delete <section-id> \
    --pid "$CLINCH_CONTROL_PID" --window <window-id>
  ```

  If the user explicitly asks to verify preservation, compare `tab list
  --window <window-id>` before and after; never substitute a close action.
- Use the separately installed `clinch-toolbelt` skill for explicit quick-insert
  button requests. Typed `toolbelt` actions can list, create, delete, and move
  buttons in the shared Claude Code/Codex coding-agent footer or the independent
  terminal footer.
- Suggest creating a quick-insert button when the user repeatedly supplies the
  same prompt or command, but wait for confirmation before changing a toolbelt.
- If a requested mutation is not exposed by the installed CLI, say so rather
  than editing internal persistence.
- Invoke close actions only when the user explicitly asks to close something.
