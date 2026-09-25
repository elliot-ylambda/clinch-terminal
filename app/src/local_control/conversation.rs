//! Bounded provider transcript reads. Executed off the app/UI thread.
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::io::{BufRead as _, Read as _, Seek as _};
use std::path::Path;

use ::local_control::agents::{AgentReadParams, MAX_READ_BYTES};
use ::local_control::{ControlError, ErrorCode};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::agent_resume::{coordination_transcript_path, AgentResumeProvider};

pub(super) struct ReadPlan {
    pub params: AgentReadParams,
    pub agent: Value,
    pub provider: AgentResumeProvider,
    pub session_id: Option<String>,
    pub transcript_path: Option<String>,
    pub remote: bool,
}

#[derive(Serialize, Deserialize)]
struct Cursor {
    agent: String,
    source: String,
    offset: u64,
    anchor: u64,
}

const SCAN_BYTES: u64 = 4 * 1024 * 1024;

impl ReadPlan {
    pub fn execute(self) -> Result<Value, ControlError> {
        let path = if self.remote {
            None
        } else {
            self.session_id.as_deref().and_then(|id| {
                coordination_transcript_path(
                    self.provider,
                    id,
                    self.transcript_path.as_deref().map(Path::new),
                )
            })
        };
        let Some(path) = path else {
            if self.params.after.is_some() {
                return Err(ControlError::new(
                    ErrorCode::StaleTarget,
                    "conversation source is unavailable; rediscover without a cursor",
                ));
            }
            return Ok(
                json!({"action": "agent.read", "agent": self.agent, "coverage": "preview_only", "source": "provider_notifications", "records": [], "next_cursor": null, "has_more": false, "reason": "No readable local provider transcript is attached; inspect latest previews in agent metadata."}),
            );
        };
        let mut result = read_path(&path, self.provider, &self.params)?;
        result["agent"] = self.agent;
        result["action"] = json!("agent.read");
        Ok(result)
    }
}

fn io_error(_: std::io::Error) -> ControlError {
    ControlError::new(
        ErrorCode::InvalidRequest,
        "could not read the attached provider transcript",
    )
}

fn anchor(file: &mut std::fs::File, offset: u64) -> Result<u64, ControlError> {
    file.seek(std::io::SeekFrom::Start(offset.saturating_sub(128)))
        .map_err(io_error)?;
    let mut bytes = vec![0; offset.min(128) as usize];
    file.read_exact(&mut bytes).map_err(io_error)?;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hash);
    Ok(hash.finish())
}

