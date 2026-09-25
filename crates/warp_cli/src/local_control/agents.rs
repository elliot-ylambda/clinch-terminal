//! CLI-agent commands use app-wide discovery, never the currently focused tab.
use std::io::Read as _;
use std::time::{Duration, Instant};

use clap::{Args, Subcommand};
use local_control::agents::{
    AgentMessageListParams, AgentMessageParams, AgentReadParams, AgentScope, AgentSendParams,
    AgentTargetParams, MAX_PROMPT_BYTES,
};
use local_control::protocol::{
    Action, ActionKind, ControlError, ControlResponse, ErrorCode, RequestEnvelope,
};
use local_control::selection::{InstanceSelector, select_instance};
use serde::Serialize;
use warp_core::channel::ChannelState;

use super::output::{write_json, write_json_line};
use crate::agent::OutputFormat;

#[derive(Debug, Clone, Args)]
pub struct InstanceArgs {
    #[arg(long, conflicts_with = "pid")]
    instance: Option<String>,
    #[arg(long)]
    pid: Option<u32>,
}

#[derive(Debug, Clone, Args)]
pub struct ScopeArgs {
    #[command(flatten)]
    instance: InstanceArgs,
    /// Filter by a returned project ID; repeat to include several projects.
    #[arg(long = "project")]
    projects: Vec<String>,
    /// Filter by a returned section ID; repeat to include several sections.
    #[arg(long = "section")]
    sections: Vec<String>,
}

impl ScopeArgs {
    fn scope(&self) -> AgentScope {
        AgentScope {
            projects: self.projects.clone(),
            sections: self.sections.clone(),
        }
    }
}

#[derive(Debug, Clone, Args)]
pub struct AgentArgs {
    /// Exact opaque agent_id returned by agent list/inspect.
    agent_id: String,
    #[command(flatten)]
    instance: InstanceArgs,
}

#[derive(Debug, Clone, Subcommand)]
pub enum WorkspaceCommand {
    /// Read the full window/project/section/tab/pane hierarchy without changing focus.
    Tree(ScopeArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum ProjectCommand {
    /// List every open project, including inactive projects.
    List(ScopeArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum AgentCommand {
    /// List Claude/Codex sessions across all projects, or the requested scope.
    List(ScopeArgs),
    /// Inspect state, input revision, capabilities, and latest previews.
    Inspect(AgentArgs),
    /// Read captured conversation records with explicit coverage and pagination.
    Read {
        #[command(flatten)]
        target: AgentArgs,
        #[arg(long, conflicts_with = "tail")]
        after: Option<String>,
        #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u32).range(1..=500))]
        limit: u32,
        /// Read the newest bounded portion, rather than starting at the beginning.
        #[arg(long)]
        tail: bool,
    },
    /// Submit one prompt, or explicitly queue it for the same agent to become ready.
    Send {
        #[command(flatten)]
        target: AgentArgs,
        #[arg(
            long,
            required_unless_present = "text_file",
            conflicts_with = "text_file"
        )]
        text: Option<String>,
        /// Read a UTF-8 prompt from a file, or '-' for standard input.
        #[arg(long)]
        text_file: Option<std::path::PathBuf>,
        /// The input_revision from a recent agent inspect/list result.
        #[arg(long)]
        expected_revision: String,
        /// Unique UUID for retrying this message within seven-day journal retention.
        #[arg(long)]
        request_id: String,
        /// Wait for this exact agent to become ready; human input cancels the queued message.
        #[arg(long, requires = "sender")]
        queue: bool,
        /// Stable UUID identifying this coordinating conversation.
        #[arg(long)]
        sender: Option<String>,
        /// Seconds before a queued message expires (maximum one day).
        #[arg(long, default_value_t = 1800, value_parser = clap::value_parser!(u32).range(1..=86400))]
        expires_in: u32,
    },
    /// Inspect, list, or cancel durable message receipts.
    #[command(subcommand)]
    Message(AgentMessageCommand),
    /// Poll scoped status snapshots. This is not a lossless/replayable event log.
    Watch {
        #[command(flatten)]
        scope: ScopeArgs,
        /// Snapshot cursor from an earlier watch result.
        #[arg(long)]
        after: Option<String>,
        /// Maximum seconds to wait for a changed snapshot.
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u32).range(0..=45))]
        wait: u32,
        /// Stream changed snapshots as NDJSON until interrupted.
        #[arg(long)]
        follow: bool,
    },
}

