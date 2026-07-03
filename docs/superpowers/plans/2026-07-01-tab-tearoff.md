# Tab Tear-Off Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enable Chrome-style tab tear-off (drag a tab out of the tab bar/vertical panel to detach it into its own window) in every Clinch build, and add a right-click "Move Tab to New Window" entry as a non-drag trigger.

**Architecture:** The tear-off feature (`FeatureFlag::DragTabsToWindows`) is fully implemented upstream (`app/src/workspace/cross_window_tab_drag.rs`) but only enabled via `RELEASE_FLAGS`, which requires the `release_bundle` cargo feature — builds made outside the bundle script silently lose it. Part 1 enables the flag through the cargo-default path so every build gets it. Part 2 adds a new `WorkspaceAction::MoveTabToNewWindow(usize)` that reuses the drag path's existing transfer machinery (`get_tab_transfer_info` → `create_transferred_window` → `remove_tab_without_undo`) — no new transfer logic.

**Tech Stack:** Rust cargo workspace; custom WarpUI framework (Entity-Component-Handle); `cargo nextest` for tests.

**Spec:** `docs/superpowers/specs/2026-07-01-tab-tearoff-design.md` (untracked in the main checkout — Task 0 commits it on the feature branch).

## Global Constraints

- **Work in an isolated git worktree** — multiple Claude sessions share the main checkout at `/Users/ellioteckholm/projects/clinch-terminal`; never build or switch branches there. Task 0 creates the worktree.
- **Branch base:** `clinch/main` (remote `clinch` = elliot-ylambda/clinch-terminal). Never base on the `fork` remote (100+ commits behind).
- **Push with plain `git push`**, never the `gp` alias (it hardcodes an unwritable remote).
- **Mutating shell commands may run with a stripped PATH** — use absolute paths: `/usr/bin/git`, `/opt/homebrew/bin/gh`, `/usr/bin/make`.
- **Exhaustive matching:** no `_` wildcard arms when adding the new action variant to existing matches.
- Rust style (WARP.md): context params named `ctx` and last; inline format args (`format!("{x}")`); no unused params; don't remove unrelated comments.
- **Before any PR:** `./script/format` and the presubmit clippy command must pass.
- The cargo package for the app is named `warp` (directory `app/`).
- Menu label copy, exact: `Move Tab to New Window`.
- Cascade offset for the new window, exact: `30.0` px right and down from the source window origin.

---

### Task 0: Worktree + commit the spec

**Files:**
- Create: worktree at `../clinch-tab-tearoff`, branch `feature/tab-tearoff`
- Create (copy): `docs/superpowers/specs/2026-07-01-tab-tearoff-design.md`
- Create (copy): `docs/superpowers/plans/2026-07-01-tab-tearoff.md`

**Interfaces:**
- Produces: the working directory for ALL subsequent tasks: `/Users/ellioteckholm/projects/clinch-tab-tearoff`

- [ ] **Step 1: Create the worktree off clinch/main**

```bash
cd /Users/ellioteckholm/projects/clinch-terminal
/usr/bin/git fetch clinch
/usr/bin/git worktree add ../clinch-tab-tearoff -b feature/tab-tearoff clinch/main
```

Expected: `Preparing worktree (new branch 'feature/tab-tearoff')`.

- [ ] **Step 2: Copy the spec and plan into the worktree (they are untracked in the main checkout)**

```bash
mkdir -p /Users/ellioteckholm/projects/clinch-tab-tearoff/docs/superpowers/specs /Users/ellioteckholm/projects/clinch-tab-tearoff/docs/superpowers/plans
cp /Users/ellioteckholm/projects/clinch-terminal/docs/superpowers/specs/2026-07-01-tab-tearoff-design.md /Users/ellioteckholm/projects/clinch-tab-tearoff/docs/superpowers/specs/
cp /Users/ellioteckholm/projects/clinch-terminal/docs/superpowers/plans/2026-07-01-tab-tearoff.md /Users/ellioteckholm/projects/clinch-tab-tearoff/docs/superpowers/plans/
```

