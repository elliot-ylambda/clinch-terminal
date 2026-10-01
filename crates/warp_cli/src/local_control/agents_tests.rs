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

fn read_options(args: &[&str]) -> AgentReadOptions {
    let args = ControlArgs::try_parse_from(
        ["clinch", "agent", "read", "id"]
            .into_iter()
            .chain(args.iter().copied()),
    )
    .unwrap();
    let crate::local_control::ControlCommand::Agent(AgentCommand::Read { options, .. }) =
        args.command
    else {
        panic!("expected agent read");
    };
    options
}

#[test]
fn recent_reads_default_to_three_and_history_requires_an_explicit_mode() {
    let recent = read_options(&[]).params("id".into());
    assert!(recent.tail);
    assert_eq!(recent.limit, 3);
    for args in [
        &["--last", "1"][..],
        &["--last", "2"],
        &["--tail", "--limit", "2"],
    ] {
        let recent = read_options(args).params("id".into());
        assert!(recent.tail);
        assert_eq!(recent.limit.to_string(), *args.last().unwrap());
    }
    for args in [&["--from-start"][..], &["--after", "cursor"], &["--all"]] {
        let history = read_options(args).params("id".into());
        assert!(!history.tail);
        assert_eq!(history.limit, 100);
    }
    let history = read_options(&["--after", "cursor", "--limit", "7"]).params("id".into());
    assert_eq!(history.after.as_deref(), Some("cursor"));
    assert_eq!(history.limit, 7);
}

#[test]
fn read_modes_reject_ambiguous_selection_and_invalid_limits() {
    for args in [
        vec!["--all", "--tail"],
        vec!["--all", "--from-start"],
        vec!["--all", "--after", "cursor"],
        vec!["--from-start", "--tail"],
        vec!["--from-start", "--after", "cursor"],
        vec!["--last", "0"],
        vec!["--limit", "501"],
    ] {
        assert!(
            ControlArgs::try_parse_from(["clinch", "agent", "read", "id"].into_iter().chain(args),)
                .is_err()
        );
    }
}

#[test]
fn full_history_streams_pages_and_stops_at_an_incomplete_provider_record() {
    let mut requests = Vec::new();
    let mut pages = Vec::new();
    read_all_pages(
        read_options(&["--all", "--limit", "1"]).params("id".into()),
        |params| {
            assert!(!params.tail);
            assert_eq!(params.limit, 1);
            requests.push(params.after.clone());
            Ok(match params.after.as_deref() {
                None => serde_json::json!({"records":[{"text":"first"}], "has_more":true, "next_cursor":"one", "coverage":"partial"}),
                Some("one") => serde_json::json!({"records":[{"text":"second"}], "has_more":true, "next_cursor":"two", "coverage":"partial"}),
                Some("two") => serde_json::json!({"records":[], "has_more":true, "pending_record":true, "next_cursor":"two", "coverage":"partial"}),
                _ => panic!("unexpected cursor"),
            })
        },
        |page| { pages.push(page.clone()); Ok(()) },
    ).unwrap();
    assert_eq!(requests, vec![None, Some("one".into()), Some("two".into())]);
    assert_eq!(pages.len(), 3);
    assert_eq!(pages[1]["records"][0]["text"], "second");
    assert!(pages.iter().all(|page| page["coverage"] == "partial"));
}

#[test]
fn full_history_stops_at_end_and_rejects_stalled_pagination() {
    let params = read_options(&["--all"]).params("id".into());
    let mut calls = 0;
    read_all_pages(
        params.clone(),
        |_| {
            calls += 1;
            Ok(serde_json::json!({"has_more":false}))
        },
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(calls, 1);
    let error = read_all_pages(
        params,
        |_| Ok(serde_json::json!({"has_more":true, "next_cursor":"same"})),
        |_| Ok(()),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
}

#[test]
fn full_history_requires_streaming_output_before_resolving_an_app() {
    let args = ControlArgs::try_parse_from(["clinch", "agent", "read", "id", "--all"]).unwrap();
    let crate::local_control::ControlCommand::Agent(command) = args.command else {
        panic!("agent command");
    };
    assert_eq!(
        run_agent(command, OutputFormat::Json).unwrap_err().code,
        ErrorCode::InvalidParams
    );
}

#[test]
fn parses_filtered_reads_and_bounded_lifecycle_commands() {
    let read = read_options(&["--role", "assistant", "--messages-only", "--last", "2"])
        .params("id".into());
    assert_eq!(read.role, Some(local_control::agents::AgentRole::Assistant));
    assert!(read.messages_only);
    assert_eq!(read.limit, 2);
    assert!(
        ControlArgs::try_parse_from(["clinch", "agent", "read", "id", "--role", "system"]).is_err()
    );
    for args in [
        vec![
            "clinch",
            "agent",
            "wait",
            "id",
            "--until",
            "ready",
            "--timeout",
            "120",
        ],
        vec![
            "clinch",
            "agent",
            "wait",
            "id",
            "--until",
            "turn-complete",
            "--timeout",
            "1",
        ],
        vec![
            "clinch",
            "agent",
            "launch",
            "--provider",
            "codex",
            "--project",
            "p",
            "--background",
        ],
        vec![
            "clinch",
            "agent",
            "inbox",
            "--reader",
            "r",
            "--peek",
            "--project",
            "p",
        ],
        vec!["clinch", "agent", "events", "--since", "cursor", "--follow"],
    ] {
        assert!(ControlArgs::try_parse_from(args).is_ok());
    }
    assert!(
        ControlArgs::try_parse_from(["clinch", "agent", "launch", "--provider", "claude"]).is_err()
    );
    assert!(ControlArgs::try_parse_from(["clinch", "agent", "interrupt", "id"]).is_err());
    assert!(
        ControlArgs::try_parse_from(["clinch", "agent", "wait", "id", "--timeout", "86401"])
            .is_err()
    );
    assert!(!condition_matches(
        WaitCondition::Ready,
        &serde_json::json!({"state": "unknown"})
    ));
    assert!(condition_matches(
        WaitCondition::Attention,
        &serde_json::json!({"state": "rate_limited"})
    ));
    assert!(!condition_matches(
        WaitCondition::TurnComplete,
        &serde_json::json!({"state": "working", "ready": false})
    ));
}
