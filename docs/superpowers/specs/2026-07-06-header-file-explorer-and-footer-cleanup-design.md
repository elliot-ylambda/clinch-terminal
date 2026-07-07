# Header File Explorer button + CLI-agent footer cleanup

**Date:** 2026-07-06
**Status:** Design — awaiting implementation plan

## Goal

Two related UI changes to the Clinch terminal chrome:

1. **Add a File Explorer toggle to the top-left header**, immediately to the
   right of the collapsible tabs button, so the file tree can be opened from
   the header.
2. **Declutter the CLI-agent bottom footer** by removing four items from its
   default layout: the `+` attach button, the `± 0` git-diff-stats chip, the
   File explorer chip, and the Rich Input chip.

The removed items stay **re-addable** via the footer's toolbar editor (they are
only removed from the *default* layout, not from the available-items list).

## Background / current state

Both the header bar and the CLI footer are **data-driven from an enum plus a
`default_*()` list**, and both read their layout from a settings enum with two
states:

- **`Default`** → resolves the layout live from the code `default_*()` function
  on every render.
- **`Custom { left, right }`** → uses a frozen, persisted list.

The user's footer currently matches the code defaults exactly
(`Fork · Compact · Continue · LGTM · + · ± 0 · /remote-control · File explorer ·
Rich Input`; `VoiceInput` is in the default list but filtered out as unsupported
on this build), which means the footer setting is in the `Default` state.
Likewise the header is in the `Default` state. **Because both are `Default`,
editing the code `default_*()` lists changes the live UI immediately after
`make update` — no settings migration or manual "reset to default" is required.**

### Key existing pieces reused

- **File tree / Project Explorer view** — already exists. Both the old footer
  "File explorer" chip (`toggle_file_tree` → `toggle_left_panel_file_tree`) and
  the workspace `WorkspaceAction::ToggleProjectExplorer` open the *same*
  left-panel Project Explorer view. **No new view is created.**
- **`WorkspaceAction::ToggleProjectExplorer`** (`app/src/workspace/action.rs:678`),
  handler at `app/src/workspace/view.rs:25407`. It is guarded by
  `CodeSettings.show_project_explorer`, and computes
  `is_showing = left_panel_view.active_view() == ToolPanelView::ProjectExplorer`.
- **`LeftPanelView::is_file_tree_active()`**
  (`app/src/workspace/view/left_panel.rs:592`) — true when the active tool view
  is `ProjectExplorer`.
- **`render_tab_bar_icon_button(...)`** (`view.rs:21466`) — shared header
  icon-button builder (active/hover styles, tooltip, keybinding, click →
  dispatch action). Used by every header button.
- **`render_header_toolbar_button`** (`view.rs:20563`) — per-item `match` that
  early-returns `None` when `!item.is_available(ctx)` and otherwise dispatches to
  the item's render function.
- **`TOGGLE_PROJECT_EXPLORER_BINDING_NAME = "workspace:toggle_project_explorer"`**
  (`view.rs:625`) — keybinding-name constant for tooltip + `SavePosition` id.
- **`Icon::Folder`** (`crates/warp_core/src/ui/icons.rs:36`,
  `bundled/svg/folder.svg`) — the chosen glyph for the new button.

## Part A — Header: new File Explorer toggle button

Add File Explorer as a first-class, configurable `HeaderToolbarItemKind`
variant (same system as Tabs/Tools/Agent buttons), not a hardcoded fixed button.
This gives it the shared styling, active-state highlight, right-click context
menu, and toolbar-editor configurability for free. It is placed in the defaults
but stays **removable** via the header toolbar editor (unlike `TabsPanel`, which
is force-pinned — we deliberately do NOT pin File Explorer).

### A1. `app/src/workspace/header_toolbar_item.rs`

- Add `FileExplorer` to the `HeaderToolbarItemKind` enum.
- `display_label()`: `Self::FileExplorer => "File Explorer"`.
- `icon()`: `Self::FileExplorer => Icon::Folder`.
- `is_supported()`: `Self::FileExplorer => true`. Match `ToolsPanel`, which
  already hosts Project Explorer unconditionally — the old footer File Explorer
  was likewise ungated and worked in this build. (Do NOT gate on
  `cfg!(feature = "local_fs")`: if the stable build compiles without that
  feature the button would silently never appear.) Real availability is handled
  by `is_available` below.
- `is_available()`: add arm
  `Self::FileExplorer => *CodeSettings::as_ref(app).show_project_explorer`
  (so the button hides when Project Explorer is disabled — consistent with the
  toggle handler being a no-op in that case). Requires importing `CodeSettings`.
- `is_panel()`: include `FileExplorer` (it opens the left panel):
  `matches!(self, Self::TabsPanel | Self::ToolsPanel | Self::CodeReview | Self::FileExplorer)`.
- `default_left()`: insert at index 1 →
  `vec![Self::TabsPanel, Self::FileExplorer, Self::ToolsPanel, Self::AgentManagement]`.
- `all_items()`: add `Self::FileExplorer` (this makes it appear in the header
  toolbar editor's available list automatically, since the editor iterates
  `all_items()` + `display_label()` + `icon()`).

### A2. `app/src/workspace/view.rs`

