#[cfg(feature = "local_fs")]
use repo_metadata::repositories::DetectedRepositories;
use warpui::{AppContext, SingletonEntity as _, ViewContext};

use crate::context_chips::display_chip::GitLineChanges;
use crate::context_chips::{git_line_changes_from_chips, ContextChipKind};
use crate::terminal::cli_agent_sessions::title_glyph::{
    agent_title_text, shows_working, standardize_agent_title,
};
use crate::terminal::cli_agent_sessions::{
    conversation_title_from_agent_title, CLIAgentSession, CLIAgentSessionsModel,
};
use crate::terminal::TerminalView;

impl TerminalView {
    pub const LINKED_WORKTREE_LABEL: &'static str = "Worktree";

    /// A CLI's OSC title can summarize the conversation. Ignore the shell's placeholder title
    /// and current directory while waiting for the agent to supply one.
    pub(super) fn cli_agent_terminal_title(&self, ctx: &AppContext) -> Option<String> {
        let title = {
            let model = self.model.lock();
            model
                .terminal_title()
                .filter(|title| title.trim() != model.shell_launch_state().display_name())?
        };
        let display_directory = self.display_working_directory(ctx);
        let directory = self.pwd();
        let is_directory = [display_directory.as_deref(), directory.as_deref()]
            .into_iter()
            .flatten()
            .any(|directory| {
                title.trim() == directory
                    || std::path::Path::new(directory)
                        .file_name()
                        .is_some_and(|name| name == title.trim())
            });
        (!is_directory).then_some(title)
    }

    fn prompt_chip_value(&self, chip_kind: &ContextChipKind, ctx: &AppContext) -> Option<String> {
        self.current_prompt
            .as_ref(ctx)
            .latest_chip_value(chip_kind, ctx)
            .map(|v| v.to_string())
            .filter(|value| !value.trim().is_empty())
    }

    pub fn display_working_directory(&self, ctx: &AppContext) -> Option<String> {
        let raw = self
            .prompt_chip_value(&ContextChipKind::WorkingDirectory, ctx)
            .or_else(|| self.pwd())?;
        let home_dir = self
            .active_block_session_id()
            .and_then(|session_id| self.sessions.as_ref(ctx).get(session_id))
            .and_then(|session| session.home_dir().map(str::to_owned));
        Some(warp_util::path::user_friendly_path(&raw, home_dir.as_deref()).to_string())
    }

    pub fn terminal_title_from_shell(&self) -> String {
        let model = self.model.lock();
        let fallback_title = model.shell_launch_state().display_name().to_owned();
        model
            .terminal_title()
            .filter(|title| !title.trim().is_empty())
            .unwrap_or(fallback_title)
    }

    pub fn current_git_branch(&self, ctx: &AppContext) -> Option<String> {
        self.prompt_chip_value(&ContextChipKind::ShellGitBranch, ctx)
            .or_else(|| {
                self.git_status_metadata(ctx)
                    .map(|metadata| metadata.current_branch_name.clone())
                    .filter(|branch| !branch.trim().is_empty())
            })
    }

    /// Whether the active local session is inside a linked Git worktree.
    pub fn is_linked_git_worktree(&self, ctx: &AppContext) -> bool {
        #[cfg(feature = "local_fs")]
        {
            let Some(cwd) = self.canonical_session_pwd_if_local(ctx) else {
                return false;
            };
            let Some(repository) = DetectedRepositories::as_ref(ctx)
                .get_local_watched_repo_for_path(cwd.as_path(), ctx)
            else {
                return false;
            };

            repository.as_ref(ctx).is_linked_worktree()
        }

        #[cfg(not(feature = "local_fs"))]
        {
            let _ = ctx;
            false
        }
    }

    pub fn last_completed_command_text(&self) -> Option<String> {
        let model = self.model.lock();
        model.block_list().blocks().iter().rev().find_map(|block| {
            if block.finished()
                && !block.is_background()
                && !block.is_static()
                && (block.bootstrap_stage().is_done() || block.is_restored())
            {
                let cmd = block.command_to_string();
                if cmd.trim().is_empty() {
                    None
                } else {
                    Some(cmd)
                }
            } else {
                None
            }
        })
    }

    /// [`Self::terminal_title_from_shell`] as tab chrome shows it: a Claude Code or Codex title
    /// gets the standardized status glyph (see `title_glyph`).
    pub fn display_title_from_shell(&self, ctx: &AppContext) -> String {
        self.standardized_agent_title(None, ctx)
            .unwrap_or_else(|| self.terminal_title_from_shell())
    }

