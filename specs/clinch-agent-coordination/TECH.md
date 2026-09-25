# Clinch agent coordination — implementation plan

## Context

Implementation plan for [PRODUCT.md](./PRODUCT.md). The user selected CLI-first delivery.
The architecture below describes the complete target; this branch implements the initial
slice documented in [CLI.md](./CLI.md).
Research baseline: Clinch commit `f9042ae544ccab75973fb69897d900ed443bfec2`.

| Existing code | Reuse and limitation |
| --- | --- |
| [Local-control selectors](https://github.com/elliot-ylambda/clinch-terminal/blob/f9042ae544ccab75973fb69897d900ed443bfec2/crates/local_control/src/selectors.rs#L27), [bridge](https://github.com/elliot-ylambda/clinch-terminal/blob/f9042ae544ccab75973fb69897d900ed443bfec2/app/src/local_control/bridge.rs#L42) | Typed authenticated actions exist; no project selector, conversation read, send queue, or event stream. |
| [Workspace registry](https://github.com/elliot-ylambda/clinch-terminal/blob/f9042ae544ccab75973fb69897d900ed443bfec2/app/src/workspace/registry.rs#L85) | `get` selects the active project. `get_all`/`all_workspaces` enumerate more; active-project lookup cannot implement app-wide discovery. |
| [Companion snapshot](https://github.com/elliot-ylambda/clinch-terminal/blob/f9042ae544ccab75973fb69897d900ed443bfec2/app/src/remote_control/workspace_adapter.rs#L1628) | Already traverses every project, section, tab, and pane and derives agent states. Share its native data access, not remote pairing/authentication. |
| [Runtime ProjectId](https://github.com/elliot-ylambda/clinch-terminal/blob/f9042ae544ccab75973fb69897d900ed443bfec2/app/src/project_window.rs#L99) | Explicitly process-local. It must not become a saved coordination scope identifier. |
| [Agent context](https://github.com/elliot-ylambda/clinch-terminal/blob/f9042ae544ccab75973fb69897d900ed443bfec2/app/src/terminal/cli_agent_sessions/mod.rs#L89), [events](https://github.com/elliot-ylambda/clinch-terminal/blob/f9042ae544ccab75973fb69897d900ed443bfec2/app/src/terminal/cli_agent_sessions/mod.rs#L599) | Provider/session identity, transcript path, previews, and lifecycle/status events exist. Some provider notifications are degraded; events are not a durable journal. |
| [Prompt history](https://github.com/elliot-ylambda/clinch-terminal/blob/f9042ae544ccab75973fb69897d900ed443bfec2/app/src/agent_resume.rs#L1067) | Reads user prompts, preferring mirrors; bounded beginning-of-file reads and short stop previews cannot provide complete/latest conversations. |
| [Native prompt submission](https://github.com/elliot-ylambda/clinch-terminal/blob/f9042ae544ccab75973fb69897d900ed443bfec2/app/src/terminal/view/use_agent_footer/mod.rs#L1217) | Provider strategies exist. The bool result acknowledges initiation; delayed Enter needs late identity/input checks and delivery callbacks. |
| [Section menu](https://github.com/elliot-ylambda/clinch-terminal/blob/f9042ae544ccab75973fb69897d900ed443bfec2/app/src/workspace/view.rs#L11011), [section header](https://github.com/elliot-ylambda/clinch-terminal/blob/f9042ae544ccab75973fb69897d900ed443bfec2/app/src/workspace/view/vertical_tabs.rs#L3777) | Contextual entry points; add an app-wide coordinator entry independently of section ownership. |
| [Shell PATH/binding](https://github.com/elliot-ylambda/clinch-terminal/blob/f9042ae544ccab75973fb69897d900ed443bfec2/app/src/terminal/local_tty/shell.rs#L28), [skill provisioning](https://github.com/elliot-ylambda/clinch-terminal/blob/f9042ae544ccab75973fb69897d900ed443bfec2/app/src/agent_skills.rs#L185) | macOS bundles/adds the CLI path; startup installs managed skills. Skill provisioning currently copies only `SKILL.md`. |

The running installed app and this checkout differ: the installed CLI advertises `tab grep`,
which is absent from this checkout. Existing live list/inspect results also showed inconsistent
target resolution. Verify new behavior with a build of the implementation branch; do not count
installed-app success as validation of changed source.

## Implemented CLI stages

- Typed `workspace.tree`, `project.list`, `agent.list/inspect/read/send`, and `pane.read`
  plus `agent.message.inspect/cancel/list` actions use the existing authenticated local-control bridge and permission catalog.
- `app/src/local_control/agents.rs` traverses every RootView/project/workspace. Opaque
  targets bind app instance, runtime project/tab/pane, provider, and conversation.
- `conversation.rs` parses provider transcripts on a blocking worker, with bounded pages,
  tail reads, complete-record cursors, source validation, and explicit partial coverage.
- Native submission checks readiness/input revision before text and before provider-specific
  Enter. A dedicated delivery thread owns a private SQLite journal, with WAL and synchronous
  durable commits before PTY dispatch. The UI thread performs only identity/readiness checks
  and native submission. CLI waits briefly for completion.
- Explicit queues use FIFO order per exact runtime target, one sender UUID while work is
  pending, a separate human-input/foreground guard, and expiry. Cancellation and dispatch claims
  use atomic state transitions across app instances. Retention is seven days / 10,000 receipts;
  pending limits are 100 per app instance and ten per target. Receipts include payload only on
  explicit inspect. The database lives under the channel's private config directory.
- Recovery cancels queued messages whose owner process exited and marks interrupted dispatches
  `delivery_unknown`; runtime IDs are never rebound after restart. Exact same-request retries
  retrieve retained receipts before checking live target availability. UUID/content/options
  conflicts fail. Storage errors never fall back to unjournaled delivery.
- `agent watch` polls status snapshots. It has neither replay nor a persistent event journal.
- Discovery preserves live records with unknown action metadata instead of deleting another
  app version's record and broker socket. Development smoke runs use an isolated registry
  to protect against older installed clients with the original destructive scanner.
- The new managed `clinch-coordinate` skill uses current-app binding and capability checks.
  Existing packaging provisions it without replacing a newer installed `clinch-control` skill.

Current differences from the full target: runtime-only IDs; exact IDs rather than names;
union within project/section filters and intersection across filter kinds; no persisted scope,
coordinator role registry, integration locks, or sidebar UI. Receipts do not prove provider
acceptance. A transcript read reports partial supported coverage, not exhaustive historical access. This staged scope
is deliberate; capability fields advertise queues/durability while marking event replay unsupported.

## Proposed changes

### 1. Shared discovery, scope, and identity

Add an app-owned `agent_coordination` module with separate discovery, scope, conversation,
delivery, events, persistence, and coordinator components. Extract narrowly reusable traversal
and exact-target resolution from `remote_control/workspace_adapter.rs`; retain remote session
authorization and writer-lease behavior at the remote boundary. Local calls continue through the
existing same-user broker, action-scoped credentials, permission checks, and bridge. No new
network service, UI automation, or edits to provider databases are needed.

The hierarchy snapshot includes projects/sections even when no supported agent exists there,
tasks, unsectioned tabs, non-terminal pane kinds, display order, cwd, live agent metadata, and
capabilities. Use a topology revision plus per-agent activity/readiness revisions; an unrelated
project rename must not invalidate an otherwise exact prompt delivery.

Add a persisted workspace UUID to `WindowSnapshot`/the corresponding project workspace storage
with a reversible migration and restore support. Keep `ProjectId` as the runtime integration ID
and expose both identities explicitly. Preserve the UUID across restart, rename, and moving a
project between windows; assign a new UUID to a duplicated/new project. Existing persisted
`TabGroupId` and pane UUIDs identify sections and restored panes. Agent targets combine app
instance, pane incarnation, provider, and provider conversation ID. A pane ID alone is insufficient.

Represent a scope as `all_instance` or unions of durable projects, project-qualified sections,
and explicit provider conversations, with explicit exclusions. Resolve to a deduplicated set
on each relevant topology event. Exclude every registered coordinator from worker expansion
to prevent coordinator-to-coordinator feedback. Preserve read access to the whole instance;
scope limits a run's automated work, not global discovery. Generic discovery APIs are independent
of CoordinatorModel so an ordinary agent can inspect Clinch without creating a run.

### 2. Typed CLI surface

Extend the catalog, protocol types, validation, output rendering, app handlers, and the
`warp_cli` local-control parser together. Use additive optional project selectors for existing
target structures and preserve legacy active-project defaults on existing commands. New
workspace/agent discovery defaults to all projects. Inspect/read accept exact returned IDs.
Human-readable name selectors resolve once and reject ambiguity.

Proposed command families (not currently shipped):

```text
clinch ctrl workspace tree
clinch ctrl project list
clinch ctrl agent list [--project ID ...] [--section ID ...]
clinch ctrl agent inspect AGENT_ID
clinch ctrl agent read AGENT_ID [--after CURSOR | --tail N] [--limit N]
clinch ctrl pane read --pane PANE_ID [--limit-bytes N]
clinch ctrl agent send AGENT_ID --text-file PATH --request-id UUID [--queue --expires-in DURATION]
clinch ctrl agent message inspect MESSAGE_ID
clinch ctrl agent message cancel MESSAGE_ID
clinch ctrl agent watch [scope filters] [--after CURSOR] [--wait SECONDS | --follow]
clinch ctrl coordinator create --name NAME --scope-file PATH --provider claude-code|codex
clinch ctrl coordinator attach --agent AGENT_ID --scope-file PATH
clinch ctrl coordinator list|inspect|pause|resume|stop|update-scope ...
```

Keep the existing global `--output-format json|ndjson` and instance/PID selection. Bound skills
invoke the exact `$CLINCH_CONTROL_WRAPPER` plus `ctrl` and `$CLINCH_CONTROL_PID`; abbreviated
examples do not supersede binding. Define `--text-file -` as stdin; allow direct `--text` for
short messages, but recommend files/stdin for multiline content. Publish typed response schemas,
capability flags, bounded output defaults, and error codes. Proposed starting limits: read 100
records/256 KiB per page; send 64 KiB; bounded watch up to 45 seconds. Reject invalid limits.

Reads receive read permissions; send/cancel/coordinator changes are mutations. Validate settings
again at dispatch, not only when queued. Capability metadata covers history availability, target
readiness, event resume, and delivery receipt support. Old app versions produce an upgrade path.

### 3. Conversation and terminal readers

Build a new provider-normalized conversation reader; do not reuse `read_prompt_history` as a
full-chat reader. Reuse its vetted provider transcript path discovery/validation, not its prompt
mirror preference or first-5-MiB algorithm. Read only supported transcripts attached to discovered
Clinch sessions. No arbitrary filesystem path is accepted from a CLI caller. Copy immutable
metadata on the UI thread, then perform file I/O/parsing on a worker executor.

Normalize user/assistant messages and identifiable tool events. Deduplicate Codex records where
the same content appears as both event and response records. Give records stable source-derived
IDs and cursors containing source generation and completed record position. Handle incomplete
trailing JSONL, rotation, truncation, missing files, malformed individual records, and schema
changes explicitly. Bound backward scanning for tail reads and paginate older content separately.
Do not expose private reasoning fields as conversation output. Report coverage and omitted record
types; preview fallback stays labelled as preview-only. Reconcile Stop-before-flush asynchronously
with bounded retries; publish a later read-update event rather than claiming an incomplete answer
is complete. Never silently skip an oversized record when advancing a cursor.

Reuse an existing bounded terminal snapshot primitive where available, otherwise extract one
from the terminal model with secret obfuscation intact. Distinguish retained shell blocks from
full-screen viewport snapshots. Return non-terminal metadata and a typed unsupported-content
error. Do not enable capture automatically or promise uncaptured historical conversation content.

### 4. Reliable native delivery

Introduce a shared delivery service above the provider-aware PTY submission strategy, not above
raw terminal input. Admission and final submission validate conversation ID, pane incarnation,
foreground agent, writable prompt state, observed input epoch, scope membership, sender ownership,
and local-control mode. Unknown prompt readiness returns a reason or waits when explicitly queued.
Human keyboard/draft activity preempts automated dispatch; remote writer ownership participates in
the same conflict check while preserving its existing desktop-preemption semantics.

Extend native send with staged callbacks and final checks before delayed Enter. If text was pasted
but the target changes before Enter, record uncertain delivery and do not submit into the new
process or try to clear unknown user input. Test both Claude delayed Enter and Codex paste/Enter.

Persist a message journal through app-owned persistence (a dedicated coordination database under
Clinch app data, following existing ownership conventions). Store request ID, payload digest,
sender/run ID, exact target binding, scope revision, text, expiry, timestamps, and state. Commit
`dispatching` before writing any bytes. Repeated same-ID/same-payload requests return the same
receipt; conflicting reuse fails. Use a per-target FIFO and sender lease. Persisted live leases
must be revalidated against the new app incarnation after restart.

State transitions: queued → dispatching → submitted → provider_accepted when provable; queued
may cancel/expire/fail, and interrupted dispatch may become delivery_unknown. A receipt does not
claim provider acceptance from a successful PTY write. Correlate available prompt-submit events
with the expected prompt and prior conversation cursor; when correlation is ambiguous leave it
submitted/unknown. Do not promise exactly-once execution across crashes. On recovery, unfinished
dispatches become unknown and are reconciled without automatic replay. Starting defaults:
100 pending messages per coordinator, ten per target, 30-minute queued-message expiry, seven-day
terminal receipt retention. Expose limits/expiry and return queue-full errors; never silently drop.

### 5. Events and the persistent coordinator

Coordinator state is optional. Normal session launch, provider turns, direct input, transcripts,
and restoration must not depend on a CoordinatorModel entry or an active coordination run.
Create subscriptions and automated wakeups only for explicitly created or attached runs.
Discovery must not enroll sessions automatically. Keep the generic inspection and messaging
APIs usable by an ordinary session without creating a coordinator.

An app-wide `CoordinatorModel` subscribes to agent lifecycle/status events and workspace topology
changes. Maintain an append-only event sequence with typed source and coverage information,
plus materialized coordinator state. Store payload references/cursors instead of copying complete
transcripts into every event. Retain seven days or 100,000 events, whichever limit is reached first;
expired cursors return a resync instruction and snapshot revision. Track read acknowledgements
separately from delivery acknowledgements.

Implement bounded asynchronous long-poll delivery outside the synchronous UI bridge handler;
dispatch subscriptions onto the app model and wait off-thread. `--follow` repeatedly obtains
authorized batches and emits NDJSON, so it can stop promptly on Ctrl-C, app exit, disabled control,
or an expired credential. Do not hold the UI model or reuse a mutation credential for watch.

Meaningful worker events accumulate for a coordinator, debounce for two seconds, and produce at
most one outstanding wakeup. Apply a ten-second minimum automatic wake interval. Wakeups contain
run ID and event range, asking the skill to read new events; they are not worker-supplied prompts.
Deliver through the same queue only when the exact coordinator is idle and writable. Ignore its
own activity and receipt-only churn as wakeup triggers. While busy, coalesce further changes into
the next range. Pause/stop cancels pending wakeups; resume first reconciles unread events.

Persist run configuration, durable scope, coordinator conversation binding, event cursor, pending
wakeup, and receipts. On restart, bind only to restored matching provider conversations/pane
identities. Missing bindings show unavailable; do not launch a replacement or target by display
name. Sleep/app exit provide no live monitoring. Reconcile snapshots and available transcripts
after resume and mark coverage gaps. Treat queue state as desired work, not evidence of delivery.

### 6. Sidebar and setup

Add an app-wide coordinator list/detail model and sidebar entry, reachable from every project
workspace. It is backed by the shared CoordinatorModel rather than the active project's tab group
state. Add `New coordinator` to the command palette and scoped convenience actions to project and
section menus. Extend section header rendering with the existing labelled-action style and a link
to relevant active coordinators. Keep status aggregates compact and expose detail on selection.

Use current Clinch typography, icons, colors, focus behavior, and provider launch controls.
Creation picks a host project and provider separately from scope. Reuse the existing local
Claude/Codex launch path with a generated initial prompt; attach the run after matching the
created session identity. Register launch intent before spawning so retry/recovery cannot duplicate
coordinators. The prompt explicitly invokes the installed coordination skill and carries run ID,
scope summary, and the user's objective. No new API key or model backend is introduced.

Keep the ordinary new-chat flow unchanged and default to an independent session. Coordinator
creation/attachment is a separate optional action, not a required launch step. Stopping or
pausing a run releases its automation while preserving all participating chats and direct input.

The normal conversation gets a coordinator badge; the global list opens that same session even
if its project is inactive. Pause/stop act on the run and never close workers. Surface launch,
capture, skill-provisioning, and local-control errors in setup/details with concrete recovery.
Implementation should include a small reviewed UI prototype using the existing styling before
wiring every state; user preference is current Clinch styling, not a new design language.

### 7. Managed skill and release coordination

Ship a self-contained `resources/bundled/agent-skills/clinch-coordinate/SKILL.md` with a managed
version marker. Existing startup provisioning can install it for Claude and Codex because it
needs no reference-directory copying. Extend installation/release verification tests and update
`clinch-control` capability guidance and stale no-transcript claims. Diagnose missing/user-owned
conflicting skills without overwriting them. The app-bundled CLI and macOS PATH integration already
meet in-Clinch installation needs; retain optional global command installation in Settings.

The skill binds to the current app, discovers all projects, resolves a run's explicit scope,
reads deltas, sends idempotent addressed messages, and uses bounded watches when working actively.
Native wakeups support idle operation; an endlessly running prompt alone does not. Summaries
distinguish observed activity from verified repository/CI/deployment outcomes. Worker text never
grants additional authority, and unsupported interactive dialogs surface to the user.

For release work, persist the run's dependency/order notes and designate one release owner per
repository/deployment resource. Use a shared coordinator resource claim to prevent cooperating
coordinators from independently dispatching the same release. Git/GitHub/deployment commands
remain normal agent tools; the skill verifies branch/commit, combined changes, checks, and deployed
revision. This is cooperative scheduling, not a sandbox capable of blocking arbitrary shell
commands or other external actors. Do not add credentials or deployment-provider coupling to Clinch.

### Implementation sequence

1. Deliver/test all-project discovery, exact identities, normalized read APIs, and scope evaluation.
2. Add/test native delivery lifecycle, durable receipts, and bounded watch.
3. Add/test persistent run state, wakeups, recovery, and shared sender ownership.
4. Wire global sidebar setup/details, contextual entry points, and the managed skill.
5. Verify mixed Claude/Codex end-to-end coordination and release a single coherent feature.

Dependencies make these stages sequential. A partial stage must advertise only implemented
capabilities; do not present a read-only prototype as complete coordination.

## Testing and validation

| Product invariants | Verification |
| --- | --- |
| 1–6 | Snapshot/scope fixtures with two windows, three projects, repeated section names, unsectioned tabs, split panes, tasks, non-agent panes, multiple selected containers, exclusions, and dynamic membership. Change active project between discovery and action; verify exact identity and unchanged focus. Test persistent project UUID restore/duplicate semantics. |
| 7–13 | App UI/action tests for global entry, scoped shortcuts, creation/attach idempotency, host/scope independence, list navigation across projects, pause/stop preservation, and disabled states. Manually inspect current theme, narrow sidebar, keyboard focus, collapsed sections, and provider unavailable flow. |
| 14–16 | Synthetic Claude/Codex transcript fixtures for roles/tools, duplicated records, >5 MiB histories, pagination/tail, delayed flush, oversized/partial/corrupt lines, rotation, missing capture, and unsupported schema. Check preview/coverage labels and secret-obfuscated terminal snapshots. No private user transcripts in tests. |
| 17–19, 21 | Fake PTY/timer tests for both providers; change conversation/foreground/input epoch between paste and Enter. Test human drafts, remote writer conflicts, target closure, same/different-payload retries, cancellation, scope removal, queue expiry/limits, and crash at each durable-write/PTY boundary. Prove uncertain delivery is never auto-replayed. |
| 20, 22, 25 | Deterministic clock/journal tests for cursor resume/expiry, event batching, zero self-wakeup loops, busy coordinator, pause/resume, local-control disable, app restart, missing/rebound identities, sleep gaps, and bounded watch cancellation. Ensure no UI-thread disk/network wait. |
| 23–24 | Staged mixed-provider scenario using disposable repositories and a fake deployment target: cross-project dependency, worker question, failing check, combined integration, competing release owner, and final verified revision. Do not merge or deploy the user's real work as a test. |
| 26–28 | Managed skill provisioning/version/user-owned-file tests, packaging verification, fresh macOS host-shell CLI discovery, exact stable/local instance binding, absent global symlink, read-only/disabled local control, and existing remote/section/toolbelt regression suites. |
| 29 | Launch and converse with ordinary Claude/Codex sessions without any coordinator; mix independent and coordinated sessions in one project; pause/stop/remove the coordinator and disable local control while verifying that independent launch, turns, direct input, and restoration continue. Verify discovery alone never enrolls a session. |

Run focused `local_control` and `warp_cli` tests plus affected `warp` app tests using the repository's
supported macOS build configuration, then its required formatting/lint checks. Validate new CLI
behavior against a freshly built app from the implementation branch, not the separately installed
version. A spec-only change requires document/link/whitespace checks, not a full Rust build.

Manual acceptance: start eight mixed Claude/Codex worker sessions across at least three projects
and several sections; create one coordinator with a union scope, then exercise All Clinch. Keep
another project active while reading and sending; move one worker, add a new one, pause/resume,
restart the app, and inspect recoverable/unknown states. Obtain useful summaries and direct replies
without computer-use clicks. Separately verify all-project observation without creating a coordinator.