- Add `render_file_explorer_button(&self, appearance, ctx)`, modeled on
  `render_tools_panel_button` (`view.rs:19902`):
  - icon: `Icon::Folder`
  - mouse state: `&self.mouse_states.file_explorer_icon` (new field, see A3)
  - action: `WorkspaceAction::ToggleProjectExplorer`
  - tooltip: `"File explorer"`
  - keybinding display: `keybinding_name_to_display_string(TOGGLE_PROJECT_EXPLORER_BINDING_NAME, ctx)`
  - `is_active`:
    `self.active_tab_pane_group().as_ref(ctx).left_panel_open
     && self.left_panel_view.as_ref(ctx).is_file_tree_active()`
  - wrapped in `Align` → `Container` → `SavePosition` with id
    `TOGGLE_PROJECT_EXPLORER_BINDING_NAME`, same as `render_tools_panel_button`.
- Add the match arm in `render_header_toolbar_button` (`view.rs:20573`):
  ```rust
  HeaderToolbarItemKind::FileExplorer => {
      if self.left_panel_views.is_empty() {
          return None;
      }
      self.render_file_explorer_button(appearance, ctx)
  }
  ```
  (mirrors the `ToolsPanel` empty-panel guard).

### A3. `app/src/workspace/util.rs`

- Add `pub(super) file_explorer_icon: MouseStateHandle,` to the
  `#[derive(Default)] WorkspaceMouseStates` struct. No construction site to
  touch — the derive initializes the new handle. (Per WARP.md, the handle is
  created once at construction and referenced during render; do NOT inline
  `MouseStateHandle::default()` in the render path.)

## Part B — CLI-agent footer: trim the default layout

### B1. `app/src/ai/blocklist/agent_view/agent_input_footer/toolbar_item.rs`

In `cli_default_left()` (`toolbar_item.rs:297`), remove four entries:

- `Self::FileAttach` (the `+` attach button)
- `Self::ContextChip(ContextChipKind::GitDiffStats)` (the `± 0` chip)
- `Self::FileExplorer`
- the `if FeatureFlag::CLIAgentRichInput.is_enabled() { items.push(Self::RichInput); }` block

Resulting `cli_default_left()`:

```rust
pub fn cli_default_left() -> Vec<Self> {
    let mut items = vec![
        Self::ForkSession,
        Self::Compact,
        Self::ContinuePrompt,
        Self::LooksGoodPrompt,
        Self::VoiceInput,
    ];
    if FeatureFlag::CreatingSharedSessions.is_enabled()
        && FeatureFlag::HOARemoteControl.is_enabled()
    {
        items.push(Self::ShareSession);
    }
    items
}
```

`VoiceInput` stays (harmless; already filtered out as unsupported on this build).
`ShareSession` stays (this is the `/remote-control` chip the user keeps).

**Do NOT modify `all_available_for_cli_input()`** — the four removed items remain
in the footer toolbar editor's available list so the user can drag them back
(honoring the "defaults-only, re-addable" decision).

### Scope note

Only `cli_default_left()` changes. The Agent-Mode (non-CLI) footer defaults
(`default_left()` / `default_right()`) are left untouched — the user only sees
the CLI footer.

## No dead code

- The footer's `file_explorer_button`, `rich_input_button`, `file_button` (`+`),
  and the `GitDiffStats` chip renderer all remain reachable: via the re-add path
  (`all_available_for_cli_input()`) and, for `FileAttach`/`GitDiffStats`, via the
  separate Agent-Mode footer defaults. Nothing is orphaned.
- Part A is purely additive.
- Minor incidental win: with `GitDiffStats` out of the default footer, its
  `git diff --shortstat` polling no longer runs for the default layout (the chip
  only computes when rendered).

## Testing

- **`app/src/ai/blocklist/agent_view/agent_input_footer/toolbar_item_tests.rs`**
  - Update/extend assertions on `cli_default_left()`: it no longer contains
    `FileAttach`, `ContextChip(GitDiffStats)`, `FileExplorer`, or `RichInput`;
    it still contains `ForkSession`, `Compact`, `ContinuePrompt`,
    `LooksGoodPrompt`.
  - Assert those four ARE still present in `all_available_for_cli_input()`.
- **Header** (`app/src/workspace/header_toolbar_item.rs` inline `#[cfg(test)]`
  module, or `view_tests.rs`)
  - `default_left()` == `[TabsPanel, FileExplorer, ToolsPanel, AgentManagement]`
    (File Explorer at index 1, right after the collapse button).
  - `all_items()` contains `FileExplorer`.
- **Manual smoke** (`make update`): header shows a folder button next to the
  collapse button that toggles the file tree and highlights when the tree is
  open; the CLI footer no longer shows `+`, `± 0`, File explorer, or Rich Input,
  while Fork/Compact/Continue/LGTM/`/remote-control` remain. Confirm the four
  removed footer items can still be dragged back via the footer toolbar editor.

## Exhaustive-match callouts (per CLAUDE.md)

Adding the `HeaderToolbarItemKind::FileExplorer` variant will surface compile
errors at every non-wildcard `match` on the enum. Known sites to handle:
`display_label`, `icon`, `is_supported`, `is_available`, `is_panel`,
`default_left`, `all_items` (all in `header_toolbar_item.rs`), and
`render_header_toolbar_button` (`view.rs`). The compiler enumerates any others.

## Out of scope

- Renaming/relocating the equivalent icons in the non-CLI Agent-Mode footer.
- Any change to the file-tree view itself.
- Removing the footer render code for the four items (kept for re-add).