fn read_path(
    path: &Path,
    provider: AgentResumeProvider,
    params: &AgentReadParams,
) -> Result<Value, ControlError> {
    let mut file = std::fs::File::open(path).map_err(io_error)?;
    let metadata = file.metadata().map_err(io_error)?;
    #[cfg(unix)]
    let source = {
        use std::os::unix::fs::MetadataExt as _;
        format!("{}:{}", metadata.dev(), metadata.ino())
    };
    #[cfg(not(unix))]
    let source = format!("{}:{:?}", path.display(), metadata.created().ok());
    let mut start = if params.tail {
        metadata.len().saturating_sub(SCAN_BYTES)
    } else {
        0
    };
    if let Some(after) = &params.after {
        let cursor: Cursor = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(after)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .ok_or_else(|| {
                ControlError::new(ErrorCode::InvalidParams, "invalid conversation cursor")
            })?;
        if cursor.agent != params.agent_id
            || cursor.source != source
            || cursor.offset > metadata.len()
            || anchor(&mut file, cursor.offset)? != cursor.anchor
        {
            return Err(ControlError::new(
                ErrorCode::StaleTarget,
                "conversation cursor source changed or was truncated; restart the read",
            ));
        }
        start = cursor.offset;
    }
    file.seek(std::io::SeekFrom::Start(start))
        .map_err(io_error)?;
    let mut reader = std::io::BufReader::new(file.take(SCAN_BYTES));
    let mut position = start;
    if params.tail && start > 0 {
        // Starting in a record: explicitly mark older coverage as omitted.
        let mut discard = Vec::new();
        position += reader.read_until(b'\n', &mut discard).map_err(io_error)? as u64;
    }
    let mut records = VecDeque::new();
    let mut total_bytes = 0;
    let mut malformed = 0;
    let mut pending = false;
    let mut scan_limit_reached = false;
    let mut output_truncated = false;
    loop {
        let record_start = position;
        let mut bytes = Vec::new();
        let read = reader.read_until(b'\n', &mut bytes).map_err(io_error)?;
        if read == 0 {
            break;
        }
        if bytes.last() != Some(&b'\n') {
            if start + SCAN_BYTES < metadata.len() {
                if record_start == start {
                    return Err(ControlError::new(ErrorCode::InvalidRequest, "transcript record exceeds the 4 MiB scan boundary; use --tail for recent records"));
                }
                scan_limit_reached = true;
                break;
            }
            pending = true;
            break;
        }
        let parsed: Value = match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(_) => {
                malformed += 1;
                position += read as u64;
                continue;
            }
        };
        let Some(mut record) = normalized_record(provider, &parsed) else {
            position += read as u64;
            continue;
        };
        record["id"] = json!(format!("{source}:{record_start}"));
        record["timestamp"] = parsed
            .get("timestamp")
            .and_then(Value::as_str)
            .map(|s| json!(bounded_text_at(s, 128).0))
            .unwrap_or(Value::Null);
        let size = record.to_string().len();
        if !params.tail && !records.is_empty() && total_bytes + size > MAX_READ_BYTES {
            output_truncated = true;
            break;
        }
        position += read as u64;
        records.push_back(record);
        total_bytes += size;
        while params.tail && (records.len() > params.limit || total_bytes > MAX_READ_BYTES) {
            if let Some(removed) = records.pop_front() {
                total_bytes -= removed.to_string().len();
                output_truncated = true;
            }
        }
        if !params.tail && records.len() >= params.limit {
            break;
        }
    }
    let mut file = reader.into_inner().into_inner();
    let cursor = Cursor {
        agent: params.agent_id.clone(),
        source,
        offset: position,
        anchor: anchor(&mut file, position)?,
    };
    let next_cursor = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&cursor).expect("cursor serialization"));
    Ok(
        json!({"source": "provider_transcript", "coverage": "partial", "coverage_detail": "Supported captured user, assistant, and tool records; images, reasoning, and provider metadata are omitted.", "records": records, "next_cursor": next_cursor, "has_more": position < metadata.len(), "pending_record": pending, "scan_limit_reached": scan_limit_reached, "malformed_records": malformed, "older_content_omitted": params.tail && (start > 0 || output_truncated), "output_truncated": output_truncated, "bytes_scanned": position - start}),
    )
}

fn bounded_text(text: &str) -> (String, bool) {
    bounded_text_at(text, 16 * 1024)
}

fn bounded_text_at(text: &str, max: usize) -> (String, bool) {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_owned(), end < text.len())
}

