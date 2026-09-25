# CLI-first coordination

This branch adds app-wide discovery, transcript reads, exact-agent messaging,
explicit message queues, durable delivery receipts, and status polling. Build and run this branch to test it; the already running
production app cannot gain these handlers by updating a client alone.

Normal Claude/Codex chats work independently by default. A coordinator is optional: creating
one is not required to launch chats or use the CLI. You can add coordination later and keep
other chats independent. Listing a session does not automatically place it under coordination.

## Run a development build

```sh
TERM=xterm-256color WARP_AGENT_RESUME_DIR="$HOME/.clinch-local/agent-resume" ./script/run --dont-open
```

Launch the dev app with isolated discovery while an older production app is running:

```sh
open -n \
  --env "WARP_AGENT_RESUME_DIR=$HOME/.clinch-local/agent-resume" \
  --env "WARP_LOCAL_CONTROL_DISCOVERY_DIR=$HOME/.clinch-local/ctrl" \
  "$PWD/target/debug/bundle/osx/ClinchDev.app"
```

Older installed CLIs can delete unfamiliar action metadata from the shared registry.
This branch fixes that scanner, and isolation also protects the dev app from older
clients. Keep production Clinch open. In a terminal **inside this development app**, run:

```sh
"$CLINCH_CONTROL_WRAPPER" ctrl --output-format json app ping --pid "$CLINCH_CONTROL_PID"
"$CLINCH_CONTROL_WRAPPER" ctrl --output-format json workspace tree --pid "$CLINCH_CONTROL_PID"
"$CLINCH_CONTROL_WRAPPER" ctrl --output-format json project list --pid "$CLINCH_CONTROL_PID"
"$CLINCH_CONTROL_WRAPPER" ctrl --output-format json agent list --pid "$CLINCH_CONTROL_PID"
```

The checkout's bundled wrapper is named `warpctrl-local`; use the injected path
rather than assuming a global `clinch` command. It accepts the `ctrl` prefix
and legacy direct commands. No global installation or production restart is
required. Outside the dev app, supply the same `WARP_LOCAL_CONTROL_DISCOVERY_DIR`
environment value when invoking its wrapper. Startup provisions the bundled `clinch-coordinate` skill for installed
Claude/Codex environments, preserving user-owned and newer managed skills.

## Work with sessions

Start Claude Code or Codex in separate tabs in the dev app. Use sessions with
Clinch's provider hooks enabled. Discover `agent_id`, `project_id`, `section_id`,
and `pane_id` from the JSON; all are exact selectors. Session status reports its
source and refuses writes when reliable identity/readiness is unavailable.

Using `<ctl>` below to mean `"$CLINCH_CONTROL_WRAPPER" ctrl`:

```text
<ctl> workspace tree --pid PID
<ctl> project list --pid PID
<ctl> agent list --project PROJECT_A --project PROJECT_B --pid PID
<ctl> agent list --section SECTION_A --section SECTION_B --pid PID
<ctl> agent inspect AGENT_ID --pid PID
<ctl> agent read AGENT_ID --tail --limit 30 --pid PID
<ctl> agent read AGENT_ID --after CURSOR --limit 100 --pid PID
<ctl> pane read --pane PANE_ID --max-bytes 65536 --pid PID
<ctl> agent watch --after SNAPSHOT_CURSOR --wait 30 --pid PID
<ctl> agent watch --follow --pid PID
```

Repeated filters are OR within projects or sections; specifying both kinds is
an intersection. No filters means all open projects, including inactive ones.
`--instance ID` is an alternative to `--pid`. Discovery does not change focus.
`agent watch` emits a snapshot or timeout with `data`, `cursor`, and explicit
`polled_snapshot` coverage. Follow streams NDJSON; it stops when interrupted.

To send: inspect the agent, require `ready: true`, retain `input_revision`, write
a UTF-8 prompt file, and generate one UUID for that logical message:

```text
<ctl> agent send AGENT_ID --text-file /absolute/prompt.txt --expected-revision REVISION --request-id UUID --pid PID
```

Alternatively use `--text` or `--text-file -` for stdin. Preserve the exact
UUID/revision/target/text when retrying. The CLI waits briefly for a receipt;
repeating an identical request retrieves it without a duplicate dispatch.

## Queue and inspect delivery

