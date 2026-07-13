use std::collections::HashMap;
use std::ffi::OsString;

use super::{
    add_session_focus_env_vars, AGENT_RESUME_DIR_ENV, FOCUS_URL_ENV, TERMINAL_SESSION_UUID_ENV,
};
use crate::channel::ChannelState;

#[test]
fn focus_env_vars_point_at_session_deeplink() {
    let uuid = [
        0x55, 0x0e, 0x84, 0x00, 0xe2, 0x9b, 0x41, 0xd4, 0xa7, 0x16, 0x44, 0x66, 0x55, 0x44, 0x00,
        0x00,
    ];
    let mut env_vars = HashMap::new();

    add_session_focus_env_vars(&mut env_vars, &uuid);

    let expected_hex = "550e8400e29b41d4a716446655440000";
    assert_eq!(
        env_vars.get(&OsString::from(TERMINAL_SESSION_UUID_ENV)),
        Some(&OsString::from(expected_hex))
    );
    assert_eq!(
        env_vars.get(&OsString::from(FOCUS_URL_ENV)),
        Some(&OsString::from(format!(
            "{}://session/{expected_hex}",
            ChannelState::url_scheme()
        )))
    );
    let expected_agent_resume_dir = std::env::var_os(AGENT_RESUME_DIR_ENV).or_else(|| {
        warp_core::paths::warp_home_config_dir()
            .map(|dir| dir.join("agent-resume").into_os_string())
    });
    assert_eq!(
        env_vars.get(&OsString::from(AGENT_RESUME_DIR_ENV)),
        expected_agent_resume_dir.as_ref()
    );
}
