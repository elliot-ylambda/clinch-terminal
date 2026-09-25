use super::*;

#[test]
fn send_requires_identity_revision_and_request_id() {
    assert!(
        serde_json::from_value::<AgentSendParams>(serde_json::json!({
            "agent_id": "pane", "text": "continue"
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<AgentScope>(serde_json::json!({
            "project": "typo-would-broaden-scope"
        }))
        .is_err()
    );
}
