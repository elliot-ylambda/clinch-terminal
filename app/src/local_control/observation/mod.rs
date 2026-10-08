//! App-owned recording and unread checkpoints. Disk and transcript IO stay off the UI thread.
mod store;
use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ::local_control::agents::{
    AgentEventsParams, AgentInboxAckParams, AgentInboxParams, AgentReadParams, AgentRole,
    AgentScope,
};
use ::local_control::{Action, ActionKind, ControlError, ErrorCode, InstanceId};
use serde_json::{json, Value};
use store::Store;
use warpui::AppContext;

use super::conversation::ReadPlan;
use super::delivery::PendingReply;

type Reply = Result<Value, ControlError>;
pub(super) struct ObservationService {
    sender: SyncSender<Command>,
    gap: Arc<AtomicBool>,
}
pub(super) struct Observation {
    agents: BTreeMap<String, Value>,
    projects: BTreeMap<String, Value>,
    signal: Option<Value>,
}
enum Operation {
    Events(AgentEventsParams),
    Inbox(AgentInboxParams, Vec<ReadPlan>),
    Ack(AgentInboxAckParams),
}
enum Command {
    Observe(Observation),
    Request(Operation, async_channel::Sender<Reply>),
}
fn unavailable() -> ControlError {
    ControlError::new(
        ErrorCode::BridgeUnavailable,
        "coordination observation worker is unavailable or busy",
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
                "reader and batch IDs must be UUIDs",
            )
        })
}

