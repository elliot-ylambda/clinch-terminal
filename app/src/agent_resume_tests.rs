use std::io::Write;

use super::*;

#[test]
fn prompt_title_prefers_the_first_sentence_within_the_limit() {
    assert_eq!(
        prompt_title("  Fix the failing test. Then update the docs.  "),
        Some("Fix the failing test.".to_owned())
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
fn production_and_dev_clinch_are_the_only_supported_agent_resume_apps() {
    assert!(app_id_enables_runtime("sh.clinch.Clinch"));
    assert!(app_id_enables_runtime("sh.clinch.ClinchDev"));
    assert!(!app_id_enables_runtime("dev.warp.Warp-Local"));
    assert!(!app_id_enables_runtime("dev.warp.WarpOss"));
}

#[test]
fn agent_resume_runtime_requires_consent_or_an_explicit_test_override() {
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
        Some("codex fork legacy-456")
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
        Some("codex fork abc-123")
    );
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
        Some("codex fork abc-123 --dangerously-bypass-approvals-and-sandbox")
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

fn conversation(
    agent: &str,
    session_id: &str,
    bridge: Option<&str>,
    flags: &str,
) -> AgentConversation {
    AgentConversation {
        agent: agent.to_string(),
        session_id: session_id.to_string(),
        cwd: None,
        bridge: bridge.map(str::to_string),
        start_ts: "2026-07-09T10:00:00Z".to_string(),
        first_prompt: None,
        flags: flags.to_string(),
    }
}

#[test]
fn reopen_command_teleports_bridged_claude_sessions() {
    // Mirrors pane restore's priority: the cloud copy is authoritative for a bridged
    // session, and launch flags are forwarded.
    assert_eq!(
        conversation("claude", "abc-123", Some("session_01XYZ"), "").reopen_command(),
        Some("claude --teleport session_01XYZ".to_string())
    );
    assert_eq!(
        conversation("claude", "abc-123", Some("session_01XYZ"), " --model opus").reopen_command(),
        Some("claude --teleport session_01XYZ --model opus".to_string())
    );
}

#[test]
fn reopen_command_resumes_local_claude_sessions() {
    assert_eq!(
        conversation("claude", "abc-123", None, "").reopen_command(),
        Some("claude --resume abc-123".to_string())
    );
    // A bridge that is not claude.ai-shaped (session_*) is not teleported — same guard
    // as the shell replay side.
    assert_eq!(
        conversation("claude", "abc-123", Some("garbage"), "").reopen_command(),
        Some("claude --resume abc-123".to_string())
    );
    assert_eq!(
        conversation("claude", "abc-123", None, " --dangerously-skip-permissions").reopen_command(),
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
            " --dangerously-bypass-approvals-and-sandbox"
        )
        .reopen_command(),
        Some("codex resume xyz-9 --dangerously-bypass-approvals-and-sandbox".to_string())
    );
    assert_eq!(
        conversation("gemini", "abc", None, "").reopen_command(),
        None
    );
}

#[test]
fn recent_conversations_aggregates_journal_and_mirror() {
    let dir = std::env::temp_dir().join(format!("agent_resume_recent_test_{}", std::process::id()));
    let prompts = dir.join("prompts");
    std::fs::create_dir_all(&prompts).unwrap();

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
        prompts.join("sid-bridged-bbb.jsonl"),
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
    assert_eq!(codex.first_prompt, None);

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

    let mut mirrored = conversation("claude", "mirrored-session", None, "");
    mirrored.first_prompt = Some("Captured mirror wins".to_string());
    let mut conversations = vec![
        conversation("claude", "claude-session", None, ""),
        conversation("codex", "codex-session", None, ""),
        mirrored,
    ];
    enrich_missing_first_prompts_from_transcripts(
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
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn codex_transcript_uses_meaningful_user_response_when_event_is_absent() {
    let dir = std::env::temp_dir().join(format!(
        "agent_resume_codex_prompt_fallback_test_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("rollout-test-session.jsonl");
    std::fs::write(
        &path,
        concat!(
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<permissions instructions>generated</permissions instructions>"}]}}"#,
            "\n",
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Explain this failure"}]}}"#,
            "\n",
        ),
    )
    .unwrap();

    assert_eq!(
        first_prompt_from_codex_transcript(&path).as_deref(),
        Some("Explain this failure")
    );
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
    assert_eq!(launch.command, "codex fork xyz-9");
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
    assert!(std::fs::read_dir(&dir).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains(".tmp.")));
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
        )
        .as_deref(),
        Some("clinch_agent_resume_launch claude old")
    );
    std::fs::remove_dir_all(dir).unwrap();
}
