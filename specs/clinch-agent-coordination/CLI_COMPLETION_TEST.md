# Coordination CLI completion validation

Date: 2026-09-30. Branch: `codex/clinch-recent-messages`.

The extension adds role/message filters, durable unread inbox checkpoints, bounded waits,
background provider launch, revision-guarded interruption, tab/section pinning, project tasks,
cross-window live transfers, and app-recorded replayable events. These controls remain optional;
ordinary Claude/Codex sessions need no coordinator.

## Automated checks

- All 375 selected app/protocol/CLI tests pass. The selection includes `local_control`,
  `project_window`, `agent_skills`, transferred sidebar tab behavior, and all `local_control`
  and `warp_cli` library tests.
- A background-launch regression checks actual keyboard focus immediately and after deferred
  effects, along with inactive-project selection and collapsed-section state. It caught focus
  changes in both pane construction and title assignment; both paths now honor background launch.
- Cross-window tests preserve the exact terminal and input-view identities, verify their new
  native window, section/index and color, and distinguish empty sources from task-owning projects.
  Pending renders of an emptied source are safe. The test platform's window-close operation is
  a no-op; native closure is a separate live check.
- Transcript fixtures cover filtered tail/pagination, changed-filter cursors, rotation, incomplete
  records, and bounded full-history streaming. Inbox/event fixtures cover independent readers,
  atomic/idempotent/conflicting acknowledgments, moves, failed reads, batch expiry/quota,
  replay after reopening storage, historical section departures, and stale lifecycle identities.
- Native interruption coverage verifies working/revision guards, the Escape write, cancellation
  epoch change, preserved conversation, and readiness after interruption.
- Clippy passes with warnings denied for `warp`, `local_control`, and `warp_cli`, including the
  app binary, libraries, and test targets. Repository formatting and `git diff --check` pass.
- Repository-wide `script/presubmit` stops at 17 existing inline-test-module violations in
  unrelated files. Its formatting phase passes. The focused checks above were run separately;
  this is not a claim that the entire workspace/release suite passed.

Commands used the repository's macOS configuration: `WARP_CHANNEL=local`, `WARP_BIN_NAME=warp`,
`FRAMEWORK_OVERRIDE=dev`, `MACOSX_DEPLOYMENT_TARGET=14.0`, with `gui,warp_control_cli` features.

## Live checks

A signed development app used a fresh data profile and isolated discovery under
`/tmp/cce-zvljx2em`. Shared development settings were checked by hash and unchanged.
Only disposable test sessions received instructions or moved. The installed production app
was not replaced. Evidence JSON and helper scripts are temporary local artifacts, not fixtures.

- Task create/update/complete/delete and section/tab pin/unpin passed in an inactive project.
  Event replay returned 12 organization changes after a saved cursor, without a connected watcher.
- Typed background Claude launch targeted a collapsed section in an inactive project. It answered
  its initial prompt; foreground project/tab selection stayed unchanged.
- Real Claude and Codex conversations each received and answered a background follow-up. Filtered
  `--role assistant --messages-only --last 2` returned their two assistant replies. Foreground
  selection stayed unchanged through reads, sends, inbox access, and waits.
- Inbox peek repeated the same unread records. Explicit acknowledgment consumed them; a second
  reader still received its own unread records. Each later follow-up appeared once and a subsequent
  read had no messages. The one-second unmatched wait returned in approximately 1.06 seconds.
- A working Codex turn accepted `agent interrupt` with its current revision. The same conversation
  subsequently answered `AFTER_INTERRUPT_4e29cc5d39`; neither the PTY nor conversation was closed.
- Claude moved to a second native window and back into a section. After removing that window's
  empty initial terminal, moving its final live session closed the source window. Tab/conversation
  identities survived, and the agent answered `AFTER_TRANSFER_4e29cc5d39`. The inbox returned only
  this new Claude reply after the move, preserving its checkpoint.

The first bare Codex 0.158.0 launch answered its prompt but lacked pane notifications while using
its shared daemon. The CLI correctly returned created IDs and an observation timeout. A local
`--no-daemon` Codex session passed the checks above. The typed launcher now probes support for
that flag inside the newly created terminal and selects local execution when available; older
versions retain their original invocation. This avoids changing global Codex configuration or
stopping another session's daemon. Regression coverage verifies both help variants, literal prompt
quoting, and that a failed provider process is never launched a second time.

Final verification repeated the live checks on the rebuilt executable in a second isolated profile,
`/tmp/ccf-tmn_iomk` (app PID 29637). Both `agent launch --background` commands discovered real
provider identities automatically, including Codex with the compatibility wrapper. Claude conversation
`6cd69421-7ba1-4d2b-a788-b354aba78343` and Codex conversation
`01a0f4ec-97ee-7dc3-a3c3-fe2a05b405b1` answered follow-ups. Inbox/role-filter checks, guarded Codex
interruption and its follow-up, and cross-window transfer/native closure passed again. The unmatched
one-second wait returned in approximately 1.03 seconds. Both sessions ended `turn_complete`.
The app executable build passed; the final source passed all 375 selected tests and Clippy again.

The final development app and its two named proof sessions remain available for inspection. These
results verify a development build, not a packaged production release or all future coordinator UI.
