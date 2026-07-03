# Tab tear-off for Clinch — design

**Date:** 2026-07-01
**Status:** Approved (pending implementation)

## Problem

Dragging a tab in the vertical (left-hand) tab panel only reorders it vertically;
it cannot be pulled out to become its own OS window (Chrome-style tear-off).

## Root cause (investigated)

Upstream Warp already ships full tab tear-off as `FeatureFlag::DragTabsToWindows`
(`app/src/workspace/cross_window_tab_drag.rs` + glue in
`app/src/workspace/view.rs`), wired for both the horizontal tab bar and the
vertical tabs panel. When the flag is **off**, tab drags are deliberately
axis-locked (`DragAxis::VerticalOnly` at
`app/src/workspace/view/vertical_tabs.rs:2462`, `HorizontalOnly` at
`app/src/tab.rs:2096`).

The flag is enabled only via `RELEASE_FLAGS`
(`crates/warp_features/src/lib.rs:995`), which applies only when the binary is
compiled with the `release_bundle` cargo feature. The installed `Clinch.app`
(built Jun 30 via the older OSS-channel path, binary `warp-oss`) was compiled
**without** `release_bundle`, so the flag — and tear-off — is off. Confirmed
empirically: `Autoupdate` (same `RELEASE_FLAGS` switch) shows no activity in
that app's log.

## Design

### Part 1 — enable the feature in every build

Add `"drag_tabs_to_windows"` to the `default = [...]` feature list in
`app/Cargo.toml` (feature is declared at line ~701; `vertical_tabs` is already
in the default list). This enables the flag through the cfg-gated path in
`app/src/features.rs:100-101` regardless of `release_bundle`, covering:
`cargo run` dev builds, `WarpLocal.app` (`make install-local`), and the Clinch
DMG (`make release`).

- Keep the upstream `RELEASE_FLAGS` entry untouched (harmless overlap; minimal
  diff vs upstream).
- Upstream cfg-gates the `RELEASE_FLAGS` entry to macOS/Windows; the cargo
  default is not OS-gated. Acceptable: Clinch's ship flow is macOS-only.

No further code changes are needed for drag tear-off itself — free-axis drag,
detach detection, floating preview, drop-into-other-window all activate with
the flag.

### Part 2 — "Move Tab to New Window" right-click entry

The drag gesture is currently the only trigger. Add a discoverable non-drag
path:

- **Action:** new `WorkspaceAction::MoveTabToNewWindow(usize)` in
  `app/src/workspace/action.rs`, modeled on `MoveTabRight(usize)`. The repo's
  exhaustive-match convention surfaces every site that must handle it.
- **Menu item:** in `Tab::modify_tab_menu_items` (`app/src/tab.rs:436`), after
  the "Move Tab Up/Down" entries, add `"Move Tab to New Window"`. Visible only
  when:
  - `FeatureFlag::DragTabsToWindows.is_enabled()` (one flag governs the whole
    feature), and
  - the window has more than one tab (`tabs_len > 1`; `tabs_len` is passed into
    this section — signature change, all call sites updated).
  Both tab layouts share this menu builder, so the entry appears in both.
- **Handler:** `Workspace::move_tab_to_new_window(index, ctx)` in
  `app/src/workspace/view.rs`, reusing the drag path's machinery:
  1. `get_tab_transfer_info(index, ctx)` — already returns `None` for
     single-tab windows (handler no-ops; menu gating is a UI courtesy, the
     handler revalidates).
  2. Compute new window bounds: same size as the source window, offset by a
     small cascade (30 px down/right). No screen-area clamping — the drag
     tear-off path this mirrors positions windows in screen space without
     clamping, and there is no clean workspace-level work-area API; the
     worst case (a window nudged slightly off a screen corner after repeated
     tear-offs) is minor and recoverable via normal window management.
  3. `ctx.unsubscribe_to_view(&tab.pane_group)` (NOT
     `prepare_for_transferred_tab_attach` — that also sets a
     suppress-detach-on-close flag that is never auto-restored and is only
     needed when the source window closes; ours never does, since ≥2 tabs are
     guaranteed), then `create_transferred_window(transferred_tab,
     source_window_id, size, position, /*is_tab_drag_preview=*/ false, ctx)`
     (`app/src/root_view.rs:586`) — creates a focused `WindowStyle::Normal`
     window and transfers the live pane group in-process
     (`transfer_view_tree_to_window`). No new transfer logic.
  4. Remove the now-transferred tab from the source workspace using the same
     mechanics as the drag handoff's `DropResult::RemoveSourceTab` branch in
     `handle_drop_result` (window creation precedes removal, matching the drag
     flow where the preview window exists before the source tab is dropped).

### Edge cases

- **Grouped / pinned tabs:** `TransferredTab` carries neither `group_id` nor
  `pinned`; a moved tab lands ungrouped/unpinned in the new window — identical
  to existing drag-detach semantics.
- **Active-tab fixup** in the source window rides the existing tab-removal
  path.
- **Single-tab window:** menu entry hidden; handler additionally no-ops.
- **Telemetry:** none added — Clinch ships no telemetry backend.

### Testing

- Unit tests (per repo `*_tests.rs` conventions):
  - menu gating: entry hidden when flag off or `tabs_len == 1`; shown
    otherwise.
  - action handler: dispatching `MoveTabToNewWindow` on a 2-tab workspace
    removes the tab from the source and creates a new window adopting the pane
    group (follow `view_tests.rs` patterns; use `override_enabled` for the
    flag).
- Manual verification after `make install-local`:
  - drag a tab out of the vertical panel → floating window; drop → standalone
    window; drag back into a tab strip → re-attaches.
  - right-click a tab → "Move Tab to New Window" → focused new window.
  - repeat the drag check in the horizontal tab bar layout.

### Dead code check

No dead code is created: the flag and both trigger paths stay live, matching
upstream structure. Nothing to remove.

## Out of scope

- Tear-off for whole tab groups (upstream intentionally locks group drags
  vertical).
- Linux support for cross-window drag (upstream excludes it; Clinch ships
  macOS-only).
- Replacing the outdated installed `Clinch.app` build is an operational step
  (`make ship`), not part of this change.
