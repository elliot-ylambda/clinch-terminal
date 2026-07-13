use super::*;

fn conversation(agent: &str) -> AgentConversation {
    AgentConversation {
        agent: agent.to_string(),
        session_id: "session-123".to_string(),
        cwd: Some("/tmp/project".to_string()),
        bridge: None,
        start_ts: "2026-07-13T12:00:00Z".to_string(),
        first_prompt: Some("fix the flaky test".to_string()),
        flags: String::new(),
    }
}

#[test]
fn searchable_text_includes_provider_name() {
    let codex = searchable_text(&conversation("codex"));
    assert!(codex.contains("codex"));
    assert!(codex.contains("Codex"));

    let claude = searchable_text(&conversation("claude"));
    assert!(claude.contains("claude"));
    assert!(claude.contains("Claude Code"));
}
