use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use fuzzy_match::{match_indices_case_insensitive, FuzzyMatchResult};
use repo_metadata::repositories::DetectedRepositories;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use super::provider_display_name;
use crate::agent_resume::{self, AgentConversation, AgentResumeProvider};
use crate::search::command_palette::agent_conversations::search_item::AgentConversationSearchItem;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_palette::separator_search_item::SeparatorSearchItem;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::DataSourceRunErrorWrapper;
use crate::search::SyncDataSource;

/// Bound the journal read separately from the number of rows shown: filters run over
/// the larger pool before the displayed result cap is applied.
const CONVERSATION_POOL: usize = 300;
const MAX_DISPLAYED: usize = 50;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScopeFilter {
    #[default]
    ThisProject,
    ProjectWorktrees,
    All,
    Bookmarked,
}

impl ScopeFilter {
    pub fn label(self) -> &'static str {
        match self {
            Self::ThisProject => "This project",
            Self::ProjectWorktrees => "All worktrees",
            Self::All => "All",
            Self::Bookmarked => "Bookmark convos",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AgentFilter {
    #[default]
    All,
    Claude,
    Codex,
}

impl AgentFilter {
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Claude => "Claude",
            Self::Codex => "Codex",
        }
    }

    fn provider(self) -> Option<AgentResumeProvider> {
        match self {
            Self::All => None,
            Self::Claude => Some(AgentResumeProvider::Claude),
            Self::Codex => Some(AgentResumeProvider::Codex),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderEntry {
    pub root: PathBuf,
    pub display_name: String,
    pub count: usize,
}

/// Data source for the "Reopen agent conversation" picker: recent CLI-agent
/// (Claude/Codex) conversations registered to Clinch panes, with titles enriched
/// from matching prompt mirrors or native agent transcripts.
#[derive(Default)]
pub struct DataSource {
    conversations: Vec<AgentConversation>,
    scope: ScopeFilter,
    selected_folder: Option<PathBuf>,
    agent: AgentFilter,
    project_root: Option<PathBuf>,
    project_worktree_roots: Vec<PathBuf>,
    roots_by_conversation: Vec<Option<PathBuf>>,
    recent_folders: Vec<FolderEntry>,
}

impl DataSource {
    /// Re-reads the on-disk journal/mirror. Called whenever the palette rebuilds its
    /// mixer (i.e. on every palette open), so each open sees fresh conversations while
    /// individual keystrokes only filter in memory. Conversations we don't know how to
    /// reopen (unknown agent) are dropped here so every listed item is acceptable.
    pub fn refresh(&mut self, ctx: &mut ModelContext<Self>) {
        let conversations = agent_resume::conversations_for_finder(CONVERSATION_POOL)
            .into_iter()
            .filter(|conversation| conversation.reopen_command().is_some())
            .collect::<Vec<_>>();
        let roots_by_conversation = conversations
            .iter()
            .map(|conversation| conversation_root(conversation, ctx))
            .collect::<Vec<_>>();
        let recent_folders = build_recent_folders(&roots_by_conversation);

        self.conversations = conversations;
        self.roots_by_conversation = roots_by_conversation;
        self.recent_folders = recent_folders;
        ctx.notify();
    }

    pub fn set_scope(&mut self, scope: ScopeFilter, ctx: &mut ModelContext<Self>) {
        if self.scope != scope {
            self.scope = scope;
            if scope == ScopeFilter::ProjectWorktrees {
                self.refresh_project_worktree_roots();
            }
            ctx.notify();
        }
    }

    pub fn set_selected_folder(
        &mut self,
        selected_folder: Option<PathBuf>,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.selected_folder != selected_folder {
            self.selected_folder = selected_folder;
            ctx.notify();
        }
    }

    pub fn set_agent(&mut self, agent: AgentFilter, ctx: &mut ModelContext<Self>) {
        if self.agent != agent {
            self.agent = agent;
            ctx.notify();
        }
    }

    pub fn set_project_root(
        &mut self,
        project_root: Option<PathBuf>,
        ctx: &mut ModelContext<Self>,
    ) {
        let project_changed = self.project_root != project_root;
        if project_changed {
            self.project_root = project_root;
        }
        let previous_worktree_roots = self.project_worktree_roots.clone();
        if self.scope == ScopeFilter::ProjectWorktrees {
            // Re-read even when the active directory is unchanged: linked worktrees can
            // be added or removed between openings of the conversation finder.
            self.refresh_project_worktree_roots();
        }
        if project_changed || self.project_worktree_roots != previous_worktree_roots {
            ctx.notify();
        }
    }

    pub fn scope(&self) -> ScopeFilter {
        self.scope
    }

    pub fn selected_folder(&self) -> Option<&Path> {
        self.selected_folder.as_deref()
    }

    pub fn agent(&self) -> AgentFilter {
        self.agent
    }

    pub fn recent_folders(&self) -> &[FolderEntry] {
        &self.recent_folders
    }

    fn matching_conversations<'a>(
        &'a self,
        needle: &str,
    ) -> Vec<(&'a AgentConversation, String, FuzzyMatchResult)> {
        let bookmarked_only = self.scope == ScopeFilter::Bookmarked;
        let selected_provider = self.agent.provider();

        self.conversations
            .iter()
            .zip(&self.roots_by_conversation)
            .filter(|(conversation, _)| !bookmarked_only || conversation.bookmarked)
            .filter(|(conversation, _)| {
                selected_provider.is_none_or(|provider| {
                    AgentResumeProvider::from_agent_name(&conversation.agent) == Some(provider)
                })
            })
            .filter(|(_, root)| self.matches_directory_filter(root.as_deref()))
            .filter_map(|(conversation, _)| {
                let command = conversation.reopen_command()?;
                let match_result = if needle.is_empty() {
                    FuzzyMatchResult::no_match()
                } else {
                    let haystack = searchable_text(conversation);
                    match_indices_case_insensitive(&haystack, needle)?
                };
                Some((conversation, command, match_result))
            })
            .take(MAX_DISPLAYED)
            .collect()
    }

    fn refresh_project_worktree_roots(&mut self) {
        let Some(project_root) = self.project_root.as_deref() else {
            self.project_worktree_roots.clear();
            return;
        };

        self.project_worktree_roots = crate::util::git::worktree_roots_sync(project_root);
        if self.project_worktree_roots.is_empty() {
            // Preserve useful project-local behavior if Git is unavailable or the active
            // project is not a repository.
            self.project_worktree_roots.push(project_root.to_path_buf());
        }
    }

    fn matches_directory_filter(&self, root: Option<&Path>) -> bool {
        if self.scope == ScopeFilter::Bookmarked {
            return true;
        }

        if let Some(selected_folder) = self.selected_folder.as_deref() {
            return root.is_some_and(|root| paths_match(root, selected_folder));
        }

        match self.scope {
            ScopeFilter::All => true,
            ScopeFilter::ThisProject => self.project_root.as_deref().is_none_or(|project_root| {
                root.is_some_and(|root| paths_match(root, project_root))
            }),
            ScopeFilter::ProjectWorktrees => {
                self.project_root.is_none()
                    || root.is_some_and(|root| {
                        self.project_worktree_roots
                            .iter()
                            .any(|worktree_root| paths_match(root, worktree_root))
                    })
            }
            ScopeFilter::Bookmarked => true,
        }
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
        let mut results: Vec<QueryResult<Self::Action>> = self
            .matching_conversations(&needle)
            .into_iter()
            .rev()
            .map(|(conversation, command, match_result)| {
                QueryResult::from(AgentConversationSearchItem::new(
                    conversation.clone(),
                    command,
                    match_result,
                    self.scope == ScopeFilter::Bookmarked,
                ))
            })
            .collect();

        // Friendly empty state (a non-interactible separator row) when nothing has ever
        // been recorded; a non-matching query keeps the palette's own no-results state.
        if needle.is_empty() && results.is_empty() {
            let message = if self.scope == ScopeFilter::Bookmarked {
                "No bookmarked conversations — use Bookmark convo in a Claude Code or Codex footer"
            } else {
                "No recent agent conversations — run claude or codex in a pane to record one"
            };
            results.push(QueryResult::from(SeparatorSearchItem::new(
                message.to_string(),
            )));
        }

        Ok(results)
    }
}

fn conversation_root(conversation: &AgentConversation, app: &AppContext) -> Option<PathBuf> {
    let cwd = PathBuf::from(conversation.cwd.as_deref()?);
    let cwd_key = LocalOrRemotePath::Local(cwd.clone());
    DetectedRepositories::as_ref(app)
        .get_root_for_path(&cwd_key)
        .and_then(|root| root.to_local_path().map(Path::to_path_buf))
        .or(Some(cwd))
}

fn build_recent_folders(roots_by_conversation: &[Option<PathBuf>]) -> Vec<FolderEntry> {
    let mut counts = HashMap::new();
    for root in roots_by_conversation.iter().flatten() {
        *counts.entry(root.clone()).or_insert(0) += 1;
    }

    let mut seen = HashSet::new();
    roots_by_conversation
        .iter()
        .flatten()
        .filter_map(|root| {
            if !seen.insert(root.clone()) {
                return None;
            }
            let display_name = root
                .file_name()
                .filter(|name| !name.is_empty())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| root.to_string_lossy().into_owned());
            Some(FolderEntry {
                root: root.clone(),
                display_name,
                count: counts[root],
            })
        })
        .collect()
}

fn paths_match(left: &Path, right: &Path) -> bool {
    left.to_string_lossy()
        .trim_end_matches(std::path::MAIN_SEPARATOR)
        == right
            .to_string_lossy()
            .trim_end_matches(std::path::MAIN_SEPARATOR)
}

fn searchable_text(conversation: &AgentConversation) -> String {
    format!(
        "{} {} {} {} {}",
        conversation.first_prompt.as_deref().unwrap_or(""),
        conversation.cwd.as_deref().unwrap_or(""),
        conversation.session_id,
        conversation.agent,
        provider_display_name(&conversation.agent),
    )
}

impl Entity for DataSource {
    type Event = ();
}

#[cfg(test)]
#[path = "data_source_tests.rs"]
mod tests;