    /// The agent's own terminal title without its status glyph, for chrome that shows status
    /// separately. `None` when the pane runs no agent Clinch has hooks for.
    pub(crate) fn agent_title_without_status(&self, ctx: &AppContext) -> Option<String> {
        let session = CLIAgentSessionsModel::as_ref(ctx)
            .session(self.view_id)
            .filter(|session| session.agent.has_clinch_hooks())?;
        let raw_title = self.terminal_title_from_shell();
        let text = self
            .agent_title_stand_in(session)
            .unwrap_or_else(|| agent_title_text(session.agent, &raw_title).trim());
        let text = if text.is_empty() {
            session.agent.display_name()
        } else {
            text
        };
        Some(text.to_owned())
    }

    /// The tab title for a Claude Code or Codex pane with the standardized status glyph, or
    /// `None` when the pane runs no such agent. `display_text` is the text chosen for the tab
    /// (such as the session's prompt); `None` uses the agent's own terminal title.
    pub(crate) fn standardized_agent_title(
        &self,
        display_text: Option<&str>,
        ctx: &AppContext,
    ) -> Option<String> {
        let session = CLIAgentSessionsModel::as_ref(ctx).session(self.view_id)?;
        standardize_agent_title(
            session.agent,
            &self.terminal_title_from_shell(),
            display_text.or_else(|| self.agent_title_stand_in(session)),
            session.into(),
        )
    }

    /// Whether the terminal title is the agent's own. It isn't while the agent is going to sleep,
    /// asleep, waking, or restored and not started: the shell holds the terminal then, and its
    /// title is usually the working directory.
    fn agent_owns_terminal_title(&self) -> bool {
        self.agent_sleep.is_none()
            && self.agent_sleep_overdue.is_none()
            && self.deferred_agent_start.is_none()
    }

    /// What tab chrome shows in place of the terminal title while the agent doesn't hold the
    /// terminal: the last title the agent gave its conversation, or its name. `None` while the
    /// terminal title is the agent's own.
    fn agent_title_stand_in<'a>(&self, session: &'a CLIAgentSession) -> Option<&'a str> {
        (!self.agent_owns_terminal_title()).then(|| {
            session
                .session_context
                .agent_title
                .as_deref()
                .unwrap_or(session.agent.display_name())
        })
    }

    /// The agent's name for its conversation, without its status glyph: the live terminal title
    /// while the agent holds the terminal and names the conversation there, and otherwise the last
    /// one it gave. An asleep tab keeps its name instead of taking the shell's.
    pub(super) fn cli_agent_conversation_title(
        &self,
        session: &CLIAgentSession,
        ctx: &AppContext,
    ) -> Option<String> {
        self.agent_owns_terminal_title()
            .then(|| self.cli_agent_terminal_title(ctx))
            .flatten()
            .and_then(|title| {
                conversation_title_from_agent_title(agent_title_text(session.agent, &title))
            })
            .or_else(|| session.session_context.agent_title.clone())
    }

    /// The terminal title changed. If the agent holds the terminal, its session keeps the title for
    /// when the agent sleeps or Clinch restarts.
    pub(super) fn remember_agent_title(&self, ctx: &mut ViewContext<Self>) {
        if !self.agent_owns_terminal_title() {
            return;
        }
        let Some(title) = self.cli_agent_terminal_title(ctx) else {
            return;
        };
        let view_id = self.view_id;
        CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, _| {
            sessions.remember_agent_title(view_id, &title);
        });
    }

    /// Whether tab chrome shows this pane's agent session as working. The title glyph, status
    /// dots, and project summaries all read this, so they cannot disagree.
    pub(crate) fn cli_agent_shows_working(&self, session: &CLIAgentSession) -> bool {
        shows_working(
            session.agent,
            &self.terminal_title_from_shell(),
            session.into(),
        )
    }

    pub fn terminal_title_text(&self) -> String {
        if !self.terminal_title.trim().is_empty() {
            return self.terminal_title.clone();
        }
        self.terminal_title_from_shell()
    }

    pub fn current_pull_request_url(&self, ctx: &AppContext) -> Option<String> {
        self.current_prompt
            .as_ref(ctx)
            .latest_chip_value(&ContextChipKind::GithubPullRequest, ctx)
            .map(|v| v.to_string())
            .filter(|value| !value.trim().is_empty())
    }

    pub fn current_diff_line_changes(&self, ctx: &AppContext) -> Option<GitLineChanges> {
        // Prefer the externally-updated GitRepoStatusModel (local filesystem
        // watcher or remote daemon push receiver) over parsing the raw shell
        // chip output. This matches the preference order used by the prompt
        // chip display (display.rs) and agent footer (chips.rs).
        let from_model = self
            .git_status_metadata(ctx)
            .map(|metadata| GitLineChanges::from_diff_stats(&metadata.stats_against_head));

        from_model
            .or_else(|| {
                git_line_changes_from_chips(&self.current_prompt.as_ref(ctx).agent_view_chips(ctx))
            })
            .filter(|line_changes| {
                line_changes.files_changed > 0
                    || line_changes.lines_added > 0
                    || line_changes.lines_removed > 0
            })
    }
}
