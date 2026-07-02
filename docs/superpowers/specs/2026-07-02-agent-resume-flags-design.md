# Carry session flags through agent-resume

**Status:** Design — approved
**Date:** 2026-07-02
**Author:** Elliot (personal build)
**Scope:** macOS only, personal Clinch build. Agents: Claude Code + Codex CLI.
**Builds on:** `docs/superpowers/specs/2026-06-20-warp-agent-session-resume-design.md`

## Problem

When Clinch restores panes on relaunch, it replays the per-pane registry command
(`~/.warp/agent-resume/<pane_uuid_hex>.json`), but the stored command carries no launch
flags. A session running with `--dangerously-skip-permissions` (Claude) or
`--dangerously-bypass-approvals-and-sandbox` (Codex), or a non-default `--model`, comes
back without them. The loss is permanent: the next capture reads the degraded resumed
process, which no longer has the flags either.

Three independent causes:

1. **Stale install.** Claude flag carry-over exists in the repo (commit `78ab9d685`:
   SessionStart hook walks process argv for mode + model) but was never re-installed;
   `~/.warp/agent-resume-bin/` still holds the pre-flag scripts. All 218 live registry
   entries are flag-less.
2. **Codex never captured flags.** `codex-session-start.sh` records only the session id.
3. **Argv-only capture is blind to live state.** Bypass toggled mid-session (shift+tab)
   never appears in argv, and a session that already lost its flag at a previous restart
   can never recover it from argv.

## Decisions (from brainstorming)

- **Flag scope: curated whitelist** — permission/approval mode + model. Not a carry-everything
  blacklist: the argv walk reads a flattened `ps` string, so only known space-free tokens are
  safe to re-quote. Claude: `--dangerously-skip-permissions`, `--permission-mode <m>`,
  `--model <m>`. Codex: `--dangerously-bypass-approvals-and-sandbox`, `--model <m>`
  (other codex modes only if the local binary confirms a safe flag mapping).
- **Live mode tracking: yes** — the registry reflects the session's *current* permission
  mode, not just its launch argv.

## Verified facts this design relies on

1. Claude Code `SessionStart` payloads do **not** include `permission_mode`;
   `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`, `SubagentStop`, and
   `PermissionRequest` payloads **do**. No hook fires on the mode toggle itself.
   (Source: code.claude.com/docs/en/hooks, checked 2026-07-02.)
2. Codex CLI (installed version has working `[[hooks.SessionStart]]` in
   `~/.codex/config.toml`) includes `permission_mode` and `model` in the SessionStart
   payload per developers.openai.com/codex/hooks. **Verify empirically during
   implementation** (dump one real payload) before relying on field names.
3. Claude hooks inherit the shell env (`WARP_TERMINAL_SESSION_UUID` is readable) —
   proven by the existing SessionStart hook, which keys 218 entries by pane UUID.
4. The replay function `warp_agent_resume_launch <agent> <id> [flags…]` (repo
   `claude.zsh`) already forwards trailing flags to both the resume and fresh-fallback
   paths. No replay changes needed.
5. The Rust side stores/replays the command opaquely (`read_on_restore_command`), so
   capture changes need zero Rust — **except** the fork path (below), which parses it.

## Design

### 1. Claude capture: one script, three hook events

Rename `tools/agent-resume/claude-session-start.sh` → `claude-capture.sh`. It branches on
the payload's `hook_event_name`:

- **SessionStart** — unchanged behavior: write the entry
  (`warp_agent_resume_launch claude <sid><flags>`, cwd), flags from the argv walk
  (`_warp_agent_resume_extract_flags` over `_warp_agent_resume_claude_argv`).
- **UserPromptSubmit / Stop** — recompute the entry with the payload's `permission_mode`
  as the authoritative mode; the argv walk still supplies `--model`. Mapping:
  - `bypassPermissions` → ` --dangerously-skip-permissions`
  - `plan` → ` --permission-mode plan`
  - `acceptEdits` → ` --permission-mode acceptEdits`
  - `default` → no mode flag (**toggling bypass off must also stick**)
  - absent/unknown value → fall back to the argv-derived mode flags
- **Updater guard:** on UserPromptSubmit/Stop, write only if the pane's entry is missing
  **or** its command references this payload's `session_id`; otherwise no-op. This
  self-heals degraded flag-less entries on the next prompt without letting a nested or
  stray claude (e.g. one launched from another session's Bash tool in the same pane env)
  clobber the pane's entry. SessionStart keeps its unconditional overwrite (a new session
  in the pane must take over the entry).

Shared behavior stays: only act when `WARP_TERMINAL_SESSION_UUID` is set; call
`warp-agent-resume` by absolute sibling path; functions unconditionally defined,
capture body runs only when executed (sourcing loads functions for tests).

### 2. Codex capture: payload-based

Rewrite `codex-session-start.sh`:

- Read `session_id`, `cwd`, `permission_mode`, `model` from the stdin payload.
- Map `bypassPermissions` → ` --dangerously-bypass-approvals-and-sandbox`; carry
  ` --model <m>` when present. Other mode values map only if verified against the local
  codex; unknown values carry nothing (fail-safe: plain resume).
- Fix the pre-existing bug: call `warp-agent-resume` by absolute sibling path (hooks do
  not reliably inherit the shell PATH; the current bare-name call silently no-ops when
  PATH lacks `~/.warp/agent-resume-bin`).
