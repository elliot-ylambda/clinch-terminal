//! Private, durable delivery journal. All database access runs on the delivery worker.
use std::path::Path;

use ::local_control::agents::{AgentMessageListParams, AgentSendParams};
use ::local_control::{ControlError, ErrorCode};
use diesel::connection::SimpleConnection;
use diesel::sql_types::{BigInt, Text};
use diesel::{Connection, OptionalExtension, QueryableByName, RunQueryDsl, SqliteConnection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub(super) const RETENTION_SECONDS: i64 = 7 * 24 * 60 * 60;
const MAX_PENDING: usize = 100;
const MAX_PER_TARGET: usize = 10;
const MAX_RECEIPTS: i64 = 10_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Message {
    #[serde(default)]
    pub sequence: i64,
    pub params: AgentSendParams,
    pub instance_id: String,
    pub owner_pid: u32,
    pub guard: String,
    pub state: String,
    pub reason: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub expires_at: i64,
}

impl Message {
    pub fn new(params: AgentSendParams, instance_id: String, guard: String, now: i64) -> Self {
        Self {
            sequence: 0,
            expires_at: now + i64::from(params.expires_in),
            params,
            instance_id,
            owner_pid: std::process::id(),
            guard,
            state: "queued".into(),
            reason: None,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn receipt(&self, include_prompt: bool) -> Value {
        let mut value = json!({
            "action": "agent.send", "request_id": self.params.request_id,
            "agent_id": self.params.agent_id, "instance_id": self.instance_id,
            "sender_id": self.params.sender_id, "sequence": self.sequence,
            "state": self.state, "reason": self.reason, "created_at": self.created_at,
            "updated_at": self.updated_at, "expires_at": self.expires_at,
            "text_bytes": self.params.text.len(), "idempotency_scope": "seven_day_journal",
            "durable": true, "provider_acceptance": "unconfirmed",
        });
        if include_prompt {
            value["text"] = json!(self.params.text);
        }
        value
    }
}

#[derive(QueryableByName)]
struct Row {
    #[diesel(sql_type = BigInt)]
    sequence: i64,
    #[diesel(sql_type = Text)]
    body: String,
}

impl Row {
    fn message(self) -> Result<Message, ControlError> {
        let mut message: Message = serde_json::from_str(&self.body).map_err(storage_error)?;
        message.sequence = self.sequence;
        Ok(message)
    }
}

#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type = BigInt)]
    value: i64,
}

pub(super) fn storage_error(_: impl std::fmt::Display) -> ControlError {
    ControlError::new(
        ErrorCode::Internal,
        "delivery journal is unavailable; no unjournaled message will be sent",
    )
}

pub(super) struct Journal {
    conn: SqliteConnection,
}

impl Journal {
    pub fn open(path: &Path) -> Result<Self, ControlError> {
        let parent = path
            .parent()
            .ok_or_else(|| storage_error("missing directory"))?;
        std::fs::create_dir_all(parent).map_err(storage_error)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
                .map_err(storage_error)?;
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(path)
                .map_err(storage_error)?;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .map_err(storage_error)?;
        }
        let path = path
            .to_str()
            .ok_or_else(|| storage_error("invalid journal path"))?;
        let conn = SqliteConnection::establish(path).map_err(storage_error)?;
        let mut journal = Self { conn };
        journal
            .conn
            .batch_execute(
                "PRAGMA busy_timeout=5000; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS messages (
                sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                request_id TEXT NOT NULL UNIQUE,
                instance_id TEXT NOT NULL,
                state TEXT NOT NULL,
                updated_at BIGINT NOT NULL,
                body TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS messages_pending ON messages(state, sequence);",
            )
            .map_err(storage_error)?;
        Ok(journal)
    }

    pub fn get(&mut self, id: &str) -> Result<Option<Message>, ControlError> {
        diesel::sql_query("SELECT sequence, body FROM messages WHERE request_id = ?")
            .bind::<Text, _>(id)
            .get_result::<Row>(&mut self.conn)
            .optional()
            .map_err(storage_error)?
            .map(Row::message)
            .transpose()
    }

    pub fn pending(&mut self) -> Result<Vec<Message>, ControlError> {
        diesel::sql_query("SELECT sequence, body FROM messages WHERE state IN ('queued', 'dispatching') ORDER BY sequence")
            .load::<Row>(&mut self.conn).map_err(storage_error)?.into_iter().map(Row::message).collect()
    }

    pub fn existing(&mut self, params: &AgentSendParams) -> Result<Option<Message>, ControlError> {
        let message = self.get(&params.request_id)?;
        if message.as_ref().is_some_and(|m| m.params != *params) {
            return Err(ControlError::new(
                ErrorCode::InvalidParams,
                "request ID already belongs to different content, target, or queue options",
            ));
        }
        Ok(message)
    }

    pub fn insert(&mut self, message: Message) -> Result<Message, ControlError> {
        // UUID claim, limits, and insert are one transaction across concurrent app instances.
        self.conn
            .batch_execute("BEGIN IMMEDIATE")
            .map_err(storage_error)?;
        let result = self.insert_inner(message);
        if result.is_ok() {
            if let Err(error) = self.conn.batch_execute("COMMIT") {
                let _ = self.conn.batch_execute("ROLLBACK");
                return Err(storage_error(error));
            }
        } else {
            let _ = self.conn.batch_execute("ROLLBACK");
        }
        result
    }

    fn insert_inner(&mut self, message: Message) -> Result<Message, ControlError> {
        if let Some(existing) = self.existing(&message.params)? {
            return Ok(existing);
        }
        let pending = self.pending()?;
        let own: Vec<_> = pending
            .iter()
            .filter(|m| m.instance_id == message.instance_id)
            .collect();
        let target: Vec<_> = own
            .iter()
            .filter(|m| m.params.agent_id == message.params.agent_id)
            .collect();
        if own.len() >= MAX_PENDING || target.len() >= MAX_PER_TARGET {
            return Err(ControlError::new(
                ErrorCode::InvalidRequest,
                "message queue is full (100 pending per app instance, 10 per target)",
            ));
        }
        if target
            .iter()
            .any(|m| m.params.sender_id != message.params.sender_id)
        {
            return Err(ControlError::new(
                ErrorCode::InvalidRequest,
                "another sender owns this target's pending queue; inspect or cancel it first",
            ));
        }
        if !message.params.queue && !target.is_empty() {
            return Err(ControlError::new(
                ErrorCode::InvalidRequest,
                "target has pending messages; use --queue with the same sender to preserve order",
            ));
        }
        let count = diesel::sql_query("SELECT COUNT(*) AS value FROM messages")
            .get_result::<Count>(&mut self.conn)
            .map_err(storage_error)?;
        if count.value >= MAX_RECEIPTS {
            return Err(ControlError::new(
                ErrorCode::InvalidRequest,
                "delivery journal has reached its 10000-receipt retention limit",
            ));
        }
        let body = serde_json::to_string(&message).map_err(storage_error)?;
        diesel::sql_query("INSERT INTO messages (request_id, instance_id, state, updated_at, body) VALUES (?, ?, ?, ?, ?)")
            .bind::<Text, _>(&message.params.request_id).bind::<Text, _>(&message.instance_id)
            .bind::<Text, _>(&message.state).bind::<BigInt, _>(message.updated_at).bind::<Text, _>(body)
            .execute(&mut self.conn).map_err(storage_error)?;
        self.get(&message.params.request_id)?
            .ok_or_else(|| storage_error("missing inserted message"))
    }

    /// Compare-and-set prevents a cancel/recovery in another app racing a dispatch claim.
    pub fn transition(
        &mut self,
        message: &Message,
        state: &str,
        reason: Option<&str>,
        now: i64,
    ) -> Result<Option<Message>, ControlError> {
        let mut next = message.clone();
        next.state = state.to_owned();
        next.reason = reason.map(str::to_owned);
        next.updated_at = now;
        let body = serde_json::to_string(&next).map_err(storage_error)?;
        let changed = diesel::sql_query("UPDATE messages SET state = ?, updated_at = ?, body = ? WHERE request_id = ? AND state = ?")
            .bind::<Text, _>(state).bind::<BigInt, _>(now).bind::<Text, _>(body)
            .bind::<Text, _>(&message.params.request_id).bind::<Text, _>(&message.state)
            .execute(&mut self.conn).map_err(storage_error)?;
        Ok((changed == 1).then_some(next))
    }

    pub fn maintain(&mut self, now: i64, alive: impl Fn(u32) -> bool) -> Result<(), ControlError> {
        let mut liveness = std::collections::HashMap::new();
        for message in self.pending()? {
            let change = if !*liveness
                .entry(message.owner_pid)
                .or_insert_with(|| alive(message.owner_pid))
            {
                if message.state == "dispatching" {
                    Some((
                        "delivery_unknown",
                        "app exited during dispatch; inspect the transcript before a new send",
                    ))
                } else {
                    Some((
                        "cancelled",
                        "app exited; rediscover the target before creating a new message",
                    ))
                }
            } else if message.state == "queued" && message.expires_at <= now {
                Some(("expired", "queue expiry reached"))
            } else {
                None
            };
            if let Some((state, reason)) = change {
                self.transition(&message, state, Some(reason), now)?;
            }
        }
        diesel::sql_query(
            "DELETE FROM messages WHERE state NOT IN ('queued', 'dispatching') AND updated_at < ?",
        )
        .bind::<BigInt, _>(now - RETENTION_SECONDS)
        .execute(&mut self.conn)
        .map_err(storage_error)?;
        Ok(())
    }

    pub fn list(&mut self, params: &AgentMessageListParams) -> Result<Value, ControlError> {
        if !(1..=100).contains(&params.limit) || params.before.is_some_and(|n| n <= 0) {
            return Err(ControlError::new(
                ErrorCode::InvalidParams,
                "limit must be 1–100 and before must be a positive receipt sequence",
            ));
        }
        let rows = diesel::sql_query("SELECT sequence, body FROM messages WHERE sequence < ? AND (? = '' OR json_extract(body, '$.params.agent_id') = ?) ORDER BY sequence DESC LIMIT ?")
            .bind::<BigInt, _>(params.before.unwrap_or(i64::MAX))
            .bind::<Text, _>(params.agent_id.as_deref().unwrap_or(""))
            .bind::<Text, _>(params.agent_id.as_deref().unwrap_or(""))
            .bind::<BigInt, _>(i64::from(params.limit) + 1).load::<Row>(&mut self.conn).map_err(storage_error)?;
        let mut messages = rows
            .into_iter()
            .map(Row::message)
            .collect::<Result<Vec<_>, _>>()?;
        let more = messages.len() > params.limit as usize;
        messages.truncate(params.limit as usize);
        Ok(
            json!({"action": "agent.message.list", "messages": messages.iter().map(|m| m.receipt(false)).collect::<Vec<_>>(),
            "next_before": if more { messages.last().map(|m| m.sequence) } else { None }, "has_more": more, "retention_seconds": RETENTION_SECONDS}),
        )
    }
}

#[cfg(test)]
#[path = "journal_tests.rs"]
mod tests;
