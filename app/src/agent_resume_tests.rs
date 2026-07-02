use super::*;
use std::io::Write;

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
fn uuid_hex_is_lowercase() {
    // Must match $WARP_TERMINAL_SESSION_UUID casing.
    assert_eq!(hex::encode([0xAB, 0xCD]), "abcd");
}

#[test]
fn derives_claude_fork_command() {
    assert_eq!(
        derive_fork_command("warp_agent_resume_launch claude abc-123").as_deref(),
        Some("claude --resume abc-123 --fork-session")
    );
}

#[test]
fn derives_codex_fork_command() {
    assert_eq!(
        derive_fork_command("warp_agent_resume_launch codex abc-123").as_deref(),
        Some("codex fork abc-123")
    );
}

#[test]
fn fork_command_carries_launch_flags() {
    assert_eq!(
        derive_fork_command(
            "warp_agent_resume_launch claude abc-123 --dangerously-skip-permissions --model opus"
        )
        .as_deref(),
        Some("claude --resume abc-123 --dangerously-skip-permissions --model opus --fork-session")
    );
    assert_eq!(
        derive_fork_command(
            "warp_agent_resume_launch codex abc-123 --dangerously-bypass-approvals-and-sandbox"
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
        derive_fork_command("warp_agent_resume_launch gemini abc-123"),
        None
    );
    assert_eq!(derive_fork_command("warp_agent_resume_launch claude"), None);
    assert_eq!(derive_fork_command("warp_agent_resume_launch claude "), None);
}

#[test]
fn read_fork_launch_reads_derived_command_and_cwd() {
    let dir = std::env::temp_dir().join(format!("agent_resume_fork_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut f = std::fs::File::create(dir.join("feedface.json")).unwrap();
    write!(
        f,
        r#"{{ "command": "warp_agent_resume_launch codex xyz-9", "cwd": "/work" }}"#
    )
    .unwrap();

    let launch = read_fork_launch_in(&dir, "feedface").unwrap();
    assert_eq!(launch.command, "codex fork xyz-9");
    assert_eq!(launch.cwd.as_deref(), Some("/work"));

    // No cwd in the file → None cwd, still derives the command (flags carried).
    let mut f2 = std::fs::File::create(dir.join("cafe.json")).unwrap();
    write!(
        f2,
        r#"{{ "command": "warp_agent_resume_launch claude id-1 --dangerously-skip-permissions" }}"#
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
