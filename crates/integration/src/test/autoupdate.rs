use warp::integration_testing::autoupdate::{
    clinch_update_is_available, set_clinch_update_available,
};
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::integration_testing::view_getters::workspace_view;
use warp::integration_testing::workspace::press_native_modal_button;
use warp::workspace::WorkspaceAction;
use warpui_core::integration::TestStep;
use warpui_core::{async_assert, TypedActionView as _};

use super::new_builder;
use crate::Builder;

pub fn test_clinch_update_requires_explicit_consent() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Open the authenticated Clinch update consent dialog")
                .with_action(|app, window_id, _| {
                    app.update(set_clinch_update_available);
                    let workspace = workspace_view(app, window_id);
                    workspace.update(app, |workspace, ctx| {
                        workspace.handle_action(&WorkspaceAction::ApplyUpdate, ctx);
                    });
                })
                .add_assertion(|app, window_id| {
                    let workspace = workspace_view(app, window_id);
                    workspace.read(app, |workspace, ctx| {
                        async_assert!(
                            workspace.is_native_quit_modal_open(ctx),
                            "Clinch update consent dialog is not open"
                        )
                    })
                }),
        )
        .with_step(press_native_modal_button(1))
        .with_step(
            TestStep::new("Declining leaves the authenticated update available").add_assertion(
                |app, _| {
                    app.read(|ctx| {
                        async_assert!(
                            clinch_update_is_available(ctx),
                            "Later unexpectedly downloaded or dismissed the update"
                        )
                    })
                },
            ),
        )
}
