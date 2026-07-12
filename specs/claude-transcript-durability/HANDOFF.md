# Claude Transcript Durability — Handoff

Status as of 2026-07-09 19:30 PT. Read `PRODUCT.md` and `TECH.md` first; they now describe
the proved root cause and the implementation that is actually in the working tree.

## Repository implementation: complete

All five TECH changes, including the root fix, are implemented. On 2026-07-09 the user
authorized shipping this scoped change directly to `clinch/main` without a PR:

| Change | Implementation | State |
|---|---|---|
| Append-only registry journal | `tools/agent-resume/clinch-agent-resume` journals every effective write/remove and provides `list [--cwd]` | done |
| Prompt mirror | `tools/agent-resume/claude-capture.sh` mirrors non-empty prompts before the pane-ownership guard, private and capped | done |
| Pre-quit snapshot | `script/update-installed-clinch` snapshots registry/mirrors/journal/referenced transcripts and marker-prunes to 15 | done |
| App relaunch scrub | sourceable `clinch_scrubbed_open` dynamically removes every exported `CLAUDE_CODE_*` plus related Claude/Make variables before `open` | done |
| Interactive launch scrub | `tools/agent-resume/claude.zsh` wraps `claude` and removes only stale identity/implementation markers while preserving argv and behavior flags | done |
| Discovery | `clinch-agent-resume list [--cwd <dir>]`, newest first with bridge URL/local marker and first prompt | done |
| Docs/spec alignment | `PRODUCT.md`, `TECH.md`, and `tools/agent-resume/README.md` now describe leaked child identity—not bridging itself—as the cause | done |

Two post-root-fix decisions required by TECH are explicit in code comments:

- Prompt mirroring remains unconditional as cheap corruption/nested-session insurance.
- Teleport remains first for a recorded bridge because remotely continued turns can make
  the cloud copy newer than the local jsonl; local resume remains the fast-failure fallback.

## Root cause: proved

The 2026-07-09 13:53 `make update` ran inside a Claude session. Its detached updater
inherited stale `CLAUDE_CODE_SESSION_ID`, `CLAUDE_CODE_BRIDGE_SESSION_ID`,
`CLAUDE_CODE_CHILD_SESSION`, `CLAUDECODE`, `AI_AGENT`, and Make/update variables. `open`
forwarded them into the relaunched Clinch app, every pane inherited them, and every new
Claude launch behaved as a child of that stale bridged session.

A controlled interactive A/B completed a real turn in both cases:

- clean environment + remote control at startup → normal local jsonl;
- stale Claude identity → no local jsonl.

Therefore `remoteControlAtStartup: true` is wanted and innocent; do not disable it.
`CLAUDE_CODE_CHILD_SESSION=1` is exported by Claude to its real children, not injected by
Clinch. There is no supported Claude setting needed for this fix: the fix is launch
environment hygiene.

## Verification completed

- All 10 `tools/agent-resume/tests/test_*.sh` files pass, including:
  - `test_registry_journal.sh` (new);
  - prompt mirror/size/ownership coverage in `test_claude_hook.sh`;
  - identity scrub + exact argv coverage in `test_claude_launch.sh`;
  - `test_update_env_scrub.sh` (new), including an arbitrary future
    `CLAUDE_CODE_FUTURE_ID` to prove the app scrub is dynamic.
- Bash/zsh syntax checks pass.
- `git diff --check` passes for all scoped files.
- Executing `script/update-installed-clinch` without arguments still fails with the
  documented usage error; sourcing it is now side-effect-free for tests.
- `/bin/bash` is macOS 3.2.57 and successfully runs the `compgen -e`/array implementation.
- The real snapshot helper ran successfully and created:
  `~/Library/Application Support/sh.clinch.Clinch/session-recovery-20260709-192708/`.
  It has `.auto-snapshot`, a copied registry, and the journal.
- `bash tools/agent-resume/install.sh` ran successfully in a scrubbed environment. Installed
  copies of the registry CLI, Claude capture hook, Claude zsh integration, and Codex hooks
  byte-match the repository versions.
- Static spec validation found no material PRODUCT/TECH mismatch. No cloud/computer-use
  validation was run because this feature has no visual UI; the real lifecycle acceptance
  below is still pending.

## Live-machine state: one deliberate operator step remains

The running `/Applications/Clinch.app` process is still the instance launched at
2026-07-09 13:53:36. Its process environment still contains the stale Claude identity and
Make variables. It was **not quit from this agent session**, because doing so would kill
active panes and this handoff. The new shell/capture integration is installed, but the app
itself remains contaminated until one clean relaunch.

Safe next action:

1. Save/finish work in active panes.
2. Quit Clinch normally and reopen it from Dock/Finder. The real recovery snapshot above
   was created immediately before this handoff, so current registry state is backed up.
3. Open a new pane (so it sources the newly installed `claude.zsh`).

Running the fixed `make update` also cleans the relaunch, but do that only from the intended
release branch/build. This checkout is a shared dirty branch and should not be used for a
release update as-is.

## Manual acceptance still pending

After the clean relaunch, run PRODUCT's real lifecycle probe:

1. In a new Clinch pane, run `claude`, submit `yo-durability-probe`, and wait for the reply.
2. Confirm a local `~/.claude/projects/**/<sid>.jsonl` exists and contains a real turn.
3. Confirm `grep -rw "yo-durability-probe" ~/.warp/agent-resume/` finds the prompt mirror
   and the journal has the session/cwd/bridge pointer.
4. Quit/reopen and confirm the pane restores via teleport or local resume.
5. On the eventual clean durability branch, run one `make update` with an active probe and
   verify the pre-quit automatic snapshot plus post-update restore.

Do not run CLI probes with the real `WARP_TERMINAL_SESSION_UUID` unless pane takeover is
intended; `claude -p` hooks can overwrite that pane's mutable registry entry. Unset the UUID
for throwaway probes.

## Direct-main shipping scope

The source checkout is shared and sits on `codex/launch-readiness-fixes` with many unrelated
dirty Rust/project-tabs changes. The direct-main commit must be based on **`clinch/main`**
(not `origin/master`) and contain only these paths:

- `script/update-installed-clinch`
- `tools/agent-resume/README.md`
- `tools/agent-resume/clinch-agent-resume`
- `tools/agent-resume/claude-capture.sh`
- `tools/agent-resume/claude.zsh`
- `tools/agent-resume/tests/test_registry_journal.sh`
- `tools/agent-resume/tests/test_update_env_scrub.sh`
- `tools/agent-resume/tests/test_claude_hook.sh`
- `tools/agent-resume/tests/test_claude_launch.sh`
- `tools/agent-resume/tests/test_codex_hooks.sh`
- `specs/claude-transcript-durability/`

Suggested changelog marker:

`CHANGELOG-BUG-FIX: Claude conversations started in Clinch panes are now recoverable from an append-only journal, local prompt mirror, and pre-update snapshot, and no longer lose local transcripts when an update inherits stale Claude session state.`

No Rust is part of this feature. Validate the exact main-based commit in an isolated
worktree before a normal fast-forward push; never stage or reset the shared checkout.
