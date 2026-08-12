use fuzzy_match::FuzzyMatchResult;

use super::*;

fn item(agent: &str, first_prompt: Option<&str>) -> AgentConversationSearchItem {
    AgentConversationSearchItem::new(
        AgentConversation {
            agent: agent.to_string(),
            session_id: "12345678-abcd".to_string(),
            cwd: Some("/tmp/project".to_string()),
            bridge: None,
            start_ts: "2026-07-13T12:00:00Z".to_string(),
            first_prompt: first_prompt.map(str::to_string),
            local_resumable: true,
            flags: String::new(),
            bookmarked: false,
        },
        "resume command".to_string(),
        FuzzyMatchResult::no_match(),
        false,
    )
}

#[test]
fn subtitle_names_the_provider() {
    assert_eq!(
        item("codex", Some("prompt")).subtitle(),
        "/tmp/project · Codex · local"
    );
    assert_eq!(
        item("claude", Some("prompt")).subtitle(),
        "/tmp/project · Claude Code · local"
    );
}

#[test]
fn promptless_title_uses_provider_display_name() {
    assert_eq!(item("codex", None).title(), "Codex session 12345678");
    assert_eq!(item("claude", None).title(), "Claude Code session 12345678");
}

#[test]
fn opening_prompt_is_used_as_the_conversation_title() {
    assert_eq!(
        item("codex", Some("Explain the flaky test")).title(),
        "Explain the flaky test"
    );
}

#[test]
fn providers_use_their_existing_brand_logos() {
    assert_eq!(
        cli_agent_for_resume_provider("claude").and_then(|agent| agent.icon()),
        Some(Icon::ClaudeLogo)
    );
    assert_eq!(
        cli_agent_for_resume_provider("codex").and_then(|agent| agent.icon()),
        Some(Icon::OpenAILogo)
    );
}

#[test]
fn bookmarked_result_resumes_in_the_current_project() {
    let mut item = item("claude", Some("prompt"));
    item.reopen_in_current_project = true;

    assert!(matches!(
        item.accept_result(),
        CommandPaletteItemAction::ReopenAgentConversation {
            use_current_project: true,
            ..
        }
    ));
    assert_eq!(
        item.accessibility_help_message().as_deref(),
        Some("Press enter to resume this conversation in a new tab in the current project.")
    );
}
