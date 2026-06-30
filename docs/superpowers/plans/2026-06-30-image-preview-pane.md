# Image Preview Pane Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an in-app image preview pane that renders `svg/png/jpg/jpeg/gif/webp` as a split viewer pane (like the Markdown viewer), with zoom, pan, a transparency backdrop, live reload, and an SVG view-source toggle.

**Architecture:** Clone the existing Markdown "file notebook" pane path. A new `ImageView` (implementing `BackingView`) renders the existing `Image` element from an `AssetSource::LocalFile`; a new `ImagePane` (implementing `PaneContent`) wraps it; a new `LeafContents::ImageViewer` persists the open path. File opening is re-routed so image files dispatch to the new pane, gated behind a feature flag.

**Tech Stack:** Rust; Warp's bespoke `warpui` element/view framework; `warpui_core` `Image`/`AssetSource`/`image_cache`; `warp_files` `FileModel` watcher; SQLite app-state persistence.

**Reference spec:** `docs/superpowers/specs/2026-06-30-image-preview-pane-design.md`

## Global Constraints

- All filesystem-touching code is gated behind `#[cfg(feature = "local_fs")]`, mirroring `FileNotebookView`.
- `FeatureFlag::ImagePreviewPane` gates **routing only**. The `LeafContents::ImageViewer` persistence variant is always handled (never flag-gated), so a saved snapshot restores cleanly regardless of flag state.
- v1 supports **local files only**. Remote paths fall through to existing behavior (no `ImageViewer` routing).
- Supported formats are exactly those in `is_supported_image_file()` (`app/src/util/openable_file_type.rs:72`): `jpg jpeg png gif webp svg`. Do not add formats.
- Mirror existing patterns: `app/src/pane_group/pane/file_pane.rs` (pane), `app/src/notebooks/file/mod.rs` (view), `NotebookPaneSnapshot::LocalFileNotebook` (persistence).
- **Enum-variant workflow:** after adding a variant to `FeatureFlag`, `OpenableFileType`, `FileTarget`, or `LeafContents`, run `cargo check -p app` and add an arm to **every** match the compiler flags. Known sites are listed in each task.
- Feature-flag query API: `FeatureFlag::ImagePreviewPane.is_enabled()` (no args). In tests: `let _g = FeatureFlag::ImagePreviewPane.override_enabled(true);`.
- Commit after each task. Build check: `cargo check -p app` (the client crate is `app`).

---

### Task 1: Add the `ImagePreviewPane` feature flag

**Files:**
- Modify: `crates/warp_features/src/lib.rs` (enum `FeatureFlag`, near `KittyImages` ~line 211)
- Modify: `app/src/features.rs` (registration list, near `FeatureFlag::KittyImages` ~line 121)
- Modify: `app/Cargo.toml` (add the `image_preview_pane` cargo feature)

**Interfaces:**
- Produces: `FeatureFlag::ImagePreviewPane`, queried via `.is_enabled()` / `.override_enabled(bool)`.

> Use the `add-feature-flag` skill if available — it knows the full wiring. The explicit edits below are the same result.

- [ ] **Step 1: Add the enum variant**

In `crates/warp_features/src/lib.rs`, after the `KittyImages` variant:

```rust
    /// Enables the in-app image preview pane (svg/png/jpg/gif/webp open as a
    /// rendered viewer pane instead of XML-source / external app).
    ImagePreviewPane,
```

- [ ] **Step 2: Register the flag**

In `app/src/features.rs`, after the `FeatureFlag::KittyImages` registration line:

```rust
        #[cfg(feature = "image_preview_pane")]
        FeatureFlag::ImagePreviewPane,
```

- [ ] **Step 3: Add the cargo feature**

In `app/Cargo.toml`, in the `[features]` table, add (alphabetical/near other flag features):

```toml
image_preview_pane = []
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p warp_features && cargo check -p app --features image_preview_pane`
Expected: builds with no errors. Confirm no `non-exhaustive` errors (the `FeatureFlag` enum is matched in `warp_features`; add arms if the compiler requests).

- [ ] **Step 5: Commit**

```bash
git add crates/warp_features/src/lib.rs app/src/features.rs app/Cargo.toml
git commit -m "feat(image-pane): add ImagePreviewPane feature flag"
```

---

### Task 2: Scaffold the `ImageView` (renders the image, no controls yet)

**Files:**
- Create: `app/src/image_viewer/mod.rs`
- Modify: `app/src/lib.rs` (or the parent `mod` file that declares top-level modules — add `pub mod image_viewer;`)

