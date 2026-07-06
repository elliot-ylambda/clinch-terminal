use settings::macros::define_settings_group;
use settings::{SupportedPlatforms, SyncToCloud};

define_settings_group!(CliAgentUsageSettings, settings: [
    // Gates the Claude Code live plan-limit gauges (the 5-hour and weekly
    // rate-limit % in the tab-bar usage widget). Populating them requires
    // reading Claude Code's OAuth token from the macOS Keychain, which prompts
    // the user for their password. Turning this off stops that read — and the
    // prompt — while leaving the local token/cost stats (scanned from
    // `~/.claude` files) untouched.
    //
    // `SyncToCloud::Never`: the Keychain is inherently per-machine, so this is a
    // machine-local preference and must not sync across devices.
    show_plan_limits: ShowCliAgentPlanLimits {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::MAC,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "ai.cli_agent_usage.show_plan_limits",
        description: "Show Claude Code's live plan-limit gauges in the usage \
                      widget. Reads the 'Claude Code-credentials' item from your \
                      macOS Keychain, which prompts for your password once per \
                      launch. Turn off to stop the prompt (local token and cost \
                      stats are unaffected).",
    }
]);
