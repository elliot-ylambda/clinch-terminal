# Agent session resume on pane restore

Make Clinch re-launch your Claude Code / Codex sessions when it restores tabs after a
quit/relaunch. Clinch restores each pane's layout + cwd as usual; this adds: capture the
agent session that was running in each pane, and on restore re-run its exact resume
command (`claude --resume <id>` / `codex resume <id>`) once the shell finishes booting.

macOS only. The complete runtime is bundled in public Clinch releases.

## How it works

```
capture (agent SessionStart hooks)          replay (Rust, in Clinch)
  claude hook     ─┐                          snapshot(): read registry[pane_uuid]
  codex hooks     ─┼─► ~/.warp/agent-resume/    → freeze command into the pane snapshot
                   │     <pane_uuid>.json        → persisted in SQLite (terminal_panes)
                   │     { "command", "cwd",             │
                   │       "bridge"? }                   │
                   └────────────────────────►   restore: after the shell's first
                                                 Bootstrapped event, run the command
```

- **Capture is driven by agent hooks** — `SessionStart` and conditional `SessionEnd` for
  both Claude and Codex. Both providers mirror `UserPromptSubmit`; Claude also refreshes
  the pane entry on `UserPromptSubmit`/`Stop`, while Codex's prompt-only helper deliberately
  leaves its SessionStart flags untouched. The hooks read the *actual* live `session_id` from
  their payload and the pane UUID
  from the inherited `WARP_TERMINAL_SESSION_UUID` env var. This captures the right session
  in every case —
  fresh start, `--resume <id>`, the interactive picker, and `--continue` — because the
  hook runs *after* the agent has decided which session is live. (An earlier `claude()`
  shell wrapper that tried to capture the id before launch was removed because it missed
  the picker and `--continue`; normal pane creation now scrubs inherited session identity,
  while the standalone replay wrapper handles replay launches.)
- **Only the outermost agent owns the pane.** Hooks combine Claude/Codex process ancestry
  with the recorded root PID/tty, so even a detached/reparented nested tool cannot replace
  a different live owner. Nested tools still receive their own prompt mirror, but cannot
  overwrite or remove the visible outer session's entry. `SessionEnd` removes only a
  matching owner. During app shutdown a live-PID marker preserves mappings while PTYs exit,
  and a stale marker self-cleans after the app is gone.
