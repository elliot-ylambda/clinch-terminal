use std::collections::HashMap;

use cli_agent_usage::{Provider, UsageSnapshot};
use warp_core::ui::theme::{Fill, WarpTheme};
use warp_core::ui::Icon;
use warpui::Element;

mod cli_agent_usage_chip;
mod cli_agent_usage_header;
mod cli_agent_usage_model;
pub mod conversation_usage_view;
pub mod rollup;
pub mod tab_model_label;

pub use cli_agent_usage_chip::{render_cli_agent_usage_panel, CliAgentUsagePanelMouseStates};
pub use cli_agent_usage_header::render_cli_agent_usage_header;
pub use cli_agent_usage_model::CliAgentUsageModel;
pub use tab_model_label::cli_agent_model_label;

pub const CLAUDE_USAGE_URL: &str = "https://claude.ai/new#settings/usage";
pub const CODEX_USAGE_URL: &str = "https://chatgpt.com/#settings/Usage";

/// The provider whose focused usage dropdown is open in the tab bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliAgentUsageProvider {
    Claude,
    Codex,
}

/// UI state for Claude's opt-in plan-limit collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanLimitsState {
    pub enabled: bool,
    pub authorization_pending: bool,
}

/// A statistic that can be independently shown or hidden in the tab-bar usage
/// header. The focused provider panel always shows every statistic; its
/// checkboxes control only the compact, at-a-glance header presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliAgentUsageMetric {
    FiveHour,
    Weekly,
    ResetTimes,
    Fable,
    SessionTokens,
    TodayTokens,
    WeekTokens,
    MonthTokens,
}

impl CliAgentUsageMetric {
    pub const COUNT: usize = 8;
    pub const ALL: [Self; Self::COUNT] = [
        Self::FiveHour,
        Self::Weekly,
        Self::ResetTimes,
        Self::Fable,
        Self::SessionTokens,
        Self::TodayTokens,
        Self::WeekTokens,
        Self::MonthTokens,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::FiveHour => "5h",
            Self::Weekly => "Weekly",
            Self::ResetTimes => "Reset times",
            Self::Fable => "Fable wk",
            Self::SessionTokens => "Session",
            Self::TodayTokens => "Today",
            Self::WeekTokens => "This week",
            Self::MonthTokens => "This month",
        }
    }

    pub fn default_visible(self) -> bool {
        matches!(
            self,
            Self::FiveHour | Self::Weekly | Self::ResetTimes | Self::Fable
        )
    }

    pub fn config_key(self, provider: CliAgentUsageProvider) -> Option<&'static str> {
        use CliAgentUsageMetric::*;
        use CliAgentUsageProvider::*;

        match (provider, self) {
            (Claude, FiveHour) => Some("claude.5_hour"),
            (Claude, Weekly) => Some("claude.weekly"),
            (Claude, ResetTimes) => Some("claude.reset_times"),
            (Claude, Fable) => Some("claude.fable"),
            (Claude, SessionTokens) => Some("claude.tokens.session"),
            (Claude, TodayTokens) => Some("claude.tokens.today"),
            (Claude, WeekTokens) => Some("claude.tokens.week"),
            (Claude, MonthTokens) => Some("claude.tokens.month"),
            (Codex, FiveHour) => Some("codex.5_hour"),
            (Codex, Weekly) => Some("codex.weekly"),
            (Codex, ResetTimes) => Some("codex.reset_times"),
            (Codex, Fable) => None,
            (Codex, SessionTokens) => Some("codex.tokens.session"),
            (Codex, TodayTokens) => Some("codex.tokens.today"),
            (Codex, WeekTokens) => Some("codex.tokens.week"),
            (Codex, MonthTokens) => Some("codex.tokens.month"),
        }
    }

    /// Toggle one sparse stored override and return the resolved new value.
    /// Returning to the metric's default removes the key entirely.
    pub fn toggle_override(
        self,
        provider: CliAgentUsageProvider,
        overrides: &mut HashMap<String, bool>,
    ) -> Option<bool> {
        let key = self.config_key(provider)?;
        let current = overrides
            .get(key)
            .copied()
            .unwrap_or_else(|| self.default_visible());
        let next = !current;
        if next == self.default_visible() {
            overrides.remove(key);
        } else {
            overrides.insert(key.to_string(), next);
        }
        Some(next)
    }

    fn index(self) -> usize {
        match self {
            Self::FiveHour => 0,
            Self::Weekly => 1,
            Self::ResetTimes => 2,
            Self::Fable => 3,
            Self::SessionTokens => 4,
            Self::TodayTokens => 5,
            Self::WeekTokens => 6,
            Self::MonthTokens => 7,
        }
    }
}

