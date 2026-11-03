use std::io::Write;

use super::*;

#[test]
fn codex_launch_prefix_is_pane_bound_only_in_local_clinch() {
    for app_id in ["sh.clinch.Clinch", "sh.clinch.ClinchDev"] {
        assert_eq!(
            AgentResumeProvider::Codex.launch_command_prefix_for(app_id, true),
            "clinch_codex_launch"
        );
        assert_eq!(
            AgentResumeProvider::Codex.launch_command_prefix_for(app_id, false),
            "codex"
        );
        assert_eq!(
            AgentResumeProvider::Claude.launch_command_prefix_for(app_id, true),
            "claude"
        );
    }
    assert_eq!(
        AgentResumeProvider::Codex.launch_command_prefix_for("dev.warp.Warp-Stable", true),
        "codex"
    );
}

#[test]
fn shutdown_owner_preserves_exact_pane_metadata_across_global_marker_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let uuid = vec![0xab, 0xcd];
    let entry = br#"{"command":"clinch_agent_resume_launch claude session-1","cwd":"/repo","owner_pid":"123","owner_tty":"ttys001"}"#;
    write_private_atomic(dir.path(), "abcd.json", entry).unwrap();
    write_private_atomic(dir.path(), APP_TERMINATING_FILE, b"456\n").unwrap();
    preserve_shutdown_owner_in(dir.path(), &uuid).unwrap();
    std::fs::remove_file(dir.path().join(APP_TERMINATING_FILE)).unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("shutdown-owners/abcd.json")).unwrap(),
        entry
    );
    preserve_shutdown_owner_in(dir.path(), &[0xef]).unwrap();
    assert!(!dir.path().join("shutdown-owners/ef.json").exists());
}

#[test]
fn prompt_mirror_version_tracks_creation_append_and_legacy_provider_scope() {
    let registry = tempfile::tempdir().unwrap();
    let version = |provider, id| prompt_mirror_version_in(provider, id, Some(registry.path()));
    let claude = AgentResumeProvider::Claude;
    let codex = AgentResumeProvider::Codex;
    let missing = version(claude, "session");
    std::fs::create_dir_all(registry.path().join("prompts/claude")).unwrap();
    let legacy = registry.path().join("prompts/session.jsonl");
    std::fs::write(&legacy, "{\"prompt\":\"first\"}\n").unwrap();
    let created = version(claude, "session");
    assert_ne!(missing, created);
    assert_eq!(created, version(claude, "session"));
    assert_eq!(missing, version(codex, "session"));
    std::fs::OpenOptions::new()
        .append(true)
        .open(&legacy)
        .unwrap()
        .write_all(b"{\"prompt\":\"second\"}\n")
        .unwrap();
    assert_ne!(created, version(claude, "session"));
    let appended = version(claude, "session");
    std::fs::write(registry.path().join("prompts/claude/session.jsonl"), "{}\n").unwrap();
    assert_ne!(appended, version(claude, "session"));
    assert_eq!(missing, version(claude, "../session"));
}

