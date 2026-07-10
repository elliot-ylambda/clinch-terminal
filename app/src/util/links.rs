use crate::channel::ChannelState;

/// Upstream Warp's docs still accurately describe the terminal features
/// Clinch inherits; linked nominatively ("Warp Documentation (upstream)").
pub const USER_DOCS_URL: &str = "https://docs.warp.dev/";
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const GITHUB_ISSUES_URL: &str = "https://github.com/elliot-ylambda/clinch-terminal/issues";
pub const COMMUNITY_URL: &str = "https://github.com/elliot-ylambda/clinch-terminal";
pub const PRIVACY_POLICY_URL: &str =
    "https://github.com/elliot-ylambda/clinch-terminal#privacy--telemetry";

pub fn feedback_form_url() -> String {
    let mut url =
        url::Url::parse("https://github.com/elliot-ylambda/clinch-terminal/issues/new/choose")
            .expect("Should not fail to parse");
    if let Some(version) = ChannelState::app_version() {
        url.query_pairs_mut().append_pair("clinch-version", version);
    }
    url.query_pairs_mut()
        .append_pair("os-version", &os_info::get().version().to_string());
    url.to_string()
}