/// Resolved per-provider visibility values. Stored overrides are sparse so new
/// metrics can acquire sensible defaults without requiring a migration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliAgentUsageHeaderVisibility {
    claude: [bool; CliAgentUsageMetric::COUNT],
    codex: [bool; CliAgentUsageMetric::COUNT],
}

impl CliAgentUsageHeaderVisibility {
    pub fn from_overrides(overrides: &HashMap<String, bool>) -> Self {
        fn resolve(
            provider: CliAgentUsageProvider,
            overrides: &HashMap<String, bool>,
        ) -> [bool; CliAgentUsageMetric::COUNT] {
            std::array::from_fn(|index| {
                let metric = CliAgentUsageMetric::ALL[index];
                metric
                    .config_key(provider)
                    .map(|key| {
                        overrides
                            .get(key)
                            .copied()
                            .unwrap_or_else(|| metric.default_visible())
                    })
                    .unwrap_or(false)
            })
        }

        Self {
            claude: resolve(CliAgentUsageProvider::Claude, overrides),
            codex: resolve(CliAgentUsageProvider::Codex, overrides),
        }
    }

    pub fn is_visible(&self, provider: CliAgentUsageProvider, metric: CliAgentUsageMetric) -> bool {
        match provider {
            CliAgentUsageProvider::Claude => self.claude[metric.index()],
            CliAgentUsageProvider::Codex => self.codex[metric.index()],
        }
    }
}

impl Default for CliAgentUsageHeaderVisibility {
    fn default() -> Self {
        Self::from_overrides(&HashMap::new())
    }
}

impl CliAgentUsageProvider {
    pub fn data(self, snapshot: &UsageSnapshot) -> &Provider {
        match self {
            Self::Claude => &snapshot.claude,
            Self::Codex => &snapshot.codex,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
        }
    }

    pub fn usage_url(self) -> &'static str {
        match self {
            Self::Claude => CLAUDE_USAGE_URL,
            Self::Codex => CODEX_USAGE_URL,
        }
    }

    pub fn usage_link_label(self) -> &'static str {
        match self {
            Self::Claude => "View usage on claude.ai",
            Self::Codex => "View usage on chatgpt.com",
        }
    }

    pub fn toggle_panel(self, current: Option<Self>) -> Option<Self> {
        if current == Some(self) {
            None
        } else {
            Some(self)
        }
    }

    /// Which clickable affordance this provider's gauge area shows instead of
    /// limit windows. Claude's plan gauges come from the opt-in Keychain +
    /// usage-endpoint poller (`show_plan_limits`): while the setting is off
    /// they offer "Turn on" (otherwise permanent dashes read as "broken"),
    /// and while the poller reports that reading the Keychain would prompt
    /// they offer "Authorize" — so the macOS credential prompt only ever
    /// appears as the direct result of a click, never unbidden at launch.
    /// Codex limits come from local files and need neither.
    fn plan_limits_affordance(
        self,
        plan_limits: PlanLimitsState,
        provider: &Provider,
    ) -> Option<PlanLimitsAffordance> {
        if self != Self::Claude {
            return None;
        }
        if !plan_limits.enabled {
            return Some(PlanLimitsAffordance::TurnOn);
        }
        if plan_limits.authorization_pending {
            return Some(PlanLimitsAffordance::Authorizing);
        }
        provider
            .plan_needs_authorization
            .then_some(PlanLimitsAffordance::Authorize)
    }
}

/// See [`CliAgentUsageProvider::plan_limits_affordance`]. The clickable
/// variants dispatch the same `EnableCliAgentPlanLimits` gesture: ensure the
/// setting is on and sanction one Keychain read (with its prompt, if macOS
/// raises one).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanLimitsAffordance {
    TurnOn,
    Authorize,
    Authorizing,
}

