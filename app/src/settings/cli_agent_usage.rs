use settings::macros::define_settings_group;
use settings::{SupportedPlatforms, SyncToCloud};

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
                      item from your macOS Keychain (asks for your password) and \
                      queries Anthropic's usage endpoint. Off by default; local \
                      token and cost stats work without it.",
    }
]);
