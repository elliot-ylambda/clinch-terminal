use warpui::AppContext;

pub fn set_clinch_update_available(ctx: &mut AppContext) {
    crate::autoupdate::set_clinch_update_available_for_integration(ctx);
}

pub fn clinch_update_is_available(ctx: &AppContext) -> bool {
    crate::autoupdate::clinch_update_is_available_for_integration(ctx)
}