- If the installed codex exposes a per-turn hook that carries `permission_mode`, wire the
  same updater semantics as Claude; otherwise mid-session codex toggles are a documented
  limitation (healed only by the next SessionStart).

### 3. Replay: no changes

Repo `claude.zsh` already handles trailing flags. It only needs installing.

### 4. Rust: fix the stale fork parser

`derive_fork_command` (`app/src/agent_resume.rs`) still parses the pre-hook formats
`claude --resume <id>` / `codex resume <id>`. Every registry entry now uses the launcher
form, so "fork this session" silently returns `None` today. Update it to parse
`warp_agent_resume_launch <agent> <id> [flags…]` and emit:

- claude → `claude --resume <id> [flags…] --fork-session`
- codex → `codex fork <id> [flags…]`

Forked sessions thereby inherit the same flags. Delete the old-format parsing (dead code
— no entry has been written in that format since the SessionStart-hook rework). This is
the only Rust change; `read_on_restore_command` stays format-agnostic.

### 5. Install + staleness prevention (root-cause fix)

- `install.sh`:
  - Install `claude-capture.sh`; delete the stale `~/.warp/agent-resume-bin/claude-session-start.sh`.
  - Migrate `~/.claude/settings.json` with jq: remove any hook entry whose command points
    at the old `claude-session-start.sh` path, then idempotently add `SessionStart`,
    `UserPromptSubmit`, and `Stop` entries for `claude-capture.sh`. Never clobber
    unrelated hooks.
  - Keep the rest idempotent as today (zshrc block, codex config block).
- `Makefile`: `install-local` additionally runs `tools/agent-resume/install.sh`, so
  `make ship` always refreshes the installed capture layer. A repo-newer-than-install
  drift caused this whole bug; this closes it permanently.

## Data flow (after)

1. `ca` (= `claude --dangerously-skip-permissions`) in a pane → SessionStart → entry
   `warp_agent_resume_launch claude <sid> --dangerously-skip-permissions`.
2. Shift+tab to plan mode, send a prompt → UserPromptSubmit(`permission_mode: "plan"`) →
   entry rewritten with ` --permission-mode plan`.
3. Clinch quits; snapshot froze the entry; relaunch replays
   `warp_agent_resume_launch claude <sid> --permission-mode plan` → session resumes in
   plan mode.
4. A pre-existing degraded session (no flags in argv, bypass re-enabled via shift+tab):
   next prompt → UserPromptSubmit(`bypassPermissions`) matches the entry's sid → entry
   healed with the flag.

## Edge cases

- **Space-free guarantee:** whitelist tokens (`--permission-mode plan`, `--model opus`)
  contain no spaces needing quoting; arbitrary flags are deliberately not carried.
- **Toggle then quit with no further activity:** neither UserPromptSubmit nor Stop fired
  → mode change lost. Accepted (Stop covers toggle-then-prompt, the common case).
- **`--model` given as a full id or alias:** carried verbatim from argv; payload `model`
  (a resolved id) is not used for the flag, preserving user intent.
- **Non-Warp shells / headless `-p` / subagents:** unchanged guards
  (`WARP_TERMINAL_SESSION_UUID` absent → no-op; `-p` runs never had entries; subagent
  events are `SubagentStop`, not `Stop`, so they don't hit the updater).
- **Old entries in the wild:** flag-less entries still replay fine (launcher accepts zero
  trailing flags) and heal on first prompt after install.
- **Fork of a flag-less legacy entry:** parser accepts zero flags; fork works again
  (it is broken today).

## Testing

- **Shell (existing conventions:** temp `WARP_AGENT_RESUME_DIR`, `WARP_AGENT_RESUME_FAKE_ARGV`):
  - `test_claude_hook.sh`: UserPromptSubmit/Stop rewrite with payload mode; `default`
    strips mode flags; unknown mode falls back to argv; session-id guard (mismatched sid
    → no write; missing entry → write); self-heal of a flag-less entry; SessionStart
    still overwrites unconditionally.
  - `test_codex_hooks.sh`: payload `permission_mode`/`model` mapping; absent fields →
    plain command; absolute-path invocation.
  - Rename-related: whatever referenced `claude-session-start.sh` in tests moves to
    `claude-capture.sh`.
- **Rust** (`app/src/agent_resume_tests.rs`, `cargo nextest`): `derive_fork_command`
  parses launcher form with/without flags for both agents; rejects unknown agents/empty
  ids; old raw formats now return `None` (dead format).
- **Manual:** `ca` → restart Clinch → bypass survives. Shift+tab toggle → one prompt →
  restart → new mode survives. Codex with bypass → restart → flag survives. Fork a
  bypass session → fork opens in bypass.

## Outdated code removed

- Old-format parsing in `derive_fork_command` (dead since the SessionStart-hook rework).
- `claude-session-start.sh` (renamed; stale installed copy deleted; stale
  `settings.json` hook entries migrated by `install.sh`).
- The comment in the capture script claiming the permission mode is unavailable to hooks
  is corrected to "SessionStart payloads lack it; per-turn payloads carry it".

## Explicitly deferred (YAGNI)

- Carrying arbitrary flags (`--add-dir`, `--allowedTools`, `--mcp-config`, codex `-c`
  overrides).
- A codex per-turn updater if the installed codex has no such hook.
- Registry GC sweep (pre-existing deferral).
