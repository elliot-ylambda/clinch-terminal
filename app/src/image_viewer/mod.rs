//! A pane-backing view that renders a single image file.
//!
//! This is the foundation for the image preview pane: it resolves a local image
//! into an [`AssetSource`] and renders it (aspect-fit) on a backdrop. Zoom, pan,
//! the backdrop checkerboard, the SVG source view, and live file-change reloading
//! are stubbed here as state + defaults and are filled in by later tasks.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use instant::Instant;
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::{vec2f, Vector2F};
#[cfg(feature = "local_fs")]
use warp_files::{FileModel, FileModelEvent};
#[cfg(feature = "local_fs")]
use warp_util::file::FileId;
use warp_core::ui::icons::ICON_DIMENSIONS;
use warpui::assets::asset_cache::{AssetCache, AssetSource};
use warpui::elements::{
    CacheOption, Clipped, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container,
    CrossAxisAlignment, DispatchEventResult, Element, Empty, EventHandler, Expanded, Fill, Flex,
    Image, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Point, ScrollbarWidth,
    Stack, Text,
};
use warpui::event::DispatchedEvent;
use warpui::image_cache::ImageCache;
use warpui::keymap::EditableBinding;
use warpui::ui_components::components::UiComponent;
use warpui::{
    AfterLayoutContext, AppContext, Entity, EventContext, LayoutContext, ModelHandle, PaintContext,
    SingletonEntity, SizeConstraint, TypedActionView, View, ViewContext,
};

use crate::appearance::Appearance;
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::pane::view;
use crate::pane_group::pane::view::header::components::{
    render_pane_header_buttons, render_pane_header_title_text, render_three_column_header,
    CenteredHeaderEdgeWidth,
};
use crate::pane_group::{BackingView, PaneConfiguration, PaneEvent};
use crate::terminal::model::session::Session;
use crate::ui_components::buttons::icon_button;
use crate::ui_components::icons::Icon;

/// How the image is sized within the pane. `Fit` contains the image; `Factor`
/// scales the intrinsic size. "100%" is `Factor(1.0)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ZoomMode {
    Fit,
    Factor(f32),
}

/// Multiplicative step applied on each zoom-in / zoom-out.
const ZOOM_STEP: f32 = 1.25;
/// Minimum zoom factor (10%).
const MIN_ZOOM: f32 = 0.1;
/// Maximum zoom factor (800%).
const MAX_ZOOM: f32 = 8.0;
/// The factor that a `Fit` view is treated as when the user begins stepping the
/// zoom, and the threshold past which the image can be panned. Documented as
/// `1.0` (i.e. "100%"); panning only makes sense once the user has zoomed past
/// this point.
const FIT_FACTOR: f32 = 1.0;

/// Returns the zoom mode after a single zoom-in step.
///
/// Stepping always produces a `Factor`. From `Fit` we start at [`FIT_FACTOR`]
/// (an effective 100%) and step up from there. The result is clamped to
/// `[MIN_ZOOM, MAX_ZOOM]`.
fn zoom_in(zoom: ZoomMode) -> ZoomMode {
    let current = match zoom {
        ZoomMode::Fit => FIT_FACTOR,
        ZoomMode::Factor(factor) => factor,
    };
    ZoomMode::Factor((current * ZOOM_STEP).clamp(MIN_ZOOM, MAX_ZOOM))
}

/// Returns the zoom mode after a single zoom-out step (the inverse of
/// [`zoom_in`]). From `Fit` we start at [`FIT_FACTOR`] and step down.
fn zoom_out(zoom: ZoomMode) -> ZoomMode {
    let current = match zoom {
        ZoomMode::Fit => FIT_FACTOR,
        ZoomMode::Factor(factor) => factor,
    };
    ZoomMode::Factor((current / ZOOM_STEP).clamp(MIN_ZOOM, MAX_ZOOM))
}

/// Clamps a pan offset so the scaled image can never be dragged fully out of
/// view. Each axis is limited to `±max(0, (scaled - viewport) / 2)`, which is
/// zero (no panning) whenever the image is not larger than the viewport on that
/// axis.
fn clamp_pan(pan: Vector2F, scaled: Vector2F, viewport: Vector2F) -> Vector2F {
    let max_x = ((scaled.x() - viewport.x()) / 2.0).max(0.0);
    let max_y = ((scaled.y() - viewport.y()) / 2.0).max(0.0);
    vec2f(pan.x().clamp(-max_x, max_x), pan.y().clamp(-max_y, max_y))
}

/// Background drawn behind the image so transparent assets stay legible.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Backdrop {
    Checkerboard,
    Light,
    Dark,
}