fn normalized_record(provider: AgentResumeProvider, record: &Value) -> Option<Value> {
    let payload = match provider {
        AgentResumeProvider::Claude => record.get("message")?,
        AgentResumeProvider::Codex => {
            // response_item is canonical. event_msg repeats the same messages.
            if record.get("type")?.as_str()? != "response_item" {
                return None;
            }
            record.get("payload")?
        }
    };
    let kind = payload
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("message");
    let role = payload
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("tool");
    if !matches!(role, "user" | "assistant" | "tool") {
        return None;
    }
    let mut text = Vec::new();
    let mut tools = Vec::new();
    if kind == "message" {
        match payload.get("content")? {
            Value::String(value) => text.push(value.clone()),
            Value::Array(blocks) => {
                for block in blocks {
                    match block.get("type").and_then(Value::as_str) {
                    Some("text" | "input_text" | "output_text") => if let Some(value) = block.get("text").and_then(Value::as_str) { text.push(value.to_owned()); },
                    Some("tool_use") => tools.push(json!({"type": "tool_call", "name": block["name"], "id": block["id"], "text": bounded_text(&block["input"].to_string()).0})),
                    Some("tool_result") => tools.push(json!({"type": "tool_result", "id": block["tool_use_id"], "text": bounded_text(&block["content"].to_string()).0})),
                    _ => {},
                }
                }
            }
            _ => return None,
        }
    } else if matches!(
        kind,
        "function_call" | "function_call_output" | "custom_tool_call" | "custom_tool_call_output"
    ) {
        let content = payload
            .get("arguments")
            .or_else(|| payload.get("input"))
            .or_else(|| payload.get("output"))
            .cloned()
            .unwrap_or(Value::Null);
        tools.push(json!({"type": kind, "name": payload["name"], "id": payload["call_id"], "text": bounded_text(content.as_str().unwrap_or(&content.to_string())).0}));
    } else {
        return None;
    }
    if text.is_empty() && tools.is_empty() {
        return None;
    }
    let (text, text_truncated) = bounded_text(&text.join("\n"));
    let tools_truncated = tools.len() > 8;
    tools.truncate(8);
    for tool in &mut tools {
        for (key, max) in [("name", 256), ("id", 256), ("text", 2048)] {
            if let Some(value) = tool.get(key).and_then(Value::as_str) {
                let (value, truncated) = bounded_text_at(value, max);
                tool[key] = json!(value);
                if truncated {
                    tool["truncated"] = json!(true);
                }
            } else if key != "text" {
                tool[key] = Value::Null;
            }
        }
    }
    Some(
        json!({"role": role, "text": text, "text_truncated": text_truncated, "tools": tools, "tools_truncated": tools_truncated}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn params() -> AgentReadParams {
        AgentReadParams {
            agent_id: "bound-conversation".into(),
            after: None,
            limit: 1,
            tail: false,
        }
    }

    fn message(text: &str) -> String {
        format!(
            "{}\n",
            json!({"type": "response_item", "payload": {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": text}]}})
        )
    }

    #[test]
    fn paginates_complete_records_and_waits_for_partial_tail() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        write!(file, "{}{}", message("first"), message("second")).unwrap();
        let partial = message("third");
        file.write_all(&partial.as_bytes()[..20]).unwrap();
        let mut request = params();
        let first = read_path(file.path(), AgentResumeProvider::Codex, &request).unwrap();
        assert_eq!(first["records"][0]["text"], "first");
        request.after = first["next_cursor"].as_str().map(str::to_owned);
        let second = read_path(file.path(), AgentResumeProvider::Codex, &request).unwrap();
        assert_eq!(second["records"][0]["text"], "second");
        request.after = second["next_cursor"].as_str().map(str::to_owned);
        let pending = read_path(file.path(), AgentResumeProvider::Codex, &request).unwrap();
        assert!(pending["records"].as_array().unwrap().is_empty());
        assert_eq!(pending["pending_record"], true);
        assert_eq!(pending["next_cursor"], second["next_cursor"]);
        file.write_all(&partial.as_bytes()[20..]).unwrap();
        let third = read_path(file.path(), AgentResumeProvider::Codex, &request).unwrap();
        assert_eq!(third["records"][0]["text"], "third");
        assert_eq!(third["has_more"], false);
    }

    #[test]
    fn rejects_cursor_from_another_conversation_or_rewritten_file() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(message("first").as_bytes()).unwrap();
        let mut request = params();
        let result = read_path(file.path(), AgentResumeProvider::Codex, &request).unwrap();
        request.after = result["next_cursor"].as_str().map(str::to_owned);
        request.agent_id = "another-conversation".into();
        assert!(read_path(file.path(), AgentResumeProvider::Codex, &request).is_err());
        request.agent_id = params().agent_id;
        file.as_file_mut().set_len(0).unwrap();
        assert!(read_path(file.path(), AgentResumeProvider::Codex, &request).is_err());
    }

    #[test]
    fn tail_reads_latest_messages_beyond_old_five_megabyte_limit() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let old = format!(
            "{}\n",
            json!({"type": "metadata", "padding": "x".repeat(1024)})
        );
        for _ in 0..6000 {
            file.write_all(old.as_bytes()).unwrap();
        }
        file.write_all(message("latest answer").as_bytes()).unwrap();
        let mut request = params();
        request.tail = true;
        let result = read_path(file.path(), AgentResumeProvider::Codex, &request).unwrap();
        assert_eq!(result["records"][0]["text"], "latest answer");
        assert_eq!(result["older_content_omitted"], true);
        assert_eq!(result["has_more"], false);
    }

    #[test]
    fn codex_uses_response_records_and_omits_reasoning() {
        assert!(normalized_record(AgentResumeProvider::Codex, &json!({"type": "event_msg", "payload": {"type": "agent_message", "message": "duplicate"}})).is_none());
        assert!(normalized_record(AgentResumeProvider::Codex, &json!({"type": "response_item", "payload": {"type": "reasoning", "summary": "private"}})).is_none());
        let result = normalized_record(AgentResumeProvider::Codex, &json!({"type": "response_item", "payload": {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "done"}]}})).unwrap();
        assert_eq!(result["text"], "done");
    }

    #[test]
    fn claude_retains_tool_results_and_omits_thinking() {
        let result = normalized_record(AgentResumeProvider::Claude, &json!({"message": {"role": "assistant", "content": [{"type": "thinking", "thinking": "private"}, {"type": "text", "text": "hello"}, {"type": "tool_use", "id": "x", "name": "Read", "input": {"path": "file"}}]}})).unwrap();
        assert_eq!(result["text"], "hello");
        assert_eq!(result["tools"][0]["name"], "Read");
        assert!(!result.to_string().contains("private"));
    }
}
