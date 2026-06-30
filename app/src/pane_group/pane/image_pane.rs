use std::sync::Arc;

use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::{AppContext, ModelHandle, View, ViewContext, ViewHandle};

use super::view::PaneView;
use super::{
    DetachType, PaneConfiguration, PaneContent, PaneGroup, PaneId, ShareableLink,
    ShareableLinkError,
};
use crate::app_state::{ImagePaneSnapshot, LeafContents};
use crate::image_viewer::{ImageView, ImageViewEvent};
use crate::terminal::model::session::Session;

pub struct ImagePane {
    view: ViewHandle<PaneView<ImageView>>,
    pane_configuration: ModelHandle<PaneConfiguration>,
}

impl ImagePane {
    fn from_view(image_view: ViewHandle<ImageView>, ctx: &mut AppContext) -> Self {
        let pane_configuration = image_view.as_ref(ctx).pane_configuration();

        let view = ctx.add_typed_action_view(image_view.window_id(ctx), |ctx| {
            let pane_id = PaneId::from_image_pane_ctx(ctx);
            PaneView::new(pane_id, image_view, (), pane_configuration.clone(), ctx)
        });

        Self {
            view,
            pane_configuration,
        }
    }

    /// Create a new image pane for the given path and optional target session.
    ///
    /// If `path` is `Some(LocalOrRemotePath::Local(p))`, the image is opened
    /// immediately. `None` or remote paths create an empty pane (v1 is
    /// local-only).
    pub fn new<V: View>(
        path: Option<LocalOrRemotePath>,
        target_session: Option<Arc<Session>>,
        ctx: &mut ViewContext<V>,
    ) -> Self {
        let view = ctx.add_typed_action_view(move |ctx| {
            let mut view = ImageView::new(ctx);

            if let Some(LocalOrRemotePath::Local(p)) = path {
                view.open_local(p, target_session, ctx);
            }

            view
        });
        Self::from_view(view, ctx)
    }

    pub fn image_view(&self, ctx: &AppContext) -> ViewHandle<ImageView> {
        self.view.as_ref(ctx).child(ctx)
    }
}

impl PaneContent for ImagePane {
    fn id(&self) -> PaneId {
        PaneId::from_image_pane_view(&self.view)
    }

    fn attach(
        &self,
        _group: &PaneGroup,
        focus_handle: crate::pane_group::focus_state::PaneFocusHandle,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        self.view
            .update(ctx, |view, ctx| view.set_focus_handle(focus_handle, ctx));

        let pane_id = self.id();

        ctx.subscribe_to_view(
            &self.image_view(ctx),
            move |pane_group, _, event, ctx| match event {
                ImageViewEvent::TitleUpdated => {
                    ctx.emit(crate::pane_group::Event::PaneTitleUpdated)
                }
                ImageViewEvent::FileLoaded => {
                    ctx.emit(crate::pane_group::Event::AppStateChanged)
                }
                ImageViewEvent::Pane(pane_event) => {
                    pane_group.handle_pane_event(pane_id, pane_event, ctx)
                }
            },
        );

        ctx.subscribe_to_view(&self.view, move |group, _, event, ctx| {
            group.handle_pane_view_event(pane_id, event, ctx);
        });
    }

    fn detach(
        &self,
        _group: &PaneGroup,
        detach_type: DetachType,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        let image_view = self.image_view(ctx);
        // Stop the file watcher when the pane is closed/hidden. On a move the
        // same view survives and `attach` re-adds subscriptions, so tearing down
        // the watch would kill live-reload after a drag.
        #[cfg(feature = "local_fs")]
        if !matches!(detach_type, DetachType::Moved) {
            image_view.update(ctx, |v, ctx| v.unsubscribe_file_watch(ctx));
        }
        ctx.unsubscribe_to_view(&image_view);
        ctx.unsubscribe_to_view(&self.view);
    }

    fn snapshot(&self, app: &AppContext) -> LeafContents {
        let path = self.image_view(app).as_ref(app).local_path();
        LeafContents::ImageViewer(ImagePaneSnapshot { path })
    }

    fn has_application_focus(&self, ctx: &mut ViewContext<PaneGroup>) -> bool {
        self.view.is_self_or_child_focused(ctx)
    }

    fn focus(&self, ctx: &mut ViewContext<PaneGroup>) {
        self.image_view(ctx)
            .update(ctx, |view, ctx| view.focus(ctx));
    }

    fn shareable_link(
        &self,
        _ctx: &mut ViewContext<PaneGroup>,
    ) -> Result<ShareableLink, ShareableLinkError> {
        Ok(ShareableLink::Base)
    }

    fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    fn is_pane_being_dragged(&self, ctx: &AppContext) -> bool {
        self.view.as_ref(ctx).is_being_dragged()
    }
}
