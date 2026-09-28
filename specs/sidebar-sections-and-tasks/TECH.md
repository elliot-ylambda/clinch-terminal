# Sidebar Sections and Tasks — Technical Plan

## Context

This plan implements [PRODUCT.md](./PRODUCT.md) against Clinch commit
`f4c8033e8e90dda4c8b6a4b30c08945a1e3d8616`.

- [`app/src/workspace/tab_group.rs`](https://github.com/elliot-ylambda/clinch-terminal/blob/f4c8033e8e90dda4c8b6a4b30c08945a1e3d8616/app/src/workspace/tab_group.rs)
  already models named, collapsible groups with UUID-backed runtime identity.
- [`app/src/workspace/view/vertical_tabs.rs`](https://github.com/elliot-ylambda/clinch-terminal/blob/f4c8033e8e90dda4c8b6a4b30c08945a1e3d8616/app/src/workspace/view/vertical_tabs.rs#L2065)
  already renders grouped vertical tabs as outlined draggable cards. The feature is currently
  exposed only through `FeatureFlag::GroupedTabs`.
- [`app/src/workspace/view.rs`](https://github.com/elliot-ylambda/clinch-terminal/blob/f4c8033e8e90dda4c8b6a4b30c08945a1e3d8616/app/src/workspace/view.rs#L7461)
  owns group CRUD, app-state snapshots, inline editors, session creation, and the shared
  Claude/Codex launch path.
- [`app/src/app_state.rs`](https://github.com/elliot-ylambda/clinch-terminal/blob/f4c8033e8e90dda4c8b6a4b30c08945a1e3d8616/app/src/app_state.rs#L84)
  and [`app/src/persistence/sqlite.rs`](https://github.com/elliot-ylambda/clinch-terminal/blob/f4c8033e8e90dda4c8b6a4b30c08945a1e3d8616/app/src/persistence/sqlite.rs#L1006)
  persist one `WindowSnapshot` per project workspace.
- [`crates/clinch_companion_protocol/src/lib.rs`](https://github.com/elliot-ylambda/clinch-terminal/blob/f4c8033e8e90dda4c8b6a4b30c08945a1e3d8616/crates/clinch_companion_protocol/src/lib.rs)
  is the Rust/TypeScript contract for Remote Control, and
  [`app/src/remote_control/workspace_adapter.rs`](https://github.com/elliot-ylambda/clinch-terminal/blob/f4c8033e8e90dda4c8b6a4b30c08945a1e3d8616/app/src/remote_control/workspace_adapter.rs#L1336)
  builds authoritative project snapshots and handles mutations.
- [`web/remote-control/src/app/App.tsx`](https://github.com/elliot-ylambda/clinch-terminal/blob/f4c8033e8e90dda4c8b6a4b30c08945a1e3d8616/web/remote-control/src/app/App.tsx#L1370)
  renders the project/session drawer and already supports Claude/Codex creation with an initial
  prompt.

## Proposed changes

### Sections

Keep `TabGroup` and `TabGroupId` as internal names to avoid a broad mechanical rewrite. Enable the
existing `grouped_tabs` Cargo feature for normal Clinch builds and replace user-facing **group**
copy with **section** copy.

Change the group-header close affordance to dispatch `UngroupTabs`, preserving sessions. Keep
`Close all sessions in section` in the overflow menu. Extend vertical drag hit testing so a
collapsed group container accepts a session drop and appends it to that group.

Add a full-width primary section action at the bottom of the vertical-tabs panel, below the Tasks
area. It dispatches the same new-section path as the menu, creating the first session and opening
the inline name editor. Add `SetTabGroupColor` and a context-menu color-dot row backed by the
existing six `TAB_COLOR_OPTIONS`. Render the chosen ANSI color as a low-opacity card fill and
outline. Reuse the existing persisted `TabGroup.color`/`TabGroupSnapshot.color` field, so no new
migration or Remote Control protocol field is required. Show a plus action with the other
section-header controls on hover and dispatch the existing `NewTabInGroup` path so the normal
new-session behavior targets that section and expands it when needed.

No new section persistence schema is required for this MVP: membership, names, collapsed state,
and effective order already round-trip through `TabGroupSnapshot`, `tabs.tab_group_id`, and tab
order. Empty sections remain intentionally unsupported.

### Project tasks

Add `workspace/task.rs` with:

- `WorkspaceTaskId(Uuid)`, serialized as a UUID.
- `WorkspaceTask { id, text }`, where vector position is display order.

Add `tasks: Vec<WorkspaceTask>` and `tasks_collapsed: bool` to `Workspace`, plus a single-line
`EditorView` for new task input. The editor handles Enter and Escape; Workspace actions handle
remove, collapse, focus, and provider launch. Task launch calls the existing shell-quoted
`remote_control_open_cli_agent`/`launch_cli_agent_in_new_tab` path, returning a boolean so removal
happens only after a local terminal manager accepts the one-shot command.

Render the Tasks area below the sessions scroller in `vertical_tabs.rs`. Keep its own bounded
scroll state and stable mouse handles in `VerticalTabsPanelState`. Reuse the existing theme,
`TextInput`, provider icons, and hover treatment. Give the Tasks container a one-pixel green top
border using the exact same fill helper as the full-width section button outline.

### Persistence

Add serialized `tasks` and `tasks_collapsed` columns to `windows` in a new reversible SQLite
migration. Extend persistence `Window`/`NewWindow`, Diesel schema, `WindowSnapshot`, save, and
restore. Serialize task snapshots as JSON with an empty-list fallback for legacy/corrupt values.

Every task mutation dispatches `workspace:save_app`. Existing full app-state replacement then keeps
project isolation aligned with tab/section persistence.

### Remote Control

Extend the companion protocol with:

- `TaskId` and `TaskSnapshot`.
- `TabSnapshot.section_id: Option<String>` and `section_name: Option<String>` so adjacent
  same-named sections remain distinct.
- `ProjectSnapshot.tasks: Vec<TaskSnapshot>`.
- `CreateTask`, `DeleteTask`, and `LaunchTask` messages.

Task mutations carry `app_instance_id`, `workspace_revision`, `project_id`, and durable `task_id`
where applicable. Create/delete require `Control`; launch requires `CreateSession`. The workspace
adapter resolves the exact runtime project, applies the Workspace mutation, bumps the topology
revision, and returns the normal command acknowledgement. Task IDs/text and section labels
participate in the topology fingerprint so connected clients receive changes.

Regenerate checked-in JSON Schema and TypeScript types. Update the Remote Control drawer to group
tabs by `section_name`, render the selected project's task list, add tasks inline, and offer Claude,
Codex, and delete actions. Buttons are disabled while disconnected or while a mutation is in
flight; authoritative snapshots always replace optimistic assumptions.

### Local coding-agent control

Extend the existing `local_control` action catalog and `warpctrl` parser with typed `section.*` and
`toolbelt.*` actions. These actions retain the existing owner-only discovery, same-UID credential
broker, short-lived action-scoped grants, and loopback bridge; no new MCP transport or
unauthenticated socket is introduced.

Section handlers resolve the target Workspace through the normal selector resolver and reuse the
existing `TabGroup` mutation paths. `section.create` creates a named group from the selected tab;
update, move, add/remove-tab, and delete dispatch the normal workspace save action. List responses
return ordered UUID-backed section IDs and member tab IDs. Delete calls the non-destructive ungroup
path, never `close_tab_group`.

Toolbelt handlers operate on one canonical coding-agent `SessionSettings` selection for Claude Code
and Codex, plus the independent terminal selection. Either provider selector reads and mutates the
same coding-agent value. A compatibility resolver imports the least-stale provider override from
the short-lived split-layout format, while exact retired shipped defaults are removed only from
pre-overlay full snapshots or re-saved layouts with the complete historical shipped signature;
edited recipes, individually selected presets, and genuine custom buttons remain. Once changed,
the new optional canonical field is the migration marker and old shared/provider keys cannot
resurrect their layouts. Rendering, the editor, Remote Control, and local control all resolve this same
effective selection. Handlers rebuild selections through
`custom_from_effective_items_and_hidden_custom_inserts`, preserving live defaults, explicit hidden
defaults, and ordered overlays. Button labels are validated as unique exact selectors. Create and
move use bounded zero-based positions; deleting a custom entry removes it, while deleting a shipped
entry records it as hidden through the existing selection normalization. Bookmark and Transfer
retain their configured slots but render disabled until the active session exposes the identity
needed to perform them.

Extend the existing local Claude/Codex prompt-mirror hook with a bounded, fail-open learner. On
`UserPromptSubmit`, normalize case and whitespace, reject secret-looking, destructive, oversized,
or obvious one-off text, and update an owner-only learning store under the existing agent-resume
registry. A singleton stores only a stable fingerprint, timestamps, and provider-scoped session
identity. On the same fingerprint's second distinct conversation, retain the latest exact prompt
as an eligible candidate. Repeats in one session do not increase the conversation count. Serialize
concurrent hooks with a short-lived registry lock, write through a mode-0600 temporary file and
atomic rename, cap the store to the most recent 512 patterns, and never fail prompt submission when
learning cannot update. Each pattern retains at most 32 recent session identities plus a monotonic
aggregate count so one exceptionally common prompt cannot make the store grow without bound.

Add typed `toolbelt.suggestion.list` and `toolbelt.suggestion.resolve` local-control actions.
`suggestion.list` accepts a Claude Code or Codex footer and returns only eligible unresolved
candidates plus aggregate conversation count, providers, and last-seen time. It does not return
transcript/session contents or singleton records, and filters exact text already installed in the
target footer. `suggestion.resolve` accepts a candidate UUID and `accepted` or `declined`, verifies
that the candidate exists, and atomically persists the decision in a separate owner-only file so
the app and capture hook never compete to rewrite one document. When session capture is disabled,
the list is empty and resolution is unavailable. Existing capture purge removes both stores.

Update the managed `clinch-toolbelt` skill so an agent performs one lightweight pending-suggestion
check at the beginning of a new Clinch conversation as well as recognizing patterns in its current
visible context. It proposes the complete label, exact text, target footer, side, and `auto_send:
false`; one affirmative response authorizes the existing typed button-create action followed by an
accepted resolution. A decline records a declined resolution. The agent sees only the candidate,
not the conversations that produced it, and the learner emits no telemetry or network traffic.

The managed `clinch-control` and `clinch-toolbelt` skills are installed at user scope for Claude
Code and Codex only when the current bundle contains its channel-specific control wrapper. Stable
and local macOS build entrypoints compile `warp_control_cli`, create that wrapper, and release
verification requires the wrapper plus both skills. Keep the existing `LocalControlSettings`
permission boundary, but make its missing-value default `Enabled` for the Clinch stable and local
app IDs. Preserve an explicitly stored `Disabled` value, keep other public Warp channels on their
existing opt-in default, and fail closed to `Disabled` when a stored value is unreadable or
malformed. Present the page as **Local control** and distinguish the optional global CLI symlink
from the bundled current-app command used by agents.

Keep the installed skill text channel-neutral. New local host terminal shells export
`CLINCH_CONTROL_COMMAND`, `CLINCH_CONTROL_WRAPPER`, and `CLINCH_CONTROL_PID`. The wrapper points to
the exact app bundle that spawned the shell, while the PID selects that exact running instance even
when multiple worktree builds share the local channel. The app process records its PID in
`PtyOptions` before terminal-server serialization. Shell creation first removes inherited and
caller-supplied binding values, then applies the verified current-app binding so starting one Clinch
build from another cannot leak or spoof the parent's identity. Agents prefer the exact wrapper and
PID, reject an incomplete binding, and treat a Clinch terminal without these variables as an older
version requiring update and relaunch. This avoids a shared user-scope file retaining the last
launched channel's absolute bundle path while preserving managed-version upgrades and all
user-owned unmarked skill files. Codex provisioning also removes an obsolete managed copy from
`~/.codex/skills` only after an equal-or-newer replacement is readable under `~/.agents/skills`;
unmarked legacy files and non-empty user directories are preserved. Docker sandbox launch strips
all three bindings rather than exposing a host-app control capability or an unreachable host bundle
path inside the container.

## End-to-end flow

1. A task is added on Mac or phone and committed to the owning Workspace vector.
2. Workspace persistence snapshots it into that project's `windows.tasks` JSON.
3. Remote Control polling observes the changed fingerprint and pushes a new full snapshot.
4. Launch resolves the task by UUID, opens the provider session with the task text as its initial
   prompt, removes the task only after launch setup succeeds, and triggers save plus another remote
   snapshot.

## Testing and validation

- Task model and Workspace tests cover trimming, empty/oversized rejection, insertion order,
  removal, and stable UUID identity. Existing section tests cover creation, collapse, ordering,
  pinning, persisted color, and persistence; focused coverage verifies color changes and that
  removing a section preserves its sessions (PRODUCT 1–17).
- A SQLite round-trip test covers task UUID/text/order and collapsed state across save/restore;
  migration defaults cover legacy rows (PRODUCT 9, 16).
- Companion protocol tests cover all task message variants, validation, and schema generation.
  Workspace-adapter tests verify the Control/CreateSession capability split and idempotency
  classification (PRODUCT 17–22).
- Remote Control unit tests cover stable-ID section grouping, including adjacent same-named
  sections. Protocol generation checks, TypeScript type checking, the web test suite, and the
  production Vite build validate the complete Rust-to-browser contract (PRODUCT 18–22).
- Rust formatting, `clinch_companion_protocol` tests, targeted Workspace/persistence/adapter tests,
  and `cargo check -p warp` are the native validation gate.
- Local-control protocol and parser tests cover every new catalog action. Toolbelt mutation tests
  cover exact insertion, range rejection, label ambiguity, and hiding shipped defaults; Workspace
  coverage verifies named section creation from an existing tab and non-destructive ungrouping.
- Managed-skill tests cover wrapper-gated provisioning, safe version upgrades, preservation of
  user-owned files, conservative cleanup of obsolete Codex copies, proactive repeated-pattern
  discovery, cross-conversation pending-suggestion checks, and the one-confirmation boundary.
  Prompt-mirror tests cover the two-distinct-session threshold, same-session rejection, bounded
  pruning, atomic owner-only storage, capture opt-out, and filters for secrets, destructive text,
  and one-off identifiers. Local-control tests cover suggestion list/resolve serialization,
  provider-footer filtering, existing-button suppression, and durable accepted/declined outcomes.
  Shell-boundary tests cover exact wrapper/PID
  injection, stale parent/override scrubbing, and denial inside Docker sandboxes. Stable
  compilation plus a no-launch local app bundle verify that build entrypoints ship an executable
  wrapper and both placeholder-free skills.
- Local-control setting tests cover the default-on Clinch app IDs, unchanged defaults for other
  public channels, persistence of an explicit opt-out, and fail-closed malformed secure storage.
- Manual launch remains recommended for final visual inspection of section outlines, collapsed
  drop targeting, provider buttons, restart restoration, and the narrow mobile drawer layout.

## Parallelization

Parallel implementation is not proposed. Native Workspace state, persistence snapshots, protocol
generation, adapter handling, and generated web types form a strict dependency chain, while the
section and task UI both edit `Workspace` and `vertical_tabs.rs`. Sequential work in the current
`jasper-mirador` worktree avoids conflicting edits and lets each generated contract be validated
before the next layer consumes it.

## Risks and mitigations

- **Task loss during launch:** launch returns success only after the new tab's terminal manager
  accepts the one-shot command; otherwise the task remains.
- **Oversized or sensitive task text:** reuse the companion prompt byte limit, validate all remote
  input, send task text only to authenticated paired devices, and emit no task-content telemetry.
- **Feature-flag data loss:** compile `grouped_tabs` by default so normal Clinch snapshots no longer
  erase section membership while the UI is unavailable.
- **Stale mobile mutations:** retain existing app-instance and revision guards and use UUID task
  identity instead of list positions.

## Follow-ups

Automation, schedules, task/session status synchronization, empty persistent sections, offline
mobile sync, and cross-Mac task sync are explicitly deferred.
