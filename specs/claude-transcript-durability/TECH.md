# Claude Conversation Durability — Tech Spec

## Context

See `PRODUCT.md` for user-visible behavior and incident history.

### Existing capture/replay architecture

- **Capture**: `~/.claude/settings.json` wires `SessionStart`, `UserPromptSubmit`, `Stop`,
  and `SessionEnd` hooks to `~/.warp/agent-resume-bin/claude-capture.sh` (source:
  `tools/agent-resume/claude-capture.sh`). The hook reads `session_id`, `cwd`, event,
  permission mode, and prompt text; reconstructs launch flags from the live process argv;
  and writes one mutable pane entry through `clinch-agent-resume`.
- **Replay**: `tools/agent-resume/claude.zsh` supplies
  `clinch_agent_resume_launch`. It tries a recorded cloud bridge first, then a resumable
  local id, then the newest unclaimed local session for the cwd, then a guarded fresh
  launch.
- **Rust persistence/replay**: `app/src/agent_resume.rs` reads the same registry for fork
  UI, publishes the active-pane manifest, and reconciles mutable registry state with the
  persisted command at restore. App shutdown enqueues one final snapshot before joining
  the SQLite writer.
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

`tools/agent-resume/clinch-agent-resume` appends one JSON line to
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
- **PTY creation**: `app/src/terminal/local_tty/unix.rs` removes current and future
  `CLAUDE_CODE_*` names plus `CLAUDECODE` and `AI_AGENT` after applying shell environment
  overrides. Every new local pane therefore starts clean without an rcfile edit, while
  legitimate user behavior controls such as `CLAUDE_EFFORT` remain intact. The standalone
  replay executable also sources `claude.zsh`, whose thin wrapper applies the same narrow
  identity scrub before it invokes the provider and forwards `"$@"` verbatim.

Remote control stays enabled. Teleport also deliberately remains the first restore path
when a valid bridge id is recorded: a cloud session may have turns continued from another
device, so its cloud copy remains authoritative; local resume is the fast-failure fallback.

### 5. Discovery command

`clinch-agent-resume list [--cwd <dir>]` joins journal and prompt-mirror records and prints a
newest-first line per known conversation:

`<start-ts>  <sid-short>  <cwd>  <bridge-url-or-local>  "<first prompt>"`

Nested sessions known only to the prompt mirror are included. The command is read-only.

### 6. Root ownership and conditional exit cleanup

Claude and Codex hook processes call the registry CLI's ownership check (`hook-owner-fields`
for capture and `is-nested-agent` for exit), which walks at most 32 process ancestors. One
provider ancestor is the CLI that invoked the hook; more than one means this agent was
launched beneath another agent and cannot own the pane.
Accepted writes also store the root CLI's PID and tty. A detached/reparented tool that has
lost its outer agent from ancestry still cannot replace a different recorded owner while
that owner PID remains an agent on the recorded tty. Nested Claude prompts are still
mirrored before the ownership return.

Both providers remove an entry on `SessionEnd` through `remove-if-matches`, so a late hook
cannot delete a newer owner's mapping. Before deletion, the CLI atomically writes a private
`tombstones/<pane>` file; a new owner clears it only after its registry entry lands. The
append-only journal remains fail-open without weakening exit semantics. During app teardown,
`.app-terminating` contains the live Clinch PID; SessionEnd preserves entries only while one
marked PID is alive. A dead marker self-cleans, preventing a previous crash/quit from
suppressing unrelated future exits.

### 7. Active pane set, final snapshot, and restore reconciliation

Every full `AppState` traversal collects terminal pane UUIDs across every physical window,
project workspace, inner tab, and split leaf. The sorted/deduplicated set is atomically
written to `~/.warp/agent-resume/active-panes` whenever app state is loaded or snapshotted.
Fallback ownership scans only those pane entries; legacy installs without a manifest scan
current `*.json` entries, never the journal.

`on_will_terminate` marks shutdown, builds a fresh full app state, sends
`ModelEvent::Snapshot`, then sends `ModelEvent::Terminate`. The synchronous writer join and
channel FIFO guarantee that SQLite commits the final window/project/tab layout and newest
agent commands before termination returns.

During pane restore, `resolve_on_restore_command` applies this precedence:

1. current per-pane registry command;
2. no command when a per-pane removal tombstone exists;
3. no command when the latest journal operation is `remove` (compatibility with builds
   predating tombstones);
4. normalized command from the SQLite snapshot.

This covers both directions of skew: a hook write newer than the last UI save and a normal
agent exit newer than an older persisted command.

### 8. Fail-closed update and live migration repair

`script/update-installed-clinch` identifies the GUI by `CFBundleIdentifier` through
LaunchServices and independently scans exact executable paths. It requests quit by bundle
id, waits, sends TERM/KILL only to resolved bundle PIDs, and refuses `rm -rf`/copy if either
check still reports a running app. The public curl installer uses the same LaunchServices
plus path detection and tells the user to quit before replacement.

Before quit, the updater refreshes the active-pane manifest from SQLite, takes the bounded
forensic recovery snapshot, and runs `repair-live`. The repair maps the oldest outer root
Claude/Codex process for each pane, using ancestry, pane UUID, and process cwd. It then:

- restores cross-provider and same-provider nested takeovers from newest durable pane history;
- removes active-pane entries whose agent has actually exited; and
- removes a duplicated bridge from every active copy, then assigns it only to the copy
  whose local transcript proves ownership.

Only after repair does the updater begin shutdown; Clinch's final snapshot then persists the
repaired entries. The relaunch environment also removes `RELEASE_NOTES`, whose arbitrary
bytes must never reach Rust dependency build scripts or the relaunched app.

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
- root/nested ownership for both providers, matching/mismatching SessionEnd, and live-PID
  shutdown marker expiry;
- tombstone persistence when the journal is unwritable and clearing after a new owner lands;
- same-provider/cross-provider live repair, stale shell entries, and duplicate bridges;
- LaunchServices detection, exact-PID escalation, public-installer refusal, and fail-closed
  bundle replacement;
- Rust unit coverage for active-pane traversal/atomic publication, registry-vs-SQLite
  precedence, and final Snapshot-before-Terminate persistence, plus an integration test
  that executes a newer registry command from a restored pane whose SQLite snapshot is stale.

Manual acceptance is the `yo-durability-probe` flow in `PRODUCT.md`, including one update
started while a conversation is active and verification of the pre-quit snapshot.

## Rollout

Public downloads bundle these changes and Clinch runs the installer idempotently before its
first GUI pane opens. Source builds run the same installer as the `_bundle` prerequisite, so
the pre-quit updater uses the matching repair runtime even when upgrading from an older app.
No shell rc edit or shell restart is required; installed hook paths update in place, and new
local PTYs apply launch hygiene in Rust. One clean app relaunch removes any identity already
inherited by the old process.