**Interfaces:**
- Produces:
  - `pub struct ImageView` implementing `warpui::View` + `crate::pane_group::pane::BackingView`.
  - `ImageView::new(ctx: &mut ViewContext<Self>) -> Self`
  - `ImageView::open_local(&mut self, path: impl Into<PathBuf>, session: Option<Arc<Session>>, ctx: &mut ViewContext<Self>)`
  - `ImageView::local_path(&self) -> Option<PathBuf>`
  - `ImageView::pane_configuration(&self) -> ModelHandle<PaneConfiguration>`
  - `ImageView::set_focus_handle(&mut self, handle: PaneFocusHandle, ctx)` / `focus(&mut self, ctx)`
  - `pub enum ImageViewEvent { TitleUpdated, FileLoaded, Pane(PaneEvent) }`
  - `pub enum ImageViewAction { Focus, Close, ToggleMaximized, ContextMenu(ContextMenuAction) }` (extended in later tasks)

This task produces a **compilable** module. Behavioral validation happens in Task 4's integration test (rendering is not unit-tested in this repo; there is no view-level unit harness — confirm with `grep -rn "FileNotebookView::new" app/src --include=*_tests.rs`, which returns nothing).

- [ ] **Step 1: Create the module skeleton**

Create `app/src/image_viewer/mod.rs`. Mirror `FileNotebookView` for the framework boilerplate (`app/src/notebooks/file/mod.rs:73-300` for fields/`new`, `:405-498` for `open_local`, `app/src/pane_group/pane/mod.rs:981-1075` for the `BackingView` trait surface). The image-specific body is below.

State fields:

```rust
use std::path::PathBuf;
use std::sync::Arc;

use warpui::{AppContext, ModelHandle, View, ViewContext};
use warpui_core::assets::asset_cache::AssetSource;
use warpui_core::elements::gui::image::{CacheOption, Image};
use warpui_core::elements::{Container, Element, Empty};

use crate::pane_group::pane::view::{self, PaneFocusHandle};
use crate::pane_group::{PaneConfiguration, PaneEvent};
use crate::terminal::model::session::Session;

/// How the image is sized within the pane. `Fit` contains the image; `Factor`
/// scales the intrinsic size. "100%" is `Factor(1.0)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ZoomMode {
    Fit,
    Factor(f32),
}

/// Background drawn behind the image so transparent assets stay legible.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Backdrop {
    Checkerboard,
    Light,
    Dark,
}

pub struct ImageView {
    /// Absolute local path of the open image, once resolved.
    path: Option<PathBuf>,
    /// `AssetSource::LocalFile { .. }` with a content fingerprint, recomputed on
    /// open and on file-change so an edited file is re-decoded.
    source: Option<AssetSource>,
    zoom: ZoomMode,
    backdrop: Backdrop,
    /// Pan offset applied only when zoomed past fit. Set in Task 7.
    pan_offset: pathfinder_geometry::vector::Vector2F,
    /// SVG-only: whether the raw XML source is shown instead of the render. Task 8.
    source_view_open: bool,
    /// Latest file bytes (for SVG source view / decode). Populated in Task 8.
    file_bytes: Option<Vec<u8>>,
    #[cfg(feature = "local_fs")]
    file_id: Option<warp_files::FileId>,
    pane_configuration: ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,
}
```

`new` mirrors `FileNotebookView::new` (`:238`) but without the rich-text editor/links — create only `pane_configuration = ctx.add_model(|_| PaneConfiguration::new(""))`, set defaults (`zoom: ZoomMode::Fit`, `backdrop: Backdrop::Checkerboard`, `source_view_open: false`, everything else `None`/zero).

- [ ] **Step 2: Implement `open_local`**

Mirror `FileNotebookView::open_local` (`:405`) for title/state, but store the asset source. Defer the `FileModel` subscription to Task 8; for now just set path + source + title:

```rust
pub fn open_local(
    &mut self,
    path: impl Into<PathBuf>,
    _session: Option<Arc<Session>>,
    ctx: &mut ViewContext<Self>,
) {
    let local_path: PathBuf = path.into();
    self.pane_configuration.update(ctx, |cfg, ctx| {
        cfg.set_title(
            local_path.file_name().map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| local_path.display().to_string()),
            ctx,
        );
    });
    self.source = Some(
        AssetSource::LocalFile {
            path: local_path.to_string_lossy().into_owned(),
            content_version: None,
        }
        .with_local_file_content_version(),
    );
    self.path = Some(local_path);
    ctx.notify();
}

pub fn local_path(&self) -> Option<PathBuf> {
    self.path.clone()
}

pub fn is_svg(&self) -> bool {
    self.path.as_ref()
        .and_then(|p| p.extension())
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("svg"))
        .unwrap_or(false)
}

pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
    self.pane_configuration.clone()
}
```

- [ ] **Step 3: Implement `View::render` (body = backdrop + image)**

The render returns the image element on the chosen backdrop. Use `CacheOption::Original` (GPU scales for zoom). Fit mode uses `.contain()`. Provide before-load and failure fallbacks.

```rust
fn render(&mut self, ctx: &mut ViewContext<Self>) -> Box<dyn Element> {
    let backdrop_fill = self.backdrop_fill(ctx); // helper: maps Backdrop -> Fill (Task 5 fills in Checkerboard)
    let mut container = Container::new(self.render_image(ctx)).with_fill(backdrop_fill);
    container.finish()
}
```

```rust
fn render_image(&self, _ctx: &AppContext) -> Box<dyn Element> {
    let Some(source) = self.source.clone() else {
        return Empty::new().finish();
    };
    let mut image = Image::new(source, CacheOption::Original).contain();
    image = image
        .before_load(/* small spinner or Empty */ Empty::new().finish())
        .on_load_failure(/* "Couldn't render this image" text element */ Empty::new().finish());
    image.finish()
}
```

For now `backdrop_fill` may return a solid neutral fill (e.g. theme background); Task 5 implements the checkerboard and the toggle. Use `Appearance::as_ref(ctx)` for theme colors as `FileNotebookView` does.

- [ ] **Step 4: Implement the `BackingView` trait**

Implement the trait from `app/src/pane_group/pane/mod.rs:981`. Most methods are mechanical (`close`, `focus_contents`, `set_focus_handle` — copy from `FileNotebookView`'s impl, search `impl BackingView for FileNotebookView`). Associated types:

```rust
impl BackingView for ImageView {
    type PaneHeaderOverflowMenuAction = ();
    type CustomAction = ();
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(&mut self, _a: &(), _ctx: &mut ViewContext<Self>) {}
    fn close(&mut self, ctx: &mut ViewContext<Self>) { ctx.emit(ImageViewEvent::Pane(PaneEvent::Close)); }
    fn focus_contents(&mut self, _ctx: &mut ViewContext<Self>) {}
    fn set_focus_handle(&mut self, handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(handle);
    }

