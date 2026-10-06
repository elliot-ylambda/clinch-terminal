//! Remote Control hooks for automated end-to-end testing.
use ::local_control::{ControlError, ErrorCode};
use serde_json::json;
use warpui::{ModelContext, SingletonEntity};

use crate::local_control::LocalControlBridge;
use crate::remote_control::RemoteControlService;

/// Mints a single-use pairing link that is approved without a click in Settings (Clinch Dev
/// only), so a test browser can pair itself.
pub(crate) fn test_pair(
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let invitation = RemoteControlService::handle(ctx)
        .update(ctx, |service, ctx| service.start_test_pairing(ctx))
        .map_err(|message| ControlError::new(ErrorCode::TargetStateConflict, message))?;
    Ok(json!({
        "pairing_url": invitation.pairing_url,
        "expires_at": invitation.expires_at,
    }))
}
