//! CLI-agent commands use app-wide discovery, never the currently focused tab.
use std::io::Read as _;
use std::time::Duration;

use clap::{Args, Subcommand};
use instant::Instant;
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
    pub(super) instance: InstanceArgs,
    /// Filter by a returned project ID; repeat to include several projects.
    #[arg(long = "project")]
    projects: Vec<String>,
    /// Filter by a returned section ID; repeat to include several sections.
    #[arg(long = "section")]
    sections: Vec<String>,
}

impl ScopeArgs {
    pub(super) fn scope(&self) -> AgentScope {
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

#[derive(Debug, Clone, Args)]
pub struct AgentReadOptions {
    /// Filter records before applying --last/--limit.
    #[arg(long)]
    role: Option<local_control::agents::AgentRole>,
    /// Exclude tool-only records with no message text.
    #[arg(long)]
    messages_only: bool,
    /// Continue an earlier read from its next_cursor.
    #[arg(long, conflicts_with_all = ["tail", "from_start", "all"])]
    after: Option<String>,
    /// Maximum records per response. Default: 3 recent records, or 100 per history page.
    #[arg(long, visible_alias = "last", env = "CLINCH_AGENT_READ_LIMIT", value_parser = clap::value_parser!(u32).range(1..=500))]
    limit: Option<u32>,
    /// Read the newest records (the default); retained for compatibility.
    #[arg(long, conflicts_with_all = ["from_start", "all"])]
    tail: bool,
    /// Read the first history page; continue with --after NEXT_CURSOR.
    #[arg(long, conflicts_with = "all")]
    from_start: bool,
    /// Stream all available history pages. Requires --output-format ndjson.
    #[arg(long)]
    all: bool,
}

impl AgentReadOptions {
    fn params(&self, agent_id: String) -> AgentReadParams {
        let recent = self.tail || !(self.from_start || self.all || self.after.is_some());
        AgentReadParams {
            agent_id,
            after: self.after.clone(),
            limit: self.limit.unwrap_or(if recent { 3 } else { 100 }) as usize,
            tail: recent,
            role: self.role,
            messages_only: self.messages_only,
        }
    }
}

#[derive(Debug, Clone, Subcommand)]
pub enum WorkspaceCommand {
    /// Read the full window/project/section/tab/pane hierarchy without changing focus.
    Tree(ScopeArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum AgentCommand {
    /// Launch Claude/Codex in an exact project; --background preserves focus.
    Launch(AgentLaunchArgs),
    /// Wait for an exact agent state without changing its tab or work.
    Wait(AgentWaitArgs),
    /// Interrupt a working turn without closing the conversation.
    Interrupt {
        #[command(flatten)]
        target: AgentArgs,
        #[arg(long)]
        expected_revision: String,
    },
    /// Read unread assistant text across projects, with independent reader checkpoints.
    Inbox {
        #[command(flatten)]
        scope: ScopeArgs,
        /// Stable UUID for this reader/coordinator; checkpoints persist across calls.
        #[arg(long, env = "CLINCH_INBOX_READER")]
        reader: String,
        /// Maximum messages per conversation per call.
        #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: u32,
        /// Leave unread checkpoints unchanged. Without this flag, acknowledge after output.
        #[arg(long)]
        peek: bool,
    },
    /// Acknowledge a batch previously returned by inbox --peek.
    InboxAck {
        #[command(flatten)]
        instance: InstanceArgs,
        #[arg(long, env = "CLINCH_INBOX_READER")]
        reader: String,
        #[arg(long)]
        batch: String,
    },
    /// Replay recorded agent and organization events, including while disconnected.
    Events {
        #[command(flatten)]
        scope: ScopeArgs,
        #[arg(long, visible_alias = "since")]
        after: Option<String>,
        #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u32).range(1..=500))]
        limit: u32,
        /// Stream journal pages as NDJSON until interrupted.
        #[arg(long)]
        follow: bool,
    },
    /// List Claude/Codex sessions across all projects, or the requested scope.
    List(ScopeArgs),
    /// Inspect state, input revision, capabilities, and latest previews.
    Inspect(AgentArgs),
    /// Read the latest three captured records by default, or select older/full history.
    Read {
        #[command(flatten)]
        target: AgentArgs,
        #[command(flatten)]
        options: AgentReadOptions,
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

pub(super) fn instance(
    args: &InstanceArgs,
) -> Result<local_control::discovery::InstanceRecord, ControlError> {
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

pub(super) fn request<T: Serialize>(
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

pub(super) fn render(data: &serde_json::Value, format: OutputFormat) -> Result<(), ControlError> {
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

pub(super) fn run_agent(command: AgentCommand, format: OutputFormat) -> Result<(), ControlError> {
    let data = match command {
        AgentCommand::InboxAck {
            instance: selection,
            reader,
            batch,
        } => request(
            &instance(&selection)?,
            ActionKind::AgentInboxAck,
            local_control::agents::AgentInboxAckParams {
                reader_id: reader,
                batch_id: batch,
            },
        )?,
        AgentCommand::Launch(args) => return launch(args, format),
        AgentCommand::Wait(args) => return wait_for_agent(args, format),
        AgentCommand::Interrupt {
            target,
            expected_revision,
        } => request(
            &instance(&target.instance)?,
            ActionKind::AgentInterrupt,
            local_control::agents::AgentInterruptParams {
                agent_id: target.agent_id,
                expected_revision,
            },
        )?,
        AgentCommand::Inbox {
            scope,
            reader,
            limit,
            peek,
        } => {
            let instance = instance(&scope.instance)?;
            let data = request(
                &instance,
                ActionKind::AgentInbox,
                local_control::agents::AgentInboxParams {
                    reader_id: reader.clone(),
                    scope: scope.scope(),
                    limit: limit as usize,
                },
            )?;
            // Never acknowledge if stdout failed. Repeated records are preferable to lost work.
            render(&data, format)?;
            if !peek && let Some(batch) = data["batch_id"].as_str() {
                request(
                    &instance,
                    ActionKind::AgentInboxAck,
                    local_control::agents::AgentInboxAckParams {
                        reader_id: reader,
                        batch_id: batch.to_owned(),
                    },
                )?;
            }
            return Ok(());
        }
        AgentCommand::Events {
            scope,
            after,
            limit,
            follow,
        } => return events(scope, after, limit, follow, format),
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
        AgentCommand::Read { target, options } => {
            if options.all && !matches!(format, OutputFormat::Ndjson) {
                return Err(ControlError::new(
                    ErrorCode::InvalidParams,
                    "--all streams history pages; use --output-format ndjson",
                ));
            }
            let instance = instance(&target.instance)?;
            let params = options.params(target.agent_id);
            if options.all {
                return read_all_pages(
                    params,
                    |params| request(&instance, ActionKind::AgentRead, params),
                    |page| render(page, format),
                );
            }
            request(&instance, ActionKind::AgentRead, params)?
        }
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

/// Preserve coverage metadata on every page without buffering an entire conversation.
fn read_all_pages(
    mut params: AgentReadParams,
    mut fetch: impl FnMut(&AgentReadParams) -> Result<serde_json::Value, ControlError>,
    mut emit: impl FnMut(&serde_json::Value) -> Result<(), ControlError>,
) -> Result<(), ControlError> {
    loop {
        let page = fetch(&params)?;
        emit(&page)?;
        if page["has_more"] != true || page["pending_record"] == true {
            return Ok(());
        }
        let cursor = page["next_cursor"]
            .as_str()
            .filter(|cursor| !cursor.is_empty() && params.after.as_deref() != Some(*cursor));
        params.after = Some(
            cursor
                .ok_or_else(|| {
                    ControlError::new(
                        ErrorCode::InvalidRequest,
                        "history did not advance; inspect the last emitted page before continuing",
                    )
                })?
                .to_owned(),
        );
    }
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

#[derive(Debug, Clone, Args)]
pub struct AgentLaunchArgs {
    #[command(flatten)]
    instance: InstanceArgs,
    #[arg(long)]
    provider: local_control::agents::AgentProvider,
    #[arg(long)]
    project: String,
    #[arg(long)]
    section: Option<String>,
    #[arg(long)]
    cwd: Option<String>,
    #[arg(long)]
    title: Option<String>,
    #[arg(long, conflicts_with = "prompt_file")]
    prompt: Option<String>,
    /// Read an initial prompt from a UTF-8 file or '-' for stdin.
    #[arg(long)]
    prompt_file: Option<std::path::PathBuf>,
    #[arg(long)]
    background: bool,
    /// Wait up to this many seconds for exact pane identity/readiness. Zero returns creation only.
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u32).range(0..=3600))]
    timeout: u32,
}

#[derive(Debug, Clone, Args)]
pub struct AgentWaitArgs {
    #[command(flatten)]
    target: AgentArgs,
    #[arg(long, value_enum, default_value_t = WaitCondition::Ready)]
    until: WaitCondition,
    #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u32).range(1..=86400))]
    timeout: u32,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum WaitCondition {
    Ready,
    Working,
    Attention,
    TurnComplete,
}
fn condition_matches(condition: WaitCondition, data: &serde_json::Value) -> bool {
    match condition {
        WaitCondition::Ready => data["ready"] == true,
        WaitCondition::Working => data["state"] == "working",
        WaitCondition::Attention => matches!(
            data["state"].as_str(),
            Some("needs_attention" | "rate_limited")
        ),
        WaitCondition::TurnComplete => data["state"] == "turn_complete",
    }
}
/// Read-only observation runs on one bounded worker per call. A hung socket cannot keep the
/// wait command alive past its deadline; an expired read is never retried or used for a mutation.
fn observe_before_deadline<T: Serialize>(
    instance: &local_control::discovery::InstanceRecord,
    kind: ActionKind,
    params: T,
    deadline: Instant,
) -> Result<Option<serde_json::Value>, ControlError> {
    let params = serde_json::to_value(params).map_err(|_| {
        ControlError::new(ErrorCode::InvalidParams, "invalid observation parameters")
    })?;
    let instance = instance.clone();
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Ok(None);
    }
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("clinch-cli-observe".into())
        .spawn(move || {
            let _ = sender.send(request(&instance, kind, params));
        })
        .map_err(|_| {
            ControlError::new(ErrorCode::Internal, "could not start bounded observation")
        })?;
    match receiver.recv_timeout(remaining) {
        Ok(result) => result.map(Some),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Ok(None),
        Err(_) => Err(ControlError::new(
            ErrorCode::TransportUnavailable,
            "observation worker stopped",
        )),
    }
}
fn wait_for_agent(args: AgentWaitArgs, format: OutputFormat) -> Result<(), ControlError> {
    let instance = instance(&args.target.instance)?;
    let deadline = Instant::now() + Duration::from_secs(args.timeout.into());
    let mut latest = serde_json::Value::Null;
    loop {
        let Some(agent) = observe_before_deadline(
            &instance,
            ActionKind::AgentInspect,
            AgentTargetParams {
                agent_id: args.target.agent_id.clone(),
            },
            deadline,
        )?
        else {
            return render(
                &serde_json::json!({"action": "agent.wait", "matched": false, "timed_out": true, "agent": latest}),
                format,
            );
        };
        if condition_matches(args.until, &agent) {
            return render(
                &serde_json::json!({"action": "agent.wait", "matched": true, "timed_out": false, "agent": agent}),
                format,
            );
        }
        latest = agent;
        std::thread::sleep(
            Duration::from_millis(250).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}

fn launch(args: AgentLaunchArgs, format: OutputFormat) -> Result<(), ControlError> {
    let instance = instance(&args.instance)?;
    let prompt = if args.prompt.is_some() || args.prompt_file.is_some() {
        Some(read_prompt(args.prompt, args.prompt_file)?)
    } else {
        None
    };
    let scope = AgentScope {
        projects: vec![args.project.clone()],
        sections: vec![],
    };
    let mut created = request(
        &instance,
        ActionKind::AgentLaunch,
        local_control::agents::AgentLaunchParams {
            provider: args.provider,
            project_id: args.project,
            section_id: args.section,
            cwd: args.cwd,
            title: args.title,
            prompt,
            background: args.background,
        },
    )?;
    let deadline = Instant::now() + Duration::from_secs(args.timeout.into());
    while Instant::now() < deadline {
        // Poll only the newly created pane; never attach a different nearby provider.
        match observe_before_deadline(&instance, ActionKind::AgentList, &scope, deadline) {
            Ok(Some(snapshot)) => {
                if let Some(agent) =
                    snapshot["agents"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|agent| {
                            agent["pane_id"] == created["pane_id"]
                                && agent["provider"]
                                    == (if args.provider
                                        == local_control::agents::AgentProvider::Claude
                                    {
                                        "claude-code"
                                    } else {
                                        "codex"
                                    })
                                && agent["conversation_id"].as_str().is_some()
                        })
                {
                    created["agent"] = agent.clone();
                    created["ready"] = agent["ready"].clone();
                    if agent["ready"] == true
                        || agent["state"] == "working"
                        || condition_matches(WaitCondition::Attention, agent)
                    {
                        created["identity_discovered"] = true.into();
                        return render(&created, format);
                    }
                }
            }
            Ok(None) => break,
            Err(error) => {
                // Creation happened: retain its exact IDs and never silently repeat the launch.
                created["observation_error"] = serde_json::to_value(error).unwrap_or_default();
                break;
            }
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(
            Duration::from_millis(250).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
    created["identity_discovered"] = created["agent"]["agent_id"].is_string().into();
    created["timed_out"] = (args.timeout > 0 && Instant::now() >= deadline).into();
    render(&created, format)
}
fn events(
    args: ScopeArgs,
    mut after: Option<String>,
    limit: u32,
    follow: bool,
    format: OutputFormat,
) -> Result<(), ControlError> {
    let instance = instance(&args.instance)?;
    loop {
        let data = request(
            &instance,
            ActionKind::AgentEvents,
            local_control::agents::AgentEventsParams {
                after: after.clone(),
                scope: args.scope(),
                limit: limit as usize,
            },
        )?;
        if !follow {
            return render(&data, format);
        }
        write_json_line(&data)?;
        after = data["next_cursor"].as_str().map(str::to_owned);
        if after.is_none() {
            return Err(ControlError::new(
                ErrorCode::InvalidRequest,
                "event journal returned no cursor",
            ));
        }
        if data["has_more"] != true {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
}
