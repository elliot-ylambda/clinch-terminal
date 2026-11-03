use warpui::AppContext;

pub use crate::autoupdate::{AutoupdateStage, ClinchDownloadProgress, ClinchUpdateFailure};

pub fn set_clinch_update_available(ctx: &mut AppContext) {
    crate::autoupdate::set_clinch_update_available_for_integration(ctx);
}

pub fn set_clinch_update_step(
    stage: AutoupdateStage,
    progress: Option<ClinchDownloadProgress>,
    failure: Option<ClinchUpdateFailure>,
    ctx: &mut AppContext,
) {
    crate::autoupdate::set_clinch_update_step_for_integration(stage, progress, failure, ctx);
}

pub fn clinch_update_is_available(ctx: &AppContext) -> bool {
    crate::autoupdate::clinch_update_is_available_for_integration(ctx)
}