- [ ] **Step 3: Commit**

```bash
cd /Users/ellioteckholm/projects/clinch-tab-tearoff
/usr/bin/git add docs/superpowers
/usr/bin/git commit -m "docs: add tab tear-off design spec and implementation plan"
```

---

### Task 1: Enable `drag_tabs_to_windows` in default cargo features

**Files:**
- Modify: `app/Cargo.toml` (the `default = [...]` list, insertion point directly after the `"tab_configs",` line, near the other tab-feature entries `"vertical_tabs"` / `"vertical_tabs_summary_mode"`)

**Interfaces:**
- Produces: `FeatureFlag::DragTabsToWindows.is_enabled() == true` in all builds (via the `#[cfg(feature = "drag_tabs_to_windows")]` entry in `app/src/features.rs:100-101`). Tasks 2–3 rely on this only at runtime; their tests use `override_enabled` and do not depend on this task.

- [ ] **Step 1: Edit the default feature list**

In `app/Cargo.toml`, find this block inside `default = [`:

```toml
    "vertical_tabs",
    "vertical_tabs_summary_mode",
    "tab_configs",
```

and change it to:

```toml
    "vertical_tabs",
    "vertical_tabs_summary_mode",
    "tab_configs",
    "drag_tabs_to_windows",
```

(The feature is already declared later in the file as `drag_tabs_to_windows = []`; do not touch that line, and leave the `RELEASE_FLAGS` entry in `crates/warp_features/src/lib.rs` untouched.)

- [ ] **Step 2: Verify it compiles and the feature resolves**

```bash
cd /Users/ellioteckholm/projects/clinch-tab-tearoff
cargo tree -p warp -e features -i warp 2>/dev/null | grep -m1 drag_tabs_to_windows
```

Expected: a line containing `drag_tabs_to_windows` (feature now active in the default graph). If `cargo tree` output is unwieldy, `cargo check -p warp --lib` succeeding after Task 2's code (which is compile-gated anyway) also covers this.

- [ ] **Step 3: Commit**

```bash
/usr/bin/git add app/Cargo.toml
/usr/bin/git commit -m "feat(tabs): enable drag-tabs-to-windows in all builds via default cargo feature"
```

---

### Task 2: `MoveTabToNewWindow` action + context-menu entry

**Files:**
- Modify: `app/src/workspace/action.rs` (~line 136, next to `MoveTabRight(usize)`; and the `should_save_app_state_on_action` match ~line 882)
- Modify: `app/src/tab.rs` (`modify_tab_menu_items` ~line 436, and its single call site ~line 236)
- Modify: `app/src/workspace/view.rs` (action dispatch match ~line 23309)
- Test: `app/src/workspace/view_tests.rs`

**Interfaces:**
- Consumes: nothing from other tasks (flag is overridden in tests).
- Produces:
  - enum variant `WorkspaceAction::MoveTabToNewWindow(usize)` (payload = tab index)
  - stub method `Workspace::move_tab_to_new_window(&mut self, tab_index: usize, ctx: &mut ViewContext<Self>) -> Option<WindowId>` (returns `None`; Task 3 implements it)
  - menu label string `"Move Tab to New Window"`

- [ ] **Step 1: Write the failing tests**

In `app/src/workspace/view_tests.rs`, near the existing tab-context-menu tests (around `test_tab_context_menu_share_session_items`, ~line 2040), add:

