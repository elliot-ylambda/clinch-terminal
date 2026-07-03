use cli_agent_usage::UsageSnapshot;

use super::cli_agent_model_label;
use crate::terminal::CLIAgent;

fn snapshot_with(session_id: &str, model: &str) -> UsageSnapshot {
    let mut snap = UsageSnapshot::default();
    snap.models_by_session
        .insert(session_id.to_string(), model.to_string());
    snap
}

#[test]
fn returns_friendly_label_for_known_session() {
    let snap = snapshot_with("sess-1", "claude-opus-4-8");
    assert_eq!(
        cli_agent_model_label(&snap, CLIAgent::Claude, Some("sess-1")),
        Some("Opus 4.8".to_string())
    );
}

#[test]
fn none_when_no_session_id_or_unknown_agent_or_miss() {
    let snap = snapshot_with("sess-1", "claude-opus-4-8");
    assert_eq!(cli_agent_model_label(&snap, CLIAgent::Claude, None), None);
    assert_eq!(
        cli_agent_model_label(&snap, CLIAgent::Unknown, Some("sess-1")),
        None
    );
    assert_eq!(
        cli_agent_model_label(&snap, CLIAgent::Claude, Some("nope")),
        None
    );
}

#[test]
fn none_when_model_is_noise() {
    let snap = snapshot_with("sess-1", "<synthetic>");
    assert_eq!(
        cli_agent_model_label(&snap, CLIAgent::Claude, Some("sess-1")),
        None
    );
}
