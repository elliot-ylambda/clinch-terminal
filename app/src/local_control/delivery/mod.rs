//! Durable, ordered agent delivery. SQLite and queue polling never run on the UI thread.
mod journal;

use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ::local_control::agents::{
    AgentMessageListParams, AgentMessageParams, AgentSendParams, MAX_PROMPT_BYTES,
};
use ::local_control::{Action, ActionKind, ControlError, ErrorCode, InstanceId};
use journal::{Journal, Message};
use serde_json::Value;
use warpui::{ModelContext, ModelSpawner};

use super::bridge::LocalControlBridge;
use super::permissions::{ensure_action_allowed, ensure_feature_enabled};

type Reply = Result<Value, ControlError>;
pub(super) type PendingReply = async_channel::Receiver<Reply>;

enum Operation {
    Send(AgentSendParams),
    Inspect(String),
    Cancel(String),
    List(AgentMessageListParams),
}
enum Command {
    Request(Operation, async_channel::Sender<Reply>),
    Finished(String, bool),
    Tick,
    Shutdown,
}

pub(super) struct DeliveryService {
    sender: Sender<Command>,
}
impl Drop for DeliveryService {
    fn drop(&mut self) {
        let _ = self.sender.send(Command::Shutdown);
    }
}

fn unavailable() -> ControlError {
    ControlError::new(
        ErrorCode::BridgeUnavailable,
        "agent delivery worker is unavailable",
    )
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
fn uuid(value: &str) -> Result<String, ControlError> {
    uuid::Uuid::parse_str(value)
        .map(|id| id.to_string())
        .map_err(|_| {
            ControlError::new(
                ErrorCode::InvalidParams,
                "message and sender IDs must be UUIDs",
            )
        })
}

impl DeliveryService {
    pub fn start(
        instance: InstanceId,
        spawner: ModelSpawner<LocalControlBridge>,
    ) -> Result<Self, ControlError> {
        let path = warp_core::paths::warp_home_config_dir()
            .ok_or_else(unavailable)?
            .join("agent-coordination/messages.sqlite3");
        let (sender, receiver) = mpsc::channel();
        let completion = sender.clone();
        std::thread::Builder::new()
            .name("clinch-agent-delivery".into())
            .spawn(move || run_worker(path, instance, spawner, receiver, completion))
            .map_err(|_| unavailable())?;
        Ok(Self { sender })
    }

    pub fn request(&self, action: &Action) -> Result<PendingReply, ControlError> {
        let operation = match action.kind {
            ActionKind::AgentSend => {
                let mut params: AgentSendParams = action.params_as()?;
                params.request_id = uuid(&params.request_id)?;
                params.sender_id = params.sender_id.as_deref().map(uuid).transpose()?;
                if params.text.trim().is_empty()
                    || params.text.len() > MAX_PROMPT_BYTES
                    || params
                        .text
                        .chars()
                        .any(|ch| ch.is_control() && ch != '\n' && ch != '\t')
                    || !(1..=86400).contains(&params.expires_in)
                    || (params.queue && params.sender_id.is_none())
                {
                    return Err(ControlError::new(ErrorCode::InvalidParams, "send requires nonempty text up to 64 KiB without terminal controls, expiry 1–86400 seconds, and a sender UUID for --queue"));
                }
                Operation::Send(params)
            }
            ActionKind::AgentMessageInspect => {
                Operation::Inspect(uuid(&action.params_as::<AgentMessageParams>()?.request_id)?)
            }
            ActionKind::AgentMessageCancel => {
                Operation::Cancel(uuid(&action.params_as::<AgentMessageParams>()?.request_id)?)
            }
            ActionKind::AgentMessageList => Operation::List(action.params_as()?),
            _ => {
                return Err(ControlError::new(
                    ErrorCode::UnsupportedAction,
                    "not a delivery action",
                ))
            }
        };
        let (sender, receiver) = async_channel::bounded(1);
        self.sender
            .send(Command::Request(operation, sender))
            .map_err(|_| unavailable())?;
        Ok(receiver)
    }
}

fn run_worker(
    path: std::path::PathBuf,
    instance: InstanceId,
    spawner: ModelSpawner<LocalControlBridge>,
    receiver: Receiver<Command>,
    completion: Sender<Command>,
) {
    let mut journal = Journal::open(&path);
    loop {
        let command = match receiver.recv_timeout(Duration::from_millis(250)) {
            Ok(Command::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(command) => command,
            Err(mpsc::RecvTimeoutError::Timeout) => Command::Tick,
        };
        let journal = match &mut journal {
            Ok(journal) => journal,
            Err(error) => {
                if let Command::Request(_, reply) = command {
                    let _ = reply.try_send(Err(error.clone()));
                }
                continue;
            }
        };
        // Expiry and crash recovery happen before any dispatch or retry.
        let maintenance = journal.maintain(now(), journal::owner_is_alive);
        let can_dispatch = maintenance.is_ok();
        match command {
            Command::Request(operation, reply) => {
                let result = maintenance
                    .and_then(|_| handle(operation, journal, &instance, &spawner, &completion));
                let _ = reply.try_send(result);
            }
            Command::Finished(id, submitted) => {
                let result = journal.get(&id).and_then(|message| {
                    if let Some(message) = message.filter(|m| m.state == "dispatching") {
                        journal.transition(
                            &message,
                            if submitted { "submitted" } else { "delivery_unknown" },
                            (!submitted).then_some("submission could not be confirmed; inspect the transcript before sending again"),
                            now(),
                        )?;
                    }
                    Ok(())
                });
                if result.is_err() {
                    log::warn!("could not persist agent delivery completion");
                }
            }
            _ => {}
        }
        // Every claim must be persisted successfully before any PTY write.
        if can_dispatch {
            if let Err(error) = dispatch_pending(journal, &instance, &spawner, &completion) {
                log::warn!("agent delivery queue paused: {}", error.message);
            }
        }
    }
    // Replacing the local-control instance must not leave runnable work behind.
    if let Ok(journal) = &mut journal {
        if let Ok(pending) = journal.pending() {
            for message in pending.into_iter().filter(|m| m.instance_id == instance.0) {
                let state = if message.state == "dispatching" {
                    "delivery_unknown"
                } else {
                    "cancelled"
                };
                let _ = journal.transition(
                    &message,
                    state,
                    Some("local-control instance ended"),
                    now(),
                );
            }
        }
    }
}

fn check_bridge(
    bridge: &LocalControlBridge,
    instance: &InstanceId,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<(), ControlError> {
    if bridge.instance_id.as_ref() != Some(instance) {
        return Err(unavailable());
    }
    ensure_feature_enabled()?;
    ensure_action_allowed(ActionKind::AgentSend, ctx)
}

fn handle(
    operation: Operation,
    journal: &mut Journal,
    instance: &InstanceId,
    spawner: &ModelSpawner<LocalControlBridge>,
    completion: &Sender<Command>,
) -> Reply {
    match operation {
        Operation::Send(params) => {
            // Exact retries remain inspectable even when the old runtime target is gone.
            if let Some(message) = journal.existing(&params)? {
                return Ok(message.receipt(false));
            }
            let target = params.clone();
            let owner = instance.clone();
            let guard = futures_lite::future::block_on(spawner.spawn(move |bridge, ctx| {
                check_bridge(bridge, &owner, ctx)?;
                let entry = super::agents::resolve(&owner, &target.agent_id, ctx)?;
                let (revision, reason) = entry.terminal.as_ref(ctx).local_control_agent_readiness(ctx);
                if revision != target.expected_revision || !(reason.is_none() || (target.queue && reason == Some("working"))) {
                    return Err(ControlError::new(ErrorCode::StaleTarget, "input revision changed or target is unavailable; inspect it again (only working agents can be queued)"));
                }
                Ok(entry.terminal.as_ref(ctx).local_control_queue_guard())
            })).map_err(|_| unavailable())??;
            let message = journal.insert(Message::new(params, instance.0.clone(), guard, now()))?;
            dispatch_pending(journal, instance, spawner, completion)?;
            Ok(journal
                .get(&message.params.request_id)?
                .ok_or_else(unavailable)?
                .receipt(false))
        }
        Operation::Inspect(id) => Ok(journal
            .get(&id)?
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::StaleTarget,
                    "message receipt not found or past seven-day retention",
                )
            })?
            .receipt(true)),
        Operation::Cancel(id) => {
            let message = journal.get(&id)?.ok_or_else(|| {
                ControlError::new(ErrorCode::StaleTarget, "message receipt not found")
            })?;
            if message.state == "cancelled" {
                return Ok(message.receipt(false));
            }
            if message.state != "queued" {
                return Err(ControlError::new(ErrorCode::InvalidRequest, "only queued messages can be cancelled; dispatched messages cannot be retracted"));
            }
            journal
                .transition(&message, "cancelled", Some("cancelled by caller"), now())?
                .map(|m| m.receipt(false))
                .ok_or_else(|| {
                    ControlError::new(
                        ErrorCode::StaleTarget,
                        "message changed before cancellation; inspect it again",
                    )
                })
        }
        Operation::List(params) => journal.list(&params),
    }
}

