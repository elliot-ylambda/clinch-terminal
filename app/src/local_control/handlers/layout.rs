//! Layout mutation handlers for local-control actions.
#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
use std::path::{Path, PathBuf};

use ::local_control::protocol::{TabCreateParams, TabType, TargetSelector};
use ::local_control::{ActionKind, ControlError, ErrorCode, InstanceId};
use serde::Serialize;
#[cfg(feature = "local_tty")]
use warpui::SingletonEntity;
use warpui::{ModelContext, TypedActionView};

use crate::local_control::resolver::{
    decode_params, target_window_id_for_target, validate_tab_create_target, workspace_for_window,
};
use crate::local_control::LocalControlBridge;
use crate::server::telemetry::AddTabWithShellSource;
use crate::terminal::available_shells::AvailableShell;
#[cfg(feature = "local_tty")]
use crate::terminal::available_shells::AvailableShells;
use crate::workspace::WorkspaceAction;

const MAX_LAUNCH_CWD_BYTES: usize = 4 * 1024;
const MAX_LAUNCH_ARGS: usize = 256;
const MAX_LAUNCH_COMMAND_BYTES: usize = 32 * 1024;

#[derive(Debug, PartialEq, Eq)]
struct TerminalLaunch {
    cwd: PathBuf,
    command: Option<String>,
}

#[derive(Serialize)]
struct TabCreateResponse<'a> {
    action: &'static str,
    created: bool,
    instance_id: Option<&'a str>,
    window: TargetWindowResponse,
    tab: TabCountsResponse,
}

#[derive(Serialize)]
struct TargetWindowResponse {
    selector: &'static str,
    id: String,
}

#[derive(Serialize)]
struct TabCountsResponse {
    id: String,
    previous_count: usize,
    count: usize,
    active_index: usize,
}

pub(crate) fn create_tab(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_tab_create_target(target)?;
    let window_id = target_window_id_for_target(ctx, target, ActionKind::TabCreate)?;
    let workspace = workspace_for_window(window_id, ActionKind::TabCreate, ctx)?;
    let params = decode_params::<TabCreateParams>(params)?;
    let launch = tab_create_launch(&params)?;
    let action = if launch.is_none() {
        Some(tab_create_action(&params, ctx)?)
    } else {
        None
    };
    let (tab_id, previous_tab_count, tab_count, active_tab_index) =
        workspace.update(ctx, |workspace, ctx| {
            let previous_tab_count = workspace.tab_count();
            match (launch, action.as_ref()) {
                (Some(TerminalLaunch { cwd, command }), _) => {
                    if let Some(command) = command {
                        if !workspace.launch_command_in_new_tab(
                            command,
                            Some(cwd.to_string_lossy().into_owned()),
                            ctx,
                        ) {
                            return Err(ControlError::new(
                                ErrorCode::Internal,
                                "tab.create opened a terminal but could not register its startup command",
                            ));
                        }
                    } else {
                        workspace.remote_control_open_terminal(Some(cwd), ctx);
                    }
                }
                (None, Some(WorkspaceAction::AddDefaultTab)) => {
                    workspace.add_default_tab_from_local_control(ctx);
                }
                (None, Some(action)) => workspace.handle_action(action, ctx),
                (None, None) => {
                    return Err(ControlError::new(
                        ErrorCode::Internal,
                        "tab.create resolved neither a launch request nor a workspace action",
                    ));
                }
            }
            let tab_id = workspace
                .get_pane_group_view(workspace.active_tab_index())
                .map(|tab| tab.id().to_string())
                .ok_or_else(|| {
                    ControlError::new(
                        ErrorCode::Internal,
                        "tab.create did not produce an active tab identifier",
                    )
                })?;
            Ok((
                tab_id,
                previous_tab_count,
                workspace.tab_count(),
                workspace.active_tab_index(),
            ))
        })?;
    serde_json::to_value(TabCreateResponse {
        action: ActionKind::TabCreate.as_str(),
        created: true,
        instance_id: instance_id.as_ref().map(|id| id.0.as_str()),
        window: TargetWindowResponse {
            selector: "target",
            id: window_id.to_string(),
        },
        tab: TabCountsResponse {
            id: tab_id,
            previous_count: previous_tab_count,
            count: tab_count,
            active_index: active_tab_index,
        },
    })
    .map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to serialize local-control tab.create response",
            err.to_string(),
        )
    })
}

