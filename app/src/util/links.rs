use crate::channel::ChannelState;

pub const USER_DOCS_URL: &str = "https://github.com/elliot-ylambda/clinch-terminal#readme";
pub const RELEASES_URL: &str = "https://github.com/elliot-ylambda/clinch-terminal/releases";

/// Upstream Warp's docs still describe terminal features that Clinch inherits.
/// Keep this separate from Clinch's user documentation so explicitly upstream
/// links cannot accidentally replace the product's own docs.
pub const UPSTREAM_DOCS_URL: &str = "https://docs.warp.dev/";
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
