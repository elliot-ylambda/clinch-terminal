use std::collections::HashMap;

use settings::macros::define_settings_group;
use settings::{RespectUserSyncSetting, SupportedPlatforms, SyncToCloud};

define_settings_group!(CliAgentUsageSettings, settings: [
    // Gates the Claude Code live plan-limit gauges (the 5-hour and weekly
    // rate-limit % in the tab-bar usage widget). Off by default: populating
    // them requires reading Claude Code's OAuth token from the macOS Keychain
    // (a password prompt) and querying Anthropic's usage endpoint — both are
    // opt-in so a fresh install never touches the Keychain or the network.
    //
    // `SyncToCloud::Never`: the Keychain is inherently per-machine, so this is a
    // machine-local preference and must not sync across devices.
    show_plan_limits: ShowCliAgentPlanLimits {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::MAC,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "ai.cli_agent_usage.show_plan_limits",
        description: "Show Claude Code's live plan-limit gauges in the usage \
                      widget. When enabled, reads the 'Claude Code-credentials' \
                      item from your macOS Keychain (may ask for your keychain \
                      password when you turn it on) and queries Anthropic's \
                      usage endpoint. Off by default; local token and cost \
                      stats work without it.",
    }

    // Sparse overrides for the per-provider statistics rendered in the tab-bar
    // usage header. Missing plan-limit keys default to visible, while missing
    // token-window keys default to hidden; see `CliAgentUsageMetric`.
    header_metric_visibility: CliAgentUsageHeaderMetricVisibility {
        type: HashMap<String, bool>,
        default: HashMap::default(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "ai.cli_agent_usage.header_metric_visibility",
        description: "Per-provider visibility overrides for statistics in the CLI-agent usage header.",
    }
]);

#[cfg(test)]
#[path = "cli_agent_usage_tests.rs"]
mod tests;
