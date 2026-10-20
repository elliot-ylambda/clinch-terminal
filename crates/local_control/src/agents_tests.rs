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

#[test]
fn pane_read_format_defaults_to_ansi_and_is_omitted_on_the_wire() {
    let legacy: PaneReadParams =
        serde_json::from_value(serde_json::json!({"pane_id": "p", "max_bytes": 10})).unwrap();
    assert_eq!(legacy.format, PaneReadFormat::Ansi);
    // Apps that predate the field reject unknown keys, so the default must not be sent.
    assert!(
        serde_json::to_value(&legacy)
            .unwrap()
            .get("format")
            .is_none()
    );
    let plain: PaneReadParams = serde_json::from_value(
        serde_json::json!({"pane_id": "p", "max_bytes": 10, "format": "plain"}),
    )
    .unwrap();
    assert_eq!(plain.format, PaneReadFormat::Plain);
    assert_eq!(serde_json::to_value(&plain).unwrap()["format"], "plain");
    assert!("html".parse::<PaneReadFormat>().is_err());
}
