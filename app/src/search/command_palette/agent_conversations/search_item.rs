use chrono::{DateTime, Utc};
use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use warpui::elements::{Container, Expanded, Flex, MainAxisSize, Text};
use warpui::fonts::{Properties, Weight};
use warpui::{AppContext, Element, SingletonEntity};

use crate::agent_resume::AgentConversation;
use crate::appearance::Appearance;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_palette::render_util::render_search_item_icon;
use crate::search::item::IconLocation;
use crate::search::result_renderer::ItemHighlightState;
use crate::search::SearchItem;
use crate::ui_components::icons::Icon;
use crate::util::time_format::format_approx_duration_from_now_utc;

/// Search item for one reopenable CLI-agent conversation: first prompt as the title,
/// directory + bridged/local as the subtitle, relative start time on the right.
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

    /// The first mirrored prompt; sessions without a mirror (e.g. codex) fall back to
    /// an agent + short-id label so the row is still recognizable.
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
                format!("{} session {short_id}", self.conversation.agent)
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
        match self.conversation.cwd.as_deref() {
            Some(cwd) => format!("{cwd} · {location}"),
            None => location.to_string(),
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
        render_search_item_icon(
            appearance,
            Icon::Conversation,
            appearance.theme().foreground().into_solid(),
            highlight_state,
        )
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
        .finish();

        let subtitle_element = Text::new_inline(
            self.subtitle(),
            appearance.ui_font_family(),
            sub_text_font_size,
        )
        .with_color(highlight_state.sub_text_fill(appearance).into_solid())
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
        format!("Agent conversation: {}", self.title())
    }

    fn accessibility_help_message(&self) -> Option<String> {
        Some("Press enter to reopen this conversation in a new tab.".into())
    }
}
