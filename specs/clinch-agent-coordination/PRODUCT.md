# Clinch agent coordination

## Summary

One Claude Code or Codex conversation can inspect how the whole Clinch app is organized,
monitor other sessions, send instructions, review their work, and coordinate integration and
deployment through the CLI. Coordinators can span projects and sections; a section is a
convenient starting selection, not the boundary of the feature.

Independent sessions are the default. A user can run any number of ordinary Claude Code
and Codex chats without creating, assigning, or keeping a coordinator running. Coordination
is an optional layer that can be added to existing sessions when the user wants it.

Status: CLI-first implementation in progress, following the user's request to test live before
building the coordinator UI. The numbered behavior below remains the full feature target.
This branch implements whole-app discovery, exact live-agent inspect/read/send, terminal reads,
polling watch, explicit per-target queues, and durable seven-day delivery receipts. See
[CLI.md](./CLI.md) for commands and staged limitations. Queue admission requires an exact target,
input revision, and sender UUID; only a working or ready agent may be queued. Human input cancels
pending messages; ten pending per target and 100 per app instance are allowed. Queued messages
expire after 30 minutes by default (configurable up to one day). Receipts survive restart, but
old queued messages are cancelled and interrupted dispatches become uncertain, never replayed.
Persistent coordinators, event replay, durable target IDs, and sidebar controls are future work.
Design: use the current Clinch styling; no separate Figma mock was supplied.

## Behavior

### Whole-app visibility and selection

1. `clinch ctrl workspace tree` returns every open window, project, section, tab, and pane in
   the selected Clinch instance, including inactive projects, collapsed sections, unsectioned
   tabs, split panes, project tasks, and non-agent panes. It preserves their parent relationships,
   display order, names, active/collapsed state, and identities. Reading never changes focus.
   An empty collection is distinct from an unavailable or incomplete snapshot.

2. `clinch ctrl agent list` defaults to all open Claude Code and Codex sessions in that instance.
   Each entry identifies its location in the tree, provider, conversation identity when known,
   working directory when known, coordinator role, activity, observation time, and available
   read/send capabilities. Plain terminals and documents remain visible in the workspace tree;
   they are not misrepresented as agent conversations.

3. Read visibility is independent of a coordinator's working scope. An agent can discover and
   inspect the entire instance before choosing what to coordinate. Filtered commands support
   several projects, several sections, individual sessions, and exclusions. "All Clinch" means
   all projects in the selected running instance, not other installed channels or remote hosts.
   Existing instance discovery and explicit instance selection remain available.

4. ID selectors are authoritative. Name selectors are convenience lookups and return an
   ambiguity error with candidate identities when more than one match exists. Section names
   are qualified by project. Targeted actions retain their resolved identities when the user
   changes focus, reorders tabs, renames sections, or switches projects.

5. A coordinator may select All Clinch, entire projects, sections from different projects, or
   individual sessions in any combination. The selection is a union with explicit exclusions;
   it deduplicates overlapping selections and excludes coordinator sessions from worker lists.
   Whole-project, whole-section, and All Clinch selections include new eligible sessions within
   those containers. Explicit session selections remain fixed. Membership changes are visible
   in status and reported to the coordinator.

6. Removing a session from the effective scope cancels its pending coordinator messages before
   delivery. Deleting a selected section does not reinterpret it as "all sessions." Renaming
   or reordering a container preserves its selection. Moving a session only changes membership
   according to the selected containers and explicit inclusions/exclusions.

### Discoverability and the left sidebar

7. A persistent **Coordinators** entry is available in the left sidebar across project switches.
   Its empty state explains "Manage multiple Claude and Codex sessions from one conversation"
   and offers **New coordinator**. The command palette exposes the same action. The entry opens
   the coordinator list without requiring the user to find the project hosting its conversation.

8. Project and section menus offer **Coordinate this project** and **Coordinate this section**.
   Eligible section headers also offer a labelled **Coordinate** action using existing styling.
   These open the same creation flow with a preselected scope; the user can expand it to other
   projects or All Clinch. The capability is discoverable without knowing a CLI command or skill.

