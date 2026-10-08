//! Project discovery, portable layout files, and live session transfers.
use std::fs::File;
use std::io::Read as _;
use std::path::PathBuf;

use clap::{Args, Subcommand};
use local_control::projects::{
    MAX_PROJECT_LAYOUT_BYTES, ProjectCreateParams, ProjectLayout, ProjectRestoreParams,
};
use local_control::protocol::EmptyParams;
use local_control::{ActionKind, ControlError, ErrorCode};

use super::TargetArgs;
use super::agents::{self, ScopeArgs};
use super::commands::{request_action_with_params, run_action_with_params};
use crate::agent::OutputFormat;

#[derive(Debug, Clone, Subcommand)]
pub enum ProjectCommand {
    /// Manage the project's pending task list. Completion removes a task, as in the UI.
    #[command(subcommand)]
    Task(ProjectTaskCommand),
    /// List all projects, including inactive ones.
    List(ScopeArgs),
    /// Read one project's complete section/tab/pane hierarchy.
    Inspect(TargetArgs),
    /// Create and activate a project tab in the selected window.
    Create {
        #[arg(long)]
        cwd: Option<String>,
        #[command(flatten)]
        target: TargetArgs,
    },
    /// Switch to an existing project.
    Activate(TargetArgs),
    /// Close a project using the app's normal close confirmation.
    Close(TargetArgs),
    /// Export a portable terminal/agent layout. Runtime IDs are not reusable after restoration.
    Export {
        /// Write only the layout to a new JSON file (fails if the file already exists).
        #[arg(long)]
        file: Option<PathBuf>,
        #[command(flatten)]
        target: TargetArgs,
    },
    /// Restore a layout into a new project, preserving the existing projects.
    Restore {
        #[arg(long)]
        file: PathBuf,
        /// Also resume captured local Claude/Codex conversations; never replay arbitrary commands.
        #[arg(long)]
        resume_agents: bool,
        #[command(flatten)]
        target: TargetArgs,
    },
}

#[derive(Debug, Clone, Args)]
pub struct TabTransferArgs {
    /// Exact destination project ID, including another native window.
    #[arg(long)]
    pub to_project: String,
    /// Destination section ID; omit to leave the session ungrouped.
    #[arg(long)]
    pub section: Option<String>,
    /// Zero-based insertion position within the destination section or ungrouped sessions.
    #[arg(long)]
    pub index: Option<usize>,
    #[command(flatten)]
    pub target: TargetArgs,
}

pub(super) fn read_layout(path: &std::path::Path) -> Result<ProjectLayout, ControlError> {
    let mut data = Vec::new();
    File::open(path)
        .and_then(|file| {
            file.take(MAX_PROJECT_LAYOUT_BYTES as u64 + 1)
                .read_to_end(&mut data)
        })
        .map_err(|err| {
            ControlError::with_details(
                ErrorCode::InvalidParams,
                "cannot read layout file",
                err.to_string(),
            )
        })?;
    if data.len() > MAX_PROJECT_LAYOUT_BYTES {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            "project layout exceeds 1 MiB",
        ));
    }
    serde_json::from_slice(&data).map_err(|err| {
        ControlError::with_details(
            ErrorCode::InvalidParams,
            "invalid project layout JSON",
            err.to_string(),
        )
    })
}

pub(super) fn run_project(
    command: ProjectCommand,
    format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        ProjectCommand::Task(command) => run_task(command, format),
        ProjectCommand::List(args) => agents::render(
            &agents::request(
                &agents::instance(&args.instance)?,
                ActionKind::ProjectList,
                args.scope(),
            )?,
            format,
        ),
        ProjectCommand::Inspect(target) => {
            run_action_with_params(target, ActionKind::ProjectInspect, EmptyParams {}, format)
        }
        ProjectCommand::Create { cwd, target } => run_action_with_params(
            target,
            ActionKind::ProjectCreate,
            ProjectCreateParams { cwd },
            format,
        ),
        ProjectCommand::Activate(target) => {
            run_action_with_params(target, ActionKind::ProjectActivate, EmptyParams {}, format)
        }
        ProjectCommand::Close(target) => {
            run_action_with_params(target, ActionKind::ProjectClose, EmptyParams {}, format)
        }
        ProjectCommand::Export { file, target } => {
            let data =
                request_action_with_params(target, ActionKind::ProjectExport, EmptyParams {})?;
            if let Some(path) = file {
                let file = File::create_new(&path).map_err(|err| {
                    ControlError::with_details(
                        ErrorCode::InvalidParams,
                        "cannot create layout file",
                        err.to_string(),
                    )
                })?;
                serde_json::to_writer(file, &data["layout"]).map_err(|err| {
                    ControlError::with_details(
                        ErrorCode::Internal,
                        "cannot write layout file",
                        err.to_string(),
                    )
                })?;
                agents::render(&serde_json::json!({"file": path, "exported": true}), format)
            } else {
                agents::render(&data, format)
            }
        }
        ProjectCommand::Restore {
            file,
            resume_agents,
            target,
        } => {
            let layout = read_layout(&file)?;
            run_action_with_params(
                target,
                ActionKind::ProjectRestore,
                ProjectRestoreParams {
                    layout,
                    resume_agents,
                },
                format,
            )
        }
    }
}

#[cfg(test)]
#[path = "projects_tests.rs"]
mod tests;

#[derive(Debug, Clone, Subcommand)]
pub enum ProjectTaskCommand {
    List(TargetArgs),
    Create {
        #[arg(long)]
        text: String,
        #[command(flatten)]
        target: TargetArgs,
    },
    Update {
        task_id: String,
        #[arg(long)]
        text: String,
        #[command(flatten)]
        target: TargetArgs,
    },
    Complete {
        task_id: String,
        #[command(flatten)]
        target: TargetArgs,
    },
    Delete {
        task_id: String,
        #[command(flatten)]
        target: TargetArgs,
    },
}
fn run_task(command: ProjectTaskCommand, format: OutputFormat) -> Result<(), ControlError> {
    use local_control::projects::{
        ProjectTaskCreateParams, ProjectTaskIdParams, ProjectTaskUpdateParams,
    };
    match command {
        ProjectTaskCommand::List(target) => {
            run_action_with_params(target, ActionKind::ProjectTaskList, EmptyParams {}, format)
        }
        ProjectTaskCommand::Create { text, target } => run_action_with_params(
            target,
            ActionKind::ProjectTaskCreate,
            ProjectTaskCreateParams { text },
            format,
        ),
        ProjectTaskCommand::Update {
            task_id,
            text,
            target,
        } => run_action_with_params(
            target,
            ActionKind::ProjectTaskUpdate,
            ProjectTaskUpdateParams { task_id, text },
            format,
        ),
        ProjectTaskCommand::Complete { task_id, target } => run_action_with_params(
            target,
            ActionKind::ProjectTaskComplete,
            ProjectTaskIdParams { task_id },
            format,
        ),
        ProjectTaskCommand::Delete { task_id, target } => run_action_with_params(
            target,
            ActionKind::ProjectTaskDelete,
            ProjectTaskIdParams { task_id },
            format,
        ),
    }
}