impl Backdrop {
    /// The next backdrop in the cycle: Checkerboard → Light → Dark → Checkerboard.
    fn next(self) -> Self {
        match self {
            Backdrop::Checkerboard => Backdrop::Light,
            Backdrop::Light => Backdrop::Dark,
            Backdrop::Dark => Backdrop::Checkerboard,
        }
    }
}

/// Events emitted by an [`ImageView`] to its owning pane.
#[derive(Debug, Clone)]
pub enum ImageViewEvent {
    /// The pane title changed (e.g. after opening a file).
    TitleUpdated,
    /// The backing image file finished loading.
    FileLoaded,
    /// A generic pane lifecycle event (close, maximize, ...).
    Pane(PaneEvent),
}

impl From<PaneEvent> for ImageViewEvent {
    fn from(event: PaneEvent) -> Self {
        ImageViewEvent::Pane(event)
    }
}

/// Actions handled by an [`ImageView`].
#[derive(Debug, Clone)]
pub enum ImageViewAction {
    Focus,
    Close,
    ToggleMaximized,
    /// Cycle to the given [`Backdrop`] variant.
    SetBackdrop(Backdrop),
    /// Aspect-fit the image and reset any pan offset.
    ZoomFit,
    /// Render the image at 100% (`Factor(1.0)`).
    Zoom100,
    /// Step the zoom factor up by [`ZOOM_STEP`].
    ZoomIn,
    /// Step the zoom factor down by [`ZOOM_STEP`].
    ZoomOut,
    /// Begin a pan drag at the given cursor position.
    PanBegin(Vector2F),
    /// Continue a pan drag to the given cursor position.
    PanMove(Vector2F),
    /// End the active pan drag.
    PanEnd,
    /// SVG only: flip between the rendered image and raw XML source.
    ToggleSource,
}

/// Register [`ImageView`] keyboard bindings. Called once at startup.
///
/// Bindings are scoped to the `ImageView` context so the conventional zoom
/// shortcuts only apply while an image pane is focused. The `=`/`-`/`0`/`9`
/// keys aren't valid with [`crate::cmd_or_ctrl_shift`] (which would panic on
/// non-mac debug builds), so per-OS bindings are registered directly.
pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_editable_bindings([
        EditableBinding::new("imageview:zoom_in", "Zoom In", ImageViewAction::ZoomIn)
            .with_context_predicate(id!("ImageView"))
            .with_mac_key_binding("cmd-=")
            .with_linux_or_windows_key_binding("ctrl-="),
        EditableBinding::new("imageview:zoom_out", "Zoom Out", ImageViewAction::ZoomOut)
            .with_context_predicate(id!("ImageView"))
            .with_mac_key_binding("cmd--")
            .with_linux_or_windows_key_binding("ctrl--"),
        EditableBinding::new("imageview:zoom_100", "Zoom to 100%", ImageViewAction::Zoom100)
            .with_context_predicate(id!("ImageView"))
            .with_mac_key_binding("cmd-0")
            .with_linux_or_windows_key_binding("ctrl-0"),
        EditableBinding::new("imageview:zoom_fit", "Fit Image to Pane", ImageViewAction::ZoomFit)
            .with_context_predicate(id!("ImageView"))
            .with_mac_key_binding("cmd-9")
            .with_linux_or_windows_key_binding("ctrl-9"),
    ]);
}

pub struct ImageView {
    /// Absolute local path of the open image, once resolved.
    path: Option<PathBuf>,
    /// `AssetSource::LocalFile { .. }` with a content fingerprint, recomputed on
    /// open and on file-change so an edited file is re-decoded.
    source: Option<AssetSource>,
    /// Current sizing mode. `Fit` aspect-fits; `Factor(f)` renders at the
    /// intrinsic size scaled by `f`.
    zoom: ZoomMode,
    /// The instant at which the current file was opened. Used as the animation
    /// start time for gif/webp files so the animation plays from the moment the
    /// file is opened (see `render_image`). Reset on each `open_local` call.
    animation_start: Instant,
    backdrop: Backdrop,
    /// Pan offset (in logical points) applied to the scaled image when zoomed
    /// past fit. Always kept clamped via [`clamp_pan`]; reset to zero on
    /// `ZoomFit`.
    pan_offset: Vector2F,
    /// Anchor for an in-progress pan drag (the last observed cursor position).
    /// `Some` only while the left mouse button is held down over a zoomed image.
    pan_drag_anchor: Option<Vector2F>,
    /// Last painted viewport size of the image area, published by the
    /// [`ImagePan`] element during layout so the action handler can clamp the
    /// pan offset against the real viewport.
    viewport_size: Rc<Cell<Vector2F>>,
    /// SVG-only: whether the raw XML source is shown instead of the render.
    source_view_open: bool,
    /// Latest file bytes (for SVG source view). Read once in `open_local`; refreshed in Task 8.
    file_bytes: Option<Vec<u8>>,
    /// File watcher id for the open file. Cancelled/replaced on each `open_local`
    /// and unsubscribed when the pane is detached.
    #[cfg(feature = "local_fs")]
    file_id: Option<FileId>,
    pane_configuration: ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,
    /// Mouse state for the backdrop-toggle icon button in the pane header.
    backdrop_button_mouse_state: MouseStateHandle,
    /// Mouse states for the header zoom-control icon buttons.
    zoom_out_button_mouse_state: MouseStateHandle,
    zoom_in_button_mouse_state: MouseStateHandle,
    zoom_100_button_mouse_state: MouseStateHandle,
    zoom_fit_button_mouse_state: MouseStateHandle,
    /// Mouse state for the SVG source-view toggle button in the pane header.
    source_toggle_mouse_state: MouseStateHandle,
    /// Scroll state for the SVG source text view.
    source_view_scroll_state: ClippedScrollStateHandle,
}

