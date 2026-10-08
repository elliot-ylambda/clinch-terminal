# Real Claude/Codex coordination test

Date: 2026-09-24. Build: this branch's signed Clinch Dev, local CLI protocol 1.
Providers: Claude Code 2.1.281 and Codex CLI 0.156.1, using the user's existing sign-ins.

## Result

The requested coordination workflow passed with three real interactive provider sessions
across two project windows. These were actual model conversations, not the earlier protocol
fixtures. All three started and answered an initial prompt as ordinary independent sessions,
without a coordinator registry entry. This existing conversation then coordinated them through
the CLI; a persistent coordinator process was not required or tested.

| Session | Provider | Project window / working directory | Delegated task | CLI prompts verified |
| --- | --- | --- | --- | --- |
| Coordination test A | Claude Code | Existing project window / `clinch-terminal` | Review current CLI behavior, answer follow-up, correct summary | 3 |
| Coordination test B | Codex | New project window / `magister-marketing` | Review delivery code, receive queued follow-up, verify finding | 3 |
| Coordination test C | Claude Code | Same new project window / `magister-marketing` | Review coordination skill and reconcile the other sessions' findings | 2 |

Review inputs were this branch's CLI documentation, delivery implementation, and coordination
skill. Tasks were read-only. No merges or deployments were attempted. The three named test
sessions were left open and ready for inspection.

## Checks completed

- Whole-app discovery found all three sessions across the two projects. Exact project filters
  returned the expected sessions.
- Every one of the eight CLI prompts appeared exactly once in its intended provider transcript.
  No delegated task appeared in another worker's transcript.
- Repeating each initial delegation with the same request ID returned the same submitted receipt
  without another prompt being delivered.
- A follow-up queued while Codex was working remained queued until its review finished, then
  produced the expected response.
- Multiline follow-up text was preserved for both Claude and Codex.
- All delivery receipts were reconciled with actual transcript records and replies. The final
  status of all three sessions was `turn_complete`, with input ready.
- Transcript cursors returned messages after the saved initial position. Coverage remained
  explicitly `partial`; this test does not claim exhaustive provider history support.
- Read and inspect commands preserved selected projects and tabs in both windows.
- The coordinator challenged an incorrect Claude summary, received its correction, and asked
  another Claude session to reconcile it with Codex's findings. Worker claims were checked
  rather than treated as authoritative simply because a turn finished.

## Findings and remaining work

- **Receipt recovery fixed before merge:** Codex identified that PID-only recovery could leave
  an abandoned dispatch pending when the OS reused its owner's PID. The journal now records
  and checks process start time as well as PID. Recovery cancels stale queued work and marks
  interrupted dispatches `delivery_unknown`, including legacy records without a process identity.
  A regression test covers two process incarnations sharing a PID and preserves the live owner's
  queue. This was tested synthetically, not by forcing OS PID reuse live.
- **Project-targeted launch:** `tab create` currently uses the active project of the selected
  window. This run used two project windows; it did not validate CLI creation directly inside
  an inactive project tab in the same window. Add an explicit project launch selector.
- **Summary accuracy:** Claude initially said nothing survives restart. The coordinator corrected
  that claim: receipts survive; runtime identities do not, and old queued work is cancelled.
- **Future coordinator UI:** Keep independent sessions as the default. Coordinator creation and
  attachment must be optional, and stopping a coordinator must preserve ordinary chats.

This test validates current discovery, messaging, reading, and active-conversation coordination.
Persistent wakeups, sidebar controls, same-window project-targeted launch, and real release
integration remain separate work. The previously recorded 131 automated tests cover the CLI
implementation and synthetic recovery cases; this run adds real-provider evidence.