- **Key = the pane UUID** (`WARP_TERMINAL_SESSION_UUID`), which is stable across
  quit/restore and unique per tab — so multiple agents in the *same directory* are
  disambiguated (a directory-based scheme can't do that).
- **The recorded command self-heals**: it is `clinch_agent_resume_launch <agent> <id>`,
  a shell function (sourced from `claude.zsh`) that resumes the session only if it has a
  real conversation on disk, and otherwise starts a *fresh* agent in that pane. This
  matters because the command is captured eagerly at launch (you usually quit with the
  agent still running), so it can point at a session that was opened but never used — and
  `claude --resume`/`codex resume` reject those with "No conversation found". Resumability
  is checked by locating the session file by its globally-unique id, so we never replicate
  each agent's brittle cwd→directory hashing.
  Entries captured before the Clinch rename are normalized when read, and a legacy shell
  alias remains installed so the first restore after an update cannot strand an older pane.
- **A dead Claude id adopts the directory's newest unclaimed session before going fresh.**
  Registry entries rot silently (overwritten by a session that was never used, transcript
  rolled away by retention), and starting fresh on a rotted entry silently orphans the
  pane's real conversation — the root cause of the 2026-07-08 blank-session incident. So
  when the recorded id has no conversation, `clinch_agent_resume_launch` first looks for the
  newest resumable session whose transcript records the pane's directory as its `cwd`
  (`clinch_agent_resume_fallback_id`, reading transcript heads newest-first) and resumes
  that instead. Only *unclaimed* sessions — recorded in no pane's registry entry — are
  adopted, so a pane whose id died can never steal a sibling pane's live session when
  several panes share one project directory. An atomic `.adopt-claim-<session>` file also
  closes the restore-time race before the adopted session's hook can write its registry entry;
  abandoned claims expire after 120 seconds. Bridged panes skip adoption (their
  conversation lives at claude.ai; adopting a local session would silently swap
  conversations), and codex keeps plain resume-or-fresh (its session filenames don't
  yield a bare id to adopt by).
- **Machinery-spawned blanks can't destroy a recoverable mapping.** When the launcher
  does start fresh, it tags the session with `WARP_AGENT_RESUME_STARTED_FRESH`; the
  capture hook then skips its usual SessionStart takeover while the pane's entry still
  points somewhere recoverable (a local session with a real conversation, or a claude.ai
  bridge id — the only durable link to a cloud conversation) and the new session has no
  conversation of its own yet. The session's first real prompt claims the pane as usual.
  In the incident above every blank restart overwrote the previous mapping at
  SessionStart, so freeze/restore cycles amplified one stale id into total loss; with
  this guard a protected entry survives any number of blank restarts. A user-started
  fresh session (no marker) still takes over unconditionally.
- **Restore reconciles mutable capture with persisted layout.** Clinch atomically publishes
  the pane UUIDs from every physical window, project, inner tab, and split. Closed/zombie
  entries cannot claim fallback sessions. At replay, a newer registry command wins over
  SQLite, while a required per-pane removal tombstone prevents an older snapshot from
  resurrecting an agent the user exited even if the best-effort journal was unwritable.
  Graceful app termination queues one final complete snapshot before the SQLite writer is
  synchronously joined.
- **Local history wins; bridges recover cloud-only sessions.** Claude `--resume` keeps the
  original session id and repaints the complete locally saved exchange. In contrast, a
  `--teleport` can restore context into a new local id without repainting that exchange, which
  looks like a blank resumed pane. The capture hook
  records the session's `CLAUDE_CODE_BRIDGE_SESSION_ID` in the entry's optional `bridge`
  field. Restore runs `claude --resume <id>` whenever that transcript has a real turn and uses
  `claude --teleport <bridge>` only when local history is absent or unusable. This favors visible
  local continuity; a bridge continued exclusively on another device can still be recovered from
  its recorded `https://claude.ai/code/<bridge>` URL. A
  teleport that fails fast (dirty tree, git lock race between panes sharing a repo, API
  error) falls back to fresh recovery, printing
  `https://claude.ai/code/<bridge>` for manual recovery; a non-zero exit after a real run
  is the user quitting and does not relaunch (`WARP_AGENT_RESUME_TELEPORT_GRACE`, default
  15s, distinguishes the two). Note teleport performs git operations (branch checkout), so
  restore of a bridged pane can move a shared checkout's branch — worktree-per-session
  layouts avoid this.
- **Permission mode + model are carried over.** A session running with a non-default
  permission mode (`--dangerously-skip-permissions`, e.g. the `CA` alias, or
  `--permission-mode <mode>`) or a `--model` reopens *the same way* — those flags are
  appended to the recorded `clinch_agent_resume_launch <agent> <id> …` and forwarded on
  resume.
  - *Claude*: `SessionStart`'s payload doesn't include the permission mode, so at launch
    the hook reads it off the live `claude` process argv (the alias expands before exec),
    matching on the flags themselves rather than the string "claude" so a plain launch
    carries nothing. The same script is also wired to `UserPromptSubmit` and `Stop`,
    whose payloads *do* carry `permission_mode` — so a mode toggled mid-session
    (shift+tab) sticks, toggling back to default strips the flag again, and entries
    written by older flag-less installs heal on the session's next prompt.
  - *Codex*: the `SessionStart` payload carries `permission_mode` and `model` directly;
    `bypassPermissions` maps to `--dangerously-bypass-approvals-and-sandbox`. Its separate
    `UserPromptSubmit` helper only appends history and cannot strip those recorded flags.
- **Clinch stays agent-agnostic**: Rust only stores/replays an opaque command string.
  Adding another agent later is just another capture script — no Rust change.

## Durability: journal and prompt mirror

Pane entries are single mutable files — the next session in a pane overwrites them — and
a Claude launch poisoned by inherited child-session identity writes no local transcript at
all. Twice (2026-07-08, 2026-07-09) an overwrite destroyed the only pointer to a live
conversation. Launch hygiene plus two durable layers reduce that risk (see
`specs/claude-transcript-durability/`):

- **Launch hygiene**: local PTY creation strips inherited `CLAUDE_CODE_*` identity after
  environment overrides, with no rcfile edit, while
  preserving user behavior toggles. The standalone replay wrapper also preserves argv
  exactly. Remote control remains enabled.

- **Registry journal** (`~/.warp/agent-resume/journal.jsonl`): every `write`/`remove` the
  CLI performs appends one line (`ts`, `op`, `pane`, `command`, `cwd`, `bridge`), so any
  historically recorded (pane, session, bridge, cwd) tuple stays greppable forever —
  `grep <sid> ~/.warp/agent-resume/journal.jsonl` recovers an overwritten pointer.
  Fail-open: a journal append failure never fails the registry mutation. No pruning in v1
  (~200 bytes/line).
- **Prompt mirror** (`~/.warp/agent-resume/prompts/<provider>/<sid>.jsonl`): every
  `UserPromptSubmit` appends the prompt text (+ ts, cwd, bridge id), and Claude `Stop` appends a
  turn boundary — so even a session that never writes a
  jsonl leaves its prompts on disk. Mirrored *before* the pane-ownership guard, so nested
  sessions' prompts survive too; capped at ~5 MB per session (one final
  `"truncated":true` marker). Identical retained-input submissions coalesce only while the same
  turn is open; the Stop boundary preserves an intentional identical prompt after an answer.
  These files are as sensitive as `~/.claude/projects`
  transcripts: `700`/`600`, never leave the machine.
  Legacy flat `prompts/<sid>.jsonl` files remain readable as Claude history; new writes are
  provider-scoped so the same id can never be attributed to the wrong agent.
