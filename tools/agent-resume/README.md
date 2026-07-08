# Agent session resume on pane restore

Make Warp re-launch your Claude Code / Codex sessions when it restores tabs after a
quit/relaunch. Warp restores each pane's layout + cwd as usual; this adds: capture the
agent session that was running in each pane, and on restore re-run its exact resume
command (`claude --resume <id>` / `codex resume <id>`) once the shell finishes booting.

macOS only. Personal build.

## How it works

```
capture (agent SessionStart hooks)          replay (Rust, in Warp)
  claude hook     ─┐                          snapshot(): read registry[pane_uuid]
  codex hooks     ─┼─► ~/.warp/agent-resume/    → freeze command into the pane snapshot
                   │     <pane_uuid>.json        → persisted in SQLite (terminal_panes)
                   │     { "command", "cwd",             │
                   │       "bridge"? }                   │
                   └────────────────────────►   restore: after the shell's first
                                                 Bootstrapped event, run the command
```

- **Capture is driven by agent hooks** — `SessionStart` for both Claude and Codex (Claude
  additionally refreshes the entry on `UserPromptSubmit`/`Stop`, see the flags bullet
  below). The hook reads the *actual* live `session_id` from its payload and the pane UUID
  from the inherited `WARP_TERMINAL_SESSION_UUID` env var. This captures the right session
  in every case —
  fresh start, `--resume <id>`, the interactive picker, and `--continue` — because the
  hook runs *after* the agent has decided which session is live. (An earlier `claude()`
  shell wrapper had to guess the id before launch, so it silently missed the picker and
  `--continue`; it was removed.)
