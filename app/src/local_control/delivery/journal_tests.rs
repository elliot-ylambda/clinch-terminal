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
    journal.maintain(200, |pid| pid == 999).unwrap();
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
    journal.maintain(1900, |_| true).unwrap();
    assert_eq!(
        journal.get(&live.params.request_id).unwrap().unwrap().state,
        "expired"
    );
    assert!(journal.pending().unwrap().is_empty());
    journal
        .maintain(1901 + RETENTION_SECONDS, |_| true)
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
