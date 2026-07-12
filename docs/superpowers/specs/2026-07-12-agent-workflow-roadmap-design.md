# Agent-Workflow Roadmap — Clinch Productivity & Performance Ideas

**Date:** 2026-07-12
**Status:** Roadmap spec. Tier 1 items (#1–#4) and the launch-audit prerequisite are
being implemented as separate PRs; the rest are candidates awaiting prioritization.

## Context

Clinch is a backendless terminal purpose-built for running CLI coding agents
(Claude Code, Codex). It already ingests a structured per-session event stream from
both agents via OSC 777 (`CLIAgentEventType` in
`app/src/terminal/cli_agent_sessions/event/mod.rs`: `SessionStart`, `PromptSubmit`,
`ToolComplete`, `Stop`, `PermissionRequest`, `PermissionReplied`, `QuestionAsked`,
`IdlePrompt`), with payloads carrying the query, response, summary, tool name, and
transcript path. It also has the agent-resume registry + append-only journal
(`tools/agent-resume/`), a usage chip in the tab bar, per-tab attention badges, and
insert-and-send footer buttons.

The unifying theme of this roadmap: **convert events Clinch already receives into
decisions the user no longer makes manually** — when to resume, which tab to visit,
whether the toolchain is healthy, where an old conversation went. The terminal's job
shifts from displaying output to scheduling the user's attention across agent
sessions.

## Prerequisite: launch-audit must-fix list (PR in flight)

Before announcing or adding features, clear the 2026-07-09 pre-announce audit:

1. **About page shows the Warp logo** — swap to Clinch branding; delete the unused
   Warp asset if nothing else references it.
2. **SECURITY.md and issue templates route to Warp** — vulnerability reports and
   issues must route to `elliot-ylambda/clinch-terminal` (GitHub private
   vulnerability reporting), not `warpdotdev/warp`.
3. **README documents the wrong binary** — instructions reference `warp-oss`; the
   distributed app is the `stable` bin (branded Clinch). Fix build/install docs.
4. **Usage widget contradicts the "no phone-home" claim** — it reads the Claude
   OAuth credential from the keychain and calls the Anthropic usage API. Make the
   behavior opt-in and/or document it precisely; the claim and the code must agree.
5. **Debug entitlements on release builds** — audit the bundle entitlements used for
   self-signed releases; remove debug-only entitlements (e.g. `get-task-allow`)
   without regressing the keychain-identity fix (`SelfSign-Entitlements.plist`).

## Tier 1 — small builds, direct hits on recurring pain (PRs in flight)

### 1. Rate-limit countdown + auto-continue
**Pain:** sessions die at the usage limit and the reset window passes unused
(observed 2026-07-11: session ended on a rate limit that reset at 10:20 pm).
**Design:** the tab-bar usage chip already polls usage. When a window is exhausted,
show a compact reset countdown in the chip. Add an opt-in **auto-continue**:
when enabled for a pane, a Claude session that stopped while the limit was exhausted
gets a queued "continue" sent automatically shortly after the reset time, reusing the
insert-and-send path built for footer quick-insert buttons. Off by default;
toggleable per pane; Command Palette entries for the setting per repo convention.
**Why now:** turns dead overnight/afternoon windows into progress; biggest
throughput win for heavy agent use.
**Dependency:** requires the usage widget to be enabled. Prerequisite item 4 makes
that widget explicitly opt-in, so the countdown and auto-continue are inert (and
make no network calls) until the user opts in.

### 2. Codex `code-mode-host` preflight (companion PR)
**Pain:** `brew upgrade --cask codex` repeatedly removes the sibling
`codex-code-mode-host` binary, silently breaking every Codex tool call
("failed to spawn code-mode host"); rediscovered by debugging each time.
**Design:** when a Codex session starts in a pane (`SessionStart`, agent == Codex),
verify `codex-code-mode-host` is resolvable (next to the resolved `codex` binary or
on PATH). If missing, show a one-time-per-app-run warning with the fix hint.
Zero cost when healthy; no network.

### 3. "Next agent needs me" cycling
**Pain:** with several parallel agents, triage is manual tab-scanning even though
Clinch already knows exactly which panes await input.
**Design:** a workspace action (palette + default keybinding) that cycles focus to
the next pane whose agent is waiting (`PermissionRequest` / `QuestionAsked` /
`IdlePrompt` not yet acknowledged), plus a small "N waiting" indicator in the
vertical-tab header; clicking it cycles. Reuses the existing attention-badge state.

### 4. Conversation finder
**Pain:** the durability work made every conversation recoverable on disk
(`clinch-agent-resume list`), but reopening one still means a manual CLI + copy/paste.
**Design:** a palette command "Reopen agent conversation…" that lists recent
conversations (newest first: first prompt, cwd, bridged/local) by consuming
`clinch-agent-resume list` (add a `--json` output mode for stable parsing), and on
selection opens a pane at that cwd running `claude --teleport <bridge>` or
`claude --resume <id>` — the same launch path pane-restore already uses.

## Tier 2 — bigger features, structural wins (not yet scheduled)

### 5. Worktree-aware agent tabs
Both 2026-07 session-loss incidents trace partly to multiple agents sharing one
dirty checkout (dirty tree blocks teleport; EAGAIN under concurrent builds). Add a
"New agent tab in fresh worktree" action: `git worktree add` off the repo's main
branch, open the tab there, launch the agent. Eliminates the incident class and
pairs naturally with project windows.

### 6. Broadcast prompt to selected agent panes
Send one prompt ("run the tests", "rebase on main") to N selected agent tabs at
once, on top of the existing insert-and-send infrastructure. Natural companion to
parallel worktree agents.

### 7. In-app transcript search
Palette command "Search agent history" that greps `~/.claude/projects/**/*.jsonl`,
the prompt mirrors, and the journal, then opens/resumes the hit. Closes the loop the
durability work started: "I discussed this with an agent last week" becomes seconds.

### 8. Tab-hover last-response preview
The `Stop` event payload already carries `summary`/`response`. Show the last
response snippet on tab hover so finished agents can be triaged without switching.

## Tier 3 — performance (measure before building)

### 9. Agent-pane scrollback cost
Agent sessions generate enormous output; background panes may pay model/render cost
for scrollback nobody is viewing. Profile with 6–8 busy agent tabs first. Related
work already landed: the idle-redraw painted-set gate (PR #25) cut background-tab
repaint CPU; occlusion/eviction follow-ups from that PR are the likely next wins.

### 10. Lazy restore on launch
Project windows restore every project eagerly. Restore the visible pane first and
defer background tabs' PTY spawn + agent resume to keep launch fast as window
counts grow.

## Dead-code / cleanup obligations attached to this roadmap

- **Prerequisite PR:** delete the Warp About-page logo asset if unreferenced after
  the swap; remove any README claims that no longer match behavior rather than
  layering new text on top.
- **#1:** the usage chip's "limited" state must reuse the existing usage data model —
  no parallel poller; if the old footer usage remnants are touched, prefer deleting
  superseded code (the footer→tab-bar move already left the footer path vestigial).
- **#3:** cycling must consume the existing attention-badge state, not introduce a
  second "needs input" tracker that can drift.
- **#4:** `--json` becomes the one machine-readable interface to
  `clinch-agent-resume list`; the app must not screen-scrape the human format.
- **General:** each feature ships behind the smallest surface that works; if a
  Tier 1 feature is later superseded (e.g. #8 replaces part of #3's indicator),
  the superseded UI is removed in the same PR that supersedes it.

## Acceptance sketches (Tier 1)

- **#1:** exhaust a usage window → chip shows countdown; enable auto-continue on a
  stopped, limited pane → after reset the pane receives exactly one "continue" and
  the agent resumes. Disabled panes receive nothing.
- **#2:** rename `codex-code-mode-host` away → start a Codex session → one warning
  with fix hint; restore binary → no warning.
- **#3:** three panes, two waiting → indicator shows 2; invoking the action cycles
  through exactly the waiting panes in stable order and clears as they're answered.
- **#4:** run the finder → most recent conversations appear newest-first with first
  prompt; selecting a bridged one opens a pane that teleports; a local one resumes.