fn tab_create_action(
    params: &TabCreateParams,
    ctx: &ModelContext<LocalControlBridge>,
) -> Result<WorkspaceAction, ControlError> {
    if let Some(shell_name) = params.shell.as_deref() {
        if matches!(params.tab_type, Some(TabType::Agent | TabType::CloudAgent)) {
            return Err(ControlError::new(
                ErrorCode::InvalidParams,
                "tab.create cannot combine an agent tab type with a shell",
            ));
        }
        return Ok(WorkspaceAction::AddTabWithShell {
            shell: resolve_shell(shell_name, ctx)?,
            source: AddTabWithShellSource::CommandPalette,
        });
    }
    match params.tab_type {
        None | Some(TabType::Terminal) => Ok(WorkspaceAction::AddTerminalTab {
            hide_homepage: false,
        }),
        Some(TabType::Agent) => Ok(WorkspaceAction::AddAgentTab),
        Some(TabType::Default) => Ok(WorkspaceAction::AddDefaultTab),
        Some(TabType::CloudAgent) => Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            "tab.create does not support cloud-agent tabs",
        )),
    }
}

fn tab_create_launch(params: &TabCreateParams) -> Result<Option<TerminalLaunch>, ControlError> {
    let has_launch_options = params.cwd.is_some() || !params.command.is_empty();
    if !has_launch_options {
        return Ok(None);
    }
    if params.shell.is_some() {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            "tab.create cannot combine --shell with --cwd or a startup command",
        ));
    }
    if !matches!(params.tab_type, None | Some(TabType::Terminal)) {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            "tab.create cwd and startup command options require a terminal tab",
        ));
    }
    let cwd = params.cwd.as_deref().ok_or_else(|| {
        ControlError::new(
            ErrorCode::InvalidParams,
            "tab.create startup commands require an explicit cwd",
        )
    })?;
    if cwd.len() > MAX_LAUNCH_CWD_BYTES || cwd.chars().any(char::is_control) {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            "tab.create cwd is too long or contains control characters",
        ));
    }
    let cwd = Path::new(cwd);
    if !cwd.is_absolute() {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            "tab.create cwd must be an absolute path",
        ));
    }
    if !cwd.is_dir() {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            "tab.create cwd must be an existing local directory",
        ));
    }
    if params.command.len() > MAX_LAUNCH_ARGS
        || params
            .command
            .iter()
            .map(String::len)
            .fold(0usize, usize::saturating_add)
            > MAX_LAUNCH_COMMAND_BYTES
        || params
            .command
            .iter()
            .any(|arg| arg.chars().any(char::is_control))
    {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            "tab.create startup command is too large or contains control characters",
        ));
    }
    if !params.command.is_empty() && !cfg!(feature = "local_tty") {
        return Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            "tab.create startup commands require local terminal support",
        ));
    }
    let command = (!params.command.is_empty()).then(|| {
        params
            .command
            .iter()
            .map(|arg| shell_words::quote(arg).into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    });
    Ok(Some(TerminalLaunch {
        cwd: cwd.to_path_buf(),
        command,
    }))
}

#[cfg_attr(not(feature = "local_tty"), allow(unused_variables))]
pub(super) fn resolve_shell(
    name: &str,
    ctx: &ModelContext<LocalControlBridge>,
) -> Result<AvailableShell, ControlError> {
    #[cfg(feature = "local_tty")]
    {
        AvailableShells::as_ref(ctx)
            .find_by_command_name(name)
            .or_else(|| AvailableShell::try_from(name).ok())
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::InvalidParams,
                    format!("cannot resolve requested shell {name:?}"),
                )
            })
    }
    #[cfg(not(feature = "local_tty"))]
    Err(ControlError::new(
        ErrorCode::UnsupportedAction,
        format!("shell selection is unavailable for requested shell {name:?}"),
    ))
}
