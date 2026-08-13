use super::*;

fn shell_starter(shell_type: ShellType, shell_path: &str) -> DirectShellStarter {
    DirectShellStarter::new_for_test(shell_type, PathBuf::from(shell_path), Vec::new())
}

fn env_value(command: &Command, key: &str) -> Option<Option<String>> {
    command
        .get_envs()
        .find(|(env_key, _)| *env_key == std::ffi::OsStr::new(key))
        .map(|(_, value)| value.map(|value| value.to_string_lossy().into_owned()))
}

#[test]
fn host_bash_command_sets_history_size_sentinels() {
    let command = build_host_shell_command(
        shell_starter(ShellType::Bash, "/bin/bash"),
        None,
        HashMap::new(),
        None,
        false,
        false,
        false,
        false,
        true,
        None,
    );

    assert_eq!(
        env_value(&command, "HISTFILESIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "HISTSIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "WARP_INITIAL_HISTFILESIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "WARP_INITIAL_HISTSIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
}

#[test]
fn host_non_bash_command_does_not_set_history_size_sentinels() {
    let command = build_host_shell_command(
        shell_starter(ShellType::Zsh, "/bin/zsh"),
        None,
        HashMap::new(),
        None,
        false,
        false,
        false,
        false,
        true,
        None,
    );

    assert_eq!(env_value(&command, "HISTFILESIZE"), None);
    assert_eq!(env_value(&command, "HISTSIZE"), None);
    assert_eq!(env_value(&command, "WARP_INITIAL_HISTFILESIZE"), None);
    assert_eq!(env_value(&command, "WARP_INITIAL_HISTSIZE"), None);
}

#[test]
fn host_shell_scrubs_parent_agent_identity_after_overrides() {
    let mut overrides = HashMap::new();
    overrides.insert("CLAUDE_CODE_SESSION_ID".into(), "stale-session".into());
    overrides.insert("CLAUDE_CODE_FUTURE_ID".into(), "future-marker".into());
    overrides.insert("CLAUDECODE".into(), "1".into());
    overrides.insert("AI_AGENT".into(), "claude-code".into());
    overrides.insert("CLAUDE_EFFORT".into(), "high".into());
    overrides.insert("CLINCH_UNRELATED".into(), "preserved".into());
    overrides.insert(CLINCH_CONTROL_COMMAND_ENV.into(), "stale-command".into());
    overrides.insert(CLINCH_CONTROL_WRAPPER_ENV.into(), "/stale/wrapper".into());
    overrides.insert(CLINCH_CONTROL_PID_ENV.into(), "999".into());

    let command = build_host_shell_command(
        shell_starter(ShellType::Zsh, "/bin/zsh"),
        None,
        overrides,
        None,
        false,
        false,
        false,
        false,
        true,
        None,
    );

    for key in [
        "CLAUDE_CODE_SESSION_ID",
        "CLAUDE_CODE_FUTURE_ID",
        "CLAUDECODE",
        "AI_AGENT",
    ] {
        assert_eq!(env_value(&command, key), Some(None), "{key} leaked");
    }
    assert_eq!(
        env_value(&command, "CLAUDE_EFFORT"),
        Some(Some("high".to_owned()))
    );
    assert_eq!(
        env_value(&command, "CLINCH_UNRELATED"),
        Some(Some("preserved".to_owned()))
    );
    for key in [
        CLINCH_CONTROL_COMMAND_ENV,
        CLINCH_CONTROL_WRAPPER_ENV,
        CLINCH_CONTROL_PID_ENV,
    ] {
        assert_eq!(env_value(&command, key), Some(None), "{key} leaked");
    }
}

#[test]
fn exact_clinch_control_binding_replaces_inherited_values() {
    let mut command = Command::new("/bin/zsh");
    command.env(CLINCH_CONTROL_COMMAND_ENV, "stale-command");
    command.env(CLINCH_CONTROL_WRAPPER_ENV, "/stale/wrapper");
    command.env(CLINCH_CONTROL_PID_ENV, "999");

    apply_clinch_control_environment(
        &mut command,
        Some((
            "warpctrl-local".to_owned(),
            PathBuf::from("/current/ClinchDev.app/Contents/Resources/bin/warpctrl-local"),
        )),
        Some(1234),
    );

    assert_eq!(
        env_value(&command, CLINCH_CONTROL_COMMAND_ENV),
        Some(Some("warpctrl-local".to_owned()))
    );
    assert_eq!(
        env_value(&command, CLINCH_CONTROL_WRAPPER_ENV),
        Some(Some(
            "/current/ClinchDev.app/Contents/Resources/bin/warpctrl-local".to_owned()
        ))
    );
    assert_eq!(
        env_value(&command, CLINCH_CONTROL_PID_ENV),
        Some(Some("1234".to_owned()))
    );
}

#[test]
fn docker_sandbox_command_sets_history_size_sentinels() {
    let docker_starter =
        DockerSandboxShellStarter::new(shell_starter(ShellType::Bash, "sbx"), None);
    let mut overrides = HashMap::new();
    overrides.insert(CLINCH_CONTROL_COMMAND_ENV.into(), "stale-command".into());
    overrides.insert(CLINCH_CONTROL_WRAPPER_ENV.into(), "/stale/wrapper".into());
    overrides.insert(CLINCH_CONTROL_PID_ENV.into(), "999".into());
    let command = build_docker_sandbox_command(
        &docker_starter,
        None,
        overrides,
        false,
        false,
        false,
        false,
        true,
    );

    assert_eq!(
        env_value(&command, "HISTFILESIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "HISTSIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "WARP_INITIAL_HISTFILESIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "WARP_INITIAL_HISTSIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    for key in [
        CLINCH_CONTROL_COMMAND_ENV,
        CLINCH_CONTROL_WRAPPER_ENV,
        CLINCH_CONTROL_PID_ENV,
    ] {
        assert_eq!(env_value(&command, key), Some(None), "{key} leaked");
    }
}

/// A PTY follower must resolve to a real device path (e.g. `/dev/ttys004`) so
/// `WARP_TTY` can point agent hooks (which run without a controlling terminal)
/// back at the pane's PTY.
#[test]
fn tty_device_path_resolves_pty_follower() {
    let size = libc::winsize {
        ws_row: 24,
        ws_col: 80,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let (leader, follower) = make_pty(size).unwrap();

    let path = tty_device_path(follower).expect("PTY follower should have a device path");
    assert!(path.starts_with("/dev/"), "unexpected tty path: {path}");
    assert!(std::path::Path::new(&path).exists());

    unsafe {
        libc::close(leader);
        libc::close(follower);
    }
}
