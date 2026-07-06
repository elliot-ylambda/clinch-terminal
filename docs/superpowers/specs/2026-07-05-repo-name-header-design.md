# Repo name header above the vertical tabs — design

**Date:** 2026-07-05
**Status:** Approved design
**Feature flag:** none (small, always-on UI addition)

## Summary

Show a subtle label at the **top of the left vertical tabs panel**, above the
search/control bar, displaying the **active tab's repository name**. This lets
you identify a window's project at a glance when flipping between windows.

The repo identity reuses the exact plumbing the shipped window-header color
feature already uses (`Workspace::active_header_project_dir`), so the name and
the per-project header tint always agree.

## Goals

- A repo name is visible at the top of the vertical tabs panel with **zero
  configuration**.
- It follows the **active tab** (matches the existing color tint, which also
  follows the active tab), updating on tab switch and `cd`.
- Subtle styling that reads as a panel header, not a prominent bar.

## Non-goals

- No git branch, ahead/behind, or dirty state — just the repo/folder name.
  (Branch is an easy future add; a `render_git_branch_text` helper already
  exists in `vertical_tabs.rs` if wanted later.)
- No click behavior — it is a passive label (no reveal-in-Finder, no copy).
- No always-visible variant in the top strip. The label lives in the panel, so
  it is hidden when the panel is collapsed. (Accepted tradeoff; the top strip
  already carries the per-project color tint for window identification.)
- No feature flag. Consistent with how the usage-in-tab-bar feature shipped.

## Decisions (locked)

| Question | Decision |
|---|---|
| Placement | First child of the vertical panel column, above `render_control_bar` |
| Which tab | **Active tab** (consistent with the header color tint) |
| Text | Repo name = basename of the git root, falling back to the cwd folder name |
| Remote / no local cwd | Header absent (renders nothing) |
| Styling | Subdued (`sub_text_color`) + small leading `Icon::Folder`, aligned to the control bar padding, single line, ellipsis-truncated |
| Rollout | Always on, no feature flag |

## Background: how things work today

- **The vertical panel** is composed in `render_vertical_tabs_panel`
  (`app/src/workspace/view/vertical_tabs.rs:1627`) as a `Flex::column`:
  1. `render_control_bar(...)` — the `[🔍 search] [⚙] [＋]` row (`:1362`), padded
     with `GROUP_HORIZONTAL_PADDING` left/right and
     `CONTROL_BAR_VERTICAL_PADDING`.
  2. `Shrinkable(scrollable_groups)` — the scrollable tab list.
  This column is exactly where a header above the tabs goes: a new first child.
- **Active tab's project dir (sync):**
  `Workspace::active_header_project_dir(&self, ctx) -> Option<PathBuf>`
  (`app/src/workspace/view.rs:20887`) resolves: active tab → its
  `canonical_session_pwd_if_local` → `DetectedRepositories::get_root_for_path`
  (git root), `unwrap_or` the cwd itself, then `to_local_path`. Returns `None`
  for remote sessions or when there is no local cwd. This is a method on
  `impl Workspace` (`view.rs:1180`); the `vertical_tabs` submodule can call it
  even though it is private, since a child module sees an ancestor's private
  items.
- **Live re-render:** the panel renders from live workspace state every frame
  (that is how the color tint "follows the active tab" with no subscription).
  A tab switch or `cd` re-renders and re-resolves the label for free — no new
  state and no new subscription.
- **Icon + text style:** `render_control_bar` builds its search icon with
  `WarpIcon::Search.to_warpui_icon(sub_text)` where
  `sub_text = theme.sub_text_color(theme.background())` and
  `WarpIcon = warp_core::ui::Icon` (aliased at `vertical_tabs.rs:19`). The icon
  enum has a `Folder` variant (`crates/warp_core/src/ui/icons.rs:36`). Reuse
  the same `sub_text` color and icon-building idiom so the header matches.

## Design

### Components

1. **Pure helper** `repo_label_for_path(path: &Path) -> Option<String>` in
   `app/src/workspace/view/vertical_tabs.rs`:
   - Returns the final path component (`path.file_name()`), lossily converted to
     a `String`.
   - Returns `None` for a path with no final component (e.g. `/`, `` , or a
     root/prefix-only path), so the caller renders nothing rather than an empty
     label.
   - Colocated unit tests (repo test convention: `#[cfg(test)]` module).

