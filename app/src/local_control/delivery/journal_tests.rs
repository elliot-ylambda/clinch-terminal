use super::*;

fn params(target: &str, sender: &str) -> AgentSendParams {
    AgentSendParams {
        agent_id: target.into(),
        text: "Review this change".into(),
        expected_revision: "revision".into(),
        request_id: uuid::Uuid::new_v4().to_string(),
        queue: true,
        expires_in: 1800,
        sender_id: Some(sender.into()),
    }
}
fn message(target: &str, sender: &str) -> Message {
    Message::new(
        params(target, sender),
        "instance-a".into(),
        "guard".into(),
        100,
    )
}
fn open() -> (tempfile::TempDir, Journal) {
    let dir = tempfile::tempdir().unwrap();
    let journal = Journal::open(&dir.path().join("private/messages.sqlite3")).unwrap();
    (dir, journal)
}

#[test]
fn durable_exact_retry_and_conflicting_uuid() {
    let (dir, mut journal) = open();
    let first = journal.insert(message("agent", "sender")).unwrap();
    drop(journal);
    let mut journal = Journal::open(&dir.path().join("private/messages.sqlite3")).unwrap();
    assert_eq!(
        journal.existing(&first.params).unwrap().unwrap().sequence,
        first.sequence
    );
    let mut retry = first.clone();
    retry.guard = "new guard must never replace original".into();
    assert_eq!(journal.insert(retry).unwrap().guard, "guard");
    let mut conflict = first.params;
    conflict.text.push('!');
    assert!(journal.existing(&conflict).is_err());
    assert_eq!(journal.pending().unwrap().len(), 1);
}

#[test]
fn fifo_single_sender_limits_and_independent_targets() {
    let (_dir, mut journal) = open();
    let first = journal.insert(message("a", "sender-1")).unwrap();
    assert!(journal.insert(message("a", "sender-2")).is_err());
    let mut immediate = message("a", "sender-1");
    immediate.params.queue = false;
    assert!(journal.insert(immediate).is_err());
    for _ in 1..10 {
        journal.insert(message("a", "sender-1")).unwrap();
    }
    assert!(journal.insert(message("a", "sender-1")).is_err());
    let other = journal.insert(message("b", "sender-2")).unwrap();
    let pending = journal.pending().unwrap();
    assert_eq!(pending.first().unwrap().sequence, first.sequence);
    assert_eq!(pending.last().unwrap().sequence, other.sequence);
    for index in 0..89 {
        journal
            .insert(message(&format!("target-{index}"), "sender"))
            .unwrap();
    }
    assert!(journal.insert(message("overflow", "sender")).is_err());
}

#[test]
fn cancellation_and_dispatch_claim_are_atomic_across_connections() {
    let (dir, mut one) = open();
    let mut two = Journal::open(&dir.path().join("private/messages.sqlite3")).unwrap();
    let queued = one.insert(message("a", "s")).unwrap();
    two.transition(&queued, "cancelled", None, 110)
        .unwrap()
        .unwrap();
    assert!(one
        .transition(&queued, "dispatching", None, 111)
        .unwrap()
        .is_none());
    let queued = one.insert(message("b", "s")).unwrap();
    one.transition(&queued, "dispatching", None, 110)
        .unwrap()
        .unwrap();
    assert!(two
        .transition(&queued, "cancelled", None, 111)
        .unwrap()
        .is_none());
}

