# Image Preview Pane — Design

**Date:** 2026-06-30
**Status:** Approved (design), pending implementation plan
**Branch:** `image-preview-pane`

## Summary

Add an in-app **image preview pane** to the Warp ("Clinch") client. Opening an
image file renders the actual picture in a viewer pane that docks to the right
of the terminal/editor (split pane) or as a new tab — the same mechanism the
Markdown viewer uses. It replaces today's behavior, where opening a `.svg` shows
its raw XML in the code editor and opening a raster image (`.png`, `.jpg`,
`.jpeg`, `.gif`, `.webp`) hands the file to an external/system app.

The pane is a first-class pane kind (`ImagePane` + `ImageView`) mirroring the
existing Markdown viewer ("file notebook": `FilePane` + `FileNotebookView`). It
persists across restarts, dedupes, supports split-pane and new-tab layouts, and
carries a small header toolbar.

## Goals

- Render `svg`, `png`, `jpg`, `jpeg`, `gif`, `webp` inside Warp.
- Open as a split viewer pane (default) or a new tab, like the Markdown viewer.
- Become the default action when opening an image from the file tree, AI
  document links, and notebook/markdown links.
- Provide: zoom (fit / 100% / in–out), pan when zoomed, a transparency backdrop
  toggle, live reload on file change, and a view-source toggle for SVG only.

## Non-goals (v1)

- Editing images. SVG "view source" is read-only.
- Previewing remote (non-local) files — falls back to current behavior.
- Persisting per-pane view state (zoom/pan/backdrop). Only the file path
  persists; view state resets to defaults on restore.
- Formats beyond the six listed (no PDF, TIFF, BMP, AVIF, etc.).

## Resolved decisions

These three open questions were resolved when the design was approved:

1. **Persistence scope:** persist the file path only; zoom/pan/backdrop reset to
   defaults on restart.
2. **Animated GIF/WebP:** animate by default. Animation is performance-sensitive
   per the `Image` element docs, so this is the primary risk to validate during
   implementation; fallback is first-frame-with-play-toggle if perf is poor.
3. **v1 scope:** ship the full feature (all six formats, all four controls) — no
   cuts.

## Chosen approach

**Mirror the `FilePane` / `FileNotebookView` path as a first-class `ImagePane`.**

The Markdown viewer is an almost exact template: it is a pane kind with
persistence, a file-watcher live-reload subscription, and a header with a
source-toggle segmented control. We clone that path for images and swap the body
for the existing `Image` element.

Alternatives considered and rejected:

- **Render images as a one-block notebook document (`BlockItem::Image`).**
  Reuses notebook persistence but shoehorns images into the markdown/notebook
  abstraction; zoom/pan/backdrop do not fit that view's model, and view-source
  still needs special-casing. Fights the framework.
- **Lightweight, non-persisted `ImageView`.** Saves a few files but preview panes
  would vanish on restart (inconsistent with code/markdown panes), and nearly all
  the pane-kind plumbing is still required. The persistence delta is small, so
  skipping it buys little.

## Feature gating

Add `FeatureFlag::ImagePreviewPane` (default off), wired with the
`add-feature-flag` skill. The flag gates **routing only**: when off, images
behave exactly as today. The `LeafContents::ImageViewer` persistence variant is
**not** flag-gated, so a persisted snapshot always restores cleanly regardless of
flag state. Promotion and eventual flag removal use the `promote-feature` and
`remove-feature-flag` skills.

## Components & file map

### File-open routing — `app/src/util/openable_file_type.rs`
- Add `OpenableFileType::Image` and `FileTarget::ImageViewer(EditorLayout)`.
- In `resolve_file_target` / `resolve_file_target_with_editor_choice`, route
  `is_supported_image_file(path)` to `FileTarget::ImageViewer(layout)` when the
  flag is on and the path is local. Remote paths fall through to existing
  behavior.
- `is_supported_image_file()` (already exists at `:72`) is the source of truth
  for the format set.
