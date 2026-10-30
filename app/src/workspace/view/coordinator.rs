//! The project coordinator: a Claude Code or Codex session pinned first in each project's tab
//! list. It starts in the project's primary checkout (never a linked worktree), falling back to
//! the home directory, and carries coordinator instructions as a system prompt so creating one
//! never spends a turn.

use std::path::{Path, PathBuf};

use blocking::unblock;
use settings::Setting;
use warpui::{AppContext, SingletonEntity, ViewContext};

use super::Workspace;
use crate::agent_resume::AgentResumeProvider;
use crate::channel::{Channel, ChannelState};
use crate::menu::{MenuItem, MenuItemFields};
use crate::settings::ClinchSettings;
use crate::terminal::cli_agent_sessions::{provider_for_agent, CLIAgentSessionsModel};
use crate::workspace::action::WorkspaceAction;

pub(super) const COORDINATOR_TAB_TITLE: &str = "Coordinator";

const COORDINATOR_INSTRUCTIONS: &str = "You are the Clinch coordinator for this project, a \
session pinned first in its tab list. When the user asks about their other sessions, use the \
clinch-coordinate skill to inspect, summarise and send instructions to the Claude Code and Codex \
sessions open in Clinch. Do not edit files in this checkout unless the user asks you to.";

const RESTORE_EXACT_PREFIX: &str = "CLINCH_AGENT_RESTORE_EXACT=1 ";

/// Where a project is in creating its default coordinator.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum CoordinatorBootstrap {
    #[default]
    Idle,
    /// Wanted, but no tab has reported a local directory yet.
    AwaitingDirectory,
    /// The primary checkout is being resolved off the UI thread.
    Resolving,
}

/// Arguments that give an agent its coordinator role without spending a turn.
fn coordinator_role_args(provider: AgentResumeProvider) -> String {
    match provider {
        AgentResumeProvider::Claude => format!(
            "--append-system-prompt {}",
            shell_words::quote(COORDINATOR_INSTRUCTIONS)
        ),
        AgentResumeProvider::Codex => {
            // Codex parses `-c` values as TOML, and a JSON string literal is a TOML basic string.
            let value = serde_json::Value::from(COORDINATOR_INSTRUCTIONS).to_string();
            format!(
                "-c {}",
                shell_words::quote(&format!("developer_instructions={value}"))
            )
        }
    }
}

pub(crate) fn coordinator_launch_command(provider: AgentResumeProvider) -> String {
    format!(
        "{} {}",
        provider.launch_command_prefix(true),
        coordinator_role_args(provider)
    )
}

/// The command a restored coordinator pane replays. A saved conversation resumes with the role
/// re-applied; anything else starts a fresh coordinator. `CLINCH_AGENT_RESTORE_FRESH` replaces
/// exact restoration so a coordinator that never ran a turn starts fresh instead of stopping at
/// a bare shell, and never adopts another session recorded for the same checkout.
pub(crate) fn coordinator_restore_command(
    pending: Option<&str>,
    default_provider: AgentResumeProvider,
) -> String {
    pending
        .map(|command| {
            command
                .strip_prefix(RESTORE_EXACT_PREFIX)
                .unwrap_or(command)
        })
        .and_then(|command| {
            let (provider, _) =
                crate::agent_resume::agent_session_seed_from_restore_command(command)?;
            Some(format!(
                "CLINCH_AGENT_RESTORE_FRESH=1 {command} {}",
                coordinator_role_args(provider)
            ))
        })
        .unwrap_or_else(|| coordinator_launch_command(default_provider))
}

/// Where a project's coordinator starts: the primary checkout of the repository containing
/// `directory` (a linked worktree maps back to its source checkout), else the home directory.
pub(crate) fn coordinator_directory(directory: Option<&Path>) -> Option<PathBuf> {
    directory
        .and_then(crate::util::git::primary_worktree_root_sync)
        .or_else(dirs::home_dir)
}

fn configured_provider(ctx: &AppContext) -> AgentResumeProvider {
    AgentResumeProvider::from_agent_name(ClinchSettings::as_ref(ctx).coordinator_agent.as_str())
        .unwrap_or(AgentResumeProvider::Claude)
}

