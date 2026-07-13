use chrono::{DateTime, Utc};
use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use warpui::elements::{Container, Expanded, Flex, MainAxisSize, ParentElement, Text};
use warpui::fonts::{Properties, Weight};
use warpui::text_layout::ClipConfig;
use warpui::{AppContext, Element, SingletonEntity};

use super::{cli_agent_for_resume_provider, provider_display_name};
use crate::agent_resume::AgentConversation;
use crate::appearance::Appearance;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_palette::render_util::render_search_item_icon;
use crate::search::item::IconLocation;
use crate::search::result_renderer::ItemHighlightState;
use crate::search::SearchItem;
use crate::ui_components::icons::Icon;
use crate::util::time_format::format_approx_duration_from_now_utc;

/// Search item for one reopenable CLI-agent conversation: opening user prompt as the
/// title, directory + bridged/local as the subtitle, relative start time on the right.
#[derive(Debug)]
pub struct AgentConversationSearchItem {
    conversation: AgentConversation,
    /// Precomputed [`AgentConversation::reopen_command`] (items are only built for
    /// conversations that have one, so accepting an item can never fail).
    command: String,
    match_result: FuzzyMatchResult,
    started_at: Option<DateTime<Utc>>,
}

impl AgentConversationSearchItem {
    pub fn new(
        conversation: AgentConversation,
        command: String,
        match_result: FuzzyMatchResult,
    ) -> Self {
        let started_at = DateTime::parse_from_rfc3339(&conversation.start_ts)
            .ok()
            .map(|started_at| started_at.with_timezone(&Utc));
        Self {
            conversation,
            command,
            match_result,
            started_at,
        }
    }

    /// The first user prompt recovered from the capture mirror or native transcript.
    /// Sessions without either fall back to an agent + short-id label so the row remains
    /// recognizable.
    fn title(&self) -> String {
        match self.conversation.first_prompt.as_deref() {
            Some(prompt) => prompt.to_string(),
            None => {
                let short_id = self
                    .conversation
                    .session_id
                    .chars()
                    .take(8)
                    .collect::<String>();
                format!(
                    "{} session {short_id}",
                    provider_display_name(&self.conversation.agent)
                )
            }
        }
    }

    fn subtitle(&self) -> String {
        // "bridged" = the conversation's authoritative copy lives at claude.ai and
        // reopening teleports it; "local" = plain `--resume` of the local transcript.
        let location = if self.conversation.bridge.is_some() {
            "bridged"
        } else {
            "local"
        };
        let provider = provider_display_name(&self.conversation.agent);
        match self.conversation.cwd.as_deref() {
            Some(cwd) => format!("{cwd} · {provider} · {location}"),
            None => format!("{provider} · {location}"),
        }
    }
}

impl SearchItem for AgentConversationSearchItem {
    type Action = CommandPaletteItemAction;

    fn is_multiline(&self) -> bool {
        true
    }

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let agent = cli_agent_for_resume_provider(&self.conversation.agent);
        let icon = agent
            .and_then(|agent| agent.icon())
            .unwrap_or(Icon::Conversation);
        let color = agent
            .and_then(|agent| agent.brand_color())
            .unwrap_or_else(|| appearance.theme().foreground().into_solid());
        render_search_item_icon(appearance, icon, color, highlight_state)
    }

    fn icon_location(&self, appearance: &Appearance) -> IconLocation {
        // Align the icon with the first text line (same offset as the conversations
        // picker's multi-line rows).
        let margin_top = (appearance.line_height_ratio() * appearance.monospace_font_size())
            - appearance.monospace_font_size();
        IconLocation::Top { margin_top }
    }

    fn render_item(
        &self,
        highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let sub_text_font_size = appearance.monospace_font_size() - 2.;

        let title_element = Text::new_inline(
            self.title(),
            appearance.ui_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(highlight_state.sub_text_fill(appearance).into_solid())
        .with_style(Properties::default().weight(Weight::Bold))
        .with_clip(ClipConfig::ellipsis())
        .finish();

        let subtitle_element = Text::new_inline(
            self.subtitle(),
            appearance.ui_font_family(),
            sub_text_font_size,
        )
        .with_color(highlight_state.sub_text_fill(appearance).into_solid())
        .with_clip(ClipConfig::ellipsis())
        .finish();

        let left_container = Flex::column()
            .with_spacing(4.)
            .with_child(title_element)
            .with_child(subtitle_element)
            .finish();

        let started = self
            .started_at
            .map(format_approx_duration_from_now_utc)
            .unwrap_or_default();
        let started_element = Container::new(
            Text::new_inline(started, appearance.ui_font_family(), sub_text_font_size)
                .with_color(highlight_state.sub_text_fill(appearance).into_solid())
                .finish(),
        )
        .with_padding_left(8.)
        .finish();

        Flex::row()
            .with_child(Expanded::new(1.0, left_container).finish())
            .with_child(started_element)
            .with_main_axis_size(MainAxisSize::Max)
            .finish()
    }

    fn score(&self) -> OrderedFloat<f64> {
        OrderedFloat::from(self.match_result.score as f64)
    }

    fn accept_result(&self) -> Self::Action {
        CommandPaletteItemAction::ReopenAgentConversation {
            command: self.command.clone(),
            cwd: self.conversation.cwd.clone(),
        }
    }

    fn execute_result(&self) -> Self::Action {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        format!(
            "{} conversation: {}",
            provider_display_name(&self.conversation.agent),
            self.title()
        )
    }

    fn accessibility_help_message(&self) -> Option<String> {
        Some("Press enter to reopen this conversation in a new tab.".into())
    }
}

#[cfg(test)]
#[path = "search_item_tests.rs"]
mod tests;