- Update the outdated comment at `:39` ("svg files… opened in a code editor
  pane").

### Existing image entry points (route to the new target)
- `app/src/ai/blocklist/block.rs:320` — currently returns
  `FileTarget::SystemGeneric` for images.
- `app/src/ai/ai_document_view.rs:969` — image-link handling.
- `app/src/notebooks/link.rs:370` — markdown image-link handling.

### Pane — `app/src/pane_group/pane/image_pane.rs` (new)
- `ImagePane` implementing `PaneContent`, wrapping `PaneView<ImageView>`. Mirrors
  `file_pane.rs`. Re-export from `app/src/pane_group/mod.rs`.
- `ImagePane::new(path: Option<LocalOrRemotePath>, ...)` builds the view and opens
  the path, mirroring `FilePane::new`.
- `ImagePane::snapshot` returns
  `LeafContents::ImageViewer(ImagePaneSnapshot { path })` using the view's local
  path (remote → `None`).

### View — `app/src/image_viewer/mod.rs` (new)
- `ImageView` implementing `BackingView`.
- Body renders
  `Image::new(AssetSource::LocalFile { path, content_version }.with_local_file_content_version(), CacheOption::Original)`
  with `.contain()` (in Fit mode), `.before_load(spinner)`, and
  `.on_load_failure(error_element)`. `on_load_timeout` shows a stall fallback.
- `render_header_content` returns `HeaderContent::Custom` built with
  `render_three_column_header(Empty, filename_title, controls_row)` plus standard
  pane buttons via `render_pane_header_buttons::<ImageViewerAction, ()>`.

### Workspace dispatch — `app/src/workspace/view.rs`
- Add `open_file_image(path, session, layout, ctx)` mirroring `open_file_notebook`
  (`:8256`): dedupe against open image panes, then insert by layout
  (`add_tab_from_existing_pane` for `NewTab`, `add_pane_with_direction(Right, …)`
  for `SplitPane`).
- Add the `FileTarget::ImageViewer(layout)` arm to the open-file `match` near
  `:6118` that calls `open_file_image`.

## View state & controls

`ImageView` holds:
- `zoom: ZoomMode` — `Fit` (default) | `Factor(f32)`. `Fit` uses `.contain()`.
  `Factor` sizes the image to `intrinsic_size * factor`, where `intrinsic_size`
  comes from `ImageType::image_size()` once the asset is `Loaded`; the GPU scales
  via `CacheOption::Original`. The "100%" button sets `Factor(1.0)`; zoom in/out
  step the factor.
- `pan_offset: Vector2F` — applied only when zoomed beyond fit; updated by
  click-drag mouse handling on the element and clamped to image bounds.
- `backdrop: Backdrop` — `Checkerboard` (default) | `Light` | `Dark`; rendered as
  a `Container` behind the image so transparent assets remain legible.
- `source_view_open: bool` (SVG only) — toggles between the rendered image and a
  read-only text view of the SVG bytes (sourced from `FileModel`). Hidden for
  raster formats.

Controls are wired through an `ImageViewerAction` enum (e.g. `ZoomIn`, `ZoomOut`,
`ZoomFit`, `Zoom100`, `SetBackdrop(Backdrop)`, `ToggleSource`) handled via
`TypedActionView`, following `FileNotebookAction`. The view-source control clones
the notebook's `MarkdownToggleView` segmented control.

## Data flow & live reload

1. On open, `ImageView` calls
   `FileModel::handle(ctx).update(ctx, |m, ctx| m.open(&path, /*subscribe=*/ true, ctx))`
   and stores the returned `FileId`, then `ctx.subscribe_to_model(...)`.
2. On `FileModelEvent::FileUpdated { id, content }` where `id` matches: re-run
   `with_local_file_content_version()` to attach a fresh fingerprint (this busts
   the image cache → re-decode), trigger a repaint, and — if the source view is
   open — refresh its text from `content`.
3. On `FileModelEvent::FailedToLoad`: show the error state.
4. On close/reopen: unsubscribe the previous file id.

This mirrors the notebook viewer's subscription at
`app/src/notebooks/file/mod.rs:440–491`. `with_local_file_content_version()` reads
filesystem metadata, so it is only called on open and on `FileUpdated`, never per
frame (per the API note at `asset_cache.rs:141`).

## Persistence

- `LeafContents::ImageViewer(ImagePaneSnapshot { path: Option<PathBuf> })` —
  added to the snapshot enum at `app/src/app_state.rs:136`.
- Add `ImageViewer` to the `is_persisted()` true-list at `app_state.rs:167`.
- Restore arm in `app/src/pane_group/mod.rs:1546` builds
  `ImagePane::new(path.map(LocalOrRemotePath::Local), ...)`, mirroring the
  `LocalFileNotebook` arm at `:1743`.
- Update the SQLite read/write codec where `NotebookPaneSnapshot` is serialized.
- Add the `ImageViewer` arm to the exhaustive `LeafContents` match in
  `app/src/launch_configs/launch_config.rs:149`.

## Animated GIF / WebP

Default to animated playback via `Image::enable_animation_with_start_time(now)`.
The `Image` docs flag animation as performance-sensitive; if profiling shows
regressions, fall back to `first_frame_preview()` with an explicit play toggle.
This is the primary implementation risk to validate.

## Error handling & edge cases

- Decode failure → `on_load_failure` element ("Couldn't render this image").
- Load stall → `on_load_timeout` fallback.
- Deleted/missing file mid-session → `FileModelEvent::FailedToLoad` error state.
- Remote / non-local sources → not supported in v1; fall through to existing
  behavior (no `ImageViewer` routing).
- SVG text rendering → call `prewarm_svg_font_db()` so SVG `<text>` renders with
  fallback fonts.
- Very large images / SVGs → rely on the asset cache's existing eviction; the
  load-timeout fallback covers pathological cases.

## Testing

- **Unit (`app/src/util/openable_file_type_tests.rs`):** image extensions resolve
  to `FileTarget::ImageViewer` when the flag is on and to current behavior when
  off; non-image and remote paths are unaffected.
- **Unit:** `ImagePaneSnapshot` path round-trips through serialize/deserialize;
  restore rebuilds an `ImagePane` with the same path.
- **Integration (`crates/integration`, via the `warp-integration-test` skill):**
  (a) opening an image from the file tree shows an image pane split to the right;
  (b) the SVG view-source toggle flips to XML and back; (c) editing the file on
  disk live-updates the rendered pane.
- Decode correctness is already covered by `crates/warpui_core/src/image_cache_tests.rs`.

## Dead-code & cleanup

- After the flag is stable, the image→`SystemGeneric` special-casing in
  `app/src/ai/blocklist/block.rs:320` is dead and is removed (tracked with
  `remove-feature-flag`).
- The outdated comment at `openable_file_type.rs:39` is corrected as part of this
  work.
- `FeatureFlag::ImagePreviewPane` is removed after promotion to Stable via the
  `remove-feature-flag` skill.

## Affected files (summary)

New:
- `app/src/pane_group/pane/image_pane.rs`
- `app/src/image_viewer/mod.rs`

Modified:
- `app/src/util/openable_file_type.rs` (routing + comment)
- `app/src/workspace/view.rs` (`open_file_image` + dispatch arm)
- `app/src/pane_group/mod.rs` (re-export + restore arm)
- `app/src/app_state.rs` (`LeafContents::ImageViewer` + `is_persisted`)
- `app/src/launch_configs/launch_config.rs` (exhaustive match)
- `app/src/ai/blocklist/block.rs`, `app/src/ai/ai_document_view.rs`,
  `app/src/notebooks/link.rs` (entry points)
- `crates/warp_features/src/lib.rs` + `app/src/features.rs` (feature flag)
- SQLite pane-snapshot codec (same module as `NotebookPaneSnapshot`)
- Tests as listed above.