- **Discovery**: `clinch-agent-resume list [--cwd <dir>] [--json]` prints a newest-first table of
  every conversation the journal + mirror know about — start time, short sid, cwd,
  `https://claude.ai/code/<bridge>` or `local`, and the first prompt. `--json` returns the
  same aggregation with full session IDs and exact prompt text for machine consumers.
- **Bridge cleanup**: `clinch-agent-resume scrub-bridge <bridge-id>` structurally removes an
  inherited/leaked bridge from matching pane entries and journals the clear so discovery does
  not resurrect the poisoned cloud URL.

## Default session capture and opt-out

Clinch enables session capture on first launch so local Claude Code and Codex restore works by
default. Users can turn it off or back on from Clinch Settings. The equivalent repository command
is:

```bash
./tools/agent-resume/install.sh enable
```

Installs the capture hooks + the registry CLI into `~/.warp/agent-resume-bin/`, and wires:
the Claude `SessionStart`/`UserPromptSubmit`/`Stop`/`SessionEnd` hooks into `~/.claude/settings.json`
(via `wire-claude-hooks.sh`, a structural JSON merge — existing settings are preserved, and entries from
  the pre-rename `claude-session-start.sh` are migrated), the Codex
  `SessionStart`/`UserPromptSubmit`/`SessionEnd`
hooks into a managed block in `~/.codex/config.toml`, and standalone replay executables into
Clinch's bundled shell path and the installed runtime directory. It uses macOS's built-in JXA
runtime for JSON handling. There is no `jq`, Homebrew, repository-clone, `.zshrc`, or shell-restart
requirement. On later launches Clinch runs `repair --quiet` while enabled. `disable` writes a
durable opt-out marker, and older disabled installations are recognized by their retained receipt.

The hooks only record when launched inside a Clinch pane (`WARP_TERMINAL_SESSION_UUID` set).
New Claude and Codex sessions are captured immediately.

Remove only managed hooks and helpers while keeping captured metadata with:

```bash
./tools/agent-resume/install.sh disable
```

Use `purge` instead only when you also intend to delete `~/.warp/agent-resume/`. No command prints
help and changes nothing. Notification plugins are a separate provider action; this manager never
installs them.

**Desktop notifications:** on a fresh profile the first agent status change shows an
in-app banner to enable notifications; enable it once, after which background-pane
completions/questions push to macOS.

## Build the app (replay layer)

```bash
./tools/agent-resume/build-app.sh
```

Builds the **OSS-channel** client with this feature compiled in, names it "Clinch"
(set `CLINCH_NAME` to change), and installs it to `/Applications`. The rebrand covers the
display name and the bundle id (`sh.clinch.Clinch`). It co-installs alongside your
downloaded Warp: different bundle id (`sh.clinch.Clinch` vs `dev.warp.Warp-Stable`) and a
separate data dir (`~/.warp-oss`), so the two never clobber each other's session state.

## What survives what

| Scenario | Resumes? |
|---|---|
| `Cmd-Q` / normal reopen | ✅ when the provider transcript and capture mapping remain available |
| Crash | ⚠️ best-effort — Clinch has **no periodic autosave**; pane state is saved on UI mutations and flushed at quit (`on_will_terminate`). A crash recovers only what was last saved. No worse than today. |
| Machine reboot | resumes the *conversation* (`--resume`), never the live process — that's physics, not a bug. |

## Known limitations

- **Graceful-exit behavior:** the Claude hook does *not* remove the registry entry when a
  session ends (only overwrites it when the next session starts in that pane). This is the
  safe default — it guarantees the entry is present when Clinch snapshots at quit (you
  usually quit with the agent still running). The cost is that a session you closed may
  reopen on the next restore. Removing on exit would risk the opposite, worse failure: the
  entry vanishing before Clinch snapshots, so a session you *were* using doesn't come back.
  (Codex removes on `SessionEnd`; that race is pre-existing and accepted there.)