```rust
fn menu_contains_item(items: &[MenuItem<WorkspaceAction>], label: &str) -> bool {
    items
        .iter()
        .any(|item| item.is_approximately_same_item_as(&MenuItemFields::new(label).into_item()))
}

#[test]
fn test_tab_context_menu_move_to_new_window_gating() {
    let _guard = FeatureFlag::DragTabsToWindows.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.read(&app, |workspace, ctx| {
            // Multi-tab window: the entry is offered.
            let items = workspace.tabs[0].menu_items(0, 3, &workspace.tab_groups, true, true, ctx);
            assert!(menu_contains_item(&items, "Move Tab to New Window"));

            // Single-tab window: moving the only tab is pointless; hidden.
            let items =
                workspace.tabs[0].menu_items(0, 1, &workspace.tab_groups, false, false, ctx);
            assert!(!menu_contains_item(&items, "Move Tab to New Window"));
        });
    });
}

#[test]
fn test_tab_context_menu_move_to_new_window_hidden_when_flag_off() {
    let _guard = FeatureFlag::DragTabsToWindows.override_enabled(false);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.read(&app, |workspace, ctx| {
            let items = workspace.tabs[0].menu_items(0, 3, &workspace.tab_groups, true, true, ctx);
            assert!(!menu_contains_item(&items, "Move Tab to New Window"));
        });
    });
}
```

If `MenuItemFields` / `MenuItem` are not already imported in `view_tests.rs`, follow the imports used by the existing menu tests in that file.

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd /Users/ellioteckholm/projects/clinch-tab-tearoff
cargo nextest run -p warp -E 'test(move_to_new_window)'
```

Expected: **compile error** — `MoveTabToNewWindow` does not exist yet. (A compile failure of the new test is the failing state here.)

- [ ] **Step 3: Add the action variant**

In `app/src/workspace/action.rs`, immediately after `MoveTabRight(usize),` (~line 136):

```rust
    MoveTabRight(usize),
    /// Detaches the tab at the given index into its own window. Gated by
    /// `FeatureFlag::DragTabsToWindows`; no-ops on single-tab windows.
    MoveTabToNewWindow(usize),
```

In the same file, in `should_save_app_state_on_action` (~line 882), extend the or-group that already contains `MoveTabLeft(_)` / `MoveTabRight(_)`:

```rust
            | MoveTabLeft(_)
            | MoveTabRight(_)
            | MoveTabToNewWindow(_)