impl Workspace {
    pub(super) fn coordinator_tab_index(&self) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.coordinator)
    }

    /// Whether this workspace may create a coordinator on its own. Unit tests opt in through
    /// `coordinator_auto_create`, and integration runs never do, so their tab layouts stay exact.
    fn coordinator_auto_create_allowed(&self) -> bool {
        self.coordinator_auto_create && !matches!(ChannelState::channel(), Channel::Integration)
    }

    /// Asks for a default coordinator if the setting is on and this project has none. It is
    /// created once a tab reports a local directory to resolve the primary checkout from.
    pub(super) fn request_default_coordinator(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.coordinator_auto_create_allowed()
            || !*ClinchSettings::as_ref(ctx).coordinator_enabled
            || self.coordinator_tab_index().is_some()
        {
            return;
        }
        self.coordinator_bootstrap = CoordinatorBootstrap::AwaitingDirectory;
        self.maybe_start_default_coordinator(ctx);
    }

    /// Called whenever a tab's directory may have become known.
    pub(super) fn maybe_start_default_coordinator(&mut self, ctx: &mut ViewContext<Self>) {
        if self.coordinator_bootstrap != CoordinatorBootstrap::AwaitingDirectory {
            return;
        }
        let Some(directory) = self.coordinator_candidate_directory(ctx) else {
            return;
        };
        self.coordinator_bootstrap = CoordinatorBootstrap::Resolving;
        ctx.spawn(
            unblock(move || coordinator_directory(Some(&directory))),
            |me, directory, ctx| {
                me.coordinator_bootstrap = CoordinatorBootstrap::Idle;
                if me.coordinator_tab_index().is_none() {
                    me.open_coordinator_tab(directory, configured_provider(ctx), false, ctx);
                }
            },
        );
    }

    /// The first local directory any ordinary tab reports, preferring the active tab. Restored
    /// tabs that have not bootstrapped yet still report the directory of their last block;
    /// remote sessions are skipped because their directories are not on this machine.
    fn coordinator_candidate_directory(&self, ctx: &AppContext) -> Option<PathBuf> {
        self.tabs
            .get(self.active_tab_index)
            .into_iter()
            .chain(self.tabs.iter())
            .filter(|tab| !tab.coordinator)
            .find_map(|tab| {
                let terminal = tab.pane_group.as_ref(ctx).active_session_view(ctx)?;
                let terminal = terminal.as_ref(ctx);
                if terminal.active_session_is_local(ctx) == Some(false) {
                    return None;
                }
                terminal
                    .canonical_session_pwd_if_local(ctx)
                    .map(PathBuf::from)
                    .or_else(|| terminal.pwd().map(PathBuf::from))
            })
    }

    /// Opens a coordinator running `provider` at `directory` and pins it first.
    pub(crate) fn open_coordinator_tab(
        &mut self,
        directory: Option<PathBuf>,
        provider: AgentResumeProvider,
        activate: bool,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let Some(pane_group) = self.launch_command_in_new_tab_placed(
            coordinator_launch_command(provider),
            directory.map(|directory| directory.to_string_lossy().into_owned()),
            Some(COORDINATOR_TAB_TITLE.to_owned()),
            Some((!activate, None)),
            None,
            ctx,
        ) else {
            return false;
        };
        let Some(index) = self
            .tabs
            .iter()
            .position(|tab| tab.pane_group.id() == pane_group.id())
        else {
            return false;
        };
        let tab = &mut self.tabs[index];
        tab.coordinator = true;
        tab.pinned = true;
        tab.group_id = None;
        self.move_tab_to_index(index, 0, ctx);
        ctx.dispatch_global_action("workspace:save_app", ());
        self.notify_project_metadata_changed(ctx);
        true
    }

    /// Gives a restored coordinator pane its role and its fresh-start fallback.
    pub(super) fn prepare_restored_coordinator(&self, index: usize, ctx: &mut ViewContext<Self>) {
        let default_provider = configured_provider(ctx);
        let Some(manager_handle) = self.tabs[index]
            .pane_group
            .read(ctx, |pane_group, ctx| pane_group.terminal_manager(0, ctx))
        else {
            return;
        };
        manager_handle.update(ctx, |terminal_manager, ctx| {
            if let Some(manager) = terminal_manager
                .as_any()
                .downcast_ref::<crate::terminal::local_tty::TerminalManager>()
            {
                manager.rewrite_on_restore_command(
                    |pending| coordinator_restore_command(pending, default_provider),
                    ctx,
                );
            }
        });
    }

    /// Opens a coordinator from the tab menu and turns the default back on.
    pub(super) fn open_coordinator_from_menu(&mut self, ctx: &mut ViewContext<Self>) {
        if self.coordinator_tab_index().is_some() {
            return;
        }
        let directory = coordinator_directory(self.coordinator_candidate_directory(ctx).as_deref());
        if self.open_coordinator_tab(directory, configured_provider(ctx), true, ctx) {
            set_coordinator_enabled(true, ctx);
        }
    }

    /// Restarts the coordinator with the other agent in the same directory, and remembers it as
    /// the default for new coordinators.
    pub(super) fn switch_coordinator_agent(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if !self.tabs.get(index).is_some_and(|tab| tab.coordinator) {
            return;
        }
        let provider = match self.coordinator_provider(index, ctx) {
            AgentResumeProvider::Claude => AgentResumeProvider::Codex,
            AgentResumeProvider::Codex => AgentResumeProvider::Claude,
        };
        let directory = self.tabs[index]
            .pane_group
            .as_ref(ctx)
            .active_session_view(ctx)
            .and_then(|terminal| terminal.as_ref(ctx).canonical_session_pwd_if_local(ctx))
            .map(PathBuf::from);
        let old_pane_group = self.tabs[index].pane_group.id();
        // Clear the marker first: closing the old session here is not the user closing the
        // coordinator, so it must not turn the default off.
        self.tabs[index].coordinator = false;
        if !self.open_coordinator_tab(directory, provider, true, ctx) {
            self.tabs[index].coordinator = true;
            return;
        }
        if let Some(old_index) = self
            .tabs
            .iter()
            .position(|tab| tab.pane_group.id() == old_pane_group)
        {
            self.close_tab(old_index, true, false, ctx);
        }
        set_coordinator_agent(provider, ctx);
    }

    /// The agent a coordinator runs now, or the configured default before it has started.
    fn coordinator_provider(&self, index: usize, ctx: &AppContext) -> AgentResumeProvider {
        self.tabs[index]
            .pane_group
            .as_ref(ctx)
            .active_session_view(ctx)
            .and_then(|terminal| CLIAgentSessionsModel::as_ref(ctx).session(terminal.id()))
            .and_then(|session| provider_for_agent(session.agent))
            .unwrap_or_else(|| configured_provider(ctx))
    }

    /// Coordinator entries for a tab's right-click menu: switching the coordinator's agent, or
    /// opening one when this project has none.
    pub(super) fn coordinator_menu_items(
        &self,
        index: usize,
        ctx: &AppContext,
    ) -> Vec<MenuItem<WorkspaceAction>> {
        if self.tabs.get(index).is_some_and(|tab| tab.coordinator) {
            let label = match self.coordinator_provider(index, ctx) {
                AgentResumeProvider::Claude => "Switch coordinator to Codex",
                AgentResumeProvider::Codex => "Switch coordinator to Claude",
            };
            return vec![MenuItemFields::new(label)
                .with_on_select_action(WorkspaceAction::SwitchCoordinatorAgent(index))
                .into_item()];
        }
        if self.coordinator_tab_index().is_none() {
            return vec![MenuItemFields::new("New coordinator session")
                .with_on_select_action(WorkspaceAction::OpenCoordinatorSession)
                .into_item()];
        }
        vec![]
    }
}