#[cfg(target_os = "macos")]
#[test]
fn capture_installer_failure_includes_compact_stderr() {
    use std::os::unix::process::ExitStatusExt;

    let status = std::process::ExitStatus::from_raw(71 << 8);
    let message = capture_installer_failure(
        status,
        b"install: /bundle/prompt-mirror.sh: No such file or directory\n",
    );

    assert_eq!(
        message,
        "install: /bundle/prompt-mirror.sh: No such file or directory (exit status: 71)"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn capture_state_is_shared_by_production_and_dev_apps() {
    let base = Path::new("/Users/test/Library/Application Support");
    assert_eq!(
        default_capture_state_dir(base),
        base.join("sh.clinch.Clinch/agent-integration")
    );
}

#[cfg(target_os = "macos")]
#[test]
fn session_capture_defaults_on_but_preserves_current_and_legacy_opt_outs() {
    assert_eq!(
        capture_startup_action(false, false, false),
        CaptureStartupAction::Enable
    );
    assert_eq!(
        capture_startup_action(true, false, false),
        CaptureStartupAction::Repair
    );
    assert_eq!(
        capture_startup_action(false, true, false),
        CaptureStartupAction::Skip
    );
    assert_eq!(
        capture_startup_action(false, false, true),
        CaptureStartupAction::Skip
    );
}

#[test]
fn prompt_title_prefers_the_first_sentence_within_the_limit() {
    assert_eq!(
        prompt_title("  Fix the failing test. Then update the docs.  "),
        Some("Fix the failing test.".to_owned())
    );
}

#[test]
fn prompt_titles_unwrap_pasted_content_before_selecting_a_sentence() {
    for text in [
        "<pasted_content id='d961'>Fix the sidebar. Then run tests.</pasted_content>",
        "<pasted_content id=\"d961\">Fix the sidebar. Then run tests.</pasted_content>",
        "<pasted_content>Fix the sidebar. Then run tests.",
    ] {
        assert_eq!(prompt_title(text).as_deref(), Some("Fix the sidebar."));
    }
    assert_eq!(
        clean_prompt_title_text("Compare <pasted_content id='a'>one</pasted_content> and <pasted_content id='b'>two</pasted_content>").as_deref(),
        Some("Compare one and two")
    );
    assert_eq!(
        prompt_title("<pasted_content id='a'> </pasted_content>"),
        None
    );
    assert_eq!(
        prompt_title("Fix <div> markup").as_deref(),
        Some("Fix <div> markup")
    );
    assert_eq!(
        prompt_title("Keep <pasted_content_example> intact").as_deref(),
        Some("Keep <pasted_content_example> intact")
    );
}

#[test]
fn prompt_title_collapses_whitespace_and_truncates_at_graphemes() {
    let long = format!("{} tail", "🧑🏽‍💻".repeat(80));
    let title = prompt_title(&long).unwrap();
    assert_eq!(
        UnicodeSegmentation::graphemes(title.trim_end_matches('…'), true).count(),
        80
    );
    assert!(title.ends_with('…'));
    assert_eq!(
        prompt_title("  first\n\nsecond  "),
        Some("first second".to_owned())
    );
    assert_eq!(prompt_title(" \n "), None);
}

#[test]
fn prompt_timestamp_formats_as_local_time() {
    let timestamp = "2026-07-14T14:32:05Z";
    let local = chrono::DateTime::parse_from_rfc3339(timestamp)
        .unwrap()
        .with_timezone(&chrono::Local);

    assert_eq!(
        format_prompt_time_short(Some(timestamp)),
        Some(local.format("%-I:%M %p").to_string())
    );
    assert_eq!(
        format_prompt_time_full(Some(timestamp)),
        Some(local.format("%b %-d, %-I:%M %p").to_string())
    );
}

#[test]
fn prompt_timestamp_formatting_rejects_missing_or_invalid_values() {
    assert_eq!(format_prompt_time_short(None), None);
    assert_eq!(format_prompt_time_short(Some("garbage")), None);
    assert_eq!(format_prompt_time_full(None), None);
    assert_eq!(format_prompt_time_full(Some("garbage")), None);
}

#[test]
fn direct_resume_command_yields_the_resumed_session_id() {
    const ID: &str = "019fbab0-2150-7f93-9b10-7195f6c800e8";
    let codex = AgentResumeProvider::Codex;
    let claude = AgentResumeProvider::Claude;

    for command in [
        format!("codex resume {ID} --dangerously-bypass-approvals-and-sandbox --model gpt-5.6-sol"),
        format!("cx resume {ID}"),
        format!("clinch_codex_launch resume {ID} --model gpt-6-astra"),
        format!("codex --model x resume '{ID}' --search"),
    ] {
        assert_eq!(
            session_id_from_direct_resume_command(codex, &command).as_deref(),
            Some(ID),
            "{command}"
        );
    }
    for command in [
        format!("claude --resume {ID} --dangerously-skip-permissions"),
        format!("ca -r {ID}"),
        format!("claude --resume={ID}"),
    ] {
        assert_eq!(
            session_id_from_direct_resume_command(claude, &command).as_deref(),
            Some(ID),
            "{command}"
        );
    }

    for (provider, command) in [
        // Forks mint a new session; the parent id is not this pane's conversation.
        (codex, format!("codex fork {ID}")),
        (claude, format!("claude --resume {ID} --fork-session")),
        // No explicit id (interactive picker / most recent) or a non-UUID token.
        (codex, "codex resume --last".to_owned()),
        (codex, "codex resume".to_owned()),
        (claude, "claude --resume".to_owned()),
        (claude, "claude --resume not-a-session".to_owned()),
        // An id outside resume position, e.g. quoted in a fresh prompt.
        (codex, format!("codex 'look at {ID}'")),
        (codex, format!("codex resume --last 'look at {ID}'")),
        (claude, format!("claude 'look at {ID}'")),
    ] {
        assert_eq!(
            session_id_from_direct_resume_command(provider, &command),
            None,
            "{command}"
        );
    }
}

#[test]
fn restore_command_seed_accepts_current_and_legacy_launchers() {
    assert_eq!(
        agent_session_seed_from_restore_command(
            "clinch_agent_resume_launch claude 0190-abcd --model opus"
        ),
        Some((AgentResumeProvider::Claude, "0190-abcd".to_string()))
    );
    assert_eq!(
        agent_session_seed_from_restore_command("warp_agent_resume_launch codex legacy-456"),
        Some((AgentResumeProvider::Codex, "legacy-456".to_string()))
    );
    assert_eq!(
        agent_session_seed_from_restore_command("claude --resume x"),
        None
    );
    assert_eq!(
        agent_session_seed_from_restore_command("clinch_agent_resume_launch gemini x"),
        None
    );
    assert_eq!(
        agent_session_seed_from_restore_command("clinch_agent_resume_launch codex ../escape"),
        None
    );
}

#[test]
fn production_and_dev_clinch_are_the_only_supported_agent_resume_apps() {
    assert!(app_id_enables_runtime("sh.clinch.Clinch"));
    assert!(app_id_enables_runtime("sh.clinch.ClinchDev"));
    assert!(!app_id_enables_runtime("dev.warp.Warp-Local"));
    assert!(!app_id_enables_runtime("dev.warp.WarpOss"));
}

#[test]
fn agent_resume_runtime_requires_capture_or_an_explicit_test_override() {
    assert!(!runtime_enabled_for("sh.clinch.Clinch", false, false));
    assert!(runtime_enabled_for("sh.clinch.Clinch", true, false));
    assert!(!runtime_enabled_for("dev.warp.Warp-Local", true, false));
    assert!(runtime_enabled_for("dev.warp.Warp-Local", false, true));
}

#[test]
fn reads_command_from_registry_file() {
    let dir = std::env::temp_dir().join(format!("agent_resume_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut f = std::fs::File::create(dir.join("deadbeef.json")).unwrap();
    write!(
        f,
        r#"{{ "command": "claude --resume abc-123", "cwd": "/tmp" }}"#
    )
    .unwrap();

    assert_eq!(
        read_command_in(&dir, "deadbeef"),
        Some("claude --resume abc-123".to_string())
    );
    assert_eq!(read_command_in(&dir, "missing"), None);
}

#[test]
fn tolerates_bridge_field_in_registry_file() {
    // The capture hook records the claude.ai bridge id in an optional "bridge" field, which
    // the shell replay side consumes; the Rust reader must keep parsing entries that carry it.
    let dir = std::env::temp_dir().join(format!("agent_resume_bridge_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut f = std::fs::File::create(dir.join("beefcafe.json")).unwrap();
    write!(
        f,
        r#"{{ "command": "clinch_agent_resume_launch claude abc-123", "cwd": "/tmp", "bridge": "session_01XYZ" }}"#
    )
    .unwrap();

    assert_eq!(
        read_command_in(&dir, "beefcafe"),
        Some("clinch_agent_resume_launch claude abc-123".to_string())
    );
    let launch = read_fork_launch_in(&dir, "beefcafe").unwrap();
    assert_eq!(launch.command, "claude --resume abc-123 --fork-session");
}

#[test]
fn uuid_hex_is_lowercase() {
    // Must match $WARP_TERMINAL_SESSION_UUID casing.
    assert_eq!(hex::encode([0xAB, 0xCD]), "abcd");
}

#[test]
fn derives_claude_fork_command() {
    assert_eq!(
        derive_fork_command("clinch_agent_resume_launch claude abc-123").as_deref(),
        Some("claude --resume abc-123 --fork-session")
    );
}

#[test]
fn normalizes_and_parses_legacy_warp_launcher() {
    assert_eq!(
        normalize_restore_command("warp_agent_resume_launch claude legacy-123".to_string()),
        "clinch_agent_resume_launch claude legacy-123"
    );
    assert_eq!(
        derive_fork_command("warp_agent_resume_launch codex legacy-456").as_deref(),
        Some(
            format!(
                "{} fork legacy-456",
                AgentResumeProvider::Codex.launch_command_prefix(true)
            )
            .as_str()
        )
    );
    assert_eq!(
        normalize_restore_command("warp_agent_resume_launcher claude untouched".to_string()),
        "warp_agent_resume_launcher claude untouched"
    );
}

#[test]
fn derives_codex_fork_command() {
    assert_eq!(
        derive_fork_command("clinch_agent_resume_launch codex abc-123").as_deref(),
        Some(
            format!(
                "{} fork abc-123",
                AgentResumeProvider::Codex.launch_command_prefix(true)
            )
            .as_str()
        )
    );
}

#[test]
fn remote_resume_command_is_typed_and_rejects_shell_syntax() {
    assert_eq!(
        build_remote_resume_command(AgentResumeProvider::Claude, "abc-123").as_deref(),
        Some("clinch_agent_resume_launch claude abc-123")
    );
    assert_eq!(
        build_remote_resume_command(AgentResumeProvider::Codex, "019a1234-5678").as_deref(),
        Some("clinch_agent_resume_launch codex 019a1234-5678")
    );
    for rejected in [
        "",
        "has space",
        "id;touch-pwned",
        "$(command)",
        "../session",
    ] {
        assert_eq!(
            build_remote_resume_command(AgentResumeProvider::Claude, rejected),
            None,
            "{rejected}"
        );
    }
}

#[test]
fn fork_command_carries_launch_flags() {
    assert_eq!(
        derive_fork_command(
            "clinch_agent_resume_launch claude abc-123 --dangerously-skip-permissions --model opus"
        )
        .as_deref(),
        Some("claude --resume abc-123 --dangerously-skip-permissions --model opus --fork-session")
    );
    assert_eq!(
        derive_fork_command(
            "clinch_agent_resume_launch codex abc-123 --dangerously-bypass-approvals-and-sandbox"
        )
        .as_deref(),
        Some(
            format!(
                "{} fork abc-123 --dangerously-bypass-approvals-and-sandbox",
                AgentResumeProvider::Codex.launch_command_prefix(true)
            )
            .as_str()
        )
    );
}

#[test]
fn no_fork_command_for_unknown() {
    assert_eq!(derive_fork_command("vim"), None);
    assert_eq!(derive_fork_command(""), None);
    // Pre-launcher registry formats are dead; nothing writes them anymore.
    assert_eq!(derive_fork_command("claude --resume abc-123"), None);
    assert_eq!(derive_fork_command("codex resume abc-123"), None);
    // Unknown agents and missing ids are not forkable.
    assert_eq!(
        derive_fork_command("clinch_agent_resume_launch gemini abc-123"),
        None
    );
    assert_eq!(
        derive_fork_command("clinch_agent_resume_launch claude"),
        None
    );
    assert_eq!(
        derive_fork_command("clinch_agent_resume_launch claude "),
        None
    );
}

#[test]
fn codex_fork_uses_live_identity_without_capture_and_rejects_stale_flags() {
    let resolve = |entry| {
        resolve_fork_launch(entry, Some((AgentResumeProvider::Codex, "live-child")))
            .expect("a known Codex conversation can fork without a registry")
    };
    let expected = format!(
        "{} fork live-child",
        AgentResumeProvider::Codex.launch_command_prefix(true)
    );
    assert_eq!(resolve(None).command, expected);
    let stale = resolve_fork_launch(
        Some(RegistryEntry {
            command: "clinch_agent_resume_launch codex stale-parent --model wrong-model".into(),
            cwd: Some("/unrelated/directory".into()),
            owner_pid: None,
            owner_tty: None,
        }),
        Some((AgentResumeProvider::Codex, "live-child")),
    );
    assert!(
        stale.is_none(),
        "conflicts wait for verified owner reconciliation"
    );
    let newer_child = RegistryEntry {
        command: "clinch_agent_resume_launch codex live-child".into(),
        cwd: None,
        owner_pid: Some("123".into()),
        owner_tty: Some("ttys-test".into()),
    };
    assert!(resolve_fork_launch(
        Some(newer_child),
        Some((AgentResumeProvider::Codex, "stale-parent"))
    )
    .is_none());
    let matching = resolve(Some(RegistryEntry {
        command: "clinch_agent_resume_launch codex live-child --model original-model".into(),
        cwd: Some("/correct/directory".into()),
        owner_pid: None,
        owner_tty: None,
    }));
    assert_eq!(
        matching.command,
        format!("{expected} --model original-model")
    );
    assert_eq!(matching.cwd.as_deref(), Some("/correct/directory"));
    assert!(resolve_fork_launch(None, None).is_none());
    assert!(resolve_fork_launch(None, Some((AgentResumeProvider::Codex, "bad;id"))).is_none());
}

#[test]
fn claude_fork_retains_matching_settings_and_rejects_another_conversation() {
    let entry = || RegistryEntry {
        command: "clinch_agent_resume_launch claude saved-id --permission-mode plan --model opus"
            .into(),
        cwd: Some("/source".into()),
        owner_pid: None,
        owner_tty: None,
    };
    let launch = resolve_fork_launch(
        Some(entry()),
        Some((AgentResumeProvider::Claude, "saved-id")),
    )
    .unwrap();
    assert_eq!(
        launch.command,
        "claude --resume saved-id --permission-mode plan --model opus --fork-session"
    );
    assert!(resolve_fork_launch(
        Some(entry()),
        Some((AgentResumeProvider::Claude, "different-id"))
    )
    .is_none());
}

fn conversation(
    agent: &str,
    session_id: &str,
    bridge: Option<&str>,
    flags: &str,
    local_resumable: bool,
) -> AgentConversation {
    AgentConversation {
        agent: agent.to_string(),
        session_id: session_id.to_string(),
        cwd: None,
        bridge: bridge.map(str::to_string),
        start_ts: "2026-07-09T10:00:00Z".to_string(),
        first_prompt: None,
        local_resumable,
        flags: flags.to_string(),
        bookmarked: false,
    }
}

#[test]
fn reopen_command_teleports_cloud_only_claude_sessions() {
    assert_eq!(
        conversation("claude", "abc-123", Some("session_01XYZ"), "", false).reopen_command(),
        Some("claude --teleport session_01XYZ".to_string())
    );
    assert_eq!(
        conversation(
            "claude",
            "abc-123",
            Some("session_01XYZ"),
            " --model opus",
            false,
        )
        .reopen_command(),
        Some("claude --teleport session_01XYZ --model opus".to_string())
    );
}

#[test]
fn reopen_command_resumes_local_claude_sessions() {
    assert_eq!(
        conversation("claude", "abc-123", Some("session_01XYZ"), "", true).reopen_command(),
        Some("claude --resume abc-123".to_string())
    );
    // A bridge that is not claude.ai-shaped (session_*) is not teleported — same guard
    // as the shell replay side.
    assert_eq!(
        conversation("claude", "abc-123", Some("garbage"), "", false).reopen_command(),
        Some("claude --resume abc-123".to_string())
    );
    assert_eq!(
        conversation(
            "claude",
            "abc-123",
            None,
            " --dangerously-skip-permissions",
            true,
        )
        .reopen_command(),
        Some("claude --resume abc-123 --dangerously-skip-permissions".to_string())
    );
}

#[test]
fn reopen_command_resumes_codex_sessions_and_rejects_unknown_agents() {
    assert_eq!(
        conversation(
            "codex",
            "xyz-9",
            None,
            " --dangerously-bypass-approvals-and-sandbox",
            true,
        )
        .reopen_command(),
        Some(format!(
            "{} resume xyz-9 --dangerously-bypass-approvals-and-sandbox",
            AgentResumeProvider::Codex.launch_command_prefix(true)
        ))
    );
    assert_eq!(
        conversation("gemini", "abc", None, "", false).reopen_command(),
        None
    );
}

#[test]
fn recent_conversations_aggregates_journal_and_mirror() {
    let dir = std::env::temp_dir().join(format!("agent_resume_recent_test_{}", std::process::id()));
    let prompts = dir.join("prompts");
    std::fs::create_dir_all(prompts.join("claude")).unwrap();
    std::fs::create_dir_all(prompts.join("codex")).unwrap();

    // Same fixture shape as tools/agent-resume/tests/test_registry_journal.sh: an
    // unbridged conversation, a conversation that bridges (and gains flags) on a later
    // write, a Codex conversation, a remove (ignored), a malformed line and a non-launch
    // command (skipped), plus a mirror-only nested session that must not leak into the
    // in-app finder.
    std::fs::write(
        dir.join("journal.jsonl"),
        concat!(
            r#"{"ts":"2026-07-09T09:00:00Z","op":"write","pane":"pane-c","command":"warp_agent_resume_launch codex sid-codex-ddd","cwd":"/tmp/projD","bridge":""}"#,
            "\n",
            r#"{"ts":"2026-07-09T10:00:00Z","op":"write","pane":"pane-a","command":"warp_agent_resume_launch claude sid-oldest-aaa","cwd":"/tmp/projA","bridge":""}"#,
            "\n",
            r#"{"ts":"2026-07-09T11:00:00Z","op":"write","pane":"pane-b","command":"warp_agent_resume_launch claude sid-bridged-bbb","cwd":"/tmp/projB","bridge":""}"#,
            "\n",
            r#"{"ts":"2026-07-09T11:30:00Z","op":"write","pane":"pane-b","command":"warp_agent_resume_launch claude sid-bridged-bbb --model opus","cwd":"/tmp/projB","bridge":"session_01LISTBRIDGE"}"#,
            "\n",
            r#"{"ts":"2026-07-09T11:45:00Z","op":"remove","pane":"pane-a"}"#,
            "\n",
            r#"{"ts":"2026-07-09T11:50:00Z","op":"write","pane":"pane-x","command":"vim","cwd":"/tmp","bridge":""}"#,
            "\n",
            "not json at all\n",
        ),
    )
    .unwrap();
    std::fs::write(
        prompts.join("claude/sid-bridged-bbb.jsonl"),
        "{\"ts\":\"2026-07-09T11:00:05Z\",\"cwd\":\"/tmp/projB\",\"bridge\":\"\",\"prompt\":\"fix the flaky test in ci\"}\n",
    )
    .unwrap();
    std::fs::write(
        prompts.join("sid-nested-ccc.jsonl"),
        "{\"ts\":\"2026-07-09T12:00:00Z\",\"cwd\":\"/tmp/projC\",\"bridge\":\"\",\"prompt\":\"nested run\\nprompt\"}\n",
    )
    .unwrap();
    std::fs::write(
        prompts.join("sid-codex-ddd.jsonl"),
        "{\"ts\":\"2026-07-09T09:00:05Z\",\"cwd\":\"/tmp/projD\",\"bridge\":\"\",\"prompt\":\"must not enrich a Codex session\"}\n",
    )
    .unwrap();
    std::fs::write(
        prompts.join("codex/sid-codex-ddd.jsonl"),
        "{\"ts\":\"2026-07-09T09:00:05Z\",\"cwd\":\"/tmp/projD\",\"bridge\":\"\",\"prompt\":\"Codex scoped prompt\"}\n",
    )
    .unwrap();

    let conversations = recent_conversations_in(&dir, 50);
    assert_eq!(
        conversations
            .iter()
            .map(|c| c.session_id.as_str())
            .collect::<Vec<_>>(),
        // Only journal-backed Clinch sessions are listed, newest first.
        vec!["sid-bridged-bbb", "sid-oldest-aaa", "sid-codex-ddd"]
    );

    let bridged = &conversations[0];
    assert_eq!(bridged.agent, "claude");
    assert_eq!(bridged.start_ts, "2026-07-09T11:00:00Z");
    assert_eq!(bridged.cwd.as_deref(), Some("/tmp/projB"));
    // Latest write wins for bridge + flags.
    assert_eq!(bridged.bridge.as_deref(), Some("session_01LISTBRIDGE"));
    assert_eq!(bridged.flags, " --model opus");
    assert_eq!(
        bridged.first_prompt.as_deref(),
        Some("fix the flaky test in ci")
    );

    assert_eq!(
        conversations
            .iter()
            .find(|conversation| conversation.session_id == "sid-nested-ccc"),
        None,
        "mirror-only background sessions must stay out of the in-app finder"
    );

    let oldest = &conversations[1];
    assert_eq!(oldest.bridge, None);
    assert_eq!(oldest.first_prompt, None);
    assert_eq!(oldest.flags, "");

    let codex = &conversations[2];
    assert_eq!(codex.agent, "codex");
    assert_eq!(codex.first_prompt.as_deref(), Some("Codex scoped prompt"));

    // The limit keeps the newest conversations.
    let capped = recent_conversations_in(&dir, 1);
    assert_eq!(
        capped
            .iter()
            .map(|c| c.session_id.as_str())
            .collect::<Vec<_>>(),
        vec!["sid-bridged-bbb"]
    );

    // A missing registry directory yields an empty list, not an error.
    assert!(recent_conversations_in(&dir.join("does-not-exist"), 50).is_empty());
}

#[test]
fn conversation_bookmarks_toggle_persist_and_validate_session_ids() {
    let dir = std::env::temp_dir().join(format!(
        "agent_resume_bookmarks_test_{}",
        uuid::Uuid::new_v4()
    ));

    assert!(read_conversation_bookmarks_in(&dir).is_empty());
    assert!(
        toggle_conversation_bookmark_in(&dir, AgentResumeProvider::Claude, "claude-session-1")
            .unwrap()
    );
    assert!(
        toggle_conversation_bookmark_in(&dir, AgentResumeProvider::Codex, "codex-session-2")
            .unwrap()
    );

    let bookmarks = read_conversation_bookmarks_in(&dir);
    assert_eq!(bookmarks.len(), 2);
    assert!(bookmarks.contains(&ConversationBookmark {
        provider: AgentResumeProvider::Claude,
        session_id: "claude-session-1".to_string(),
    }));

    assert!(!toggle_conversation_bookmark_in(
        &dir,
        AgentResumeProvider::Claude,
        "claude-session-1"
    )
    .unwrap());
    assert_eq!(read_conversation_bookmarks_in(&dir).len(), 1);
    assert!(
        toggle_conversation_bookmark_in(&dir, AgentResumeProvider::Claude, "../escape").is_err()
    );

    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn conversation_bookmarks_can_be_set_idempotently() {
    let dir = std::env::temp_dir().join(format!(
        "agent_resume_set_bookmarks_test_{}",
        uuid::Uuid::new_v4()
    ));

    for _ in 0..2 {
        assert!(set_conversation_bookmark_in(
            &dir,
            AgentResumeProvider::Claude,
            "claude-session-1",
            true,
        )
        .unwrap());
    }
    assert_eq!(read_conversation_bookmarks_in(&dir).len(), 1);

    for _ in 0..2 {
        assert!(!set_conversation_bookmark_in(
            &dir,
            AgentResumeProvider::Claude,
            "claude-session-1",
            false,
        )
        .unwrap());
    }
    assert!(read_conversation_bookmarks_in(&dir).is_empty());
    assert!(
        set_conversation_bookmark_in(&dir, AgentResumeProvider::Codex, "../escape", true,).is_err()
    );
    assert!(read_conversation_bookmarks_in(&dir).is_empty());

    if dir.exists() {
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn finder_keeps_bookmarks_older_than_the_recent_pool_limit() {
    let dir = std::env::temp_dir().join(format!(
        "agent_resume_old_bookmark_test_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("journal.jsonl"),
        concat!(
            r#"{"ts":"2026-07-09T08:00:00Z","op":"write","pane":"old","command":"clinch_agent_resume_launch claude old-session","cwd":"/tmp/old"}"#,
            "\n",
            r#"{"ts":"2026-07-09T09:00:00Z","op":"write","pane":"middle","command":"clinch_agent_resume_launch codex middle-session","cwd":"/tmp/middle"}"#,
            "\n",
            r#"{"ts":"2026-07-09T10:00:00Z","op":"write","pane":"new","command":"clinch_agent_resume_launch claude new-session","cwd":"/tmp/new"}"#,
            "\n",
        ),
    )
    .unwrap();
    toggle_conversation_bookmark_in(&dir, AgentResumeProvider::Claude, "old-session").unwrap();
    let bookmarks = read_conversation_bookmarks_in(&dir);

    assert_eq!(
        recent_conversations_in(&dir, 1)
            .iter()
            .map(|conversation| conversation.session_id.as_str())
            .collect::<Vec<_>>(),
        vec!["new-session"]
    );
    let finder = recent_conversations_including_bookmarks(&dir, 1, &bookmarks);
    assert_eq!(
        finder
            .iter()
            .map(|conversation| conversation.session_id.as_str())
            .collect::<Vec<_>>(),
        vec!["new-session", "old-session"]
    );
    assert!(!finder[0].bookmarked);
    assert!(finder[1].bookmarked);

    let bookmarked = bookmarked_conversations_in(&dir);
    assert_eq!(bookmarked.len(), 1);
    assert_eq!(bookmarked[0].session_id, "old-session");
    assert!(bookmarked[0].bookmarked);

    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn native_transcripts_backfill_claude_and_codex_first_prompts() {
    let dir = std::env::temp_dir().join(format!(
        "agent_resume_transcript_prompt_test_{}",
        uuid::Uuid::new_v4()
    ));
    let claude_projects = dir.join("claude-projects/project");
    let codex_sessions = dir.join("codex-sessions/2026/07/13");
    std::fs::create_dir_all(&claude_projects).unwrap();
    std::fs::create_dir_all(&codex_sessions).unwrap();

    std::fs::write(
        claude_projects.join("claude-session.jsonl"),
        concat!(
            "not json\n",
            r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"generated metadata"}}"#,
            "\n",
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"not the prompt"}]}}"#,
            "\n",
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"Claude opening question.\nMore detail"}]}}"#,
            "\n",
        ),
    )
    .unwrap();
    std::fs::write(
        codex_sessions.join("rollout-test-codex-session.jsonl"),
        concat!(
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>generated</environment_context>"}]}}"#,
            "\n",
            r#"{"type":"event_msg","payload":{"type":"user_message","message":"Codex opening question.\nMore detail"}}"#,
            "\n",
        ),
    )
    .unwrap();

    let mut mirrored = conversation("claude", "mirrored-session", None, "", false);
    mirrored.first_prompt = Some("Captured mirror wins".to_string());
    let mut conversations = vec![
        conversation("claude", "claude-session", None, "", false),
        conversation("codex", "codex-session", None, "", false),
        mirrored,
    ];
    enrich_conversations_from_transcripts(
        &mut conversations,
        &AgentTranscriptRoots {
            claude_projects: Some(dir.join("claude-projects")),
            codex_sessions: Some(dir.join("codex-sessions")),
        },
    );

    assert_eq!(
        conversations[0].first_prompt.as_deref(),
        Some("Claude opening question. More detail")
    );
    assert_eq!(
        conversations[1].first_prompt.as_deref(),
        Some("Codex opening question. More detail")
    );
    assert_eq!(
        conversations[2].first_prompt.as_deref(),
        Some("Captured mirror wins")
    );
    assert!(conversations[0].local_resumable);
    assert!(conversations[1].local_resumable);
    assert!(!conversations[2].local_resumable);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn codex_transcript_ignores_agent_instructions_and_titles_from_first_user_sentence() {
    let dir = std::env::temp_dir().join(format!(
        "agent_resume_codex_prompt_fallback_test_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("rollout-test-session.jsonl");
    std::fs::write(
        &path,
        concat!(
            r##"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions\n\n<INSTRUCTIONS>generated</INSTRUCTIONS>"}]}}"##,
            "\n",
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<permissions instructions>generated</permissions instructions>"}]}}"#,
            "\n",
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Explain this failure. Include the relevant logs."}]}}"#,
            "\n",
        ),
    )
    .unwrap();

    let prompt = first_prompt_from_codex_transcript(&path).unwrap();
    assert_eq!(prompt, "Explain this failure. Include the relevant logs.");
    assert_eq!(
        prompt_title(&prompt).as_deref(),
        Some("Explain this failure.")
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn provider_mirror_coalesces_inflight_retries_but_preserves_repeats_after_stop() {
    let dir = std::env::temp_dir().join(format!(
        "agent_resume_provider_mirror_test_{}",
        uuid::Uuid::new_v4()
    ));
    let scoped = dir.join("prompts/claude");
    let native = dir.join("native/project");
    std::fs::create_dir_all(&scoped).unwrap();
    std::fs::create_dir_all(&native).unwrap();
    std::fs::write(
        scoped.join("session-1.jsonl"),
        concat!(
            "malformed\n",
            r#"{"ts":"2026-07-14T01:02:03Z","prompt":"same\nmessage"}"#,
            "\n",
            r#"{"prompt":"same\nmessage"}"#,
            "\n",
            r#"{"ts":"2026-07-14T01:02:30Z","stop":true}"#,
            "\n",
            r#"{"ts":"2026-07-14T01:02:45Z","prompt":"<task-notification> <task-id>t1</task-id> <summary>Background command \"suite\" completed (exit code 0)</summary> </task-notification>"}"#,
            "\n",
            r#"{"ts":"2026-07-14T01:03:00Z","prompt":"same\nmessage"}"#,
            "\n",
            r#"{"ts":"2026-07-14T01:03:00Z","truncated":true}"#,
            "\n",
        ),
    )
    .unwrap();
    std::fs::write(
        native.join("session-1.jsonl"),
        r#"{"type":"user","message":{"content":"native must not replace mirror"}}"#,
    )
    .unwrap();

    let roots = AgentTranscriptRoots {
        claude_projects: Some(dir.join("native")),
        codex_sessions: None,
    };
    let history = read_prompt_history_in(
        AgentResumeProvider::Claude,
        "session-1",
        Some(&native.join("session-1.jsonl")),
        Some(&dir),
        &roots,
    );
    assert_eq!(
        history.prompts,
        vec![
            AgentPrompt {
                timestamp: Some("2026-07-14T01:02:03Z".to_string()),
                text: "same\nmessage".to_string(),
            },
            AgentPrompt {
                timestamp: Some("2026-07-14T01:03:00Z".to_string()),
                text: "same\nmessage".to_string(),
            },
        ]
    );
    assert!(
        history.is_partial,
        "malformed lines and the cap marker are surfaced"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn legacy_flat_mirror_is_claude_only_and_scoped_mirror_wins() {
    let dir = std::env::temp_dir().join(format!(
        "agent_resume_legacy_mirror_test_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(dir.join("prompts/claude")).unwrap();
    std::fs::write(
        dir.join("prompts/legacy-id.jsonl"),
        "{\"ts\":\"old\",\"prompt\":\"legacy prompt\"}\n",
    )
    .unwrap();
    let roots = AgentTranscriptRoots::default();
    assert_eq!(
        read_prompt_history_in(
            AgentResumeProvider::Claude,
            "legacy-id",
            None,
            Some(&dir),
            &roots,
        )
        .prompts[0]
            .text,
        "legacy prompt"
    );
    assert!(
        read_prompt_history_in(
            AgentResumeProvider::Codex,
            "legacy-id",
            None,
            Some(&dir),
            &roots,
        )
        .prompts
        .is_empty(),
        "legacy flat mirrors must never be attributed to Codex"
    );

    std::fs::write(
        dir.join("prompts/claude/legacy-id.jsonl"),
        "{\"ts\":\"new\",\"prompt\":\"scoped prompt\"}\n",
    )
    .unwrap();
    assert_eq!(
        read_prompt_history_in(
            AgentResumeProvider::Claude,
            "legacy-id",
            None,
            Some(&dir),
            &roots,
        )
        .prompts[0]
            .text,
        "scoped prompt"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn claude_history_parser_keeps_exact_user_text_and_timestamps() {
    let dir = std::env::temp_dir().join(format!(
        "agent_resume_claude_history_test_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("session.jsonl");
    std::fs::write(
        &path,
        concat!(
            r#"{"type":"user","isMeta":true,"timestamp":"ignored","message":{"content":"metadata"}}"#,
            "\n",
            r#"{"type":"user","timestamp":"2026-07-14T02:00:00Z","message":{"content":"first\nline"}}"#,
            "\n",
            r#"{"type":"user","timestamp":"2026-07-14T02:01:00Z","message":{"content":[{"type":"tool_result","content":"ignored"},{"type":"text","text":"second"},{"type":"text","text":"detail"}]}}"#,
            "\n",
            r#"{"type":"user","timestamp":"2026-07-14T02:01:05Z","origin":{"kind":"human"},"message":{"content":"second\ndetail"}}"#,
            "\n",
            r#"{"type":"user","timestamp":"2026-07-14T02:01:06Z","interruptedMessageId":"msg-1","message":{"content":"[Request interrupted by user]"}}"#,
            "\n",
            r#"{"type":"user","timestamp":"2026-07-14T02:01:07Z","message":{"content":"<local-command-stdout>hidden</local-command-stdout>"}}"#,
            "\n",
            r#"{"type":"user","timestamp":"2026-07-14T02:01:08Z","message":{"content":"<task-notification> <task-id>t1</task-id> <status>completed</status> <summary>Background command \"suite\" completed (exit code 0)</summary> </task-notification>"}}"#,
            "\n",
            r#"{"type":"assistant","timestamp":"2026-07-14T02:01:10Z","message":{"content":[{"type":"text","text":"visible answer"}]}}"#,
            "\n",
            r#"{"type":"user","timestamp":"2026-07-14T02:02:00Z","origin":{"kind":"human"},"message":{"content":"second\ndetail"}}"#,
            "\n",
        ),
    )
    .unwrap();
    assert_eq!(
        prompt_history_from_claude_transcript(&path),
        AgentPromptHistory {
            prompts: vec![
                AgentPrompt {
                    timestamp: Some("2026-07-14T02:00:00Z".to_string()),
                    text: "first\nline".to_string(),
                },
                AgentPrompt {
                    timestamp: Some("2026-07-14T02:01:00Z".to_string()),
                    text: "second\ndetail".to_string(),
                },
                AgentPrompt {
                    timestamp: Some("2026-07-14T02:02:00Z".to_string()),
                    text: "second\ndetail".to_string(),
                },
            ],
            is_partial: false,
            inherited_prompts: 0,
        }
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn codex_event_messages_are_canonical_without_collapsing_repeated_turns() {
    let dir = std::env::temp_dir().join(format!(
        "agent_resume_codex_history_test_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("rollout-test-session.jsonl");
    std::fs::write(
        &path,
        concat!(
            r#"{"timestamp":"generated","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<skills_instructions>generated</skills_instructions>"}]}}"#,
            "\n",
            r#"{"timestamp":"duplicate-form","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"same"}]}}"#,
            "\n",
            r##"{"timestamp":"generated-event","type":"event_msg","payload":{"type":"user_message","message":"# AGENTS.md instructions\n\n<INSTRUCTIONS>generated</INSTRUCTIONS>"}}"##,
            "\n",
            r#"{"timestamp":"turn-1","type":"event_msg","payload":{"type":"user_message","message":"same"}}"#,
            "\n",
            r#"{"timestamp":"turn-2","type":"event_msg","payload":{"type":"user_message","message":"same"}}"#,
            "\n",
        ),
    )
    .unwrap();
    assert_eq!(
        prompt_history_from_codex_transcript(&path).prompts,
        vec![
            AgentPrompt {
                timestamp: Some("turn-1".to_string()),
                text: "same".to_string(),
            },
            AgentPrompt {
                timestamp: Some("turn-2".to_string()),
                text: "same".to_string(),
            },
        ]
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn explicit_transcript_must_stay_inside_matching_provider_root() {
    let dir = std::env::temp_dir().join(format!(
        "agent_resume_safe_transcript_test_{}",
        uuid::Uuid::new_v4()
    ));
    let root = dir.join("claude/project");
    let outside = dir.join("outside");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let good = root.join("safe-id.jsonl");
    let escaped = outside.join("escape-id.jsonl");
    std::fs::write(&good, r#"{"type":"user","message":{"content":"safe"}}"#).unwrap();
    std::fs::write(
        &escaped,
        r#"{"type":"user","message":{"content":"must not read"}}"#,
    )
    .unwrap();
    let roots = AgentTranscriptRoots {
        claude_projects: Some(dir.join("claude")),
        codex_sessions: None,
    };
    assert_eq!(
        read_prompt_history_in(
            AgentResumeProvider::Claude,
            "safe-id",
            Some(&good),
            None,
            &roots,
        )
        .prompts[0]
            .text,
        "safe"
    );
    assert!(read_prompt_history_in(
        AgentResumeProvider::Claude,
        "escape-id",
        Some(&escaped),
        None,
        &roots,
    )
    .prompts
    .is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn source_byte_cap_marks_history_partial_without_parsing_a_cut_record() {
    let dir = std::env::temp_dir().join(format!(
        "agent_resume_history_cap_test_{}",
        uuid::Uuid::new_v4()
    ));
    let scoped = dir.join("prompts/codex");
    std::fs::create_dir_all(&scoped).unwrap();
    let path = scoped.join("large-id.jsonl");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(MAX_PROMPT_HISTORY_SOURCE_BYTES + 1).unwrap();
    let history = read_prompt_history_in(
        AgentResumeProvider::Codex,
        "large-id",
        None,
        Some(&dir),
        &AgentTranscriptRoots::default(),
    );
    assert!(history.prompts.is_empty());
    assert!(history.is_partial);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn recent_conversations_honors_explicit_bridge_scrub() {
    let dir = std::env::temp_dir().join(format!("agent_resume_scrub_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("journal.jsonl"),
        concat!(
            r#"{"ts":"2026-07-09T10:00:00Z","op":"write","pane":"pane-a","command":"clinch_agent_resume_launch claude sid-a","cwd":"/tmp/proj","bridge":"session_01LEAK"}"#,
            "\n",
            r#"{"ts":"2026-07-09T10:01:00Z","op":"scrub-bridge","pane":"pane-a","command":"clinch_agent_resume_launch claude sid-a","cwd":"/tmp/proj","bridge":"session_01LEAK"}"#,
            "\n",
        ),
    )
    .unwrap();

    let conversations = recent_conversations_in(&dir, 50);
    assert_eq!(conversations.len(), 1);
    assert_eq!(conversations[0].session_id, "sid-a");
    assert_eq!(conversations[0].bridge, None);
    assert_eq!(
        conversations[0].reopen_command().as_deref(),
        Some("claude --resume sid-a")
    );
}

#[test]
fn single_line_excerpt_collapses_and_caps() {
    assert_eq!(single_line_excerpt("plain prompt", 160), "plain prompt");
    assert_eq!(
        single_line_excerpt("line1\nline2\t \tline3", 160),
        "line1 line2 line3"
    );
    let long = "word ".repeat(100);
    let excerpt = single_line_excerpt(&long, 12);
    assert_eq!(excerpt, "word word wo…");
    // Cap counts characters, not bytes (no panic on multi-byte boundaries).
    assert_eq!(single_line_excerpt("ééééé", 3), "ééé…");
}

#[test]
fn read_fork_launch_reads_derived_command_and_cwd() {
    let dir = std::env::temp_dir().join(format!("agent_resume_fork_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut f = std::fs::File::create(dir.join("feedface.json")).unwrap();
    write!(
        f,
        r#"{{ "command": "clinch_agent_resume_launch codex xyz-9", "cwd": "/work" }}"#
    )
    .unwrap();

    let launch = read_fork_launch_in(&dir, "feedface").unwrap();
    assert_eq!(
        launch.command,
        format!(
            "{} fork xyz-9",
            AgentResumeProvider::Codex.launch_command_prefix(true)
        )
    );
    assert_eq!(launch.cwd.as_deref(), Some("/work"));

    // No cwd in the file → None cwd, still derives the command (flags carried).
    let mut f2 = std::fs::File::create(dir.join("cafe.json")).unwrap();
    write!(
        f2,
        r#"{{ "command": "clinch_agent_resume_launch claude id-1 --dangerously-skip-permissions" }}"#
    )
    .unwrap();
    let launch2 = read_fork_launch_in(&dir, "cafe").unwrap();
    assert_eq!(
        launch2.command,
        "claude --resume id-1 --dangerously-skip-permissions --fork-session"
    );
    assert_eq!(launch2.cwd, None);

    assert!(read_fork_launch_in(&dir, "missing").is_none());
}

#[test]
fn active_pane_manifest_is_atomic_sorted_and_deduplicated() {
    let dir = std::env::temp_dir().join(format!(
        "agent_resume_manifest_test_{}",
        uuid::Uuid::new_v4()
    ));
    write_active_pane_manifest_in(
        &dir,
        &[vec![0xbb], vec![0xaa], vec![0xbb], vec![0x00, 0xff]],
    )
    .unwrap();

    assert_eq!(
        std::fs::read_to_string(dir.join(ACTIVE_PANES_FILE)).unwrap(),
        "00ff\naa\nbb\n"
    );
    assert!(std::fs::read_dir(&dir).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".tmp.")
    }));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(dir.join(ACTIVE_PANES_FILE))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn restore_prefers_newer_registry_command_over_sqlite_snapshot() {
    let dir = std::env::temp_dir().join(format!(
        "agent_resume_reconcile_test_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("aabb.json"),
        r#"{"command":"clinch_agent_resume_launch claude current","cwd":"/tmp"}"#,
    )
    .unwrap();

    assert_eq!(
        resolve_on_restore_command_in(
            &dir,
            "aabb",
            Some("clinch_agent_resume_launch codex stale".to_string()),
            None,
        )
        .as_deref(),
        Some("clinch_agent_resume_launch claude current")
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn journaled_remove_prevents_stale_sqlite_agent_resurrection() {
    let dir =
        std::env::temp_dir().join(format!("agent_resume_remove_test_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("journal.jsonl"),
        concat!(
            "{\"ts\":\"2026-07-12T12:00:00Z\",\"op\":\"write\",\"pane\":\"aabb\",",
            "\"command\":\"clinch_agent_resume_launch claude old\",\"cwd\":\"/tmp\",\"bridge\":\"\"}\n",
            "{\"ts\":\"2026-07-12T12:01:00Z\",\"op\":\"remove\",\"pane\":\"aabb\"}\n"
        ),
    )
    .unwrap();

    assert_eq!(
        resolve_on_restore_command_in(
            &dir,
            "aabb",
            Some("clinch_agent_resume_launch claude old".to_string()),
            None,
        ),
        None
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn removal_tombstone_prevents_resurrection_when_journal_is_unavailable() {
    let dir = std::env::temp_dir().join(format!(
        "agent_resume_tombstone_test_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(dir.join(TOMBSTONES_DIR)).unwrap();
    std::fs::write(
        dir.join(TOMBSTONES_DIR).join("aabb"),
        "2026-07-12T12:00:00Z\n",
    )
    .unwrap();

    assert_eq!(
        resolve_on_restore_command_in(
            &dir,
            "aabb",
            Some("clinch_agent_resume_launch claude old".to_string()),
            None,
        ),
        None
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn persisted_command_survives_without_a_removal_tombstone() {
    let dir = std::env::temp_dir().join(format!(
        "agent_resume_persisted_test_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();

    assert_eq!(
        resolve_on_restore_command_in(
            &dir,
            "aabb",
            Some("warp_agent_resume_launch claude old".to_string()),
            None,
        )
        .as_deref(),
        Some("clinch_agent_resume_launch claude old")
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn toolbelt_learning_requires_two_distinct_safe_conversations() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(TOOLBELT_LEARNING_FILE),
        serde_json::json!({
            "schema": 1,
            "patterns": [
                {
                    "id": "00000000-0000-4000-8000-000000000001",
                    "sessions": ["codex:same-session", "codex:same-session"],
                    "providers": ["codex"],
                    "last_seen": "2026-08-13T01:00:00Z",
                    "text": "Run the same local server"
                },
                {
                    "id": "00000000-0000-4000-8000-000000000002",
                    "sessions": ["codex:session-one", "claude:session-two"],
                    "providers": ["codex", "claude", "invalid"],
                    "last_seen": "2026-08-13T02:00:00Z",
                    "text": "Run the reusable local server"
                },
                {
                    "id": "not-a-uuid",
                    "sessions": ["codex:session-three", "codex:session-four"],
                    "providers": ["codex"],
                    "last_seen": "2026-08-13T03:00:00Z",
                    "text": "This invalid record stays hidden"
                }
            ]
        })
        .to_string(),
    )
    .unwrap();

    let suggestions = learned_toolbelt_suggestions_in(dir.path());
    assert_eq!(suggestions.len(), 1);
    assert_eq!(
        suggestions[0],
        LearnedToolbeltSuggestion {
            id: "00000000-0000-4000-8000-000000000002".to_owned(),
            text: "Run the reusable local server".to_owned(),
            conversation_count: 2,
            providers: vec!["claude".to_owned(), "codex".to_owned()],
            last_seen: "2026-08-13T02:00:00Z".to_owned(),
        }
    );
    assert!(learned_toolbelt_suggestions_for_state(false, Some(dir.path())).is_empty());
}

#[test]
fn toolbelt_suggestion_resolution_is_private_durable_and_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let suggestion_id = "00000000-0000-4000-8000-000000000003";
    std::fs::write(
        dir.path().join(TOOLBELT_LEARNING_FILE),
        serde_json::json!({
            "schema": 1,
            "patterns": [{
                "id": suggestion_id,
                "sessions": ["codex:session-one", "codex:session-two"],
                "providers": ["codex"],
                "last_seen": "2026-08-13T02:00:00Z",
                "text": "Run the reusable local server"
            }]
        })
        .to_string(),
    )
    .unwrap();

    resolve_toolbelt_suggestion_in(
        dir.path(),
        suggestion_id,
        ToolbeltSuggestionResolutionOutcome::Declined,
    )
    .unwrap();
    resolve_toolbelt_suggestion_in(
        dir.path(),
        suggestion_id,
        ToolbeltSuggestionResolutionOutcome::Declined,
    )
    .expect("repeating the same resolution is idempotent");
    assert!(learned_toolbelt_suggestions_in(dir.path()).is_empty());

    let saved = std::fs::read_to_string(dir.path().join(TOOLBELT_RESOLUTIONS_FILE)).unwrap();
    let value: serde_json::Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(value["schema"], 1);
    assert_eq!(value["resolutions"][0]["outcome"], "declined");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(dir.path().join(TOOLBELT_RESOLUTIONS_FILE))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[cfg(unix)]
#[test]
fn toolbelt_learning_rejects_symlinked_and_oversized_stores() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("outside.json");
    std::fs::write(
        &outside,
        r#"{"schema":1,"patterns":[{"id":"00000000-0000-4000-8000-000000000004","sessions":["codex:one","codex:two"],"providers":["codex"],"text":"Run the local preview server"}]}"#,
    )
    .unwrap();
    let store = dir.path().join(TOOLBELT_LEARNING_FILE);
    symlink(&outside, &store).unwrap();
    assert!(learned_toolbelt_suggestions_in(dir.path()).is_empty());

    std::fs::remove_file(&store).unwrap();
    let file = std::fs::File::create(&store).unwrap();
    file.set_len(MAX_TOOLBELT_LEARNING_BYTES + 1).unwrap();
    assert!(learned_toolbelt_suggestions_in(dir.path()).is_empty());
}

#[test]
fn codex_registry_owner_requires_exact_pane_process_and_tty() {
    use std::ffi::OsString;
    let environment = vec![
        OsString::from("WARP_TERMINAL_SESSION_UUID=abcd"),
        OsString::from("WARP_TTY=/dev/ttys001"),
    ];
    let executable = Some(Path::new("/native/codex"));
    let args = vec![OsString::from("codex"), OsString::from("--no-daemon")];
    assert!(codex_registry_owner_matches(
        executable,
        &args,
        &environment,
        "abcd",
        "ttys001"
    ));
    assert!(!codex_registry_owner_matches(
        executable,
        &args,
        &environment,
        "other-pane",
        "ttys001"
    ));
    assert!(!codex_registry_owner_matches(
        executable,
        &args,
        &environment,
        "abcd",
        "ttys002"
    ));
    assert!(!codex_registry_owner_matches(
        Some(Path::new("/bin/sh")),
        &args,
        &environment,
        "abcd",
        "ttys001"
    ));
    assert!(!codex_registry_owner_matches(
        executable,
        &[OsString::from("codex"), OsString::from("app-server")],
        &environment,
        "abcd",
        "ttys001"
    ));
    assert!(!codex_registry_owner_matches(
        executable,
        &args,
        &[],
        "abcd",
        "ttys001"
    ));
}

fn prompts(texts: &[&str]) -> Vec<AgentPrompt> {
    texts
        .iter()
        .map(|text| AgentPrompt {
            timestamp: None,
            text: (*text).to_owned(),
        })
        .collect()
}

fn mirror_history(texts: &[&str]) -> AgentPromptHistory {
    AgentPromptHistory {
        prompts: prompts(texts),
        ..Default::default()
    }
}

fn prompt_texts(history: &AgentPromptHistory) -> Vec<&str> {
    history
        .prompts
        .iter()
        .map(|prompt| prompt.text.as_str())
        .collect()
}

#[test]
fn claude_fork_keeps_inherited_prompts_ahead_of_its_mirror() {
    let history = with_claude_inherited_prompts(
        mirror_history(&["fork one", "fork two"]),
        prompts(&["parent one", "parent two", "fork one", "fork two"]),
    );
    assert_eq!(
        prompt_texts(&history),
        ["parent one", "parent two", "fork one", "fork two"]
    );
    assert_eq!(history.inherited_prompts, 2);

    // The transcript has not recorded the in-flight prompt yet.
    let history = with_claude_inherited_prompts(
        mirror_history(&["fork one", "fork two"]),
        prompts(&["parent one", "fork one"]),
    );
    assert_eq!(
        prompt_texts(&history),
        ["parent one", "fork one", "fork two"]
    );
    assert_eq!(history.inherited_prompts, 1);
}

#[test]
fn fork_that_repeats_its_parents_prompts_keeps_both() {
    let history = with_claude_inherited_prompts(
        mirror_history(&["a", "b", "c"]),
        prompts(&["a", "b", "a", "b", "c"]),
    );
    assert_eq!(prompt_texts(&history), ["a", "b", "a", "b", "c"]);
    assert_eq!(history.inherited_prompts, 2);

    let history = with_inherited_prompts(mirror_history(&["a"]), prompts(&["a"]));
    assert_eq!(prompt_texts(&history), ["a", "a"]);
    assert_eq!(history.inherited_prompts, 1);
}

#[test]
fn claude_session_that_repeats_a_prompt_is_not_a_fork() {
    let same = ["same", "other", "same"];
    let history = with_claude_inherited_prompts(mirror_history(&same), prompts(&same));
    assert_eq!(prompt_texts(&history), same);
    assert_eq!(history.inherited_prompts, 0);

    // Text the transcript records differently cannot align, so the mirror stands alone.
    let history =
        with_claude_inherited_prompts(mirror_history(&["mirrored"]), prompts(&["recorded"]));
    assert_eq!(prompt_texts(&history), ["mirrored"]);
    assert_eq!(history.inherited_prompts, 0);
}

fn codex_rollout(root: &Path, id: &str, meta: serde_json::Value, user_prompts: &[&str]) {
    let mut lines = vec![serde_json::json!({
        "type": "session_meta",
        "payload": meta,
    })
    .to_string()];
    lines.extend(user_prompts.iter().map(|prompt| {
        serde_json::json!({
            "type": "event_msg",
            "payload": {"type": "user_message", "message": prompt},
        })
        .to_string()
    }));
    std::fs::write(
        root.join(format!("rollout-2026-10-30T13-00-00-{id}.jsonl")),
        lines.join("\n") + "\n",
    )
    .unwrap();
}

#[test]
fn codex_fork_reads_prompts_inherited_through_its_parents() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("2026/10/30");
    std::fs::create_dir_all(&root).unwrap();
    let id = |n: u8| format!("01a18e96-3ab8-76c2-b597-d78f48bdea{n:02}");
    codex_rollout(
        &root,
        &id(1),
        serde_json::json!({"id": id(1)}),
        &["grand one", "grand two", "after child forked"],
    );
    // Record 0 is session_meta, so an ordinal of 3 keeps the first two prompts.
    codex_rollout(
        &root,
        &id(2),
        serde_json::json!({
            "id": id(2),
            "forked_from_id": id(1),
            "forked_from_ordinal_exclusive": 3,
        }),
        &["parent one", "after fork forked"],
    );
    codex_rollout(
        &root,
        &id(3),
        serde_json::json!({
            "id": id(3),
            "forked_from_id": id(2),
            "forked_from_ordinal_exclusive": 2,
        }),
        &["fork one"],
    );
    let roots = AgentTranscriptRoots {
        claude_projects: None,
        codex_sessions: Some(dir.path().to_path_buf()),
    };

    let history = read_prompt_history_in(AgentResumeProvider::Codex, &id(3), None, None, &roots);
    assert_eq!(
        prompt_texts(&history),
        ["grand one", "grand two", "parent one", "fork one"]
    );
    assert_eq!(history.inherited_prompts, 3);

    // The mirror holds the fork's own prompts; inheritance is unchanged.
    let registry = dir.path().join("registry");
    std::fs::create_dir_all(registry.join("prompts/codex")).unwrap();
    std::fs::write(
        registry.join(format!("prompts/codex/{}.jsonl", id(3))),
        "{\"ts\":\"t\",\"prompt\":\"fork one\"}\n",
    )
    .unwrap();
    let history = read_prompt_history_in(
        AgentResumeProvider::Codex,
        &id(3),
        None,
        Some(&registry),
        &roots,
    );
    assert_eq!(
        prompt_texts(&history),
        ["grand one", "grand two", "parent one", "fork one"]
    );

    // An unforked session is untouched.
    let history = read_prompt_history_in(AgentResumeProvider::Codex, &id(1), None, None, &roots);
    assert_eq!(history.inherited_prompts, 0);
    assert_eq!(history.prompts.len(), 3);
}

#[test]
fn interrupted_turns_round_trip_and_expire() {
    let dir = tempfile::tempdir().unwrap();
    let now = Utc::now();
    assert!(read_interrupted_turns_in(dir.path(), now).is_empty());

    let panes = BTreeSet::from([vec![0xab, 0xcd], vec![0x01]]);
    write_interrupted_turns_in(dir.path(), &panes, now).unwrap();
    let read = read_interrupted_turns_in(dir.path(), now + chrono::Duration::hours(23));
    assert_eq!(read, panes.iter().cloned().collect::<HashSet<_>>());

    // Days later, or from a clock that has since moved backwards, the list means nothing.
    assert!(read_interrupted_turns_in(dir.path(), now + chrono::Duration::hours(25)).is_empty());
    assert!(read_interrupted_turns_in(dir.path(), now - chrono::Duration::hours(1)).is_empty());

    // An empty write replaces the old list.
    write_interrupted_turns_in(dir.path(), &BTreeSet::new(), now).unwrap();
    assert!(read_interrupted_turns_in(dir.path(), now).is_empty());
}

#[test]
fn restart_continue_prompts_stay_out_of_history() {
    assert!(is_injected_prompt("[Clinch restart] Continue."));
    assert!(!is_injected_prompt("Clinch restart notes"));
}

#[test]
fn auto_coordinate_wakeups_are_not_user_history() {
    assert!(is_injected_prompt(
        "[Clinch auto-coordinate] 1 update: \"Fix\" finished a turn. Tactics on: watch."
    ));
    assert!(is_injected_prompt(
        "  <task-notification> <task-id>t1</task-id>"
    ));
    // Only a leading marker counts; a user quoting it is still the user.
    assert!(!is_injected_prompt(
        "Why does the [Clinch auto-coordinate] message show up?"
    ));
}

#[test]
fn zsh_gets_a_command_its_spelling_correction_leaves_alone() {
    let typed = |command| command_to_type(command, ShellType::Zsh);
    assert_eq!(
        typed("CLINCH_AGENT_RESTORE_EXACT=1 clinch_agent_resume_launch claude abc-1 --dangerously-skip-permissions"),
        "nocorrect CLINCH_AGENT_RESTORE_EXACT=1 clinch_agent_resume_launch 'claude' 'abc-1' '--dangerously-skip-permissions'"
    );
    assert_eq!(
        typed("clinch_codex_launch resume"),
        "nocorrect clinch_codex_launch 'resume'"
    );
    // An argument that is already quoted, like a coordinator's instructions, stays as it is.
    assert_eq!(
        typed("claude --append-system-prompt 'You are the coordinator.' --model opus"),
        "nocorrect claude '--append-system-prompt' 'You are the coordinator.' '--model' 'opus'"
    );
}

#[test]
fn a_command_the_shell_has_to_interpret_only_gets_nocorrect() {
    for command in [
        "cd repo && claude",
        "claude $FLAGS",
        "claude ~/notes",
        "claude 'unterminated",
    ] {
        assert_eq!(
            command_to_type(command, ShellType::Zsh),
            format!("nocorrect {command}")
        );
    }
}

#[test]
fn shells_without_nocorrect_get_the_command_as_stored() {
    let command = "CLINCH_AGENT_RESTORE_EXACT=1 clinch_agent_resume_launch claude abc";
    for shell in [ShellType::Bash, ShellType::Fish, ShellType::PowerShell] {
        assert_eq!(command_to_type(command, shell), command);
    }
}

#[test]
fn a_typed_command_still_identifies_its_session() {
    for command in [
        "clinch_agent_resume_launch codex abc-1 --model gpt-5",
        "CLINCH_AGENT_RESTORE_EXACT=1 clinch_agent_resume_launch codex abc-1 --model gpt-5",
        "nocorrect CLINCH_AGENT_RESTORE_EXACT=1 clinch_agent_resume_launch 'codex' 'abc-1' '--model' 'gpt-5'",
    ] {
        assert_eq!(
            agent_session_seed_from_restore_command(command),
            Some((AgentResumeProvider::Codex, "abc-1".to_owned())),
            "{command}"
        );
    }
    assert_eq!(without_typed_prefixes("A=1 B=2 claude x"), "claude x");
    assert_eq!(without_typed_prefixes("claude a=1"), "claude a=1");
}

/// Types into a real interactive zsh on a real pseudo-terminal, the way a restored pane does.
#[cfg(target_os = "macos")]
mod real_zsh {
    use std::io::{Read as _, Write as _};
    use std::os::unix::fs::PermissionsExt as _;
    use std::process::Stdio;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use command::blocking::Command;
    use instant::Instant;

    use super::super::command_to_type;
    use crate::terminal::shell::ShellType;

    const PROMPT_QUESTION: &str = "[nyae]";

    struct Zsh {
        child: std::process::Child,
        output: Arc<Mutex<String>>,
    }

    impl Zsh {
        fn start(directory: &std::path::Path) -> Self {
            let mut child = Command::new("/usr/bin/script")
                .args(["-q", "/dev/null", "/bin/zsh", "-f", "-i"])
                .current_dir(directory)
                .env("PATH", format!("{}/bin:/usr/bin:/bin", directory.display()))
                .env("HOME", directory)
                .env("TERM", "xterm")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .expect("start zsh");
            let mut stdout = child.stdout.take().expect("zsh output");
            let output = Arc::new(Mutex::new(String::new()));
            let collected = output.clone();
            std::thread::spawn(move || {
                let mut buffer = [0; 4096];
                while let Ok(read) = stdout.read(&mut buffer) {
                    if read == 0 {
                        break;
                    }
                    collected
                        .lock()
                        .unwrap()
                        .push_str(&String::from_utf8_lossy(&buffer[..read]));
                }
            });
            let mut zsh = Self { child, output };
            zsh.type_line("setopt correct_all; PROMPT='ready> '; PS2=''");
            zsh.wait_for("ready> ");
            zsh
        }

        fn write(&mut self, bytes: &[u8]) {
            let stdin = self.child.stdin.as_mut().expect("zsh input");
            stdin.write_all(bytes).unwrap();
            stdin.flush().unwrap();
        }

        fn type_line(&mut self, line: &str) {
            self.write(format!("{line}\n").as_bytes());
        }

        fn output(&self) -> String {
            self.output.lock().unwrap().clone()
        }

        fn wait_for(&self, text: &str) {
            let deadline = Instant::now() + Duration::from_secs(30);
            while !self.output().contains(text) {
                assert!(
                    Instant::now() < deadline,
                    "never saw {text:?} in zsh output:\n{}",
                    self.output()
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }

    impl Drop for Zsh {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    /// A project folder holding `.claude` and `.codex`, and a stand-in launcher that reports what
    /// it was given.
    fn project() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join(".claude")).unwrap();
        std::fs::create_dir(directory.path().join(".codex")).unwrap();
        std::fs::create_dir(directory.path().join("bin")).unwrap();
        let launcher = directory.path().join("bin/clinch_agent_resume_launch");
        std::fs::write(
            &launcher,
            "#!/bin/sh\necho \"STARTED[$1][$2][$3][${CLINCH_AGENT_RESTORE_EXACT:-unset}]\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o755)).unwrap();
        directory
    }

    #[test]
    fn a_resume_command_starts_with_correct_all_on_beside_the_agents_own_folder() {
        for agent in ["claude", "codex"] {
            let directory = project();
            let mut zsh = Zsh::start(directory.path());
            let stored = format!(
                "CLINCH_AGENT_RESTORE_EXACT=1 clinch_agent_resume_launch {agent} abc --flag"
            );

            // As stored, the shell stops and asks. This is the hang, and it shows this test can
            // see it.
            zsh.type_line(&stored);
            zsh.wait_for(PROMPT_QUESTION);
            assert!(!zsh.output().contains("STARTED["));

            // Retry's Ctrl-C gets the shell back from the question, and the form Clinch types
            // starts with no question at all.
            zsh.write(&[0x03]);
            std::thread::sleep(Duration::from_millis(300));
            let questions_so_far = zsh.output().matches(PROMPT_QUESTION).count();
            zsh.type_line(&command_to_type(&stored, ShellType::Zsh));
            zsh.wait_for(&format!("STARTED[{agent}][abc][--flag][1]"));
            assert_eq!(
                zsh.output().matches(PROMPT_QUESTION).count(),
                questions_so_far,
                "{}",
                zsh.output()
            );
        }
    }
}

/// A registry where pane `aabb`'s session was removed at 12:00:00, the way an exit hook records it.
fn registry_with_removal() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(TOMBSTONES_DIR)).unwrap();
    std::fs::write(
        dir.path().join(TOMBSTONES_DIR).join("aabb"),
        "2026-08-13T12:00:00Z\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("journal.jsonl"),
        concat!(
            "{\"ts\":\"2026-08-13T11:00:00Z\",\"op\":\"write\",\"pane\":\"aabb\",",
            "\"command\":\"clinch_agent_resume_launch codex s1\",\"cwd\":\"/tmp\",\"bridge\":\"\"}\n",
            "{\"ts\":\"2026-08-13T12:00:00Z\",\"op\":\"remove\",\"pane\":\"aabb\"}\n"
        ),
    )
    .unwrap();
    dir
}

fn at(time: &str) -> Option<DateTime<Utc>> {
    Some(
        DateTime::parse_from_rfc3339(time)
            .unwrap()
            .with_timezone(&Utc),
    )
}

const SAVED_CODEX: &str = "clinch_agent_resume_launch codex s1";

#[test]
fn a_codex_session_removed_by_a_late_hook_during_an_update_is_still_restored() {
    let dir = registry_with_removal();
    assert_eq!(
        resolve_on_restore_command_in(
            dir.path(),
            "aabb",
            Some(SAVED_CODEX.to_owned()),
            at("2026-08-13T12:00:20Z"),
        )
        .as_deref(),
        Some(SAVED_CODEX)
    );
    // With only the journal to go on, as when a tombstone couldn't be read.
    std::fs::remove_file(dir.path().join(TOMBSTONES_DIR).join("aabb")).unwrap();
    assert_eq!(
        resolve_on_restore_command_in(
            dir.path(),
            "aabb",
            Some(SAVED_CODEX.to_owned()),
            at("2026-08-13T12:00:20Z"),
        )
        .as_deref(),
        Some(SAVED_CODEX)
    );
}

#[test]
fn a_session_the_user_exited_stays_gone() {
    let dir = registry_with_removal();
    let resolve = |persisted: Option<&str>, launch| {
        resolve_on_restore_command_in(dir.path(), "aabb", persisted.map(str::to_owned), launch)
    };
    // An ordinary launch, not the updater's.
    assert_eq!(resolve(Some(SAVED_CODEX), None), None);
    // Removed long before the update.
    assert_eq!(resolve(Some(SAVED_CODEX), at("2026-08-13T12:30:00Z")), None);
    // Removed after this launch, so it says nothing about the quit before it.
    assert_eq!(resolve(Some(SAVED_CODEX), at("2026-08-13T11:59:00Z")), None);
    // Exited before quitting: the old app had no command left to save.
    assert_eq!(resolve(None, at("2026-08-13T12:00:20Z")), None);
}

#[test]
fn a_removal_without_a_readable_time_stays_a_removal() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(TOMBSTONES_DIR)).unwrap();
    std::fs::write(dir.path().join(TOMBSTONES_DIR).join("aabb"), "not a time\n").unwrap();
    assert_eq!(
        resolve_on_restore_command_in(
            dir.path(),
            "aabb",
            Some(SAVED_CODEX.to_owned()),
            at("2026-08-13T12:00:20Z"),
        ),
        None
    );
}

#[test]
fn an_unreadable_tombstone_stays_a_removal() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(TOMBSTONES_DIR)).unwrap();
    std::fs::write(dir.path().join(TOMBSTONES_DIR).join("aabb"), [0xff, 0xfe]).unwrap();
    for launch in [None, at("2026-08-13T12:00:20Z")] {
        assert_eq!(
            resolve_on_restore_command_in(dir.path(), "aabb", Some(SAVED_CODEX.to_owned()), launch,),
            None
        );
    }
}

#[test]
fn a_saved_codex_session_restores_with_its_identity_like_a_claude_one() {
    let dir = tempfile::tempdir().unwrap();
    for (command, provider) in [
        (
            "clinch_agent_resume_launch codex 019a-id --model gpt-5",
            AgentResumeProvider::Codex,
        ),
        (
            "clinch_agent_resume_launch claude 2040-id --model opus",
            AgentResumeProvider::Claude,
        ),
    ] {
        let restored =
            resolve_on_restore_command_in(dir.path(), "aabb", Some(command.to_owned()), None)
                .unwrap();
        assert_eq!(restored, command);
        assert_eq!(
            agent_session_seed_from_restore_command(&restored).map(|(provider, _)| provider),
            Some(provider)
        );
    }
}
