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

pub use cli_agent_usage_chip::render_cli_agent_usage_panel;
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
    use super::{CliAgentUsageProvider, CLAUDE_USAGE_URL, CODEX_USAGE_URL};

    #[test]
    fn clicking_a_provider_toggles_only_its_panel() {
        let claude = CliAgentUsageProvider::Claude;
        let codex = CliAgentUsageProvider::Codex;

        assert_eq!(claude.toggle_panel(None), Some(claude));
        assert_eq!(claude.toggle_panel(Some(claude)), None);
        assert_eq!(codex.toggle_panel(Some(claude)), Some(codex));
    }

    #[test]
    fn providers_link_to_their_authoritative_usage_pages() {
        assert_eq!(CliAgentUsageProvider::Claude.usage_url(), CLAUDE_USAGE_URL);
        assert_eq!(CliAgentUsageProvider::Codex.usage_url(), CODEX_USAGE_URL);
        assert_eq!(CLAUDE_USAGE_URL, "https://claude.ai/new#settings/usage");
        assert_eq!(CODEX_USAGE_URL, "https://chatgpt.com/#settings/Usage");
    }
}
