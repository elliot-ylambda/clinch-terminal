use warp::integration_testing::autoupdate::{
    clinch_update_is_available, set_clinch_update_available,
};
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::integration_testing::view_getters::workspace_view;
use warp::workspace::WorkspaceAction;
use warpui_core::integration::TestStep;
use warpui_core::{async_assert, TypedActionView as _};

use super::new_builder;
use crate::Builder;

pub fn test_clinch_update_requires_explicit_consent() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Open the update pop-up for an authenticated Clinch update")
                .with_action(|app, window_id, _| {
                    app.update(set_clinch_update_available);
                    let workspace = workspace_view(app, window_id);
                    workspace.update(app, |workspace, ctx| {
                        workspace.handle_action(&WorkspaceAction::ApplyUpdate, ctx);
                    });
                })
                .add_assertion(|app, window_id| {
                    let workspace = workspace_view(app, window_id);
                    workspace.read(app, |workspace, _| {
                        async_assert!(
                            workspace.is_clinch_update_popup_open(),
                            "The Clinch update pop-up is not open"
                        )
                    })
                }),
        )
        .with_step(
            TestStep::new("Later closes the pop-up and leaves the update available")
                .with_action(|app, window_id, _| {
                    let workspace = workspace_view(app, window_id);
                    workspace.update(app, |workspace, ctx| {
                        workspace.handle_action(&WorkspaceAction::CloseClinchUpdatePopup, ctx);
                    });
                })
                .add_assertion(|app, window_id| {
                    let workspace = workspace_view(app, window_id);
                    let popup_open =
                        workspace.read(app, |workspace, _| workspace.is_clinch_update_popup_open());
                    app.read(|ctx| {
                        async_assert!(
                            !popup_open && clinch_update_is_available(ctx),
                            "Later unexpectedly downloaded or dismissed the update"
                        )
                    })
                }),
        )
}