/// Returns `true` if `path` has an `.svg` extension (case-insensitive).
fn path_is_svg(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
}

/// Returns `true` if `path` has an animated format extension (gif or webp,
/// case-insensitive). Used to decide whether to call
/// `enable_animation_with_start_time` in `render_image`.
fn is_animated_ext(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("gif") || e.eq_ignore_ascii_case("webp"))
}

impl ImageView {
    /// Create a new image view with no open image.
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new(""));

        Self {
            path: None,
            source: None,
            zoom: ZoomMode::Fit,
            animation_start: Instant::now(),
            backdrop: Backdrop::Checkerboard,
            pan_offset: vec2f(0.0, 0.0),
            pan_drag_anchor: None,
            viewport_size: Rc::new(Cell::new(vec2f(0.0, 0.0))),
            source_view_open: false,
            file_bytes: None,
            #[cfg(feature = "local_fs")]
            file_id: None,
            pane_configuration,
            focus_handle: None,
            backdrop_button_mouse_state: Default::default(),
            zoom_out_button_mouse_state: Default::default(),
            zoom_in_button_mouse_state: Default::default(),
            zoom_100_button_mouse_state: Default::default(),
            zoom_fit_button_mouse_state: Default::default(),
            source_toggle_mouse_state: Default::default(),
            source_view_scroll_state: Default::default(),
        }
    }

    /// Open a local image file: set the title, resolve the asset source, and
    /// remember the path. The live file-change subscription is added in Task 8.
    pub fn open_local(
        &mut self,
        path: impl Into<PathBuf>,
        _session: Option<Arc<Session>>,
        ctx: &mut ViewContext<Self>,
    ) {
        let local_path: PathBuf = path.into();

        self.pane_configuration.update(ctx, |cfg, ctx| {
            cfg.set_title(
                local_path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
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

        // Reset the animation clock so gif/webp plays from frame 0 on open.
        self.animation_start = Instant::now();

        // Reset the source-view toggle whenever a new file is opened.
        self.source_view_open = false;

        #[cfg(feature = "local_fs")]
        {
            // Cancel in-flight loads and unsubscribe from any previously watched file.
            if let Some(prev_id) = self.file_id.take() {
                FileModel::handle(ctx).update(ctx, |m, ctx| {
                    m.cancel(prev_id);
                    m.unsubscribe(prev_id, ctx);
                });
            }

            let file_model = FileModel::handle(ctx);
            // `self.path` was set just above; unwrap is infallible here.
            let open_path = self.path.clone().expect("path was just set");
            let file_id = file_model.update(ctx, |m, ctx| m.open(&open_path, true, ctx));
            self.file_id = Some(file_id);

            ctx.subscribe_to_model(
                &file_model,
                move |me, _file_model: ModelHandle<FileModel>, event: &FileModelEvent, ctx| {
                    if event.file_id() != file_id {
                        return;
                    }
                    match event {
                        FileModelEvent::FileLoaded { content, .. } => {
                            // `content` is a `String` (see warp_files/src/lib.rs:36,53).
                            if me.is_svg() {
                                me.file_bytes = Some(content.as_bytes().to_vec());
                            }
                            me.refresh_source();
                            ctx.emit(ImageViewEvent::FileLoaded);
                            ctx.notify();
                        }
                        FileModelEvent::FileUpdated { content, .. } => {
                            if me.is_svg() {
                                me.file_bytes = Some(content.as_bytes().to_vec());
                            }
                            me.refresh_source();
                            // Restart the animation clock so an edited gif/webp
                            // plays from frame 0 rather than resuming mid-loop.
                            me.animation_start = instant::Instant::now();
                            ctx.emit(ImageViewEvent::FileLoaded);
                            ctx.notify();
                        }
                        FileModelEvent::FailedToLoad { .. } => {
                            ctx.notify();
                        }
                        FileModelEvent::FileSaved { .. } | FileModelEvent::FailedToSave { .. } => {}
                    }
                },
            );
        }

        ctx.emit(ImageViewEvent::TitleUpdated);
        ctx.notify();
    }

    /// Recompute `self.source` with a fresh content-version fingerprint so the
    /// image cache decodes the updated bytes rather than serving the stale copy.
    fn refresh_source(&mut self) {
        if let Some(path) = &self.path {
            self.source = Some(
                AssetSource::LocalFile {
                    path: path.to_string_lossy().into_owned(),
                    content_version: None,
                }
                .with_local_file_content_version(),
            );
        }
    }

    /// Cancel the active file-watcher subscription, if any.  Called from
    /// `ImagePane::detach` so the watcher is torn down when the pane closes.
    #[cfg(feature = "local_fs")]
    pub fn unsubscribe_file_watch(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(prev_id) = self.file_id.take() {
            FileModel::handle(ctx).update(ctx, |m, ctx| {
                m.cancel(prev_id);
                m.unsubscribe(prev_id, ctx);
            });
        }
    }

    /// The local path of the open image, if any.
    pub fn local_path(&self) -> Option<PathBuf> {
        self.path.clone()
    }

    /// Whether the open image is an SVG (decided purely by extension).
    pub fn is_svg(&self) -> bool {
        self.path.as_deref().is_some_and(path_is_svg)
    }

    /// Whether the SVG source-view toggle should be shown: SVG extension AND source bytes available.
    fn has_svg_source(&self) -> bool {
        self.is_svg() && self.file_bytes.is_some()
    }

    /// Handle to the pane configuration (title, overflow menu, ...).
    pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    /// Move focus to this view's contents.
    pub fn focus(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus_self();
    }

    /// Render the image element with before-load and failure fallbacks, or an
    /// empty element when no source has been resolved yet.
    ///
    /// `Fit` aspect-fits the image into the pane. `Factor(f)` renders the image
    /// at its intrinsic pixel size scaled by `f`, centred and offset by the
    /// (clamped) pan offset, clipped to the pane. When zoomed past
    /// [`FIT_FACTOR`] the image is wrapped in an [`EventHandler`] so it can be
    /// dragged to pan.
    fn render_image(&self, app: &AppContext) -> Box<dyn Element> {
        let Some(source) = self.source.clone() else {
            return Empty::new().finish();
        };
        let appearance = Appearance::as_ref(app);

        // `CacheOption::Original` keeps a single decoded copy; the GPU scales it.
        //
        // For animated formats (gif, webp) we enable animation using the start
        // time recorded when the file was opened. For all other formats the
        // image element defaults to static rendering (no animation plumbing
        // needed). The framework drives repaints internally once animation is
        // enabled, so no extra repaint-scheduling is required here.
        let animated = self.path.as_deref().is_some_and(is_animated_ext);
        let image_builder = Image::new(source.clone(), CacheOption::Original)
            .contain()
            .before_load(Empty::new().finish())
            .on_load_failure(
                appearance
                    .ui_builder()
                    .paragraph("Couldn't render this image")
                    .build()
                    .finish(),
            );
        let image = if animated {
            image_builder.enable_animation_with_start_time(self.animation_start)
        } else {
            image_builder
        };

        let factor = match self.zoom {
            ZoomMode::Fit => return image.finish(),
            ZoomMode::Factor(factor) => factor,
        };

        // The intrinsic size is only available once the asset has decoded; until
        // then we fall back to a contained fit so the pane isn't blank.
        let Some(intrinsic) = ImageCache::as_ref(app).image_size(source, AssetCache::as_ref(app))
        else {
            return image.finish();
        };

        let scaled = vec2f(
            intrinsic.x() as f32 * factor,
            intrinsic.y() as f32 * factor,
        );

        // Force the image element to the scaled size; `.contain()` then fills it
        // exactly (same aspect ratio), and the GPU scales the decoded bitmap.
        let sized = ConstrainedBox::new(image.finish())
            .with_width(scaled.x())
            .with_height(scaled.y())
            .finish();

        // Centre + pan offset happen in `ImagePan`; `Clipped` keeps an oversized
        // image inside the pane bounds.
        let panned = Clipped::new(
            ImagePan::new(sized, scaled, self.pan_offset, self.viewport_size.clone()).finish(),
        )
        .finish();

        if factor > FIT_FACTOR {
            EventHandler::new(panned)
                // Propagate the down/up events so a click still focuses the pane;
                // the pan anchor is set/cleared via the dispatched action either
                // way. The drag itself is consumed so it can't be read as a pane
                // resize.
                .on_left_mouse_down(|ctx, _, position| {
                    ctx.dispatch_typed_action(ImageViewAction::PanBegin(position));
                    DispatchEventResult::PropagateToParent
                })
                .on_mouse_dragged(|ctx, _, position| {
                    ctx.dispatch_typed_action(ImageViewAction::PanMove(position));
                    DispatchEventResult::StopPropagation
                })
                .on_left_mouse_up(|ctx, _, _| {
                    ctx.dispatch_typed_action(ImageViewAction::PanEnd);
                    DispatchEventResult::PropagateToParent
                })
                .finish()
        } else {
            panned
        }
    }

    /// Clamp `offset` against the current viewport and scaled image size so the
    /// image can never be panned fully out of view. Falls back to the raw offset
    /// when the intrinsic size or viewport isn't known yet.
    fn clamp_pan_offset(&self, offset: Vector2F, app: &AppContext) -> Vector2F {
        let ZoomMode::Factor(factor) = self.zoom else {
            return vec2f(0.0, 0.0);
        };
        let Some(source) = self.source.clone() else {
            return offset;
        };
        let Some(intrinsic) = ImageCache::as_ref(app).image_size(source, AssetCache::as_ref(app))
        else {
            return offset;
        };
        let viewport = self.viewport_size.get();
        if viewport.x() <= 0.0 || viewport.y() <= 0.0 {
            return offset;
        }
        let scaled = vec2f(
            intrinsic.x() as f32 * factor,
            intrinsic.y() as f32 * factor,
        );
        clamp_pan(offset, scaled, viewport)
    }

    /// Render a 2×2 checkerboard grid that expands to fill the available space.
    ///
    /// Standard image-editor colors: light gray (#CCCCCC) and medium gray (#999999).
    /// The coarse 2×2 tile is clearly visible at any pane size.
    /// Render the raw SVG XML source as read-only monospace text in a scrollable container.
    /// Only called when `source_view_open && is_svg()`.
    fn render_source_view(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let content = self
            .file_bytes
            .as_deref()
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default();

        let text = Text::new(
            content,
            appearance.monospace_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(theme.main_text_color(theme.background()).into())
        .finish();

        let padded = Container::new(text)
            .with_padding_top(8.0)
            .with_padding_left(12.0)
            .with_padding_right(12.0)
            .with_padding_bottom(8.0)
            .finish();

        Container::new(
            ClippedScrollable::vertical(
                self.source_view_scroll_state.clone(),
                padded,
                ScrollbarWidth::Custom(8.0),
                theme.nonactive_ui_detail().into(),
                theme.active_ui_detail().into(),
                Fill::None,
            )
            .finish(),
        )
        .with_background_color(theme.background().into())
        .finish()
    }

    fn render_checkerboard_grid() -> Box<dyn Element> {
        let light = ColorU::new(204, 204, 204, 255);
        let dark = ColorU::new(153, 153, 153, 255);

        let make_cell = |color: ColorU| -> Box<dyn Element> {
            Expanded::new(1.0, Container::new(Empty::new().finish()).with_background_color(color).finish()).finish()
        };

        let mut row1 = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        row1.add_child(make_cell(light));
        row1.add_child(make_cell(dark));

        let mut row2 = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        row2.add_child(make_cell(dark));
        row2.add_child(make_cell(light));

        let mut col = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        col.add_child(Expanded::new(1.0, row1.finish()).finish());
        col.add_child(Expanded::new(1.0, row2.finish()).finish());

        col.finish()
    }
}

/// An element that paints a fixed-size (already scaled) child centred within the
/// available viewport, shifted by a pan offset and clamped so the child can't be
/// dragged fully out of view.
///
/// The child is laid out unbounded so it keeps its full scaled size even when it
/// is larger than the viewport (panning then reveals different regions). The
/// viewport size observed during layout is published back to the owning view via
/// a shared cell so the pan action handler can clamp the stored offset against
/// the real viewport.
struct ImagePan {
    child: Box<dyn Element>,
    /// Logical scaled size of the child (intrinsic size × zoom factor).
    scaled: Vector2F,
    /// Requested pan offset before clamping.
    offset: Vector2F,
    /// Shared cell updated with the viewport size on each layout.
    viewport_cell: Rc<Cell<Vector2F>>,
    size: Option<Vector2F>,
    origin: Option<Point>,
}

impl ImagePan {
    fn new(
        child: Box<dyn Element>,
        scaled: Vector2F,
        offset: Vector2F,
        viewport_cell: Rc<Cell<Vector2F>>,
    ) -> Self {
        Self {
            child,
            scaled,
            offset,
            viewport_cell,
            size: None,
            origin: None,
        }
    }
}

impl Element for ImagePan {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        // Let the child take its full scaled size, even past the viewport.
        let child_constraint =
            SizeConstraint::new(Vector2F::zero(), Vector2F::splat(f32::INFINITY));
        self.child.layout(child_constraint, ctx, app);

        let size = constraint.max;
        self.size = Some(size);
        if size.x().is_finite() && size.y().is_finite() {
            self.viewport_cell.set(size);
        }
        size
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));

        let viewport = self.size.unwrap_or(self.scaled);
        let clamped = clamp_pan(self.offset, self.scaled, viewport);
        let centering = (viewport - self.scaled) / 2.0;
        self.child.paint(origin + centering + clamped, ctx, app);
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        self.child.dispatch_event(event, ctx, app)
    }
}

