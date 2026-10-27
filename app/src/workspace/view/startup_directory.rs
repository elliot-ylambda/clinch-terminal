//! Logic to determine the working directory for new terminal sessions.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use warpui::{AppContext, SingletonEntity, ViewContext, ViewHandle, WindowId};

use super::Workspace;
use crate::terminal::available_shells::AvailableShell;
#[cfg(feature = "local_tty")]
use crate::terminal::available_shells::AvailableShells;
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::session_settings::{NewSessionSource, SessionSettings};
use crate::terminal::view::TerminalView;
use crate::terminal::ShellLaunchData;
use crate::workspace::WorkspaceRegistry;

impl Workspace {
    /// Resolve a repository even if a running agent has reported a temporary
    /// scratch directory through OSC 7. Only use sibling panes when this project
    /// contains one unambiguous repository; never borrow another project's cwd.
    pub(super) fn automatic_worktree_repo(&self, ctx: &AppContext) -> Option<PathBuf> {
        let terminal = self
            .tabs
            .get(self.active_tab_index)?
            .pane_group
            .as_ref(ctx)
            .active_session_view(ctx)?;
        if terminal.as_ref(ctx).active_session_is_local(ctx) != Some(true) {
            return None;
        }

        resolve_automatic_worktree_repo(
            local_worktree_repo_candidates(&terminal, ctx),
            self.tabs.iter().flat_map(|tab| {
                tab.pane_group
                    .as_ref(ctx)
                    .terminal_views(ctx)
                    .into_iter()
                    .flat_map(|terminal| local_worktree_repo_candidates(&terminal, ctx))
            }),
        )
    }

    /// Helper function to compute the initial directory for a new session
    /// that is inheriting its initial directory from the active session in
    /// the given workspace.
    fn initial_directory_from_active_session(&self, ctx: &AppContext) -> Option<PathBuf> {
        (!self.tabs.is_empty())
            .then(|| {
                self.active_tab_pane_group().read(ctx, |pane_group, ctx| {
                    pane_group.active_session_id(ctx).and_then(|base_pane_id| {
                        pane_group.startup_path_for_new_session(Some(base_pane_id), ctx)
                    })
                })
            })
            .flatten()
    }

    /// Helper function to retrieve the shell launch data of the active session,
    /// which tells us whether it's a native or WSL session.
    fn shell_launch_info_from_active_session(&self, ctx: &AppContext) -> Option<ShellLaunchData> {
        (!self.tabs.is_empty())
            .then(|| {
                self.active_tab_pane_group().read(ctx, |pane_group, ctx| {
                    pane_group.active_session_id(ctx).and_then(|base_pane_id| {
                        pane_group.launch_data_for_session(base_pane_id, ctx)
                    })
                })
            })
            .flatten()
    }

    /// Helper function to compute the initial directory for a new session.
    /// Returns Some(path) if inheriting the initial directory from an active
    /// session or using the user's custom path setting,
    /// and None if the default startup directory (the user's home directory) should be used.
    pub(super) fn get_new_tab_startup_directory(
        &mut self,
        new_session_source: NewSessionSource,
        previous_session_window_id: Option<WindowId>,
        chosen_shell: Option<&AvailableShell>,
        ctx: &mut ViewContext<Self>,
    ) -> Option<PathBuf> {
        // Get the Workspace from the window that hosted the previously-active
        // session.
        let active_session_info = match previous_session_window_id {
            // If the previous window is the one hosting this workspace, don't
            // do any indirection through AppContext.
            Some(window_id) if window_id == ctx.window_id() => {
                let prior_workspace = WorkspaceRegistry::as_ref(ctx)
                    .get(window_id, ctx)
                    .filter(|workspace| workspace.id() != ctx.handle().id());
                prior_workspace.map_or_else(
                    || {
                        Some((
                            self.initial_directory_from_active_session(ctx),
                            self.shell_launch_info_from_active_session(ctx),
                        ))
                    },
                    |workspace| {
                        Some(workspace.read(ctx, |workspace, ctx| {
                            (
                                workspace.initial_directory_from_active_session(ctx),
                                workspace.shell_launch_info_from_active_session(ctx),
                            )
                        }))
                    },
                )
            }
            // Otherwise, lookup the Workspace in that window and query it.
            Some(window_id) => {
                let workspace_handle = WorkspaceRegistry::as_ref(ctx).get(window_id, ctx);
                workspace_handle.map(|workspace| {
                    workspace.read(ctx, |workspace, ctx| {
                        (
                            workspace.initial_directory_from_active_session(ctx),
                            workspace.shell_launch_info_from_active_session(ctx),
                        )
                    })
                })
            }
            None => None,
        };

        let (prev_session_working_directory, prev_session_shell) =
            active_session_info.unwrap_or_default();

        cfg_if::cfg_if! {
            if #[cfg(feature = "local_tty")] {
                let is_wsl = new_session_shell(chosen_shell, ctx)
                .wsl_distro()
                .is_some();
            } else {
                let is_wsl = false;
            }
        }

