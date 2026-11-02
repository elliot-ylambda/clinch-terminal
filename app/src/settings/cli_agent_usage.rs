use std::collections::HashMap;

use settings::macros::define_settings_group;
use settings::{RespectUserSyncSetting, SupportedPlatforms, SyncToCloud};

define_settings_group!(CliAgentUsageSettings, settings: [
    // Gates the Claude Code live plan-limit gauges (the 5-hour and weekly
    // rate-limit % in the tab-bar usage widget). Off by default: populating
    // them requires reading Claude Code's OAuth token from the macOS Keychain
    // without prompting and querying Anthropic's usage endpoint — both are
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
                      item from your macOS Keychain only when access is already \
                      available, without asking for your computer password, and \
                      queries Anthropic's usage endpoint. Off by default; local \
                      token and cost stats work without it.",
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

    // When a Claude Code or Codex session stops at its usage limit, send
    // "Continue" once the limit resets. On by default: the send is causal
    // (only a provider-reported limit stop arms it), one-shot, and cancelled by
    // any typing in the pane. Each pane's footer toggle overrides this for its
    // own provider session (see `auto_continue_session_overrides`).
    auto_continue_on_limit_reset: CliAgentAutoContinueOnLimitReset {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "ai.cli_agent_usage.auto_continue_on_limit_reset",
        description: "Automatically send \"Continue\" to a Claude Code or Codex session \
                      when its usage limit resets.",
    }

    // Per-session exceptions to `auto_continue_on_limit_reset`, set only by a
    // pane's footer toggle. Keyed by durable provider session ("claude:<id>",
    // "codex:<id>") rather than entity ID, so a restored pane keeps the choice.
    // This replaces the old opt-in-only `auto_continue_sessions` map; its
    // entries are not migrated because every one of them asked for the value
    // that is now the default.
    auto_continue_session_overrides: CliAgentAutoContinueSessionOverrides {
        type: HashMap<String, bool>,
        default: HashMap::default(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
        toml_path: "ai.cli_agent_usage.auto_continue_session_overrides",
        description: "Machine-local per-session overrides of rate-limit auto-continue.",
    }

    // Scheduled fire timestamps for causally confirmed limit stops. Kept
    // separate from the opt-in map so a process restart can reconstruct the
    // timer without treating every enabled session as limit-stopped.
    auto_continue_armed_sessions: CliAgentAutoContinueArmedSessions {
        type: HashMap<String, i64>,
        default: HashMap::default(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
        toml_path: "ai.cli_agent_usage.auto_continue_armed_sessions",
        description: "Machine-local scheduled reset times for rate-limit auto-continue.",
    }
]);

#[cfg(test)]
#[path = "cli_agent_usage_tests.rs"]
mod tests;
