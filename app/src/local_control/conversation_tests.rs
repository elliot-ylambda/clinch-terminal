use std::io::Write as _;

use super::*;

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
    assert!(normalized_record(
        AgentResumeProvider::Codex,
        &json!({"type": "event_msg", "payload": {"type": "agent_message", "message": "duplicate"}})
    )
    .is_none());
    assert!(normalized_record(
        AgentResumeProvider::Codex,
        &json!({"type": "response_item", "payload": {"type": "reasoning", "summary": "private"}})
    )
    .is_none());
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