impl PlanLimitsAffordance {
    pub fn label(self) -> &'static str {
        match self {
            Self::TurnOn => "Turn on",
            Self::Authorize => "Authorize",
            Self::Authorizing => "Authorizing…",
        }
    }

    pub fn is_clickable(self) -> bool {
        !matches!(self, Self::Authorizing)
    }
}

pub fn icon_for_context_window_usage(context_window_usage: f32) -> Icon {
    // Match the context window usage to the nearest 10% icon.
    if context_window_usage >= 0.95 {
        Icon::ConversationContext100
    } else if context_window_usage >= 0.85 {
        Icon::ConversationContext90
    } else if context_window_usage >= 0.75 {
        Icon::ConversationContext80
    } else if context_window_usage >= 0.65 {
        Icon::ConversationContext70
    } else if context_window_usage >= 0.55 {
        Icon::ConversationContext60
    } else if context_window_usage >= 0.45 {
        Icon::ConversationContext50
    } else if context_window_usage >= 0.35 {
        Icon::ConversationContext40
    } else if context_window_usage >= 0.25 {
        Icon::ConversationContext30
    } else if context_window_usage >= 0.15 {
        Icon::ConversationContext20
    } else if context_window_usage >= 0.05 {
        Icon::ConversationContext10
    } else {
        Icon::ConversationContext0
    }
}

pub fn render_context_window_usage_icon(
    context_window_usage: f32,
    theme: &WarpTheme,
    color_override: Option<Fill>,
) -> Box<dyn Element> {
    let icon = icon_for_context_window_usage(context_window_usage);

    let fill = if context_window_usage >= 0.8 {
        Fill::Solid(theme.ansi_fg_red())
    } else {
        color_override.unwrap_or_else(|| theme.main_text_color(theme.background()))
    };

    icon.to_warpui_icon(fill).finish()
}

#[cfg(test)]
mod cli_agent_usage_provider_tests {
    use std::collections::HashMap;

    use cli_agent_usage::Provider;

    use super::{
        CliAgentUsageHeaderVisibility, CliAgentUsageMetric, CliAgentUsageProvider,
        PlanLimitsAffordance, PlanLimitsState, CLAUDE_USAGE_URL, CODEX_USAGE_URL,
    };

    #[test]
    fn clicking_a_provider_toggles_only_its_panel() {
        let claude = CliAgentUsageProvider::Claude;
        let codex = CliAgentUsageProvider::Codex;

        assert_eq!(claude.toggle_panel(None), Some(claude));
        assert_eq!(claude.toggle_panel(Some(claude)), None);
        assert_eq!(codex.toggle_panel(Some(claude)), Some(codex));
    }

    #[test]
    fn plan_limits_affordance_is_claude_only_turn_on_then_authorize() {
        let claude = CliAgentUsageProvider::Claude;
        let codex = CliAgentUsageProvider::Codex;
        let idle = Provider::default();
        let needs_auth = Provider {
            plan_needs_authorization: true,
            ..Provider::default()
        };
        let disabled = PlanLimitsState {
            enabled: false,
            authorization_pending: false,
        };
        let enabled = PlanLimitsState {
            enabled: true,
            authorization_pending: false,
        };
        let authorizing = PlanLimitsState {
            enabled: true,
            authorization_pending: true,
        };

        // Setting off: Claude offers "Turn on" regardless of poller state.
        assert_eq!(
            claude.plan_limits_affordance(disabled, &idle),
            Some(PlanLimitsAffordance::TurnOn)
        );
        assert_eq!(
            claude.plan_limits_affordance(disabled, &needs_auth),
            Some(PlanLimitsAffordance::TurnOn)
        );
        // Setting on: Authorize appears only while the poller reports that
        // reading the Keychain would prompt.
        assert_eq!(claude.plan_limits_affordance(enabled, &idle), None);
        assert_eq!(
            claude.plan_limits_affordance(enabled, &needs_auth),
            Some(PlanLimitsAffordance::Authorize)
        );
        assert_eq!(
            claude.plan_limits_affordance(authorizing, &needs_auth),
            Some(PlanLimitsAffordance::Authorizing)
        );
        // Codex limits come from local files; no affordance ever.
        assert_eq!(codex.plan_limits_affordance(disabled, &idle), None);
        assert_eq!(codex.plan_limits_affordance(authorizing, &needs_auth), None);
    }