```

- [ ] **Step 4: Add the menu entry**

In `app/src/tab.rs`, change `modify_tab_menu_items` (~line 436) to take `tabs_len`:

```rust
    fn modify_tab_menu_items(
        &self,
        index: usize,
        tabs_len: usize,
        can_move_left: bool,
        can_move_right: bool,
        pane_name_target: Option<PaneNameMenuTarget>,
        ctx: &AppContext,
    ) -> Vec<MenuItem<WorkspaceAction>> {
```

and at the end of that function, after the `if can_move_left { ... }` block and before `menu_items`:

```rust
        // Non-drag path for tab tear-off: only offered when the feature that
        // powers the underlying window transfer is on, and when the window
        // has another tab left to keep (the handler re-validates both).
        if FeatureFlag::DragTabsToWindows.is_enabled() && tabs_len > 1 {
            menu_items.push(
                MenuItemFields::new("Move Tab to New Window")
                    .with_on_select_action(WorkspaceAction::MoveTabToNewWindow(index))
                    .into_item(),
            );
        }
        menu_items
```

Update its single call site inside `menu_items_with_pane_name_target` (~line 236):

```rust
            self.modify_tab_menu_items(index, tabs_len, can_move_left, can_move_right, pane_name_target, ctx),
```

- [ ] **Step 5: Add the dispatch arm and stub handler**

In `app/src/workspace/view.rs`, in the `handle_action` match next to the existing arms (~line 23309):

```rust
            MoveTabRight(index) => self.move_tab(*index, TabMovement::Right, ctx),
            MoveTabToNewWindow(index) => {
                self.move_tab_to_new_window(*index, ctx);
            }
```

And add the stub method near `remove_tab_without_undo` (~line 27367):

```rust
    /// Moves the tab at `tab_index` into its own new window. Returns the new
    /// window's id, or `None` when the move is not possible (single-tab
    /// window or unknown window bounds). Implemented in the next commit.
    pub(crate) fn move_tab_to_new_window(
        &mut self,
        tab_index: usize,
        ctx: &mut ViewContext<Self>,
    ) -> Option<WindowId> {
        let _ = (tab_index, ctx);
        None
    }
```

- [ ] **Step 6: Fix remaining exhaustive matches**

```bash
cargo check -p warp --lib 2>&1 | head -50
```

If the compiler reports other non-exhaustive matches on `WorkspaceAction` (the codebase forbids `_` arms), add `MoveTabToNewWindow(_)` to each, grouped with `MoveTabLeft(_)` / `MoveTabRight(_)` and mirroring whatever those two return in that match. Repeat until `cargo check -p warp --lib` passes.

- [ ] **Step 7: Run the tests to verify they pass**

```bash
cargo nextest run -p warp -E 'test(move_to_new_window)'
```

Expected: 2 tests PASS (`test_tab_context_menu_move_to_new_window_gating`, `test_tab_context_menu_move_to_new_window_hidden_when_flag_off`).

- [ ] **Step 8: Commit**

```bash
/usr/bin/git add app/src/workspace/action.rs app/src/tab.rs app/src/workspace/view.rs app/src/workspace/view_tests.rs
/usr/bin/git commit -m "feat(tabs): add Move Tab to New Window context-menu entry and action"
```

---

### Task 3: Implement the `move_tab_to_new_window` handler

**Files:**
- Modify: `app/src/workspace/view.rs` (replace the Task 2 stub, ~line 27367)
- Test: `app/src/workspace/view_tests.rs`

**Interfaces:**
- Consumes: `WorkspaceAction::NewTab` (existing), `Workspace::move_tab_to_new_window` stub signature from Task 2, and these existing helpers (all already in `view.rs` / `root_view.rs`):
  - `self.get_tab_transfer_info(index, ctx) -> Option<TransferredTab>` (returns `None` when `tabs.len() <= 1`)
  - `ctx.window_bounds(&ctx.window_id()) -> Option<RectF>`
  - `crate::root_view::create_transferred_window(transferred_tab, source_window_id, window_size, window_position, is_tab_drag_preview, ctx) -> WindowId`
  - `self.remove_tab_without_undo(index, ctx)`
- Produces: working handler returning `Some(new_window_id)` on success.

- [ ] **Step 1: Write the failing tests**

In `app/src/workspace/view_tests.rs`, after the menu tests from Task 2:

```rust
#[test]
fn test_move_tab_to_new_window_transfers_tab() {
    let _guard = FeatureFlag::DragTabsToWindows.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            workspace.handle_action(&WorkspaceAction::NewTab, ctx);
            assert_eq!(workspace.tab_count(), 2);
        });
        let moved_pane_group_id =
            workspace.read(&app, |workspace, _| workspace.tabs[1].pane_group.id());

        let new_window_id = workspace.update(&mut app, |workspace, ctx| {
            workspace.move_tab_to_new_window(1, ctx)
        });
        let new_window_id =
            new_window_id.expect("moving a tab out of a 2-tab window should create a window");

        // Source window keeps one tab; the moved pane group is gone from it.
        workspace.read(&app, |workspace, _| {
            assert_eq!(workspace.tab_count(), 1);
            assert_ne!(workspace.tabs[0].pane_group.id(), moved_pane_group_id);
        });

        // The new window's workspace adopted the transferred pane group.
        app.read(|ctx| {
            let new_workspace = WorkspaceRegistry::as_ref(ctx)
                .get(new_window_id, ctx)
                .expect("new window should have a registered workspace");
            new_workspace.read(ctx, |new_workspace, _| {
                assert_eq!(new_workspace.tab_count(), 1);
                assert_eq!(new_workspace.tabs[0].pane_group.id(), moved_pane_group_id);
            });
        });
    });
}

