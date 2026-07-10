# Claude Conversation Durability — Tech Spec

## Context

See `PRODUCT.md` for user-visible behavior and incident history.

### Existing capture/replay architecture

- **Capture**: `~/.claude/settings.json` wires `SessionStart`, `UserPromptSubmit`, and
  `Stop` hooks to `~/.warp/agent-resume-bin/claude-capture.sh` (source:
  `tools/agent-resume/claude-capture.sh`). The hook reads `session_id`, `cwd`, event,
  permission mode, and prompt text; reconstructs launch flags from the live process argv;
  and writes one mutable pane entry through `warp-agent-resume`.
- **Replay**: `tools/agent-resume/claude.zsh` supplies
  `warp_agent_resume_launch`. It tries a recorded cloud bridge first, then a resumable
  local id, then the newest unclaimed local session for the cwd, then a guarded fresh
  launch.
- **Fork UI**: `app/src/agent_resume.rs` reads the same registry. This feature does not
  change the Rust side.
- **Install**: `tools/agent-resume/install.sh` installs the scripts and wires shell/hook
  configuration; the `agent-resume` Make target runs before bundling.

## Root-cause findings (verified 2026-07-09)

The investigation gate is resolved:

1. The running Clinch app carried a stale Claude session's environment:
   `CLAUDE_CODE_SESSION_ID`, `CLAUDE_CODE_BRIDGE_SESSION_ID`,
   `CLAUDE_CODE_CHILD_SESSION=1`, `CLAUDECODE=1`, `AI_AGENT`, and Make/update variables.
   The chain was: `make update` ran inside Claude → the detached updater inherited its
   environment → `open` forwarded that environment into the new app → every pane shell
   inherited it → each new Claude process believed it was a child of the stale bridged
   session.
2. A controlled interactive A/B completed one real turn in each environment. Clean env
   wrote a normal local jsonl even though remote control bridged at startup; the poisoned
   env wrote no jsonl. Therefore remote control is not the cause and must remain enabled.
3. `CLAUDE_CODE_CHILD_SESSION=1` is exported by Claude to its children; it is not injected
   by Clinch. Inheriting it at a new top-level launch is the harmful condition.
4. Claude Code exposes no supported "bridge but force a second local durability record"
   knob. The user's `remoteControlAtStartup: true` is wanted for phone access and does not
   need to change.
5. OSC 777 `prompt_submit` events in `~/Library/Logs/clinch.log` provided forensic evidence
   for the lost session, but they are truncated and rotated, so they are not a durability
   mechanism.

## Implemented changes

### 1. Append-only registry journal

`tools/agent-resume/warp-agent-resume` appends one JSON line to
`$DIR/journal.jsonl` after every successful `write` and before every effective `remove`:

```json
{"ts":"<ISO8601 UTC>","op":"write","pane":"<uuid>","command":"<resume cmd>","cwd":"<cwd>","bridge":"<bridge-or-empty>"}
```

- Pane entries remain atomic temp-file + `mv` writes.
- Journal appends are fail-open and mode `600`; the registry directory remains `700`.
- There is no pruning in v1. At roughly 200 bytes per line, growth is modest and the full
  history is valuable during recovery.

### 2. Local prompt mirror

On every non-empty `UserPromptSubmit`, `tools/agent-resume/claude-capture.sh` appends:

```json
{"ts":"<ISO8601>","cwd":"<cwd>","bridge":"<bridge-or-empty>","prompt":"<exact prompt>"}
```

to `$DIR/prompts/<sid>.jsonl`.

- Mirroring occurs before the pane-ownership guard, so nested sessions keep their own
  prompt history without taking over the pane entry.
- Mirroring stays unconditional even after launch hygiene restores normal jsonls. The
  redundancy is deliberate, cheap corruption insurance and covers child/nested sessions.
- Files are private (`700` directory, `600` files), local-only, and capped at about 5 MB
  per session with one final `{"truncated":true}` marker.
- Failure never fails or delays the Claude hook path beyond the attempted local append.

### 3. Pre-quit update snapshot

Before quitting the running app, `script/update-installed-clinch` copies the full registry
(pane entries, journal, and mirrors) plus any locally present referenced Claude transcripts
to:

`~/Library/Application Support/sh.clinch.Clinch/session-recovery-<stamp>/`

Automatic snapshots contain `.auto-snapshot`; only the newest 15 marked directories are
kept. Unmarked, hand-curated recovery directories are never pruned. Snapshot failure is
best-effort and never blocks an update.

### 4. Launch environment hygiene (root fix)

Two boundaries prevent a stale Claude identity from becoming a new top-level session:

- **App relaunch**: immediately before `open`, `script/update-installed-clinch` enumerates
  exported names with macOS Bash 3.2's `compgen -e` and constructs `env -u` arguments for
  every `CLAUDE_CODE_*` variable plus `CLAUDECODE`, `CLAUDE_EFFORT`, and `AI_AGENT`. It
  also retains the existing Make/SKIP_SYNC scrub. This path is a full application reset,
  so no Claude session-shaped environment is intentionally preserved.
- **Interactive launch**: `tools/agent-resume/claude.zsh` defines a thin `claude()` wrapper
  that removes only identity/implementation markers (`CLAUDE_CODE_SESSION_ID`, bridge and
  remote ids, child marker, entrypoint/execpath, `CLAUDECODE`, and `AI_AGENT`) before
  resolving the real executable through `env`. It forwards `"$@"` verbatim and preserves
  legitimate user behavior controls such as `CLAUDE_EFFORT` and adaptive-thinking flags.
  Claude tool shells are non-interactive and do not source this wrapper.

Remote control stays enabled. Teleport also deliberately remains the first restore path
when a valid bridge id is recorded: a cloud session may have turns continued from another
device, so its cloud copy remains authoritative; local resume is the fast-failure fallback.

### 5. Discovery command

`warp-agent-resume list [--cwd <dir>]` joins journal and prompt-mirror records and prints a
newest-first line per known conversation:

`<start-ts>  <sid-short>  <cwd>  <bridge-url-or-local>  "<first prompt>"`

Nested sessions known only to the prompt mirror are included. The command is read-only.

## Testing

The shell suite under `tools/agent-resume/tests/` covers:

- journal write/overwrite/remove history, escaping, permissions, list output, and fail-open
  behavior;
- prompt JSON escaping, exact round-trip text, ownership-guard ordering, permissions,
  bridge capture, empty/Stop behavior, and the single-marker size cap;
- Claude launch wrapper identity scrubbing, unrelated/behavior environment preservation,
  exact argv boundaries, and unchanged teleport/resume/fallback behavior;
- update relaunch scrubbing for current and future `CLAUDE_CODE_*` names, Make/update
  variables, unrelated-environment preservation, and exact bundle argv;
- existing Claude flags, hooks, Codex hooks, installer wiring, and registry behavior.

Manual acceptance is the `yo-durability-probe` flow in `PRODUCT.md`, including one update
started while a conversation is active and verification of the pre-quit snapshot.

## Rollout

Run `bash tools/agent-resume/install.sh` to activate the journal, mirror, CLI, capture hook,
and launch wrapper. Existing shells must source `~/.zshrc` (or be reopened) for the wrapper;
capture changes take effect immediately. The currently running Clinch app must then receive
one clean relaunch (Dock/Finder, or the fixed update path) to remove any already-inherited
poisoned environment.