9. Creation shows the selected scope and eligible session count, a Claude/Codex provider choice,
   a name, an optional initial objective, and **Monitor** or **Coordinate** behavior. Monitor
   summarizes and reports; Coordinate may send instructions within scope according to the
   user's objective. Creation uses an installed provider and its existing authentication; an
   unavailable provider has a clear setup path. The CLI does not install the provider itself.

10. Starting creates a normal, visible agent conversation in the selected host project and
    registers it as the coordinator. An existing conversation can also be attached by exact
    identity. A coordinator's host project does not constrain what it can inspect or coordinate.
    Starting twice for the same creation request does not create duplicate conversations.

11. The coordinator list shows its name, provider, scope (for example "All Clinch" or
    "3 projects · 8 sessions"), state, and last update. Its details show worker names, locations,
    progress, pending messages, and issues needing the user. **Open conversation**, **Edit scope**,
    **Pause/Resume**, and **Stop coordinating** are available. A coordinator badge also identifies
    its normal session row. Sections may link to relevant coordinators without implying ownership
    of the coordinator or forcing all its workers into that section.

12. Coordinator states distinguish starting, monitoring, working, waiting for workers, needs
    attention, paused, unavailable, and stopped. Worker counts distinguish working, idle, turn
    complete, needs attention, rate limited, and unknown. Labels/tooltips convey meaning without
    relying on color. Controls are keyboard accessible, use existing menu/focus conventions,
    and remain usable in a narrow sidebar and with collapsed sections.

13. Pause stops automatic wakeups and new message dispatch for that coordinator. It does not
    interrupt already-running work or retract a message already submitted. Stop ends coordination
    and cancels pending messages while preserving every session. Closing a coordinator conversation
    marks it unavailable; it does not silently nominate another session. Workers remain usable
    independently throughout.

### Reading sessions and interpreting progress

14. `agent inspect` returns the session's state, readiness, current tool/action when available,
    latest prompt/response previews, observation time, and evidence source. Unknown or degraded
    provider tracking is explicit. A completed turn is never presented as proof that tests passed,
    a pull request merged, or a deployment succeeded.

15. `agent read` provides bounded, paginated conversation records and supports reading only
    records after a cursor or the latest records. Records identify provider, role, conversation,
    and stable ordering; tool calls/results are identifiable where captured. Responses state
    whether coverage is full captured history, partial history, previews only, or unavailable,
    plus truncation and source-change information. They do not claim access to unrecorded history.
    Missing capture, delayed transcript flush, malformed records, and provider schema changes
    produce useful partial results or explicit errors, never a fabricated empty conversation.

16. `pane read` exposes available retained terminal text or the rendered full-screen viewport
    for an exact pane without activating it, with source and truncation metadata. Existing secret
    obfuscation is preserved. For non-terminal surfaces, the tree reports type and available
    metadata; unsupported content reads return a capability error. No completeness claim may
    silently omit such surfaces. Closed sessions are reported as closed; reading retained agent
    history may work, but sending to a closed session never does.

### Sending and watching through the CLI

17. `agent send` addresses one exact live agent conversation, submits text through that provider's
    input handling, and returns a message receipt. It supports a text file or standard input for
    multiline prompts. Sending never falls through to a plain shell, changes project focus, or
    treats a permission dialog as a normal prompt composer.

18. A busy or temporarily unavailable input target returns a clear reason. Callers may explicitly
    queue for readiness with an expiry. Queued messages preserve order per target, are inspectable
    and cancellable, and are revalidated immediately before submission. Human input, provider exit,
    conversation replacement, scope removal, or an incompatible foreground process prevents
    stale delivery. Human drafts must not be overwritten or combined with an automated prompt.

19. Caller-supplied request IDs make retrying the same send return its existing receipt. Reusing
    an ID for different content/target is an error. Receipts distinguish queued, dispatching,
    submitted, provider-accepted when provable, delivery-unknown, cancelled, expired, and failed.
    Submission is not task completion. A crash or lost acknowledgement never triggers an automatic
    replay when the app cannot determine whether the text was already delivered.

