//! Private durable observations and inbox checkpoints. Used only on the observation worker.
use std::path::Path;

use ::local_control::agents::AgentEventsParams;
use ::local_control::{ControlError, ErrorCode, InstanceId};
use base64::Engine as _;
use diesel::connection::SimpleConnection;
use diesel::sql_types::{BigInt, Text};
use diesel::{Connection, OptionalExtension, QueryableByName, RunQueryDsl, SqliteConnection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const RETENTION: i64 = 7 * 24 * 60 * 60;
const MAX_EVENTS: i64 = 10_000;
#[derive(QueryableByName)]
struct Body {
    #[diesel(sql_type = Text)]
    body: String,
}
#[derive(QueryableByName)]
struct Number {
    #[diesel(sql_type = BigInt)]
    value: i64,
}
#[derive(QueryableByName)]
struct EventRow {
    #[diesel(sql_type = BigInt)]
    sequence: i64,
    #[diesel(sql_type = Text)]
    body: String,
}
#[derive(Serialize, Deserialize)]
struct Cursor {
    instance: String,
    sequence: i64,
}

pub(super) fn storage_error(_: impl std::fmt::Display) -> ControlError {
    ControlError::new(
        ErrorCode::Internal,
        "coordination observation journal is unavailable; no inbox checkpoint was acknowledged",
    )
}
pub(super) struct Store {
    conn: SqliteConnection,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self, ControlError> {
        let parent = path.parent().ok_or_else(|| storage_error("directory"))?;
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
        let mut conn =
            SqliteConnection::establish(path.to_str().ok_or_else(|| storage_error("path"))?)
                .map_err(storage_error)?;
        conn.batch_execute("PRAGMA busy_timeout=5000; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS observations (sequence INTEGER PRIMARY KEY AUTOINCREMENT, instance TEXT NOT NULL, created_at BIGINT NOT NULL, body TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS observations_instance ON observations(instance, sequence);
            CREATE TABLE IF NOT EXISTS observation_state (key TEXT PRIMARY KEY, updated_at BIGINT NOT NULL, body TEXT NOT NULL);").map_err(storage_error)?;
        Ok(Self { conn })
    }
    pub fn get(&mut self, key: &str) -> Result<Option<Value>, ControlError> {
        diesel::sql_query("SELECT body FROM observation_state WHERE key = ?")
            .bind::<Text, _>(key)
            .get_result::<Body>(&mut self.conn)
            .optional()
            .map_err(storage_error)?
            .map(|row| serde_json::from_str(&row.body).map_err(storage_error))
            .transpose()
    }
    pub fn put(&mut self, key: &str, value: &Value, now: i64) -> Result<(), ControlError> {
        diesel::sql_query("INSERT INTO observation_state(key, updated_at, body) VALUES (?, ?, ?) ON CONFLICT(key) DO UPDATE SET updated_at=excluded.updated_at, body=excluded.body")
            .bind::<Text,_>(key).bind::<BigInt,_>(now).bind::<Text,_>(value.to_string()).execute(&mut self.conn).map_err(storage_error)?;
        Ok(())
    }
    pub fn append(
        &mut self,
        instance: &InstanceId,
        mut value: Value,
        now: i64,
    ) -> Result<(), ControlError> {
        value["recorded_at"] = now.into();
        // Record metadata only; transcript access stays on the bounded read/inbox surface.
        if value.to_string().len() > 32 * 1024 {
            value = json!({"kind": value["kind"], "project_id": value["project_id"], "section_id": value["section_id"],
                "agent_id": value["agent_id"], "previous_section_id": value["previous_section_id"], "section_ids": value["section_ids"], "recorded_at": now, "metadata_truncated": true});
        }
        diesel::sql_query("INSERT INTO observations(instance, created_at, body) VALUES (?, ?, ?)")
            .bind::<Text, _>(&instance.0)
            .bind::<BigInt, _>(now)
            .bind::<Text, _>(value.to_string())
            .execute(&mut self.conn)
            .map_err(storage_error)?;
        self.prune(instance, now)
    }
    fn prune(&mut self, instance: &InstanceId, now: i64) -> Result<(), ControlError> {
        // Global sequence retention bounds total disk usage across app instances and restarts.
        let cutoff =
            diesel::sql_query("SELECT COALESCE(MAX(sequence), 0) - ? AS value FROM observations")
                .bind::<BigInt, _>(MAX_EVENTS)
                .get_result::<Number>(&mut self.conn)
                .map_err(storage_error)?
                .value;
        // Floors are retained per instance, so old cursors never masquerade as an empty page.
        self.conn
            .batch_execute("BEGIN IMMEDIATE")
            .map_err(storage_error)?;
        let result = (|| {
            diesel::sql_query("INSERT INTO observation_state(key, updated_at, body)
                SELECT 'floor:' || instance, ?, CAST(MAX(sequence) AS TEXT) FROM observations
                WHERE created_at < ? OR sequence <= ? GROUP BY instance
                ON CONFLICT(key) DO UPDATE SET updated_at=excluded.updated_at, body=CAST(MAX(CAST(observation_state.body AS INTEGER), CAST(excluded.body AS INTEGER)) AS TEXT)")
                .bind::<BigInt,_>(now).bind::<BigInt,_>(now - RETENTION).bind::<BigInt,_>(cutoff)
                .execute(&mut self.conn).map_err(storage_error)?;
            diesel::sql_query("DELETE FROM observations WHERE created_at < ? OR sequence <= ?")
                .bind::<BigInt, _>(now - RETENTION)
                .bind::<BigInt, _>(cutoff)
                .execute(&mut self.conn)
                .map_err(storage_error)?;
            // Reader checkpoints expire after 90 days of inactivity. First-read coverage is explicit.
            diesel::sql_query("DELETE FROM observation_state WHERE (key LIKE 'batch:%' AND updated_at < ?) OR (key LIKE 'reader:%' AND updated_at < ?) OR (key LIKE 'floor:%' AND updated_at < ?)")
                .bind::<BigInt,_>(now - 600).bind::<BigInt,_>(now - 90 * 86400).bind::<BigInt,_>(now - 90 * 86400)
                .execute(&mut self.conn).map_err(storage_error)?;
            Ok(())
        })();
        self.finish_transaction(result)?;
        let _ = instance;
        Ok(())
    }
    fn finish_transaction(&mut self, result: Result<(), ControlError>) -> Result<(), ControlError> {
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
    pub fn events(
        &mut self,
        instance: &InstanceId,
        params: AgentEventsParams,
        now: i64,
    ) -> Result<Value, ControlError> {
        self.prune(instance, now)?;
        let floor = self
            .get(&format!("floor:{}", instance.0))?
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let mut sequence = floor;
        if let Some(after) = &params.after {
            let cursor: Cursor = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(after)
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .ok_or_else(|| {
                    ControlError::new(ErrorCode::InvalidParams, "invalid event cursor")
                })?;
            if cursor.instance != instance.0 || cursor.sequence < floor || cursor.sequence < 0 {
                return Err(ControlError::new(ErrorCode::StaleTarget, "event cursor belongs to another instance or expired; inspect current state and restart replay"));
            }
            sequence = cursor.sequence;
        }
        let newest = diesel::sql_query(
            "SELECT COALESCE(MAX(sequence), 0) AS value FROM observations WHERE instance = ?",
        )
        .bind::<Text, _>(&instance.0)
        .get_result::<Number>(&mut self.conn)
        .map_err(storage_error)?
        .value
        .max(floor);
        if sequence > newest {
            return Err(ControlError::new(
                ErrorCode::InvalidParams,
                "event cursor is beyond the journal",
            ));
        }
        let rows = diesel::sql_query("SELECT sequence, body FROM observations WHERE instance = ? AND sequence > ? ORDER BY sequence LIMIT 1000")
            .bind::<Text,_>(&instance.0).bind::<BigInt,_>(sequence).load::<EventRow>(&mut self.conn).map_err(storage_error)?;
        let mut events = Vec::new();
        let mut bytes = 0;
        for row in rows {
            let mut value: Value = serde_json::from_str(&row.body).map_err(storage_error)?;
            let matches = scope_matches(&params, &value);
            if matches
                && (!events.is_empty()
                    && (events.len() >= params.limit || bytes + row.body.len() > 512 * 1024))
            {
                break;
            }
            sequence = row.sequence;
            if matches {
                value["sequence"] = sequence.into();
                bytes += row.body.len();
                events.push(value);
            }
        }
        let cursor = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&Cursor {
                instance: instance.0.clone(),
                sequence,
            })
            .map_err(storage_error)?,
        );
        Ok(
            json!({"action": "agent.events", "events": events, "next_cursor": cursor, "has_more": sequence < newest,
            "retention_seconds": RETENTION, "retention_count": MAX_EVENTS, "older_events_expired": floor > 0,
            "coverage": "recorded_agent_lifecycle_and_organization", "instance_id": instance.0}),
        )
    }
    pub fn maintain_state(&mut self, now: i64) -> Result<(), ControlError> {
        diesel::sql_query("DELETE FROM observation_state WHERE (key LIKE 'batch:%' AND updated_at < ?) OR (key LIKE 'reader:%' AND updated_at < ?)")
            .bind::<BigInt,_>(now - 600).bind::<BigInt,_>(now - 90 * 86400).execute(&mut self.conn).map_err(storage_error)?;
        diesel::sql_query("DELETE FROM observation_state WHERE key IN (SELECT key FROM observation_state WHERE key LIKE 'batch:%' AND json_extract(body, '$.acknowledged') = 1 ORDER BY updated_at DESC LIMIT -1 OFFSET 1000)")
            .execute(&mut self.conn).map_err(storage_error)?;
        Ok(())
    }
    pub fn batch(
        &mut self,
        reader: &str,
        before: &Option<Value>,
        after: &Value,
        now: i64,
    ) -> Result<String, ControlError> {
        self.maintain_state(now)?;
        let count = diesel::sql_query("SELECT COUNT(*) AS value FROM observation_state WHERE key LIKE 'batch:%' AND updated_at >= ? AND json_extract(body, '$.acknowledged') IS NOT 1")
            .bind::<BigInt,_>(now - 600).get_result::<Number>(&mut self.conn).map_err(storage_error)?.value;
        if count >= 100 {
            return Err(ControlError::new(
                ErrorCode::InvalidRequest,
                "too many unacknowledged inbox batches; acknowledge or wait ten minutes",
            ));
        }
        let readers = diesel::sql_query(
            "SELECT COUNT(*) AS value FROM observation_state WHERE key LIKE 'reader:%'",
        )
        .get_result::<Number>(&mut self.conn)
        .map_err(storage_error)?
        .value;
        if before.is_none() && readers >= 256 {
            return Err(ControlError::new(
                ErrorCode::InvalidRequest,
                "inbox reader limit reached",
            ));
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.put(
            &format!("batch:{id}"),
            &json!({"reader": reader, "before": before, "after": after}),
            now,
        )?;
        Ok(id)
    }
    pub fn ack(&mut self, reader: &str, batch: &str, now: i64) -> Result<Value, ControlError> {
        self.maintain_state(now)?;
        self.conn
            .batch_execute("BEGIN IMMEDIATE")
            .map_err(storage_error)?;
        let result = (|| {
            let key = format!("batch:{batch}");
            let row = diesel::sql_query(
                "SELECT body FROM observation_state WHERE key = ? AND updated_at >= ?",
            )
            .bind::<Text, _>(&key)
            .bind::<BigInt, _>(now - 600)
            .get_result::<Body>(&mut self.conn)
            .optional()
            .map_err(storage_error)?
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::StaleTarget,
                    "inbox batch expired or does not exist; read again",
                )
            })?;
            let value: Value = serde_json::from_str(&row.body).map_err(storage_error)?;
            if value["reader"] != reader {
                return Err(ControlError::new(
                    ErrorCode::InvalidParams,
                    "inbox batch belongs to another reader",
                ));
            }
            if value["acknowledged"] == true {
                return Ok(());
            }
            let reader_key = format!("reader:{reader}");
            if self.get(&reader_key)?.unwrap_or(Value::Null) != value["before"] {
                return Err(ControlError::new(
                    ErrorCode::StaleTarget,
                    "inbox checkpoints changed concurrently; read again before acknowledging",
                ));
            }
            self.put(&reader_key, &value["after"], now)?;
            self.put(&key, &json!({"reader": reader, "acknowledged": true}), now)?;
            Ok(())
        })();
        self.finish_transaction(result)?;
        Ok(
            json!({"action": "agent.inbox.ack", "acknowledged": true, "reader_id": reader, "batch_id": batch}),
        )
    }
}
fn scope_matches(params: &AgentEventsParams, value: &Value) -> bool {
    // Boundary/gap markers are relevant to every scope.
    value["kind"]
        .as_str()
        .is_some_and(|kind| matches!(kind, "collection_started" | "collection_gap"))
        || ((params.scope.projects.is_empty()
            || value["project_id"]
                .as_str()
                .is_some_and(|id| params.scope.projects.iter().any(|p| p == id)))
            && (params.scope.sections.is_empty()
                || value["previous_section_id"]
                    .as_str()
                    .is_some_and(|id| params.scope.sections.iter().any(|s| s == id))
                || value["section_id"]
                    .as_str()
                    .is_some_and(|id| params.scope.sections.iter().any(|s| s == id))
                || value["section_ids"].as_array().is_some_and(|ids| {
                    ids.iter().any(|id| {
                        id.as_str()
                            .is_some_and(|id| params.scope.sections.iter().any(|s| s == id))
                    })
                })))
}
