use fuzzy_match::{match_indices_case_insensitive, FuzzyMatchResult};
use warpui::{AppContext, Entity};

use crate::agent_resume::{self, AgentConversation};
use crate::search::command_palette::agent_conversations::search_item::AgentConversationSearchItem;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_palette::separator_search_item::SeparatorSearchItem;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::DataSourceRunErrorWrapper;
use crate::search::SyncDataSource;

/// Cap on how many recent conversations the picker offers: the journal is append-only
/// and unpruned, so the full history would grow without bound.
const MAX_RECENT_CONVERSATIONS: usize = 50;

/// Data source for the "Reopen agent conversation" picker: recent CLI-agent
/// (Claude/Codex) conversations read from the agent-resume journal + prompt mirror.
#[derive(Default)]
pub struct DataSource {
    conversations: Vec<AgentConversation>,
}

impl DataSource {
    /// Re-reads the on-disk journal/mirror. Called whenever the palette rebuilds its
    /// mixer (i.e. on every palette open), so each open sees fresh conversations while
    /// individual keystrokes only filter in memory. Conversations we don't know how to
    /// reopen (unknown agent) are dropped here so every listed item is acceptable.
    pub fn refresh(&mut self) {
        self.conversations = agent_resume::recent_conversations(MAX_RECENT_CONVERSATIONS)
            .into_iter()
            .filter(|conversation| conversation.reopen_command().is_some())
            .collect();
    }
}

impl SyncDataSource for DataSource {
    type Action = CommandPaletteItemAction;

    fn run_query(
        &self,
        query: &Query,
        _app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        let needle = query.text.trim().to_lowercase();

        // Items are pushed oldest-first: the mixer sorts ascending by score and the
        // palette renders the result list reversed, so among equal scores the last
        // pushed item displays at the top — pushing oldest-first shows newest-first.
        let mut results: Vec<QueryResult<Self::Action>> = Vec::new();
        for conversation in self.conversations.iter().rev() {
            let Some(command) = conversation.reopen_command() else {
                continue;
            };
            let match_result = if needle.is_empty() {
                FuzzyMatchResult::no_match()
            } else {
                let haystack = format!(
                    "{} {} {}",
                    conversation.first_prompt.as_deref().unwrap_or(""),
                    conversation.cwd.as_deref().unwrap_or(""),
                    conversation.session_id
                );
                match match_indices_case_insensitive(&haystack, &needle) {
                    Some(match_result) => match_result,
                    None => continue,
                }
            };
            results.push(QueryResult::from(AgentConversationSearchItem::new(
                conversation.clone(),
                command,
                match_result,
            )));
        }

        // Friendly empty state (a non-interactible separator row) when nothing has ever
        // been recorded; a non-matching query keeps the palette's own no-results state.
        if needle.is_empty() && results.is_empty() {
            results.push(QueryResult::from(SeparatorSearchItem::new(
                "No recent agent conversations — run claude or codex in a pane to record one"
                    .to_string(),
            )));
        }

        Ok(results)
    }
}

impl Entity for DataSource {
    type Event = ();
}
