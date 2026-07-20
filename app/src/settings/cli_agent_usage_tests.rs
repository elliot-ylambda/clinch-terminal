use settings::Setting as _;

use super::{
    CliAgentAutoContinueArmedSessions, CliAgentAutoContinueSessions,
    CliAgentUsageHeaderMetricVisibility, ShowCliAgentPlanLimits,
};

/// The plan-limit gauges read Claude Code's OAuth token from the macOS
/// Keychain and query Anthropic's usage endpoint, so they must stay opt-in:
/// on a fresh install the usage widget must never touch the Keychain or the
/// network (the producer loop in `CliAgentUsageModel` checks this setting
/// before either). If you intentionally change the default, also update the
/// privacy claims in README.md.
#[test]
fn show_plan_limits_defaults_to_off() {
    assert!(!ShowCliAgentPlanLimits::default_value());
}

#[test]
fn header_metric_visibility_defaults_to_sparse_overrides() {
    assert!(CliAgentUsageHeaderMetricVisibility::default_value().is_empty());
}

#[test]
fn auto_continue_sessions_default_to_no_opt_ins() {
    assert!(CliAgentAutoContinueSessions::default_value().is_empty());
    assert!(CliAgentAutoContinueArmedSessions::default_value().is_empty());
}
