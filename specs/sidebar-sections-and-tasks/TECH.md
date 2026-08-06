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
`TextInput`, provider icons, hover treatment, and chrome divider tokens.

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
  pinning, and persistence; a dedicated test verifies that removing a section preserves its
  sessions (PRODUCT 1–17).
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