#[derive(Debug, Clone, Args)]
pub struct MessageArgs {
    request_id: String,
    #[command(flatten)]
    instance: InstanceArgs,
}

#[derive(Debug, Clone, Subcommand)]
pub enum AgentMessageCommand {
    Inspect(MessageArgs),
    Cancel(MessageArgs),
    List {
        #[command(flatten)]
        instance: InstanceArgs,
        #[arg(long = "agent")]
        agent_id: Option<String>,
        /// Continue with next_before from the preceding response.
        #[arg(long)]
        before: Option<i64>,
        #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: u32,
    },
}

fn instance(args: &InstanceArgs) -> Result<local_control::discovery::InstanceRecord, ControlError> {
    let selector = match (&args.instance, args.pid) {
        (Some(id), _) => InstanceSelector::Id(local_control::InstanceId(id.clone())),
        (_, Some(pid)) => InstanceSelector::Pid(pid),
        _ => InstanceSelector::Active,
    };
    select_instance(
        &local_control::discovery::list_instances(&ChannelState::channel().to_string()),
        &selector,
    )
}

fn request<T: Serialize>(
    instance: &local_control::discovery::InstanceRecord,
    kind: ActionKind,
    params: T,
) -> Result<serde_json::Value, ControlError> {
    let request = RequestEnvelope::new(Action::with_params(kind, params)?);
    match local_control::client::send_request(instance, &request)?.response {
        ControlResponse::Ok { data } => Ok(data),
        ControlResponse::Error { error } => Err(error),
    }
}

fn render(data: &serde_json::Value, format: OutputFormat) -> Result<(), ControlError> {
    match format {
        OutputFormat::Ndjson => write_json_line(data),
        _ => write_json(data),
    }
}

pub(super) fn run_workspace(
    command: WorkspaceCommand,
    format: OutputFormat,
) -> Result<(), ControlError> {
    let WorkspaceCommand::Tree(args) = command;
    render(
        &request(
            &instance(&args.instance)?,
            ActionKind::WorkspaceTree,
            args.scope(),
        )?,
        format,
    )
}

pub(super) fn run_project(
    command: ProjectCommand,
    format: OutputFormat,
) -> Result<(), ControlError> {
    let ProjectCommand::List(args) = command;
    render(
        &request(
            &instance(&args.instance)?,
            ActionKind::ProjectList,
            args.scope(),
        )?,
        format,
    )
}

pub(super) fn run_agent(command: AgentCommand, format: OutputFormat) -> Result<(), ControlError> {
    let data = match command {
        AgentCommand::List(args) => request(
            &instance(&args.instance)?,
            ActionKind::AgentList,
            args.scope(),
        )?,
        AgentCommand::Inspect(args) => request(
            &instance(&args.instance)?,
            ActionKind::AgentInspect,
            AgentTargetParams {
                agent_id: args.agent_id,
            },
        )?,
        AgentCommand::Read {
            target,
            after,
            limit,
            tail,
        } => request(
            &instance(&target.instance)?,
            ActionKind::AgentRead,
            AgentReadParams {
                agent_id: target.agent_id,
                after,
                limit: limit as usize,
                tail,
            },
        )?,
        AgentCommand::Send {
            target,
            text,
            text_file,
            expected_revision,
            request_id,
            queue,
            sender,
            expires_in,
        } => {
            let text = read_prompt(text, text_file)?;
            let instance = instance(&target.instance)?;
            let params = AgentSendParams {
                agent_id: target.agent_id,
                text,
                expected_revision,
                request_id,
                queue,
                sender_id: sender,
                expires_in,
            };
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut receipt = request(&instance, ActionKind::AgentSend, &params)?;
            while receipt["state"] == "dispatching" && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(100));
                // An exact replay retrieves the receipt without dispatching another prompt.
                receipt = request(&instance, ActionKind::AgentSend, &params)?;
            }
            receipt
        }
        AgentCommand::Message(command) => match command {
            AgentMessageCommand::Inspect(args) => request(
                &instance(&args.instance)?,
                ActionKind::AgentMessageInspect,
                AgentMessageParams {
                    request_id: args.request_id,
                },
            )?,
            AgentMessageCommand::Cancel(args) => request(
                &instance(&args.instance)?,
                ActionKind::AgentMessageCancel,
                AgentMessageParams {
                    request_id: args.request_id,
                },
            )?,
            AgentMessageCommand::List {
                instance: selection,
                agent_id,
                before,
                limit,
            } => request(
                &instance(&selection)?,
                ActionKind::AgentMessageList,
                AgentMessageListParams {
                    agent_id,
                    before,
                    limit,
                },
            )?,
        },
        AgentCommand::Watch {
            scope,
            after,
            wait,
            follow,
        } => return watch(scope, after, wait, follow, format),
    };
    render(&data, format)
}