    #[test]
    fn affordance_labels_name_the_gesture() {
        assert_eq!(PlanLimitsAffordance::TurnOn.label(), "Turn on");
        assert_eq!(PlanLimitsAffordance::Authorize.label(), "Authorize");
        assert_eq!(PlanLimitsAffordance::Authorizing.label(), "Authorizing…");
        assert!(PlanLimitsAffordance::Authorize.is_clickable());
        assert!(!PlanLimitsAffordance::Authorizing.is_clickable());
    }

    #[test]
    fn providers_link_to_their_authoritative_usage_pages() {
        assert_eq!(CliAgentUsageProvider::Claude.usage_url(), CLAUDE_USAGE_URL);
        assert_eq!(CliAgentUsageProvider::Codex.usage_url(), CODEX_USAGE_URL);
        assert_eq!(CLAUDE_USAGE_URL, "https://claude.ai/new#settings/usage");
        assert_eq!(CODEX_USAGE_URL, "https://chatgpt.com/#settings/Usage");
    }

    #[test]
    fn header_visibility_preserves_existing_defaults_and_keeps_tokens_opt_in() {
        let visibility = CliAgentUsageHeaderVisibility::default();

        for provider in [CliAgentUsageProvider::Claude, CliAgentUsageProvider::Codex] {
            assert!(visibility.is_visible(provider, CliAgentUsageMetric::FiveHour));
            assert!(visibility.is_visible(provider, CliAgentUsageMetric::Weekly));
            assert!(visibility.is_visible(provider, CliAgentUsageMetric::ResetTimes));
            assert!(!visibility.is_visible(provider, CliAgentUsageMetric::SessionTokens));
            assert!(!visibility.is_visible(provider, CliAgentUsageMetric::TodayTokens));
            assert!(!visibility.is_visible(provider, CliAgentUsageMetric::WeekTokens));
            assert!(!visibility.is_visible(provider, CliAgentUsageMetric::MonthTokens));
        }
        assert!(visibility.is_visible(CliAgentUsageProvider::Claude, CliAgentUsageMetric::Fable));
        assert!(!visibility.is_visible(CliAgentUsageProvider::Codex, CliAgentUsageMetric::Fable));
    }

    #[test]
    fn header_visibility_overrides_are_provider_and_metric_specific() {
        let overrides = HashMap::from([
            ("claude.weekly".to_string(), false),
            ("codex.tokens.today".to_string(), true),
        ]);
        let visibility = CliAgentUsageHeaderVisibility::from_overrides(&overrides);

        assert!(!visibility.is_visible(CliAgentUsageProvider::Claude, CliAgentUsageMetric::Weekly));
        assert!(visibility.is_visible(CliAgentUsageProvider::Codex, CliAgentUsageMetric::Weekly));
        assert!(visibility.is_visible(
            CliAgentUsageProvider::Codex,
            CliAgentUsageMetric::TodayTokens
        ));
        assert!(!visibility.is_visible(
            CliAgentUsageProvider::Claude,
            CliAgentUsageMetric::TodayTokens
        ));
    }

    #[test]
    fn toggling_visibility_keeps_stored_overrides_sparse() {
        let mut overrides = HashMap::new();
        let metric = CliAgentUsageMetric::TodayTokens;

        assert_eq!(
            metric.toggle_override(CliAgentUsageProvider::Codex, &mut overrides),
            Some(true)
        );
        assert_eq!(overrides.get("codex.tokens.today"), Some(&true));

        assert_eq!(
            metric.toggle_override(CliAgentUsageProvider::Codex, &mut overrides),
            Some(false)
        );
        assert!(overrides.is_empty());
        assert_eq!(
            CliAgentUsageMetric::Fable
                .toggle_override(CliAgentUsageProvider::Codex, &mut overrides),
            None
        );
    }
}