2. **Render function** `render_repo_header(workspace: &Workspace, app: &AppContext)
   -> Option<Box<dyn Element>>` in the same file:
   - `let project_dir = workspace.active_header_project_dir(app)?;`
   - `let name = repo_label_for_path(&project_dir)?;`
   - Build a `Flex::row` (cross-axis centered): leading
     `Icon::Folder.to_warpui_icon(sub_text)` sized like the search icon
     (`SEARCH_ICON_SIZE`), then a `Shrinkable` text element with `name` in
     `sub_text` color, single line, ellipsis overflow.
   - Wrap in a `Container` with the same horizontal padding as the control bar
     (`GROUP_HORIZONTAL_PADDING` left/right) and a small vertical padding so it
     reads as a header. Returns `Some(element)`.
   - Returns `None` when there is no project dir or no name → the row is simply
     not added.

3. **Wire-in** in `render_vertical_tabs_panel` (`:1647`): make the header the
   first child of `panel_content`, before `render_control_bar`:
   ```
   let mut panel_content = Flex::column()
       .with_main_axis_size(MainAxisSize::Max)
       .with_cross_axis_alignment(CrossAxisAlignment::Stretch);
   if let Some(header) = render_repo_header(workspace, app) {
       panel_content = panel_content.with_child(header);
   }
   panel_content = panel_content
       .with_child(render_control_bar(...))
       .with_child(Shrinkable::new(1., scrollable_groups).finish());
   let panel_content = panel_content.finish();
   ```
   (Builder shape adjusted to whatever `Flex::column`'s API allows; the intent
   is: conditional first child, then the existing two children unchanged.)

### Data flow

```
active tab ─(active_header_project_dir)─▶ Option<PathBuf>  (git root or cwd; None if remote)
                                              │
                                              ▼
                                   repo_label_for_path  ─▶ Option<String> (basename)
                                              │
                                              ▼
                     render_repo_header ─▶ Option<Element>  (icon + subdued text)
                                              │ Some
                                              ▼
                 first child of the vertical panel column (above control bar)
```

### Edge cases

- **Remote session / no local cwd / WASM** → `active_header_project_dir` is
  `None` → header absent.
- **Not in a git repo** → `get_root_for_path` is `None`, the helper falls back
  to the cwd, so the header shows the **folder name**. Intentional and useful.
- **Root path** (`/`) or nameless path → `repo_label_for_path` is `None` →
  header absent (no empty label).
- **Panel collapsed** → the whole panel is not rendered, so the header is gone
  too. Accepted.
- **Long repo names / narrow panel** → the text element shrinks and
  ellipsis-truncates (via `Shrinkable` + single-line overflow), so it never
  pushes the panel wider or wraps.

## Testing

**Unit (`vertical_tabs_tests.rs` or a colocated `#[cfg(test)] mod`):**
- `repo_label_for_path(Path::new("/Users/me/clinch-terminal"))` → `Some("clinch-terminal")`.
- Trailing-slash equivalence: `".../clinch-terminal/"` → `Some("clinch-terminal")`
  (Rust's `file_name()` already ignores a trailing separator).
- `repo_label_for_path(Path::new("/"))` → `None`.
- `repo_label_for_path(Path::new(""))` → `None`.

**Manual (`cargo run`):**
- Open a tab in a git repo → the repo name shows at the top of the panel and
  matches the top-strip tint.
- `cd` into a subdirectory of the same repo → name unchanged (still the repo
  root's name).
- Switch to a tab in a different repo → name and tint both update.
- Open a plain (non-repo) folder → shows the folder name.
- Collapse the vertical tabs panel → header gone; expand → back.

## Cleanup / dead code

- Reuse (do **not** duplicate) `active_header_project_dir`, the `sub_text`
  color idiom, `SEARCH_ICON_SIZE`, `GROUP_HORIZONTAL_PADDING`, and the
  `to_warpui_icon` icon builder.
- No feature flag, so no flag-cleanup debt. The feature is purely additive: one
  new pure helper and one new render function, plus a single conditional child
  in `render_vertical_tabs_panel`. No branches are obsoleted.

## Files touched (anticipated)

- `app/src/workspace/view/vertical_tabs.rs` — `repo_label_for_path` (+ tests),
  `render_repo_header`, and the conditional first child in
  `render_vertical_tabs_panel`.