fn read_prompt(
    text: Option<String>,
    path: Option<std::path::PathBuf>,
) -> Result<String, ControlError> {
    let text = if let Some(text) = text {
        text
    } else {
        let path = path.ok_or_else(|| {
            ControlError::new(ErrorCode::InvalidParams, "provide --text or --text-file")
        })?;
        let reader: Box<dyn std::io::Read> = if path.as_os_str() == "-" {
            Box::new(std::io::stdin())
        } else {
            Box::new(std::fs::File::open(path).map_err(|_| {
                ControlError::new(ErrorCode::InvalidParams, "could not open prompt file")
            })?)
        };
        let mut text = String::new();
        reader
            .take(MAX_PROMPT_BYTES as u64 + 1)
            .read_to_string(&mut text)
            .map_err(|_| {
                ControlError::new(ErrorCode::InvalidParams, "could not read UTF-8 prompt")
            })?;
        text
    };
    if text.trim().is_empty() || text.len() > MAX_PROMPT_BYTES {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            "prompt must be nonempty and at most 64 KiB",
        ));
    }
    Ok(text)
}

fn watch(
    args: ScopeArgs,
    mut after: Option<String>,
    wait: u32,
    follow: bool,
    format: OutputFormat,
) -> Result<(), ControlError> {
    // Resolve once: a newly launched instance must never silently replace this watch's target.
    let instance = instance(&args.instance)?;
    let deadline = Instant::now() + Duration::from_secs(wait.into());
    loop {
        let data = request(&instance, ActionKind::AgentList, args.scope())?;
        let cursor = data
            .get("snapshot_cursor")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::ProtocolVersionUnsupported,
                    "app does not support status snapshot cursors",
                )
            })?;
        let changed = after.as_deref() != Some(cursor);
        if changed || (!follow && Instant::now() >= deadline) {
            let event = serde_json::json!({"type": if changed { "snapshot" } else { "timeout" }, "cursor": cursor, "coverage": "polled_snapshot", "data": data});
            if follow {
                write_json_line(&event)?;
            } else {
                return render(&event, format);
            }
            after = event
                .get("cursor")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[derive(Debug, Clone, Args)]
pub struct PaneReadArgs {
    #[command(flatten)]
    instance: InstanceArgs,
    #[arg(long = "pane")]
    pane_id: String,
    #[arg(long, default_value_t = 65536, value_parser = clap::value_parser!(u32).range(1..=262144))]
    max_bytes: u32,
}

pub(super) fn run_pane_read(args: PaneReadArgs, format: OutputFormat) -> Result<(), ControlError> {
    render(
        &request(
            &instance(&args.instance)?,
            ActionKind::PaneRead,
            local_control::agents::PaneReadParams {
                pane_id: args.pane_id,
                max_bytes: args.max_bytes as usize,
            },
        )?,
        format,
    )
}

#[cfg(test)]
#[path = "agents_tests.rs"]
mod tests;