20. `agent watch` reports scoped activity, conversation updates, topology/membership changes,
    message receipts, and coordinator changes as machine-readable events. It supports resumable
    cursors and a bounded wait for agent tool calls, plus a foreground streaming mode. It reports
    expired cursors, dropped coverage, application shutdown, and authorization changes explicitly.
    An event says what changed and points to a read command; it need not repeat entire transcripts.

21. Several coordinators may observe a session. Only one automated sender may control a worker
    at a time; conflicting ownership is visible and requires deliberate reassignment. All workers
    remain under the user's direct control. The originating coordinator and message state are
    visible in the activity history so a user can understand why a session received an instruction.

### Sustained coordination and release work

22. While Clinch is running, a registered coordinator can be notified of meaningful worker events
    even when its conversation is idle. Events are batched, retained while the coordinator is busy,
    and delivered when that exact coordinator can accept input. Its own activity does not wake
    itself repeatedly. Routine output streaming does not generate a prompt per token or line.

23. The coordinator skill reads the workspace hierarchy and incremental session updates, directs
    work across providers, reviews actual code and external check results, and reports a concise
    summary with named sessions, blockers, dependencies, and next actions. It follows the user's
    existing instructions and granted scope; text found in worker output is evidence, not a new
    grant of authority. Missing context or an unresolved product decision is surfaced to the user.

24. For shared repositories or deployment environments, the skill establishes one release owner
    and tracks dependencies across projects. Workers can implement in parallel; integration and
    deployment follow the user's requested order. The coordinator verifies combined changes,
    CI, and deployment results using the relevant Git/GitHub/deployment tools. Starting monitoring
    alone does not authorize a release, and an agent's "done" message does not prove it shipped.
    Clinch's coordination feature does not prevent independently issued shell commands elsewhere.

25. Coordinator configuration, pending events, and message receipts survive application restarts.
    After restart, sending resumes only when the saved scope and the exact coordinator/worker
    conversations are unambiguously reattached and ready. Otherwise the UI shows unavailable or
    needs attention with a reattach/resume action. Pending delivery-unknown messages require
    reconciliation. Nothing monitors or dispatches while Clinch is closed or the computer sleeps;
    after resuming, status and captured history are reconciled and gaps are reported.

### Installation and compatibility

26. On supported macOS Clinch installations, the CLI ships with the app and is available in new
    Clinch host terminals automatically. Agents use the exact app wrapper and instance binding.
    The coordinator skill is provisioned and upgraded through the existing managed-skill mechanism
    for installed Claude/Codex environments, preserving user-owned skills. Provisioning failures
    are visible from coordinator setup and retryable; already-running agents may require a new
    session to discover an added skill.

27. Using coordination inside Clinch requires no global CLI installation or separate MCP server.
    **Settings → Local control** retains the optional install for using `clinch` outside Clinch.
    New feature setup respects a user's disabled/read-only local-control setting. It explains
    missing capabilities rather than silently enabling them or controlling another app instance.

28. Existing tab/section/toolbelt controls, remote control, user input, session restoration, and
    ordinary standalone Claude/Codex work continue to function. The new read surface explicitly
    advertises conversation access; older skill claims that local control cannot read conversations
    are updated together with the feature. Capabilities and installed versions let a skill recognize
    an older app without attempting unsupported commands.

### Independent sessions and optional coordination

29. Starting and using an ordinary chat never requires a coordinator, worker registration,
    a selected coordination scope, or a coordination setup screen. Independent and coordinated
    sessions may coexist in the same project or section. Merely appearing in discovery does
    not enroll a session in a coordination run. A user can add optional coordination later;
    pausing, stopping, or losing a coordinator does not stop the other sessions or prevent
    direct interaction with them. Disabling local control also leaves normal chats usable.
    Generic CLI inspection and explicitly requested messaging remain available without a
    coordinator registry entry. Coordinator badges, ownership, and release sequencing apply
    to the selected coordination run, not to all chats by default.