enum Readiness {
    Ready(String),
    Wait,
    Cancel(&'static str),
}
fn probe(
    message: &Message,
    bridge: &LocalControlBridge,
    instance: &InstanceId,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<Readiness, ControlError> {
    check_bridge(bridge, instance, ctx)?;
    let entry = super::agents::resolve(instance, &message.params.agent_id, ctx)?;
    let terminal = entry.terminal.as_ref(ctx);
    if terminal.local_control_queue_guard() != message.guard {
        return Ok(Readiness::Cancel(
            "user input or foreground process changed",
        ));
    }
    let (revision, reason) = terminal.local_control_agent_readiness(ctx);
    if let Some(reason) = reason {
        // Drafts caused by our own still-unacknowledged PTY submission also wait.
        // Human edits change the guard above and cancel instead.
        if message.params.queue
            && matches!(
                reason,
                "working"
                    | "human_input_or_draft"
                    | "submission_pending"
                    | "interactive_attention_required"
                    | "rate_limited"
            )
        {
            Ok(Readiness::Wait)
        } else {
            Ok(Readiness::Cancel(reason))
        }
    } else {
        Ok(Readiness::Ready(revision))
    }
}

fn dispatch_pending(
    journal: &mut Journal,
    instance: &InstanceId,
    spawner: &ModelSpawner<LocalControlBridge>,
    completion: &Sender<Command>,
) -> Result<(), ControlError> {
    let mut seen = HashSet::new();
    for message in journal
        .pending()?
        .into_iter()
        .filter(|m| m.instance_id == instance.0)
    {
        if !seen.insert(message.params.agent_id.clone()) || message.state != "queued" {
            continue;
        }
        if message.expires_at <= now() {
            journal.transition(&message, "expired", Some("queue expiry reached"), now())?;
            continue;
        }
        let candidate = message.clone();
        let owner = instance.clone();
        let ready = futures_lite::future::block_on(
            spawner.spawn(move |bridge, ctx| probe(&candidate, bridge, &owner, ctx)),
        )
        .map_err(|_| unavailable())
        .and_then(|result| result);
        let revision = match ready {
            Ok(Readiness::Ready(revision)) => revision,
            Ok(Readiness::Wait) => continue,
            other => {
                let reason = match &other {
                    Ok(Readiness::Cancel(reason)) => *reason,
                    _ => "target or local control is no longer available",
                };
                journal.transition(&message, "cancelled", Some(reason), now())?;
                continue;
            }
        };
        let Some(claimed) = journal.transition(&message, "dispatching", None, now())? else {
            continue;
        };
        let owner = instance.clone();
        let sender = completion.clone();
        let dispatched = futures_lite::future::block_on(spawner.spawn(move |bridge, ctx| {
            // Check again after the durable dispatch claim, immediately before native insertion.
            if !matches!(probe(&claimed, bridge, &owner, ctx), Ok(Readiness::Ready(current)) if current == revision) { return false; }
            let Ok(entry) = super::agents::resolve(&owner, &claimed.params.agent_id, ctx) else { return false; };
            let text = claimed.params.text;
            let id = claimed.params.request_id;
            entry.terminal.update(ctx, |terminal, ctx| {
                terminal.local_control_send_agent_text(text, &revision, move |submitted| {
                    let _ = sender.send(Command::Finished(id, submitted));
                }, ctx)
            })
        }));
        if !matches!(dispatched, Ok(true)) {
            // A bridge error is uncertain; an explicit false means no insertion was initiated.
            let current = journal
                .get(&message.params.request_id)?
                .ok_or_else(unavailable)?;
            journal.transition(
                &current,
                if dispatched.is_err() {
                    "delivery_unknown"
                } else {
                    "failed"
                },
                Some("target changed before insertion"),
                now(),
            )?;
        }
    }
    Ok(())
}