    fn render_header_content(&self, ctx: &view::HeaderRenderContext<'_>, app: &AppContext)
        -> view::HeaderContent
    {
        // Task 2: standard header with just the title. Tasks 5/6/7 add the controls row.
        let title = self.pane_configuration.as_ref(app).title().to_owned();
        view::HeaderContent::Standard(view::StandardHeader {
            title,
            title_secondary: None,
            title_style: None,
            title_clip_config: warpui::text_layout::ClipConfig::start(),
            title_max_width: None,
            left_of_title: None,
            right_of_title: None,
            // ...replicate every remaining field from FileNotebookView:1250-1258
        })
    }
}
```

> Copy the exact `StandardHeader { .. }` field list from `app/src/notebooks/file/mod.rs:1250-1258` (it does not use `..Default::default()`); replicate every field.

- [ ] **Step 5: Declare the module and compile**

Add `pub mod image_viewer;` to the top-level module list (find it: `grep -n "pub mod notebooks;" app/src/lib.rs`).

Run: `cargo check -p app --features "local_fs image_preview_pane"`
Expected: compiles. Resolve any missing imports against the real `warpui`/`warpui_core` paths (mirror the `use` blocks in `notebooks/file/mod.rs` and `pane_group/pane/file_pane.rs`).

- [ ] **Step 6: Commit**

```bash
git add app/src/image_viewer/mod.rs app/src/lib.rs
git commit -m "feat(image-pane): scaffold ImageView rendering the image on a backdrop"
```

---

### Task 3: Add `ImagePane` and persistence wiring

**Files:**
- Create: `app/src/pane_group/pane/image_pane.rs`
- Modify: `app/src/pane_group/pane/mod.rs` (declare `mod image_pane;`) and `app/src/pane_group/mod.rs` (re-export + restore arm ~`:1546`)
- Modify: `app/src/app_state.rs` (`LeafContents` enum `:136`, `is_persisted` `:167`, new `ImagePaneSnapshot`)
- Modify: `app/src/launch_configs/launch_config.rs` (match `:149`)
- Modify: `app/src/tab_configs/session_config.rs` (match `:281`)
- Modify: `app/src/persistence/sqlite.rs` (pane-kind const `:1125`, serialize `:1204-1227`, deserialize `:2197`)
- Test: `app/src/app_state_tests.rs` (snapshot round-trip)

**Interfaces:**
- Consumes: `ImageView` (Task 2).
- Produces:
  - `pub struct ImagePane` implementing `PaneContent`; `ImagePane::new(path: Option<LocalOrRemotePath>, session: Option<Arc<Session>>, ctx) -> Self` (mirror `FilePane::new`, minus `code_source`).
  - `LeafContents::ImageViewer(ImagePaneSnapshot)`, where `pub struct ImagePaneSnapshot { pub path: Option<PathBuf> }`.

- [ ] **Step 1: Add the snapshot type and enum variant**

In `app/src/app_state.rs`, add near `NotebookPaneSnapshot` (`:220`):

```rust
/// Snapshot of an image preview pane. Only the local path is persisted;
/// zoom/pan/backdrop reset to defaults on restore.
#[derive(Clone, Debug, PartialEq)]
pub struct ImagePaneSnapshot {
    /// `None` if the pane held an unreadable/remote image.
    pub path: Option<PathBuf>,
}
```

Add the variant to `LeafContents` (`:136`), after `Notebook`:

```rust
    ImageViewer(ImagePaneSnapshot),
```

Add it to the `is_persisted` true-list (`:176-187`), in the `| LeafContents::Notebook(_)` group:

```rust
            | LeafContents::ImageViewer(_)
```

- [ ] **Step 2: Run the compiler to enumerate every match site**

Run: `cargo check -p app --features "local_fs image_preview_pane"`
Expected: `non-exhaustive patterns: LeafContents::ImageViewer(_) not covered` at each site below. Add an arm at each:

- `app/src/launch_configs/launch_config.rs:149` — group with `LeafContents::Notebook(_)` (returns the same "not a terminal template" branch).
- `app/src/tab_configs/session_config.rs:281` — map like `Notebook` (check the existing `Notebook` arm's `TabConfigPaneType`; use the same non-terminal mapping).
- `app/src/persistence/sqlite.rs:1125` (kind): add `LeafContents::ImageViewer(_) => IMAGE_VIEWER_PANE_KIND,` and define `const IMAGE_VIEWER_PANE_KIND: &str = "image_viewer";` next to the other `*_PANE_KIND` consts.
- `app/src/persistence/sqlite.rs:1204` (serialize): mirror the `Notebook → LocalFileNotebook` arm (`:1213`) — serialize `snapshot.path` as a string.
- `app/src/persistence/sqlite.rs:2197` (deserialize): mirror the path→snapshot reconstruction, producing `LeafContents::ImageViewer(ImagePaneSnapshot { path })`.

> The exact serialize/deserialize columns: copy the `NotebookPaneSnapshot::LocalFileNotebook` path handling verbatim (`sqlite.rs:1213` and `:2197`), swapping the constructed type.

- [ ] **Step 3: Create `ImagePane`**

Create `app/src/pane_group/pane/image_pane.rs` by cloning `file_pane.rs` (the whole file is the template). Changes: `FileNotebookView`→`ImageView`, `FileNotebookEvent`→`ImageViewEvent`, drop `code_source`/`subscribe_to_link_model`/`links`, and `snapshot` returns the image variant:

```rust
fn snapshot(&self, app: &AppContext) -> LeafContents {
    let path = self.image_view(app).as_ref(app).local_path();
    LeafContents::ImageViewer(ImagePaneSnapshot { path })
}
```

The `attach` subscription only needs the `ImageViewEvent` arms that exist (`TitleUpdated`→`PaneTitleUpdated`, `FileLoaded`→`AppStateChanged`, `Pane(e)`→`handle_pane_event`). Declare `mod image_pane;` in `pane/mod.rs` and `pub use pane::image_pane::ImagePane;` in `pane_group/mod.rs` (next to the `NotebookPane` re-export ~`:187`).

- [ ] **Step 4: Add the restore arm**

In `app/src/pane_group/mod.rs`, in `restore_pane_leaf` near the `Notebook` arm (`:1737`/`:1743`):

```rust
            LeafContents::ImageViewer(snapshot) => Box::new(ImagePane::new(
                snapshot.path.clone().map(LocalOrRemotePath::Local),
                None,
                ctx,
            )),
```

- [ ] **Step 5: Write the snapshot round-trip test**

In `app/src/app_state_tests.rs`, add:

```rust
#[test]
fn image_viewer_snapshot_is_persisted() {
    let snap = LeafContents::ImageViewer(ImagePaneSnapshot {
        path: Some(std::path::PathBuf::from("/tmp/diagram.svg")),
    });
    assert!(snap.is_persisted());
}
```

Run: `cargo test -p app --features "local_fs image_preview_pane" image_viewer_snapshot_is_persisted`
Expected: PASS.

- [ ] **Step 6: Full compile + commit**

Run: `cargo check -p app --features "local_fs image_preview_pane"` → no errors.

```bash
git add app/src/pane_group/pane/image_pane.rs app/src/pane_group/pane/mod.rs \
        app/src/pane_group/mod.rs app/src/app_state.rs app/src/app_state_tests.rs \
        app/src/launch_configs/launch_config.rs app/src/tab_configs/session_config.rs \
        app/src/persistence/sqlite.rs
git commit -m "feat(image-pane): add ImagePane and persist the open image path"
```

---

### Task 4: Route image files to the new pane + first integration test

**Files:**
- Modify: `app/src/util/openable_file_type.rs` (`OpenableFileType` `:34`, `FileTarget` `:45`, `resolve_file_target_with_editor_choice` `:196`, comment `:39`)
- Modify: `app/src/workspace/view.rs` (add `open_file_image`, dispatch arm near `:6118`)
- Modify: image entry points: `app/src/ai/blocklist/block.rs:320`, `app/src/ai/ai_document_view.rs:969`, `app/src/notebooks/link.rs:370`
- Test: `app/src/util/openable_file_type_tests.rs`; integration test under `crates/integration`

**Interfaces:**
- Consumes: `FeatureFlag::ImagePreviewPane`, `ImagePane` (Task 3), `is_supported_image_file` (`:72`).
- Produces: `FileTarget::ImageViewer(EditorLayout)`; `Workspace::open_file_image(path, session, layout, ctx)`.

- [ ] **Step 1: Write the failing routing unit tests**

In `app/src/util/openable_file_type_tests.rs`:

```rust
#[test]
fn svg_routes_to_image_viewer_when_flag_on() {
    let _g = warp_features::FeatureFlag::ImagePreviewPane.override_enabled(true);
    let target = resolve_file_target_with_editor_choice(
        std::path::Path::new("/tmp/logo.svg"),
        EditorChoice::Warp, false, EditorLayout::SplitPane, None,
    );
    assert_eq!(target, FileTarget::ImageViewer(EditorLayout::SplitPane));
}

#[test]
fn png_routes_to_image_viewer_when_flag_on() {
    let _g = warp_features::FeatureFlag::ImagePreviewPane.override_enabled(true);
    let target = resolve_file_target_with_editor_choice(
        std::path::Path::new("/tmp/pic.png"),
        EditorChoice::Warp, false, EditorLayout::SplitPane, None,
    );
    assert_eq!(target, FileTarget::ImageViewer(EditorLayout::SplitPane));
}

#[test]
fn svg_keeps_code_editor_when_flag_off() {
    let _g = warp_features::FeatureFlag::ImagePreviewPane.override_enabled(false);
    let target = resolve_file_target_with_editor_choice(
        std::path::Path::new("/tmp/logo.svg"),
        EditorChoice::Warp, false, EditorLayout::SplitPane, None,
    );
    assert_eq!(target, FileTarget::CodeEditor(EditorLayout::SplitPane));
}
```

- [ ] **Step 2: Run them to confirm they fail**

Run: `cargo test -p app --features "local_fs image_preview_pane" -- openable_file_type_tests`
Expected: FAIL — `FileTarget::ImageViewer` does not exist.

- [ ] **Step 3: Add the enum variants and routing**

In `openable_file_type.rs`: add `Image` to `OpenableFileType` (`:34`) and `ImageViewer(EditorLayout)` to `FileTarget` (`:45`). Fix the comment at `:39` (drop "svg files… code editor pane"; note svg now renders in the image viewer when the flag is on).

In `resolve_file_target_with_editor_choice` (`:196`), add the image check **before** the markdown/code checks, gated by the flag and local-only:

```rust
    // 0. Image preview pane (feature-flagged; local files only).
    if FeatureFlag::ImagePreviewPane.is_enabled() && is_supported_image_file(path) {
        return FileTarget::ImageViewer(layout);
    }
```

(`layout` is already computed at `:205`.) Add `use warp_features::FeatureFlag;` if not present.

- [ ] **Step 4: Run the compiler to enumerate `FileTarget` match sites**

Run: `cargo check -p app --features "local_fs image_preview_pane"`
Expected: `non-exhaustive` at the dispatch in `app/src/workspace/view.rs:6118`. (Producers like `block.rs:320` are not matches and won't error.) Handle in Step 5.

- [ ] **Step 5: Add `open_file_image` and the dispatch arm**

In `app/src/workspace/view.rs`, add `open_file_image` mirroring `open_file_notebook` (`:8256`): dedupe against open image panes, then insert by layout:

```rust
fn open_file_image(
    &mut self,
    path: LocalOrRemotePath,
    session: Option<Arc<Session>>,
    layout: EditorLayout,
    ctx: &mut ViewContext<Self>,
) {
    // (Optional dedupe: skip if an image pane for this path is already open —
    //  mirror file_notebook_panes() dedupe at view.rs:8267. Omit for v1 if no
    //  image_panes() helper exists yet.)
    let pane = ImagePane::new(Some(path), session, ctx);
    match layout {
        EditorLayout::NewTab => {
            let (new_idx, group_id) = self.next_tab_index_and_group(ctx); // see open_file_notebook:8290
            self.add_tab_from_existing_pane(Box::new(pane), new_idx, group_id, ctx);
        }
        EditorLayout::SplitPane => {
            self.active_tab_pane_group().update(ctx, |pg, ctx| {
                pg.add_pane_with_direction(Direction::Right, Box::new(pane), true, ctx);
            });
        }
    }
}
```

> Copy the exact `NewTab` index/group plumbing from `open_file_notebook` (`view.rs:8290-8305`); the snippet above is the shape, not the verbatim arg list.

Add the dispatch arm in the `match target` block (`:6118`, alongside `FileTarget::MarkdownViewer`):

```rust
            FileTarget::ImageViewer(layout) => {
                let session = self.get_active_session(ctx);
                self.open_file_image(LocalOrRemotePath::Local(path.clone()), session, layout, ctx);
            }
```

Mirror the neighboring `MarkdownViewer` arm exactly for `path`/`session` access — it wraps the local `path` in `LocalOrRemotePath::Local`. Locality is already guaranteed here: `resolve_file_target_with_editor_choice` only returns `ImageViewer` for a `&Path` (the remote-open code path never reaches this dispatch).

- [ ] **Step 6: Route the other image entry points**

- `app/src/ai/blocklist/block.rs:320` currently: `is_supported_image_file(absolute_path).then_some(FileTarget::SystemGeneric)`. Change to return `FileTarget::ImageViewer(EditorLayout::NewTab)` when `FeatureFlag::ImagePreviewPane.is_enabled()`, else keep `SystemGeneric`.
- `app/src/ai/ai_document_view.rs:969` and `app/src/notebooks/link.rs:370`: where they branch on `is_supported_image_file`, prefer `FileTarget::ImageViewer(layout)` when the flag is on; otherwise keep current behavior.

- [ ] **Step 7: Run unit tests (green)**

Run: `cargo test -p app --features "local_fs image_preview_pane" -- openable_file_type_tests`
Expected: PASS.

- [ ] **Step 8: Add the first integration test**

Use the `warp-integration-test` skill (framework in `crates/integration`). Add a test that: enables the flag, opens a fixture `*.svg` from the file tree (or via `open_file_image`), and asserts an image pane is present split to the right with a non-empty rendered body. Place a tiny fixture SVG under the integration test's assets. Mirror an existing "open file opens a pane" integration test (search `crates/integration` for `open_file_notebook` or `FilePane`).

Run the integration test per the skill's runner. Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add app/src/util/openable_file_type.rs app/src/util/openable_file_type_tests.rs \
        app/src/workspace/view.rs app/src/ai/blocklist/block.rs \
        app/src/ai/ai_document_view.rs app/src/notebooks/link.rs crates/integration
git commit -m "feat(image-pane): open image files in the image preview pane"
```

---

### Task 5: Transparency backdrop toggle (Checkerboard / Light / Dark)

**Files:** Modify `app/src/image_viewer/mod.rs`; integration test in `crates/integration`.

**Interfaces:** Consumes `Backdrop` (Task 2). Produces `ImageViewAction::SetBackdrop(Backdrop)` and a header control.

- [ ] **Step 1: Implement `backdrop_fill` and the checkerboard**

Render the checkerboard as a tiled 2-color pattern behind the image (two interleaved `Container`s or a bundled checkerboard asset via `AssetSource::Bundled`). `Light`/`Dark` are solid theme-appropriate fills. Implement `fn backdrop_fill(&self, app) -> Fill` and, for checkerboard, return a layered element from `render` instead of a flat fill.

- [ ] **Step 2: Add the header control**

Switch `render_header_content` to the **custom** header (mirror `FileNotebookView:1165-1247`): build a `right_row` `Flex::row`, add a small segmented control or icon-button cycling `Checkerboard → Light → Dark`, then `render_pane_header_buttons::<ImageViewAction, ()>(...)`, and wrap with `render_three_column_header(Empty, title, right_row, ...)`.

- [ ] **Step 3: Handle the action**

Add `ImageViewAction::SetBackdrop(Backdrop)`; in `handle_action`, set `self.backdrop` and `ctx.notify()`.

- [ ] **Step 4: Integration test + commit**

Test: toggling the control changes `backdrop` (assert via a test hook on the view, or visually assert the header control exists). Run; expected PASS.

```bash
git add app/src/image_viewer/mod.rs crates/integration
git commit -m "feat(image-pane): transparency backdrop toggle"
```

---

### Task 6: Zoom & pan

Zoom and pan share the `render_image` sizing path and the zoom handlers must reset pan, so they are one task.

**Files:** Modify `app/src/image_viewer/mod.rs`; integration test.

**Interfaces:** Consumes `ZoomMode`, `pan_offset`, `ImageType::image_size()` (`warpui_core/src/image_cache.rs:472`). Produces `ImageViewAction::{ZoomFit, Zoom100, ZoomIn, ZoomOut}` and drag handling.

- [ ] **Step 1: Read intrinsic size once loaded**

When the asset is `AssetState::Loaded`, read `ImageType::image_size()` to get intrinsic `Vector2I`. Cache it on the view (`intrinsic_size: Option<Vector2I>`), set when the load completes (Task 8's subscription, or query the asset cache in `render`).

- [ ] **Step 2: Apply zoom in `render_image`**

`ZoomMode::Fit` → `.contain()`. `ZoomMode::Factor(f)` → set an explicit size `intrinsic * f` (sizing element) and keep `CacheOption::Original`. If intrinsic size is not yet known, fall back to `Fit`. Wrap in a clip container so an over-sized image clips rather than overflows.

- [ ] **Step 3: Zoom actions + header buttons**

Add `ZoomFit`/`Zoom100`/`ZoomIn`/`ZoomOut` to `ImageViewAction`; `ZoomIn`/`ZoomOut` step the factor (×1.25, clamped to [0.1, 8.0]); `Zoom100` = `Factor(1.0)`; `ZoomFit` = `Fit` **and resets `pan_offset` to zero**. Add buttons to the header `right_row`. Bind keyboard shortcuts (mirror `cmd_or_ctrl_shift!` usage in `notebooks/file/mod.rs`).

- [ ] **Step 4: Pan drag handling (only when zoomed)**

Wrap the image in a mouse-interactive element (mirror `MouseStateHandle` usage, e.g. `retry_button_mouse_state` in `notebooks/file/mod.rs`, or a drag-capable element). On drag delta, update `self.pan_offset` + `ctx.notify()`, only when `matches!(self.zoom, ZoomMode::Factor(f) if f > fit_factor)`.

- [ ] **Step 5: Apply and clamp the offset**

Offset the image's paint origin by `pan_offset`, clamped so the image can't be dragged fully out of view (`±(scaled_size - viewport)/2`). `pan_offset` is reset to zero by the `ZoomFit` handler (Step 3).

- [ ] **Step 6: Integration test + commit**

Test: `Zoom100` sets `Factor(1.0)`; `ZoomIn` then `ZoomOut` returns to the prior factor; with zoom > fit a simulated drag changes `pan_offset`; `ZoomFit` resets it to zero. Run; PASS.

```bash
git add app/src/image_viewer/mod.rs crates/integration
git commit -m "feat(image-pane): zoom and pan"
```

---

### Task 7: SVG view-source toggle

**Files:** Modify `app/src/image_viewer/mod.rs`; integration test.

**Interfaces:** Consumes `is_svg()`, `file_bytes` (populated by Task 8; until then read the file once in `open_local`). Produces `ImageViewAction::ToggleSource`.

- [ ] **Step 1: Read source bytes for SVG**

In `open_local`, when `is_svg()`, read the file to `self.file_bytes` (Task 8 keeps this fresh on change). For non-SVG, leave `None`.

- [ ] **Step 2: Render source vs image**

In `render`, if `self.source_view_open && self.is_svg()`, render a read-only monospace `Text` of `String::from_utf8_lossy(file_bytes)` inside a scrollable container; otherwise render the image (existing path).

- [ ] **Step 3: Header toggle (SVG only)**

When `is_svg()`, add a segmented control to `right_row` (clone `MarkdownToggleView` → e.g. `ImageSourceToggleView` with `Rendered`/`Source`, mirroring `notebooks/file/mod.rs:274-281` + `view_components::MarkdownToggleView`). Hide it for raster formats. Wire `ImageViewAction::ToggleSource` to flip `self.source_view_open` + `ctx.notify()`.

- [ ] **Step 4: Integration test + commit**

Test: open an SVG, toggle shows XML containing `"<svg"`; toggle back shows the render; for a PNG the toggle control is absent. Run; PASS.

```bash
git add app/src/image_viewer/mod.rs crates/integration
git commit -m "feat(image-pane): SVG view-source toggle"
```

---

### Task 8: Live reload on file change

**Files:** Modify `app/src/image_viewer/mod.rs`; integration test.

**Interfaces:** Consumes `warp_files::FileModel`, `FileModelEvent` (`warp_files/src/lib.rs:34`). Mirrors `FileNotebookView::open_local` subscription (`notebooks/file/mod.rs:429-498`).

- [ ] **Step 1: Subscribe on open**

In `open_local`, under `#[cfg(feature = "local_fs")]`: cancel/unsubscribe a previous `file_id`, then `let file_id = FileModel::handle(ctx).update(ctx, |m, ctx| m.open(&local_path, true, ctx)); self.file_id = Some(file_id);` and `ctx.subscribe_to_model(&file_model, move |me, fm, event, ctx| { if event.file_id() != file_id { return; } ... })`.

- [ ] **Step 2: Handle events**

```rust
match event {
    FileModelEvent::FileLoaded { content, .. } | FileModelEvent::FileUpdated { content, .. } => {
        // Store the bytes for the SVG source view. Match `content`'s real type at
        // warp_files/src/lib.rs:52 — if it is a `String`, use `content.clone().into_bytes()`;
        // if it is already `Vec<u8>`/`Bytes`, clone directly.
        me.file_bytes = Some(/* content as Vec<u8> per the type at lib.rs:52 */);
        me.refresh_source();   // re-run AssetSource::LocalFile{..}.with_local_file_content_version()
        ctx.emit(ImageViewEvent::FileLoaded);   // triggers AppStateChanged so path persists
        ctx.notify();
    }
    FileModelEvent::FailedToLoad { .. } => { /* set an error flag; render failure element */ ctx.notify(); }
    _ => {}
}
```

`refresh_source` rebuilds `self.source = Some(AssetSource::LocalFile { path, content_version: None }.with_local_file_content_version())` — the fresh fingerprint busts the image cache so the new bytes are decoded. (`content` is the file text/bytes from `FileModel`; match its exact type — see `FileModelEvent::FileUpdated` at `warp_files/src/lib.rs:52`.)

- [ ] **Step 3: Unsubscribe on detach**

Ensure `ImagePane::detach` unsubscribes (mirror `file_pane.rs:125`). The view should also unsubscribe its `file_id` when reopening a different file.

- [ ] **Step 4: Integration test + commit**

Test: open an image, modify the file on disk (rewrite fixture bytes), pump the watcher, assert the view's `source` content_version changed (and, for SVG source view, the text updated). Run; PASS.

```bash
git add app/src/image_viewer/mod.rs crates/integration
git commit -m "feat(image-pane): live reload on file change"
```

---

### Task 9: Animated GIF/WebP + final verification

**Files:** Modify `app/src/image_viewer/mod.rs`; run the full check.

- [ ] **Step 1: Animate gif/webp by default**

In `render_image`, when the path extension is `gif`/`webp`, call `.enable_animation_with_start_time(now)` (get `now` from the view's existing time source; mirror any `Instant`-based animation in the codebase). **Risk gate:** if profiling shows jank, switch to `.first_frame_preview()` and add a play toggle action instead (document whichever you chose in a code comment).

- [ ] **Step 2: Full test + build**

Run:
```bash
cargo check -p app --features "local_fs image_preview_pane"
cargo test  -p app --features "local_fs image_preview_pane" -- openable_file_type_tests image_viewer
```
Plus the integration suite per the `warp-integration-test` skill. Expected: all green.

- [ ] **Step 3: Manual smoke (per `/run` or `verify` skill)**

Launch the app with the flag on; open `.svg`, `.png`, `.gif`, a transparent `.png`; verify render, backdrop toggle, zoom, pan, SVG source toggle, and live reload (edit a file and watch it update). Confirm flag **off** restores legacy behavior (svg→source, png→external app).

- [ ] **Step 4: Commit**

```bash
git add app/src/image_viewer/mod.rs
git commit -m "feat(image-pane): animate gif/webp and finalize image preview pane"
```

---

## Follow-ups (not in this plan)

- **Flag promotion:** use the `promote-feature` skill to roll `ImagePreviewPane` to Dogfood→Preview→Stable.
- **Dead-code removal at promotion:** once stable, delete the `SystemGeneric` image branch in `app/src/ai/blocklist/block.rs:320` and remove the flag via the `remove-feature-flag` skill.

## Self-Review notes

- **Spec coverage:** flag (T1), routing+entry points (T4), view+pane (T2-T3), persistence (T3), backdrop (T5), zoom+pan (T6), SVG source (T7), live reload (T8), animation+verify (T9), cleanup follow-ups (closing). All spec sections map to a task.
- **Type consistency:** `ImageView`, `ImagePane`, `ImagePaneSnapshot`, `LeafContents::ImageViewer`, `FileTarget::ImageViewer`, `OpenableFileType::Image`, `ZoomMode`, `Backdrop`, `ImageViewAction`, `ImageViewEvent`, `IMAGE_VIEWER_PANE_KIND` used consistently across tasks.
- **Known soft spots (verify against the compiler/templates during execution, not assumed):** exact `StandardHeader` fields (`notebooks/file/mod.rs:1250`), the `NewTab` index/group args in `open_file_image` (`view.rs:8290`), `FileModelEvent` content type (`warp_files/src/lib.rs:52`), and whether the repo has a view-level unit harness (it does not — UI is validated via `crates/integration`).