#[test]
fn test_move_tab_to_new_window_noops_on_single_tab_window() {
    let _guard = FeatureFlag::DragTabsToWindows.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);

        let result = workspace.update(&mut app, |workspace, ctx| {
            assert_eq!(workspace.tab_count(), 1);
            workspace.move_tab_to_new_window(0, ctx)
        });

        assert!(result.is_none());
        workspace.read(&app, |workspace, _| assert_eq!(workspace.tab_count(), 1));
    });
}
```

Adaptation notes for the implementer (only if the harness objects — otherwise change nothing):
- If `WorkspaceRegistry` is not imported in `view_tests.rs`, import it from `crate::workspace::registry::WorkspaceRegistry` (check `app/src/workspace/registry.rs` for the exact path/import style used elsewhere).
- If `ViewHandle::read` inside `app.read` has a different closure shape, mirror how `pane_group.read(ctx, |pg, ctx| ...)` is called in `view.rs:27076`.
- If the test panics on a missing singleton when `create_transferred_window` runs, register that singleton in this test right after `initialize_app(&mut app)` following the `app.add_singleton_model(...)` pattern in `initialize_app` (view_tests.rs:110–230) — do not modify `initialize_app` itself.

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo nextest run -p warp -E 'test(move_tab_to_new_window)'
```

Expected: `test_move_tab_to_new_window_transfers_tab` FAILS on the `expect` (stub returns `None`); `test_move_tab_to_new_window_noops_on_single_tab_window` may already pass (stub returns `None`) — that's fine.

- [ ] **Step 3: Implement the handler**

Replace the Task 2 stub in `app/src/workspace/view.rs` with:

```rust
    /// Moves the tab at `tab_index` into its own new window, reusing the
    /// cross-window drag machinery's transfer path (create the target window
    /// first, then remove the source tab — the same order as a drag handoff).
    /// Returns the new window's id, or `None` when the move is not possible
    /// (single-tab window or unknown window bounds).
    pub(crate) fn move_tab_to_new_window(
        &mut self,
        tab_index: usize,
        ctx: &mut ViewContext<Self>,
    ) -> Option<WindowId> {
        let transferred_tab = self.get_tab_transfer_info(tab_index, ctx)?;
        let window_bounds = ctx.window_bounds(&ctx.window_id())?;

        // Cascade the new window off the source so it's visibly a new window
        // rather than appearing perfectly stacked on the old one.
        const CASCADE_OFFSET_PX: f32 = 30.0;
        let window_position =
            window_bounds.origin() + vec2f(CASCADE_OFFSET_PX, CASCADE_OFFSET_PX);

        if let Some(tab) = self.tabs.get(tab_index) {
            ctx.unsubscribe_to_view(&tab.pane_group);
        }

        let source_window_id = ctx.window_id();
        let new_window_id = crate::root_view::create_transferred_window(
            transferred_tab,
            source_window_id,
            window_bounds.size(),
            window_position,
            false,
            ctx,
        );
        self.remove_tab_without_undo(tab_index, ctx);
        ctx.notify();
        Some(new_window_id)
    }
```