        let is_same_system = same_system(prev_session_shell.as_ref(), chosen_shell, ctx);

        compute_startup_directory_from_prev_session(
            new_session_source,
            if is_same_system {
                prev_session_working_directory
            } else {
                None
            },
            is_wsl,
            ctx,
        )
    }
}

fn local_worktree_repo_candidates(
    terminal: &ViewHandle<TerminalView>,
    ctx: &AppContext,
) -> Vec<PathBuf> {
    let view = terminal.as_ref(ctx);
    if view.active_session_is_local(ctx) != Some(true) {
        return Vec::new();
    }

    let cwd = view
        .canonical_session_pwd_if_local(ctx)
        .map(|path| path.as_path().to_path_buf());
    let agent_cwd = CLIAgentSessionsModel::as_ref(ctx)
        .session(terminal.id())
        .filter(|session| session.remote_host.is_none())
        .and_then(|session| session.session_context.cwd.as_deref())
        .map(PathBuf::from);
    [
        cwd,
        agent_cwd,
        view.current_local_repo_path().map(PathBuf::from),
    ]
    .into_iter()
    .flatten()
    .collect()
}

pub(super) fn resolve_automatic_worktree_repo(
    active_paths: impl IntoIterator<Item = PathBuf>,
    project_paths: impl IntoIterator<Item = PathBuf>,
) -> Option<PathBuf> {
    let mut checked_repositories = HashSet::new();
    for path in active_paths {
        if path.is_absolute() && checked_repositories.insert(worktree_repository_key(&path)) {
            if let Some(root) = crate::util::git::primary_worktree_root_sync(&path) {
                return Some(root);
            }
        }
    }

    let mut project_repo = None;
    for path in project_paths {
        if !path.is_absolute() || !checked_repositories.insert(worktree_repository_key(&path)) {
            continue;
        }
        let Some(root) = crate::util::git::primary_worktree_root_sync(&path) else {
            continue;
        };
        if project_repo
            .as_ref()
            .is_some_and(|previous| previous != &root)
        {
            return None;
        }
        project_repo = Some(root);
    }
    project_repo
}

fn worktree_repository_key(path: &Path) -> PathBuf {
    // Many project tabs share Git metadata. Discover it without a subprocess so
    // the fallback runs `git worktree list` only once per repository, including
    // across linked checkouts and their subdirectories.
    #[cfg(not(target_family = "wasm"))]
    if let Ok(repository) = git2::Repository::discover(path) {
        return dunce::canonicalize(repository.commondir())
            .unwrap_or_else(|_| repository.commondir().to_path_buf());
    }
    path.to_path_buf()
}

/// The shell to be used in the new session,
/// based on the shell explicitly chosen by the user or
/// the default startup shell specified in settings.
#[cfg(feature = "local_tty")]
fn new_session_shell(chosen_shell: Option<&AvailableShell>, ctx: &AppContext) -> AvailableShell {
    chosen_shell.cloned().unwrap_or_else(move || {
        AvailableShells::handle(ctx).read(ctx, |shells, ctx| shells.get_user_preferred_shell(ctx))
    })
}

/// Windows-specific helper function to determine whether the old and
/// new shell sessions will exist in the same system, i.e. whether
/// they're both on native Windows or both in the same WSL distribution.
///
/// Returns `true` if `old_session_launch_data` is `None`.
#[cfg(feature = "local_tty")]
fn same_system(
    old_session_launch_data: Option<&ShellLaunchData>,
    chosen_shell: Option<&AvailableShell>,
    ctx: &AppContext,
) -> bool {
    // If there's no prior session, there is no prior system.
    // We're not crossing a system boundary, so return true.
    let Some(old_launch_data) = old_session_launch_data else {
        return true;
    };

    let wsl_distro = new_session_shell(chosen_shell, ctx).wsl_distro();
    match old_launch_data {
        ShellLaunchData::WSL { distro: old_distro } => {
            wsl_distro.is_some_and(|new_distro| new_distro == *old_distro)
        }
        _ => wsl_distro.is_none(),
    }
}

#[cfg(not(feature = "local_tty"))]
const fn same_system(
    _old_session_launch_data: Option<&ShellLaunchData>,
    _chosen_shell: Option<&AvailableShell>,
    _ctx: &AppContext,
) -> bool {
    true
}

/// Helper function to compute the actual startup directory for the
/// new session based on the user's settings.
fn compute_startup_directory_from_prev_session(
    new_session_source: NewSessionSource,
    initial_directory_from_prev_session: Option<PathBuf>,
    ignore_custom_directory: bool,
    ctx: &ViewContext<Workspace>,
) -> Option<PathBuf> {
    SessionSettings::handle(ctx).read(ctx, |settings, _ctx| {
        settings
            .working_directory_config
            .initial_directory_for_new_session(
                new_session_source,
                initial_directory_from_prev_session,
                ignore_custom_directory,
            )
    })
}