/// Adds the coordinator entries as their own section just above a tab menu's closing section.
pub(super) fn with_coordinator_section(
    mut items: Vec<MenuItem<WorkspaceAction>>,
    coordinator_items: Vec<MenuItem<WorkspaceAction>>,
) -> Vec<MenuItem<WorkspaceAction>> {
    if coordinator_items.is_empty() {
        return items;
    }
    match items
        .iter()
        .rposition(|item| matches!(item, MenuItem::Separator))
    {
        Some(close_section) => {
            let section = coordinator_items
                .into_iter()
                .chain(std::iter::once(MenuItem::Separator));
            items.splice(close_section + 1..close_section + 1, section);
        }
        None => {
            if !items.is_empty() {
                items.push(MenuItem::Separator);
            }
            items.extend(coordinator_items);
        }
    }
    items
}

/// Persists whether projects get a default coordinator.
pub(super) fn set_coordinator_enabled(enabled: bool, ctx: &mut AppContext) {
    ClinchSettings::handle(ctx).update(ctx, |settings, ctx| {
        if let Err(error) = settings.coordinator_enabled.set_value(enabled, ctx) {
            log::error!("Error persisting the coordinator setting: {error}");
        }
    });
}

/// Persists the agent that new coordinators start.
fn set_coordinator_agent(provider: AgentResumeProvider, ctx: &mut AppContext) {
    ClinchSettings::handle(ctx).update(ctx, |settings, ctx| {
        if let Err(error) = settings
            .coordinator_agent
            .set_value(provider.as_str().to_owned(), ctx)
        {
            log::error!("Error persisting the coordinator agent: {error}");
        }
    });
}