impl ObservationService {
    pub fn start(instance: InstanceId) -> Result<Self, ControlError> {
        let path = warp_core::paths::warp_home_config_dir()
            .ok_or_else(unavailable)?
            .join("agent-coordination/observations.sqlite3");
        let (sender, receiver) = mpsc::sync_channel(64);
        let gap = Arc::new(AtomicBool::new(false));
        let worker_gap = gap.clone();
        std::thread::Builder::new().name("clinch-agent-observations".into()).spawn(move || {
            let mut store = Store::open(&path).and_then(|mut store| {
                store.append(&instance, json!({"kind": "collection_started", "instance_id": instance.0}), now())?;
                Ok(store)
            });
            let mut previous = Observation { agents: BTreeMap::new(), projects: BTreeMap::new(), signal: None };
            while let Ok(command) = receiver.recv() {
                let result = match &mut store {
                    Err(error) => { if let Command::Request(_, reply) = command { let _ = reply.try_send(Err(error.clone())); } continue; }
                    Ok(store) => {
                        if worker_gap.swap(false, Ordering::AcqRel) {
                            if let Err(error) = store.append(&instance, json!({"kind": "collection_gap", "reason": "capture_queue_overflow", "recovery": "inspect current workspace and agent state"}), now()) {
                                if let Command::Request(_, reply) = command { let _ = reply.try_send(Err(error.clone())); }
                                Err(error)
                            } else { process(command, store, &instance, &mut previous) }
                        } else { process(command, store, &instance, &mut previous) }
                    }
                };
                if let Err(error) = result { log::warn!("coordination recording stopped: {}", error.message); store = Err(error); }
            }
        }).map_err(|_| unavailable())?;
        Ok(Self { sender, gap })
    }
    pub fn observe(&self, observation: Observation) {
        if self.sender.try_send(Command::Observe(observation)).is_err() {
            self.gap.store(true, Ordering::Release);
        }
    }
    pub fn request(
        &self,
        action: &Action,
        instance: &InstanceId,
        ctx: &AppContext,
    ) -> Result<PendingReply, ControlError> {
        let operation = match action.kind {
            ActionKind::AgentEvents => {
                let params: AgentEventsParams = action.params_as()?;
                if !(1..=500).contains(&params.limit) {
                    return Err(ControlError::new(
                        ErrorCode::InvalidParams,
                        "event limit must be 1–500",
                    ));
                }
                Operation::Events(params)
            }
            ActionKind::AgentInbox => {
                let mut params: AgentInboxParams = action.params_as()?;
                params.reader_id = uuid(&params.reader_id)?;
                if !(1..=100).contains(&params.limit) {
                    return Err(ControlError::new(
                        ErrorCode::InvalidParams,
                        "inbox limit must be 1–100",
                    ));
                }
                let snapshot = super::agents::snapshot(instance, &params.scope, ctx)?;
                if snapshot.agents.len() > 100 {
                    return Err(ControlError::new(
                        ErrorCode::InvalidParams,
                        "inbox scope exceeds 100 agents; narrow --project or --section",
                    ));
                }
                let plans = snapshot
                    .agents
                    .into_iter()
                    .map(|entry| ReadPlan {
                        params: AgentReadParams {
                            agent_id: entry.data["agent_id"]
                                .as_str()
                                .unwrap_or_default()
                                .to_owned(),
                            after: None,
                            limit: params.limit,
                            tail: true,
                            role: Some(AgentRole::Assistant),
                            messages_only: true,
                        },
                        agent: compact_agent(entry.data),
                        provider: entry.provider,
                        session_id: entry.conversation_id,
                        transcript_path: entry.transcript_path,
                        remote: entry.remote,
                    })
                    .collect();
                Operation::Inbox(params, plans)
            }
            ActionKind::AgentInboxAck => {
                let mut params: AgentInboxAckParams = action.params_as()?;
                params.reader_id = uuid(&params.reader_id)?;
                params.batch_id = uuid(&params.batch_id)?;
                Operation::Ack(params)
            }
            _ => {
                return Err(ControlError::new(
                    ErrorCode::UnsupportedAction,
                    "not an observation action",
                ))
            }
        };
        let (sender, receiver) = async_channel::bounded(1);
        self.sender
            .try_send(Command::Request(operation, sender))
            .map_err(|_| unavailable())?;
        Ok(receiver)
    }
}
fn process(
    command: Command,
    store: &mut Store,
    instance: &InstanceId,
    previous: &mut Observation,
) -> Result<(), ControlError> {
    match command {
        Command::Observe(mut observation) => {
            if let Some(signal) = observation.signal.take() {
                let entries = if signal["kind"] == "agent.ended" {
                    &previous.agents
                } else {
                    &observation.agents
                };
                let agent = entries.values().find(|agent| {
                    agent["terminal_id"] == signal["terminal_id"]
                        && agent["conversation_id"] == signal["conversation_id"]
                });
                let mut event = agent.cloned().unwrap_or_else(|| signal.clone());
                event["kind"] = signal["kind"].clone();
                event["native_status"] = signal["status"].clone();
                store.append(instance, event, now())?;
            }
            record_changes(
                store,
                instance,
                "agent",
                &previous.agents,
                &observation.agents,
            )?;
            record_changes(
                store,
                instance,
                "project",
                &previous.projects,
                &observation.projects,
            )?;
            *previous = observation;
        }
        Command::Request(operation, reply) => {
            let result = match operation {
                Operation::Events(params) => store.events(instance, params, now()),
                Operation::Inbox(params, plans) => inbox(store, params, plans),
                Operation::Ack(params) => store.ack(&params.reader_id, &params.batch_id, now()),
            };
            let _ = reply.try_send(result);
        }
    }
    Ok(())
}
fn record_changes(
    store: &mut Store,
    instance: &InstanceId,
    family: &str,
    before: &BTreeMap<String, Value>,
    after: &BTreeMap<String, Value>,
) -> Result<(), ControlError> {
    for (id, value) in after {
        if before.get(id) != Some(value) {
            let mut event = value.clone();
            if let Some(old) = before.get(id) {
                event["previous_section_id"] = old["section_id"].clone();
                let mut sections = value["section_ids"].as_array().cloned().unwrap_or_default();
                for id in old["section_ids"].as_array().into_iter().flatten() {
                    if !sections.contains(id) {
                        sections.push(id.clone());
                    }
                }
                event["section_ids"] = sections.into();
            }
            event["kind"] = format!(
                "{family}.{}",
                if before.contains_key(id) {
                    "changed"
                } else {
                    "discovered"
                }
            )
            .into();
            store.append(instance, event, now())?;
        }
    }
    for (id, value) in before {
        if !after.contains_key(id) {
            let mut event = value.clone();
            event["kind"] = format!("{family}.removed").into();
            store.append(instance, event, now())?;
        }
    }
    Ok(())
}
fn compact_agent(mut agent: Value) -> Value {
    for field in [
        "latest_prompt",
        "latest_response_preview",
        "tool_input_preview",
    ] {
        agent.as_object_mut().unwrap().remove(field);
    }
    agent
}
pub(super) fn capture(
    instance: &InstanceId,
    signal: Option<Value>,
    ctx: &AppContext,
) -> Result<Observation, ControlError> {
    let snapshot = super::agents::snapshot(instance, &AgentScope::default(), ctx)?;
    let agents = snapshot
        .agents
        .into_iter()
        .map(|entry| {
            let mut data = compact_agent(entry.data);
            // Readiness changes can be sampled by other metadata events, but are not a replay guarantee.
            for key in [
                "input_revision",
                "ready",
                "unavailable_reason",
                "capabilities",
            ] {
                data.as_object_mut().unwrap().remove(key);
            }
            data["terminal_id"] = entry.terminal.id().to_string().into();
            (data["agent_id"].as_str().unwrap().to_owned(), data)
        })
        .collect();
    let mut projects = BTreeMap::new();
    for window in snapshot.tree["windows"].as_array().into_iter().flatten() {
        for project in window["projects"].as_array().into_iter().flatten() {
            let mut value = project.clone();
            value["window_id"] = window["window_id"].clone();
            value["section_ids"] = json!(project["sections"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|section| section["section_id"].clone())
                .collect::<Vec<_>>());
            // Pane output and process state are recorded separately. Project events carry layout only.
            for tab in value["tabs"].as_array_mut().into_iter().flatten() {
                tab.as_object_mut().unwrap().remove("panes");
            }
            for task in value["tasks"].as_array_mut().into_iter().flatten() {
                use std::hash::{Hash, Hasher};
                let mut hash = std::collections::hash_map::DefaultHasher::new();
                task["text"].as_str().hash(&mut hash);
                task["text_revision"] = format!("{:016x}", hash.finish()).into();
                task.as_object_mut().unwrap().remove("text");
            }
            projects.insert(value["project_id"].as_str().unwrap().to_owned(), value);
        }
    }
    Ok(Observation {
        agents,
        projects,
        signal,
    })
}
fn inbox(
    store: &mut Store,
    params: AgentInboxParams,
    plans: Vec<ReadPlan>,
) -> Result<Value, ControlError> {
    collect_inbox(store, params, plans, ReadPlan::execute)
}
fn collect_inbox(
    store: &mut Store,
    params: AgentInboxParams,
    plans: Vec<ReadPlan>,
    mut read: impl FnMut(ReadPlan) -> Reply,
) -> Reply {
    store.maintain_state(now())?;
    let before = store.get(&format!("reader:{}", params.reader_id))?;
    let mut checkpoints = before.clone().unwrap_or_else(|| json!({}));
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    let mut bytes = 0;
    let mut deferred = 0;
    for mut plan in plans {
        let key = json!([plan.provider.as_str(), plan.session_id]).to_string();
        if plan.session_id.is_some() && !seen.insert(key.clone()) {
            continue;
        }
        let checkpoint = checkpoints.get(&key).cloned().unwrap_or(Value::Null);
        let state = json!([plan.agent["state"], plan.agent["unavailable_reason"]]);
        let agent = plan.agent.clone();
        // Checkpoints follow a provider conversation through tab moves and app restarts.
        plan.params.agent_id = key.clone();
        plan.params.after = checkpoint["cursor"].as_str().map(str::to_owned);
        plan.params.tail = plan.params.after.is_none();
        let result = read(plan);
        let (mut item, next) = match result {
            Ok(page) => {
                let next = page["next_cursor"]
                    .as_str()
                    .map(|cursor| json!({"cursor": cursor, "state": state}));
                (page, next)
            }
            Err(error) => (json!({"agent": agent, "error": error}), None),
        };
        item["state_changed"] = (checkpoint["state"] != state).into();
        item["initial_read"] = checkpoint.is_null().into();
        let visible = item["records"]
            .as_array()
            .is_some_and(|records| !records.is_empty())
            || item["state_changed"] == true
            || item.get("error").is_some()
            || item["has_more"] == true;
        if visible {
            let size = item.to_string().len();
            if bytes + size > 512 * 1024 {
                deferred += 1;
                continue;
            }
            bytes += size;
            items.push(item);
        }
        if let Some(next) = next {
            checkpoints[&key] = next;
        }
    }
    if checkpoints
        .as_object()
        .is_none_or(|entries| entries.len() > 2048)
    {
        return Err(ControlError::new(
            ErrorCode::InvalidRequest,
            "reader checkpoint limit reached; use a new reader UUID",
        ));
    }
    let batch = if before.as_ref() != Some(&checkpoints) {
        Some(store.batch(&params.reader_id, &before, &checkpoints, now())?)
    } else {
        None
    };
    Ok(
        json!({"action": "agent.inbox", "reader_id": params.reader_id, "items": items, "batch_id": batch,
        "deferred_conversations": deferred, "checkpoint_retention_days": 90, "acknowledged": false,
        "coverage": "assistant_text_from_attached_local_transcripts", "first_read": "latest_messages_with_older_coverage_disclosed"}),
    )
}

#[cfg(test)]
mod tests;
