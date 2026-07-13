use std::collections::HashMap;
use std::ffi::OsString;

use crate::channel::ChannelState;

pub(crate) const FOCUS_URL_ENV: &str = "WARP_FOCUS_URL";
pub(crate) const TERMINAL_SESSION_UUID_ENV: &str = "WARP_TERMINAL_SESSION_UUID";
pub(crate) const AGENT_RESUME_DIR_ENV: &str = "WARP_AGENT_RESUME_DIR";

pub(crate) fn session_focus_url(session_uuid_hex: &str) -> String {
    format!(
        "{}://session/{session_uuid_hex}",
        ChannelState::url_scheme()
    )
}

pub(crate) fn add_session_focus_env_vars(
    env_vars: &mut HashMap<OsString, OsString>,
    session_uuid: &[u8],
) {
    let session_uuid_hex = hex::encode(session_uuid);
    env_vars.insert(
        OsString::from(TERMINAL_SESSION_UUID_ENV),
        OsString::from(session_uuid_hex.clone()),
    );
    env_vars.insert(
        OsString::from(FOCUS_URL_ENV),
        OsString::from(session_focus_url(&session_uuid_hex)),
    );
    let agent_resume_dir = std::env::var_os(AGENT_RESUME_DIR_ENV)
        .map(std::path::PathBuf::from)
        .or_else(|| warp_core::paths::warp_home_config_dir().map(|dir| dir.join("agent-resume")));
    if let Some(agent_resume_dir) = agent_resume_dir {
        env_vars.insert(
            OsString::from(AGENT_RESUME_DIR_ENV),
            agent_resume_dir.into_os_string(),
        );
    }
}

#[cfg(test)]
#[path = "focus_env_tests.rs"]
mod tests;