impl Entity for ImageView {
    type Event = ImageViewEvent;
}

impl View for ImageView {
    fn ui_name() -> &'static str {
        "ImageView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        // When the SVG source view is active, replace the backdrop+image with the raw XML text.
        if self.source_view_open && self.is_svg() {
            return self.render_source_view(app);
        }

        match self.backdrop {
            Backdrop::Checkerboard => {
                // Layer the checkerboard grid behind the image.
                let mut stack = Stack::new();
                stack.add_child(Self::render_checkerboard_grid());
                stack.add_child(self.render_image(app));
                stack.finish()
            }
            Backdrop::Light => Container::new(self.render_image(app))
                .with_background_color(ColorU::new(250, 250, 250, 255))
                .finish(),
            Backdrop::Dark => Container::new(self.render_image(app))
                .with_background_color(ColorU::new(24, 24, 24, 255))
                .finish(),
        }
    }
}

impl TypedActionView for ImageView {
    type Action = ImageViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ImageViewAction::Focus => self.focus(ctx),
            ImageViewAction::Close => ctx.emit(ImageViewEvent::Pane(PaneEvent::Close)),
            ImageViewAction::ToggleMaximized => {
                ctx.emit(ImageViewEvent::Pane(PaneEvent::ToggleMaximized))
            }
            ImageViewAction::SetBackdrop(backdrop) => {
                self.backdrop = *backdrop;
                ctx.notify();
            }
            ImageViewAction::ZoomFit => {
                self.zoom = ZoomMode::Fit;
                self.pan_offset = vec2f(0.0, 0.0);
                ctx.notify();
            }
            ImageViewAction::Zoom100 => {
                self.zoom = ZoomMode::Factor(1.0);
                self.pan_offset = self.clamp_pan_offset(self.pan_offset, ctx);
                ctx.notify();
            }
            ImageViewAction::ZoomIn => {
                self.zoom = zoom_in(self.zoom);
                self.pan_offset = self.clamp_pan_offset(self.pan_offset, ctx);
                ctx.notify();
            }
            ImageViewAction::ZoomOut => {
                self.zoom = zoom_out(self.zoom);
                self.pan_offset = self.clamp_pan_offset(self.pan_offset, ctx);
                ctx.notify();
            }
            ImageViewAction::PanBegin(position) => {
                // Only start a drag when zoomed past fit (the only state where
                // there is anything to pan).
                if matches!(self.zoom, ZoomMode::Factor(factor) if factor > FIT_FACTOR) {
                    self.pan_drag_anchor = Some(*position);
                }
            }
            ImageViewAction::PanMove(position) => {
                if let Some(anchor) = self.pan_drag_anchor {
                    let delta = *position - anchor;
                    self.pan_offset = self.clamp_pan_offset(self.pan_offset + delta, ctx);
                    self.pan_drag_anchor = Some(*position);
                    ctx.notify();
                }
            }
            ImageViewAction::PanEnd => {
                self.pan_drag_anchor = None;
            }
            ImageViewAction::ToggleSource => {
                self.source_view_open = !self.source_view_open;
                ctx.notify();
            }
        }
    }
}