Use one sender UUID per coordinating conversation. A queued send may target a ready or working
agent, and still requires the latest input revision. It waits for that exact conversation to
be ready. A target with pending work accepts only the same sender until its queue drains.

```sh
<ctl> agent send AGENT_ID --text-file /absolute/prompt.txt \
  --expected-revision REVISION --request-id MESSAGE_UUID \
  --queue --sender COORDINATOR_UUID --expires-in 1800 --pid "$CLINCH_CONTROL_PID"
<ctl> agent message inspect MESSAGE_UUID --pid "$CLINCH_CONTROL_PID"
<ctl> agent message list --agent AGENT_ID --limit 50 --pid "$CLINCH_CONTROL_PID"
<ctl> agent message cancel MESSAGE_UUID --pid "$CLINCH_CONTROL_PID"
```

Here `<ctl>` means the exact bound wrapper plus `ctrl`, as above. List returns metadata only;
inspect includes the prompt. Use `next_before` with `--before` for older receipt pages. Cancel
works only while queued. Human edits or a changed foreground/session cancel queued work.
Expiry defaults to 30 minutes, configurable from one second to one day. Pending limits are
ten per target and 100 per app instance. An immediate send cannot jump a pending queue.

## Current boundaries

- Read pages: up to 500 records, approximately 256 KiB of normalized records,
  with explicit text/tool truncation; at most 4 MiB of source scanning per call.
  A cursor resumes complete JSONL records and rejects changed/truncated sources.
  `--tail` scans recent content, including files larger than the previous prompt
  history reader's 5 MiB limit. Unsupported records, images, and reasoning are
  omitted; missing/remote transcripts are `preview_only`. No full-history claim.
- Send: nonempty UTF-8 prompts up to 64 KiB, no terminal control characters.
  Immediate sends reject busy targets. Both admission paths reject blocked, remote,
  drafted, or unknown targets. Explicit queues wait for working targets. Input,
  identity, settings, foreground state, and remote writer ownership are rechecked
  before Enter. If a user intervenes after insertion, text may remain unsubmitted;
  the receipt becomes `delivery_unknown` without clearing the user's input.
- Receipts: persisted privately for seven days, capped at 10,000 retained messages.
  States: `queued`, `dispatching`, `submitted`, `failed`, `cancelled`, `expired`, and
  `delivery_unknown`. `submitted` confirms PTY submission, not provider acceptance.
  Exact UUID/target/revision/text/queue-option retries return the saved receipt;
  conflicting reuse fails. If storage is unavailable, no new message is sent.
- On restart, receipts remain inspectable. Queued work from exited instances is
  cancelled; interrupted dispatches become uncertain. There is no automatic replay
  or rebinding to restored tabs. Inspect uncertain delivery before a new send.
- IDs last for this app instance. Watch polls once per second, may miss intermediate
  transitions, and has no replay. `turn_complete` is not test/deployment success.
- No persistent coordinator, sidebar UI, scope registry, deployment lock, or automatic
  merge policy yet. An active coordinating agent uses these primitives plus repository,
  CI, and deployment tools within the user's instructions.

The complete future behavior remains in [PRODUCT.md](./PRODUCT.md), with the
staged architecture in [TECH.md](./TECH.md).

## Validation

- 65 app tests, 42 protocol/discovery tests, and 24 CLI tests pass. Queue coverage
  includes persistence, atomic cancellation/dispatch claims, sender ownership, limits,
  paging, expiry, restart recovery, and native human-input preemption.
- Signed Clinch Dev live checks passed for both disposable Claude and Codex protocol
  fixtures: busy queues, FIFO, cancellation, expiry, sender conflicts, exact retries,
  conflicting UUIDs, receipt paging, and transcript verification.
- A live app restart preserved submitted receipts, cancelled old queued work, and
  returned saved receipts for exact retries without rebinding or resending.
- A live composer edit cancelled queued work; the transcript confirmed that it was
  never submitted. Managed `clinch-coordinate` 1.1.0 was provisioned automatically
  for both Claude and Codex.

These fixtures make no AI API calls and validate Clinch's PTY and notification contract,
not the providers' current TUIs. Whole-app discovery was also checked across three projects.

Real-provider validation is recorded separately in [LIVE_TEST.md](./LIVE_TEST.md): three real
Claude/Codex sessions across two project windows, eight verified CLI prompts, queued follow-up,
cross-session review, and the remaining launch/recovery gaps.
