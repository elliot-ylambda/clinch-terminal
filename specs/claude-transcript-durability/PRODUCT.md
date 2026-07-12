# Claude Conversation Durability in Clinch Panes

## Summary
Every Claude Code conversation started in a Clinch pane must be recoverable from the
local machine — findable, greppable, and re-openable — even when Claude remote control
bridges the session to claude.ai. A 2026-07-09 update leaked one Claude session's identity
into the relaunched app, causing later sessions to behave as children and write **no local
transcript at all**. The pane registry's single mutable cloud pointer was then overwritten,
so conversations could silently vanish from disk. This feature fixes the launch leak and
adds append-only records so no future overwrite can erase every local recovery clue.

## Problem

Claude Code sessions in Clinch intentionally bridge to claude.ai ("remote control") at
startup. That feature is not the transcript-loss cause: a controlled clean-environment
probe bridged at startup and still wrote a normal
`~/.claude/projects/<slug>/<session-id>.jsonl`.

The actual cause was `make update` being run from inside a Claude session. The detached
updater inherited that session's `CLAUDE_CODE_*`, `CLAUDECODE`, and `AI_AGENT` variables;
`open` forwarded them into the relaunched app; and every new pane inherited the stale
child/session/bridge identity. Claude then treated each nominally new session as a child
of the old bridged session and omitted its top-level jsonl (while still writing side
artifacts such as `tool-results/`, which made the loss easy to miss).

Before this work, the only local pointer to a bridged conversation was the pane's registry
entry (`~/.warp/agent-resume/<pane-uuid>.json`, `"bridge"` field). That entry is a single
mutable file per pane, overwritten by the next session in the pane. Two data-loss incidents
motivated the change:

- **2026-07-08** — machinery-spawned blank sessions overwrote entries pointing at recoverable conversations on every restart (partially mitigated by the `WARP_AGENT_RESUME_STARTED_FRESH` guard in `claude-capture.sh`).
- **2026-07-09** — a `make update` at 13:53 quit Clinch mid-session; an active
  conversation (session `778e4460`, started 13:44) lost both its pane entry and — because
  the poisoned app environment made it a child session — had no local transcript. Its
  bridge id is unrecoverable from disk; the only surviving record was
  `~/Library/Logs/clinch.log.old.0` (OSC 777 `prompt_submit` events, truncated at ~200
  chars).

Verified A/B evidence (2026-07-09):

| Launch environment | Session | Bridged? | Local jsonl |
|---|---|---|---|
| Warp pane, clean | `db54fec3` | yes (late) | ✅ real turns + `bridge-session` records |
| Clinch pane, poisoned app env | `1e456e90` | yes (at startup) | ❌ none — only `session-env`/`security` artifacts |
| Clinch pane, poisoned app env | `778e4460` (lost) | presumed | ❌ none — only `subagents/` + `tool-results/` |
| Controlled PTY, clean env | clean A/B probe | yes (at startup) | ✅ real turns |
| Controlled PTY, stale Claude identity | poisoned A/B probe | inherited bridge | ❌ none after a completed turn |

Before this work there was also **no automated pre-update snapshot**: the
`session-recovery-20260709-141157/` directory under Clinch's app-support dir was created
*by hand* during incident response, 18 minutes after the damage.

## Behavior

### Durable local record (the core guarantee)
1. Every Claude conversation started in a Clinch pane leaves a durable, append-only local
   record containing at minimum: session id, cwd, launch flags, bridge id (once known),
   and every user prompt with timestamp. To prevent a runaway loop from filling the disk,
   prompt capture stops after about 5 MB for one session and appends an explicit truncation
   marker. This record exists even when Claude Code writes no local transcript.
2. The record is plain text/JSONL under the user's home directory, greppable without Clinch running, and is never overwritten in place — only appended.
3. Overwriting a pane's registry entry never destroys information: every registry write (including the overwrite and `remove`) is journaled, so any historically recorded (pane, session, bridge, cwd) tuple remains recoverable.

### Restore & update safety
4. After Clinch quits (user quit, crash, or self-update) and relaunches, every pane's conversation is re-openable: bridged sessions via `claude --teleport <bridge>`, local sessions via `claude --resume <id>` — same behavior as today's `clinch_agent_resume_launch`, but the pointer it needs can no longer be lost.
5. `make update` snapshots the registry (and journal) **before** quitting the running Clinch, not after relaunch. A conversation active at update time is findable after the update from the journal alone.

### Discovery
6. A user who remembers only "I had a conversation about X earlier" can find it locally:
   a single command lists recent conversations (newest first, filterable by directory)
   showing session id, bridge URL if any, and the first prompt text.

### Non-regression
7. Sessions launched from stock Warp, and codex sessions, are unaffected.
8. The existing fresh-session guard (a machinery-spawned blank session must not clobber a protected entry before its first real prompt) keeps working.
9. The capture hooks stay silent and fast: no user-visible output, no measurable prompt latency (the journal/mirror writes are single appends).

### Root fix
10. Updating/relaunching Clinch strips all inherited Claude session identity before
    invoking `open`, and interactive shells strip identity again whenever they launch
    `claude`. Clean sessions therefore write their normal local jsonl again. Remote
    control remains enabled: phone access and cloud teleport are wanted behavior and were
    proved innocent of the loss.

## Acceptance test
The hi/yo test, formalized:
1. Open a Clinch pane, run `claude`, send the prompt `yo-durability-probe`, wait for the reply, close the pane.
2. Confirm the session has a real local jsonl. Without Clinch running:
   `grep -rw "yo-durability-probe" ~/.warp/agent-resume/` finds the mirrored prompt, and
   the journal line for that session includes its bridge id (if the session bridged).
3. Reopen Clinch: the pane restores the conversation (teleport or resume).
4. Run `make update` with an active conversation; after relaunch, the journal contains that conversation's pointer and the pane restores it.

## Out of scope
- Recovering conversations already lost before this ships (the 2026-07-09 auth conversation is only findable by browsing https://claude.ai/code for a session started ~1:44 PM PT that day).
- Mirroring **assistant** output into the durability store (prompts only; Claude's normal
  jsonl and the cloud copy remain the full-content records).
- Any change to codex capture/resume.