impl BackingView for ImageView {
    type PaneHeaderOverflowMenuAction = ();
    type CustomAction = ();
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        _action: &(),
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(ImageViewEvent::Pane(PaneEvent::Close));
    }

    fn focus_contents(&mut self, ctx: &mut ViewContext<Self>) {
        self.focus(ctx);
    }

    fn render_header_content(
        &self,
        ctx: &view::HeaderRenderContext<'_>,
        app: &AppContext,
    ) -> view::HeaderContent {
        let title = self.pane_configuration.as_ref(app).title().to_owned();
        let appearance = Appearance::as_ref(app);
        let is_pane_dragging = ctx.draggable_state.is_dragging();

        // Build right-side controls: zoom controls + backdrop toggle + standard
        // pane buttons.
        let mut right_row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min);

        // Zoom controls: fit, out, 100%, in.
        let zoom_fit_active = matches!(self.zoom, ZoomMode::Fit);
        let zoom_100_active =
            matches!(self.zoom, ZoomMode::Factor(factor) if (factor - 1.0).abs() < f32::EPSILON);

        right_row.add_child(
            icon_button(
                appearance,
                Icon::Maximize,
                zoom_fit_active,
                self.zoom_fit_button_mouse_state.clone(),
            )
            .build()
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(ImageViewAction::ZoomFit))
            .finish(),
        );
        right_row.add_child(
            icon_button(
                appearance,
                Icon::Minus,
                false,
                self.zoom_out_button_mouse_state.clone(),
            )
            .build()
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(ImageViewAction::ZoomOut))
            .finish(),
        );
        right_row.add_child(
            icon_button(
                appearance,
                Icon::Image,
                zoom_100_active,
                self.zoom_100_button_mouse_state.clone(),
            )
            .build()
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(ImageViewAction::Zoom100))
            .finish(),
        );
        right_row.add_child(
            icon_button(
                appearance,
                Icon::Plus,
                false,
                self.zoom_in_button_mouse_state.clone(),
            )
            .build()
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(ImageViewAction::ZoomIn))
            .finish(),
        );

        let next = self.backdrop.next();
        let backdrop_btn = icon_button(
            appearance,
            Icon::Grid,
            false,
            self.backdrop_button_mouse_state.clone(),
        )
        .build()
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(ImageViewAction::SetBackdrop(next)))
        .finish();
        right_row.add_child(backdrop_btn);

        // SVG-only: source-view toggle button. Only shown when source bytes are available.
        if self.has_svg_source() {
            let source_active = self.source_view_open;
            right_row.add_child(
                icon_button(
                    appearance,
                    Icon::Code2,
                    source_active,
                    self.source_toggle_mouse_state.clone(),
                )
                .build()
                .on_click(move |ctx, _, _| ctx.dispatch_typed_action(ImageViewAction::ToggleSource))
                .finish(),
            );
        }

        let show_close_button = self
            .focus_handle
            .as_ref()
            .is_some_and(|h| h.is_in_split_pane(app));

        right_row.add_child(render_pane_header_buttons::<ImageViewAction, ()>(
            ctx,
            appearance,
            show_close_button,
            None,
            None,
        ));

        let button_count = 4 // zoom: fit, out, 100%, in
            + 1 // backdrop button
            + self.has_svg_source() as u32 // source-view toggle (SVG + bytes only)
            + show_close_button as u32
            + ctx.has_overflow_items as u32;
        let buttons_width = button_count as f32 * ICON_DIMENSIONS;

        let title_element = render_pane_header_title_text(
            title,
            appearance,
            warpui::text_layout::ClipConfig::start(),
        );

        view::HeaderContent::Custom {
            element: render_three_column_header(
                Empty::new().finish(),
                title_element,
                right_row.finish(),
                CenteredHeaderEdgeWidth {
                    min: buttons_width,
                    max: 220.0,
                },
                ctx.header_left_inset,
                is_pane_dragging,
            ),
            has_custom_draggable_behavior: false,
        }
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `is_animated_ext` determines whether `render_image` enables animation;
    /// test gif/webp recognition including uppercase and mixed-case extensions.
    #[test]
    fn is_animated_ext_check() {
        use std::path::Path;

        // Positive cases – animated formats
        assert!(is_animated_ext(Path::new("anim.gif")), ".gif should be animated");
        assert!(is_animated_ext(Path::new("anim.GIF")), ".GIF (uppercase) should be animated");
        assert!(is_animated_ext(Path::new("anim.Gif")), ".Gif (mixed-case) should be animated");
        assert!(is_animated_ext(Path::new("image.webp")), ".webp should be animated");
        assert!(is_animated_ext(Path::new("image.WEBP")), ".WEBP (uppercase) should be animated");
        assert!(is_animated_ext(Path::new("path/to/anim.gif")), "nested .gif path should work");

        // Negative cases – static or unknown formats
        assert!(!is_animated_ext(Path::new("photo.png")), ".png is not animated");
        assert!(!is_animated_ext(Path::new("photo.jpg")), ".jpg is not animated");
        assert!(!is_animated_ext(Path::new("vector.svg")), ".svg is not animated");
        assert!(!is_animated_ext(Path::new("noextension")), "no extension returns false");
    }

    /// `path_is_svg` drives both `is_svg` and the source-view toggle guard; test it directly.
    #[test]
    fn is_svg_extension_check() {
        use std::path::Path;

        // Positive cases
        assert!(path_is_svg(Path::new("icon.svg")), ".svg should be recognised");
        assert!(path_is_svg(Path::new("ICON.SVG")), ".SVG (uppercase) should be recognised");
        assert!(path_is_svg(Path::new("path/to/image.Svg")), "mixed-case .Svg should be recognised");

        // Negative cases
        assert!(!path_is_svg(Path::new("photo.png")), ".png should not be recognised");
        assert!(!path_is_svg(Path::new("photo.jpg")), ".jpg should not be recognised");
        assert!(!path_is_svg(Path::new("photo.jpeg")), ".jpeg should not be recognised");
        assert!(!path_is_svg(Path::new("noextension")), "no extension should return false");
    }

    #[test]
    fn test_backdrop_next_cycle() {
        assert_eq!(Backdrop::Checkerboard.next(), Backdrop::Light);
        assert_eq!(Backdrop::Light.next(), Backdrop::Dark);
        assert_eq!(Backdrop::Dark.next(), Backdrop::Checkerboard);
    }

    fn factor(zoom: ZoomMode) -> f32 {
        match zoom {
            ZoomMode::Factor(f) => f,
            ZoomMode::Fit => panic!("expected a Factor, got Fit"),
        }
    }

    #[test]
    fn zoom_in_from_fit_starts_at_one_step() {
        // From Fit, the first zoom-in starts at the fit factor (1.0) and steps up.
        assert!((factor(zoom_in(ZoomMode::Fit)) - 1.25).abs() < 1e-6);
        // ...and the first zoom-out steps down from the same fit factor.
        assert!((factor(zoom_out(ZoomMode::Fit)) - 0.8).abs() < 1e-6);
    }

    #[test]
    fn zoom_in_then_out_returns_to_prior_factor() {
        let start = ZoomMode::Factor(2.0);
        let round_trip = zoom_out(zoom_in(start));
        assert!((factor(round_trip) - 2.0).abs() < 1e-6);
    }

    #[test]
    fn zoom_step_sequence_is_geometric() {
        let mut zoom = ZoomMode::Fit;
        zoom = zoom_in(zoom); // 1.25
        assert!((factor(zoom) - 1.25).abs() < 1e-6);
        zoom = zoom_in(zoom); // 1.5625
        assert!((factor(zoom) - 1.5625).abs() < 1e-6);
        zoom = zoom_in(zoom); // 1.953125
        assert!((factor(zoom) - 1.953_125).abs() < 1e-6);
    }

    #[test]
    fn zoom_clamps_to_bounds() {
        // Far past the max clamps to MAX_ZOOM.
        let mut zoom = ZoomMode::Factor(MAX_ZOOM);
        for _ in 0..5 {
            zoom = zoom_in(zoom);
        }
        assert!((factor(zoom) - MAX_ZOOM).abs() < 1e-6);

        // Far below the min clamps to MIN_ZOOM.
        let mut zoom = ZoomMode::Factor(MIN_ZOOM);
        for _ in 0..5 {
            zoom = zoom_out(zoom);
        }
        assert!((factor(zoom) - MIN_ZOOM).abs() < 1e-6);
    }

    #[test]
    fn clamp_pan_limits_to_overflow_bounds() {
        // Image larger than the viewport: 400x300 scaled inside a 100x100 view.
        let scaled = vec2f(400.0, 300.0);
        let viewport = vec2f(100.0, 100.0);
        // Max offset is (scaled - viewport) / 2 per axis: (150, 100).
        let clamped = clamp_pan(vec2f(1000.0, -1000.0), scaled, viewport);
        assert!((clamped.x() - 150.0).abs() < 1e-6);
        assert!((clamped.y() - (-100.0)).abs() < 1e-6);

        // A small offset within bounds is unchanged.
        let small = clamp_pan(vec2f(20.0, -30.0), scaled, viewport);
        assert!((small.x() - 20.0).abs() < 1e-6);
        assert!((small.y() - (-30.0)).abs() < 1e-6);
    }

    #[test]
    fn clamp_pan_disallows_panning_when_image_smaller_than_viewport() {
        // Image smaller than the viewport on both axes => no panning room.
        let scaled = vec2f(50.0, 40.0);
        let viewport = vec2f(100.0, 100.0);
        let clamped = clamp_pan(vec2f(25.0, -25.0), scaled, viewport);
        assert_eq!(clamped, vec2f(0.0, 0.0));
    }

    #[test]
    fn clamp_pan_is_per_axis() {
        // Wider than the viewport but shorter than it: pan allowed on x, not y.
        let scaled = vec2f(400.0, 50.0);
        let viewport = vec2f(100.0, 100.0);
        let clamped = clamp_pan(vec2f(1000.0, 1000.0), scaled, viewport);
        assert!((clamped.x() - 150.0).abs() < 1e-6);
        assert_eq!(clamped.y(), 0.0);
    }
}