- **Key = the pane UUID** (`WARP_TERMINAL_SESSION_UUID`), which is stable across
  quit/restore and unique per tab — so multiple agents in the *same directory* are
  disambiguated (a directory-based scheme can't do that).
- **The recorded command self-heals**: it is `warp_agent_resume_launch <agent> <id>`,
  a shell function (sourced from `claude.zsh`) that resumes the session only if it has a
  real conversation on disk, and otherwise starts a *fresh* agent in that pane. This
  matters because the command is captured eagerly at launch (you usually quit with the
  agent still running), so it can point at a session that was opened but never used — and
  `claude --resume`/`codex resume` reject those with "No conversation found". Resumability
  is checked by locating the session file by its globally-unique id, so we never replicate
  each agent's brittle cwd→directory hashing.
- **A dead Claude id adopts the directory's newest unclaimed session before going fresh.**
  Registry entries rot silently (overwritten by a session that was never used, transcript
  rolled away by retention), and starting fresh on a rotted entry silently orphans the
  pane's real conversation — the root cause of the 2026-07-08 blank-session incident. So
  when the recorded id has no conversation, `warp_agent_resume_launch` first looks for the
  newest resumable session whose transcript records the pane's directory as its `cwd`
  (`warp_agent_resume_fallback_id`, reading transcript heads newest-first) and resumes
  that instead. Only *unclaimed* sessions — recorded in no pane's registry entry — are
  adopted, so a pane whose id died can never steal a sibling pane's live session when
  several panes share one project directory. Bridged panes skip adoption (their
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
- **Bridged sessions teleport their cloud copy back.** Claude sessions attached to the
  claude.ai "repl bridge" stop persisting a full local transcript (the jsonl is missing or
  a stale husk), and `claude --resume <id>` then fails with "No conversation found" or
  silently resumes the husk (verified on 2.1.202 — resume is local-only). The capture hook
  records the session's `CLAUDE_CODE_BRIDGE_SESSION_ID` in the entry's optional `bridge`
  field, and on restore `warp_agent_resume_launch` runs `claude --teleport <bridge>`
  instead of a local resume — the cloud copy is authoritative for bridged sessions. A
  teleport that fails fast (dirty tree, git lock race between panes sharing a repo, API
  error) falls back to local resume, then fresh, printing
  `https://claude.ai/code/<bridge>` for manual recovery; a non-zero exit after a real run
  is the user quitting and does not relaunch (`WARP_AGENT_RESUME_TELEPORT_GRACE`, default
  15s, distinguishes the two). Note teleport performs git operations (branch checkout), so
  restore of a bridged pane can move a shared checkout's branch — worktree-per-session
  layouts avoid this.
- **Permission mode + model are carried over.** A session running with a non-default
  permission mode (`--dangerously-skip-permissions`, e.g. the `CA` alias, or
  `--permission-mode <mode>`) or a `--model` reopens *the same way* — those flags are
  appended to the recorded `warp_agent_resume_launch <agent> <id> …` and forwarded on
  resume.
  - *Claude*: `SessionStart`'s payload doesn't include the permission mode, so at launch
    the hook reads it off the live `claude` process argv (the alias expands before exec),
    matching on the flags themselves rather than the string "claude" so a plain launch
    carries nothing. The same script is also wired to `UserPromptSubmit` and `Stop`,
    whose payloads *do* carry `permission_mode` — so a mode toggled mid-session
    (shift+tab) sticks, toggling back to default strips the flag again, and entries
    written by older flag-less installs heal on the session's next prompt.
  - *Codex*: the `SessionStart` payload carries `permission_mode` and `model` directly;
    `bypassPermissions` maps to `--dangerously-bypass-approvals-and-sandbox`.
- **Warp stays agent-agnostic**: Rust only stores/replays an opaque command string.
  Adding another agent later is just another capture script — no Rust change.

## Install (capture layer)

```bash
./tools/agent-resume/install.sh
# then restart your shell (or: source ~/.zshrc) to load the replay functions
```

Installs the capture hooks + the registry CLI into `~/.warp/agent-resume-bin/`, and wires:
the Claude `SessionStart`/`UserPromptSubmit`/`Stop` hooks into `~/.claude/settings.json`
(via `wire-claude-hooks.sh`, a jq merge — existing settings are preserved, and entries from
the pre-rename `claude-session-start.sh` are migrated), the Codex `SessionStart`/`SessionEnd`
hooks into `~/.codex/config.toml`, and the replay functions into `~/.zshrc`. Requires `jq`
(`brew install jq` if needed). `make release` runs this installer automatically so the
installed copies never drift from the repo.

The hooks only record when launched inside a Warp pane (`WARP_TERMINAL_SESSION_UUID` set).
New Claude sessions are captured immediately — no shell restart needed for capture; the
restart only loads the replay functions used on the next restore.

The installer also runs `install-agent-plugins.sh`, which installs Warp's CLI-agent
notification plugins into Claude (`warp@claude-code-warp`) and Codex (`warp@codex-warp`).
These make the agents emit OSC-777 status events that Clinch turns into tab badges and
desktop notifications. It is best-effort (skips a missing CLI, warns and continues if
offline) and requires restarting the agent to take effect. Removing the plugins
(`claude plugin uninstall warp@claude-code-warp`) disables the badges/notifications;
everything else keeps working.

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
| `Cmd-Q` / update-and-restart | ✅ yes |
| Crash | ⚠️ best-effort — Warp has **no periodic autosave**; pane state is saved on UI mutations and flushed at quit (`on_will_terminate`). A crash recovers only what was last saved. No worse than today. |
| Machine reboot | resumes the *conversation* (`--resume`), never the live process — that's physics, not a bug. |

## Known limitations

- **Graceful-exit behavior:** the Claude hook does *not* remove the registry entry when a
  session ends (only overwrites it when the next session starts in that pane). This is the
  safe default — it guarantees the entry is present when Warp snapshots at quit (you
  usually quit with the agent still running). The cost is that a session you closed may
  reopen on the next restore. Removing on exit would risk the opposite, worse failure: the
  entry vanishing before Warp snapshots, so a session you *were* using doesn't come back.
  (Codex removes on `SessionEnd`; that race is pre-existing and accepted there.)
- **`claude --print` / `-p` is also captured.** The hook can't tell a one-off print
  invocation from an interactive session, so a pane whose last Claude activity was a
  `claude -p` may reopen that conversation on restore. Harmless — you can exit it — and an
  interactive session started afterward overwrites the entry.
- **Stub / vanished sessions resume as fresh, not as an error.** A pane whose agent was
  opened but never used has no resumable conversation (0 turns), and a session file can
  also be rolled away. Rather than replaying a bare `claude --resume <id>` that errors
  with "No conversation found", the recorded `warp_agent_resume_launch` checks first,
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
  `SessionStart` only: the local codex does expose a `user_prompt_submit` hook event, but
  its payload fields are unverified, and wiring it blind could rewrite a flagged entry
  flag-less. A Claude mode toggled *after* its last prompt/turn (then quitting Warp with
  no further activity) is similarly missed — the common toggle-then-prompt flow is
  covered by `UserPromptSubmit`.

## Files

| File | Role |
|---|---|
| `warp-agent-resume` | registry CLI: `write <uuid> <cmd> <cwd> [bridge]` / `remove <uuid>` |
| `claude-capture.sh` | Claude `SessionStart`/`UserPromptSubmit`/`Stop` hook — captures the live session per pane and keeps its permission-mode/`--model` flags in sync with the live session |
| `claude.zsh` | replay functions (`warp_agent_resume_resumable` / `warp_agent_resume_launch`) |
| `codex-session-start.sh` / `codex-session-end.sh` | Codex hooks — session id plus bypass/model flags from the payload |
| `config.toml.snippet` | Codex hook registration (installer applies it) |
| `install-agent-plugins.sh` | install Warp's Claude/Codex notification plugins (emit the OSC-777 status events) |
| `wire-claude-hooks.sh` | idempotent jq merge of the Claude hook entries into settings.json (used by `install.sh`) |
| `install.sh` | install capture hooks + replay functions into the shell/agent config |
| `build-app.sh` | build + brand + install the co-installable app |
| `tests/` | self-contained shell tests for the scripts |

Rust side: `app/src/agent_resume.rs` (registry reader), the `on_restore_command` field
on `TerminalPaneSnapshot`, the `terminal_panes.on_restore_command` column, and the
replay in `app/src/pane_group/mod.rs` + `pty_controller.rs`.