#[test]
fn restart_never_replays_inflight_or_rebinds_queued_targets() {
    let (dir, mut journal) = open();
    let queued = journal.insert(message("a", "s")).unwrap();
    let dispatching = journal.insert(message("b", "s")).unwrap();
    journal
        .transition(&dispatching, "dispatching", None, 110)
        .unwrap();
    let mut live = message("c", "s");
    live.owner_pid = 999;
    let live = journal.insert(live).unwrap();
    drop(journal);
    let mut journal = Journal::open(&dir.path().join("private/messages.sqlite3")).unwrap();
    journal.maintain(200, |pid, _| pid == 999).unwrap();
    assert_eq!(
        journal
            .get(&queued.params.request_id)
            .unwrap()
            .unwrap()
            .state,
        "cancelled"
    );
    assert_eq!(
        journal
            .get(&dispatching.params.request_id)
            .unwrap()
            .unwrap()
            .state,
        "delivery_unknown"
    );
    assert_eq!(
        journal.get(&live.params.request_id).unwrap().unwrap().state,
        "queued"
    );
    journal.maintain(1900, |_, _| true).unwrap();
    assert_eq!(
        journal.get(&live.params.request_id).unwrap().unwrap().state,
        "expired"
    );
    assert!(journal.pending().unwrap().is_empty());
    journal
        .maintain(1901 + RETENTION_SECONDS, |_, _| true)
        .unwrap();
    assert!(journal.get(&live.params.request_id).unwrap().is_none());
}

#[test]
fn bounded_paging_omits_prompts_and_can_filter_targets() {
    let (_dir, mut journal) = open();
    let first = journal.insert(message("a", "s")).unwrap();
    journal.insert(message("b", "s")).unwrap();
    journal.insert(message("a", "s")).unwrap();
    let list = journal
        .list(&AgentMessageListParams {
            agent_id: Some("a".into()),
            before: None,
            limit: 1,
        })
        .unwrap();
    assert_eq!(list["has_more"], true);
    assert!(list["messages"][0].get("text").is_none());
    let next = journal
        .list(&AgentMessageListParams {
            agent_id: Some("a".into()),
            before: list["next_before"].as_i64(),
            limit: 1,
        })
        .unwrap();
    assert_eq!(next["messages"][0]["sequence"], first.sequence);
    assert_eq!(next["has_more"], false);
    assert_eq!(first.receipt(true)["text"], first.params.text);
}

#[test]
fn corrupt_or_unwritable_journal_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.sqlite3");
    std::fs::write(&path, "not a sqlite database").unwrap();
    assert!(Journal::open(&path).is_err());
    assert!(Journal::open(dir.path()).is_err());
}

#[cfg(unix)]
#[test]
fn journal_and_directory_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, _journal) = open();
    assert_eq!(
        std::fs::metadata(dir.path().join("private"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(dir.path().join("private/messages.sqlite3"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn reused_pid_recovers_abandoned_messages_without_disturbing_new_owner() {
    let (_dir, mut journal) = open();
    let mut old = message("old-queued", "s");
    old.owner_pid = 999;
    old.owner_start_time = Some(10);
    let queued = journal.insert(old.clone()).unwrap();
    old.params = params("old-dispatching", "s");
    let dispatching = journal.insert(old.clone()).unwrap();
    journal
        .transition(&dispatching, "dispatching", None, 110)
        .unwrap();
    old.params = params("legacy", "s");
    old.owner_start_time = None;
    let legacy = journal.insert(old.clone()).unwrap();
    old.params = params("new-owner", "s");
    old.instance_id = "new-instance".into();
    old.owner_start_time = Some(20);
    let current = journal.insert(old).unwrap();
    journal
        .maintain(200, |pid, start| pid == 999 && start == Some(20))
        .unwrap();
    for (id, expected) in [
        (&queued.params.request_id, "cancelled"),
        (&dispatching.params.request_id, "delivery_unknown"),
        (&legacy.params.request_id, "cancelled"),
        (&current.params.request_id, "queued"),
    ] {
        assert_eq!(journal.get(id).unwrap().unwrap().state, expected);
    }
    assert_eq!(journal.pending().unwrap().len(), 1);
}

#[test]
fn process_identity_requires_the_original_start_time() {
    let pid = std::process::id();
    let start = process_start_time(pid).expect("current process must have a start time");
    assert!(owner_is_alive(pid, Some(start)));
    assert!(!owner_is_alive(pid, Some(start + 1)));
    assert!(!owner_is_alive(pid, None));
}
