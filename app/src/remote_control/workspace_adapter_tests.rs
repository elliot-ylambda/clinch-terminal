use super::*;

#[test]
fn pane_ids_are_protocol_safe_and_stable() {
    let pane_id: PaneId = crate::pane_group::TerminalPaneId::dummy_terminal_pane_id().into();
    let encoded = pane_opaque_id(pane_id);

    assert!(encoded.len() <= MAX_OPAQUE_ID_BYTES);
    assert_eq!(pane_opaque_id(pane_id), encoded);
    TargetRef {
        app_instance_id: AppInstanceId::new(),
        project_id: "project".to_owned(),
        tab_id: "tab".to_owned(),
        pane_id: encoded,
    }
    .validate()
    .unwrap();
}

#[test]
fn project_creation_uses_the_session_creation_capability() {
    assert_eq!(
        required_capability(&ClientMessage::CreateProject(CreateProject {
            app_instance_id: AppInstanceId::new(),
            workspace_revision: 1,
            project_id: "project".to_owned(),
            cwd: None,
        })),
        Some(Capability::CreateSession)
    );
}

#[test]
fn activity_aggregation_keeps_attention_and_work_visible() {
    assert_eq!(
        merge_activity(ProjectActivity::Working, ProjectActivity::NeedsAttention),
        ProjectActivity::NeedsAttention
    );
    assert_eq!(
        merge_activity(ProjectActivity::Done, ProjectActivity::RunningCommand),
        ProjectActivity::RunningCommand
    );
}

#[test]
fn local_directory_validation_rejects_relative_and_file_paths() {
    assert_eq!(optional_local_directory(None).unwrap(), None);
    assert!(canonical_local_directory("relative/path").is_err());
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(
        canonical_local_directory(directory.path().to_str().unwrap()).unwrap(),
        std::fs::canonicalize(directory.path()).unwrap()
    );
    let file = directory.path().join("file.txt");
    std::fs::write(&file, b"x").unwrap();
    assert!(canonical_local_directory(file.to_str().unwrap()).is_err());
}

#[test]
fn mobile_titles_are_nonempty_collapsed_and_bounded() {
    assert_eq!(
        nonempty_title("  hello   world  ".to_owned(), "fallback"),
        "hello world"
    );
    assert_eq!(nonempty_title("   ".to_owned(), "fallback"), "fallback");
    assert_eq!(
        nonempty_title("x".repeat(500), "fallback").chars().count(),
        120
    );
}

#[test]
fn workspace_revisions_remain_exact_javascript_numbers() {
    assert_eq!(initial_workspace_revision(0), 1);
    assert!(initial_workspace_revision(u64::MAX) <= MAX_JAVASCRIPT_SAFE_INTEGER);
    assert_eq!(next_workspace_revision(41), 42);
    assert_eq!(next_workspace_revision(MAX_JAVASCRIPT_SAFE_INTEGER), 1);
}

#[test]
fn writer_leases_survive_expiry_while_their_session_stays_connected() {
    let session_id = AuthSessionId::new();
    let lease = WriterLease {
        session_id,
        device_id: clinch_companion_protocol::DeviceId::new(),
        device_name: "Elliot's phone".to_owned(),
        expires_at: Utc::now() - Duration::seconds(120),
    };
    let mut connected = HashSet::new();

    // An expired lease from a vanished session is pruned by the TTL backstop.
    assert!(writer_lease_expired(&connected, &lease, Utc::now()));

    // The same lease stays valid while its holder's WebSocket is still connected, so an idle
    // phone reading a long response never loses the PTY's viewer-driven dimensions.
    connected.insert(session_id);
    assert!(!writer_lease_expired(&connected, &lease, Utc::now()));

    // A different connected session does not keep someone else's lease alive.
    connected.clear();
    connected.insert(AuthSessionId::new());
    assert!(writer_lease_expired(&connected, &lease, Utc::now()));

    // An unexpired lease is valid regardless of connectivity.
    let fresh = WriterLease {
        expires_at: Utc::now() + Duration::seconds(WRITER_LEASE_TTL_SECS as i64),
        ..lease
    };
    assert!(!writer_lease_expired(&HashSet::new(), &fresh, Utc::now()));
}