Notes:
- `vec2f` and `WindowId` are already used throughout `view.rs`; no new imports should be needed. If the compiler disagrees, add the import at the top of the file per the existing import style.
- `remove_tab_without_undo` → `remove_tab(index, false, false, ctx)` handles active-tab-index fixup; do not duplicate it.
- The unsubscribe-before-remove pairing mirrors `handle_drop_result`'s `DropResult::RemoveSourceTab` branch (`view.rs:27944-27951`).

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo nextest run -p warp -E 'test(move_tab_to_new_window) or test(move_to_new_window)'
```

Expected: all 4 tests PASS.

- [ ] **Step 5: Commit**

```bash
/usr/bin/git add app/src/workspace/view.rs app/src/workspace/view_tests.rs
/usr/bin/git commit -m "feat(tabs): implement move_tab_to_new_window via cross-window transfer"
```

---

### Task 4: Presubmit, regression tests, PR

**Files:**
- No new files; runs checks over the branch.

**Interfaces:**
- Consumes: all previous tasks committed.
- Produces: an open PR against `main` on `elliot-ylambda/clinch-terminal`.

- [ ] **Step 1: Format and lint (must pass before any PR)**

```bash
cd /Users/ellioteckholm/projects/clinch-tab-tearoff
./script/format
cargo clippy --workspace --all-targets --all-features --tests -- -D warnings
```

Expected: format makes no changes (or commit its changes with `style: format`); clippy exits 0. Fix any warnings before proceeding.

- [ ] **Step 2: Run the surrounding regression suites**

```bash
cargo nextest run -p warp -E 'binary_id(warp) and test(workspace::)' --no-fail-fast
```

Expected: PASS (pre-existing failures, if any, must also be failing on `clinch/main` — verify with `git stash`-free comparison only if failures appear).

- [ ] **Step 3: Push and open the PR**

```bash
/usr/bin/git push -u clinch feature/tab-tearoff
```

Then create the PR with `/opt/homebrew/bin/gh`, using the template at `.github/pull_request_template.md`, base `main`, repo `elliot-ylambda/clinch-terminal`:

```bash
/opt/homebrew/bin/gh pr create --repo elliot-ylambda/clinch-terminal --base main \
  --title "feat(tabs): tab tear-off — drag tabs out to new windows + Move Tab to New Window menu entry" \
  --body-file <(cat <<'EOF'
[Fill in the PR template sections from .github/pull_request_template.md:
- Summary: enable DragTabsToWindows in all builds via default cargo feature; add
  right-click "Move Tab to New Window" entry reusing the cross-window transfer path.
- Testing: menu gating unit tests, handler transfer/no-op unit tests, manual
  drag + menu verification in WarpLocal (Task 5).]

CHANGELOG-NEW-FEATURE: Tabs can now be torn off into their own window — drag a tab out of the tab bar or vertical tab panel, or right-click a tab and choose "Move Tab to New Window".
EOF
)
```

(If the template has required sections, copy them verbatim and fill them in; do not skip the template.)

---

### Task 5: Manual verification in the real app (human-in-the-loop)

**Files:**
- None (build + hand-testing).

**Interfaces:**
- Consumes: the feature branch.
- Produces: verified UX; go/no-go for merge.

- [ ] **Step 1: Build and install WarpLocal from the worktree**

```bash
cd /Users/ellioteckholm/projects/clinch-tab-tearoff
/usr/bin/make install-local
```

Expected: `/Applications/WarpLocal.app` updated (self-signed; ~10–20 min release build).

- [ ] **Step 2: Hand-verify with the user (relaunch WarpLocal.app first)**

Checklist to walk through with the user:
1. Vertical panel: drag a tab well to the right, out of the panel → tab detaches into a floating window that follows the cursor; release → standalone window.
2. Drag a tab from one window into another window's tab strip → it re-attaches there.
3. Right-click a tab (≥2 tabs) → "Move Tab to New Window" appears; click → focused new window, offset ~30px from the source; source window keeps remaining tabs.
4. Right-click the only tab of a window → entry absent.
5. Horizontal tab bar layout (toggle vertical tabs off in settings): repeat checks 1 and 3.
6. Terminal sessions in the moved tab keep running (scrollback, running process intact) — the transfer is in-process.

- [ ] **Step 3: After user sign-off, merge per repo flow and ship**

Merge the PR, then from the main checkout on `main`: `/usr/bin/make ship` to rebuild/install the personal app and publish the Clinch release (replaces the outdated OSS-built `Clinch.app` that had the feature compiled off).

---

## Self-Review Notes (already applied)

- Spec coverage: Part 1 → Task 1; Part 2 (action/menu/handler) → Tasks 2–3; edge cases (single-tab no-op, flag off) → tests in Tasks 2–3; manual drag verification + both layouts → Task 5; grouped/pinned semantics need no code (TransferredTab carries neither) — asserted by upstream behavior, not re-tested.
- Type consistency: `move_tab_to_new_window(&mut self, tab_index: usize, ctx: &mut ViewContext<Self>) -> Option<WindowId>` identical in Task 2 (stub) and Task 3 (impl); menu label string identical in Task 2 menu code and tests.
- No placeholders: every code step contains the actual code; adaptation notes are bounded to concrete, named fallbacks.
