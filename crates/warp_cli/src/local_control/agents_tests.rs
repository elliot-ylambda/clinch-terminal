use clap::Parser as _;

use super::*;
use crate::local_control::ControlArgs;

#[test]
fn parses_cross_project_scope_and_requires_send_revision() {
    assert!(
        ControlArgs::try_parse_from([
            "clinch",
            "agent",
            "list",
            "--project",
            "a",
            "--project",
            "b",
            "--section",
            "s",
            "--pid",
            "10"
        ])
        .is_ok()
    );
    assert!(
        ControlArgs::try_parse_from(["clinch", "agent", "send", "id", "--text", "continue"])
            .is_err()
    );
    assert!(
        ControlArgs::try_parse_from([
            "clinch", "agent", "read", "id", "--tail", "--after", "cursor"
        ])
        .is_err()
    );
    assert!(ControlArgs::try_parse_from(["clinch", "agent", "watch", "--wait", "46"]).is_err());
}

#[test]
fn queue_requires_sender_and_enforces_bounds() {
    let base = vec![
        "clinch",
        "agent",
        "send",
        "id",
        "--text",
        "continue",
        "--expected-revision",
        "revision",
        "--request-id",
        "id",
        "--queue",
    ];
    assert!(ControlArgs::try_parse_from(&base).is_err());
    let mut valid = base;
    valid.extend(["--sender", "sender-id"]);
    assert!(ControlArgs::try_parse_from(&valid).is_ok());
    for expiry in ["0", "86401"] {
        let mut invalid = valid.clone();
        invalid.extend(["--expires-in", expiry]);
        assert!(ControlArgs::try_parse_from(invalid).is_err());
    }
    assert!(
        ControlArgs::try_parse_from(["clinch", "agent", "message", "list", "--limit", "101"])
            .is_err()
    );
}

#[test]
fn prompt_preserves_multiline_text_and_rejects_oversize() {
    assert_eq!(
        read_prompt(Some("line one\nline two".into()), None).unwrap(),
        "line one\nline two"
    );
    assert!(read_prompt(Some("x".repeat(MAX_PROMPT_BYTES + 1)), None).is_err());
    assert!(read_prompt(Some(" \n".into()), None).is_err());
}