- **`claude --print` / `-p` is also captured.** The hook can't tell a one-off print
  invocation from an interactive session, so a pane whose last Claude activity was a
  `claude -p` may reopen that conversation on restore. Harmless — you can exit it — and an
  interactive session started afterward overwrites the entry.
- **Stub / vanished sessions resume as fresh, not as an error.** A pane whose agent was
  opened but never used has no resumable conversation (0 turns), and a session file can
  also be rolled away. Rather than replaying a bare `claude --resume <id>` that errors
  with "No conversation found", the recorded `clinch_agent_resume_launch` checks first,
  tries the cwd adoption fallback (Claude only, see above), and otherwise starts a fresh
  agent in that pane. The trade-off is a (rare) false negative: if the resumability check
  can't find a conversation that actually exists, you get a fresh agent and can still
  `claude --resume <id>` by hand.
- **A session claimed by a zombie registry entry is not adopted.** The cwd fallback skips
  sessions recorded in *any* pane's registry entry so it can never steal a live sibling
  pane's session — at the cost of not adopting one claimed by an entry whose pane no
  longer exists. Those stay recoverable by hand (`claude --resume <id>`), and are no
  worse off than before the fallback existed.
- **Nested claude runs inside a machinery-spawned fresh session** inherit
  `WARP_AGENT_RESUME_STARTED_FRESH`, so such a nested run's first prompt event can claim
  the pane entry (the marker relaxes the usual owner guard). This needs a machinery-spawned
  blank *and* a nested interactive-hook claude run before the user's first prompt — rare
  double condition, accepted.
- **Codex mode changes made mid-session are only re-captured at the next session start.**
  Claude's per-turn hooks keep the recorded mode live, but codex is captured at
  `SessionStart` only for resume flags. The Codex prompt hook is intentionally history-only,
  so it cannot rewrite a flagged entry flag-less. A Claude mode toggled *after* its last
  prompt/turn (then quitting Clinch with no further activity) is similarly missed — the common
  toggle-then-prompt flow is covered by `UserPromptSubmit`.
- **Codex prompt-hook schema baseline:** capture is fixture-tested against `codex-cli 0.144.3`'s
  `UserPromptSubmit` stdin object: `session_id`, `turn_id`, optional `agent_id`/`agent_type`,
  `transcript_path`, `cwd`, `hook_event_name`, `model`, `permission_mode`, and `prompt`. The
  helper consumes prompt text only from stdin and fails open if a future payload is missing the
  required event, session, or prompt fields; native rollout history remains the fallback.

## Files

| File | Role |
|---|---|
| `agent-json` / `agent-json.js` | native macOS JSON parsing, encoding, settings merge, and conversation listing without third-party runtimes |
| `clinch-agent-resume` | registry CLI: `write`, `remove`, `scrub-bridge`, and `list [--cwd <dir>] [--json]`; journals every mutation to `journal.jsonl` |
| `claude-capture.sh` | Claude `SessionStart`/`UserPromptSubmit`/`Stop` hook — captures the live session per pane and keeps its permission-mode/`--model` flags in sync |
| `prompt-mirror.sh` | shared private, capped append-only writer for provider-scoped prompt history; prompt payload stays on stdin |
| `claude.zsh` | Claude launch-identity scrub + replay functions (`clinch_agent_resume_resumable` / `clinch_agent_resume_launch`) loaded by the standalone launcher |
| `clinch_agent_resume_launch` | executable replay entrypoint bundled in `Clinch.app/Contents/Resources/bin`; works without an rcfile edit |
| `codex-session-start.sh` / `codex-prompt-submit.sh` / `codex-session-end.sh` | Codex registry lifecycle plus prompt-only history capture |
| `config.toml.snippet` | Codex hook registration (installer applies it) |
| `install-agent-plugins.sh` | legacy/manual development helper; not bundled or run by the public installer |
| `wire-claude-hooks.sh` | idempotent native merge of Claude hook entries into settings.json (used by `install.sh`) |
| `unwire-claude-hooks.sh` | remove only Clinch/legacy Warp managed Claude hook entries |
| `install.sh` | `enable`, enabled-state `repair`, persistent `disable`, `status`, and `purge` manager |
| `build-app.sh` | build + brand + install the co-installable app |
| `tests/` | self-contained shell tests for the scripts |

Rust side: `app/src/agent_resume.rs` (registry reader), the `on_restore_command` field
on `TerminalPaneSnapshot`, the `terminal_panes.on_restore_command` column, and the
replay in `app/src/pane_group/mod.rs` + `pty_controller.rs`.
