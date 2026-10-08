use super::*;
fn events(after: Option<String>) -> AgentEventsParams {
    AgentEventsParams {
        after,
        scope: AgentScope::default(),
        limit: 100,
    }
}
fn cursor(page: &Value) -> Option<String> {
    page["next_cursor"].as_str().map(str::to_owned)
}
#[test]
fn local_control_replay_survives_reopen_and_reports_expiry_and_instance_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.db");
    let instance = InstanceId("one".into());
    let mut store = Store::open(&path).unwrap();
    store
        .append(
            &instance,
            json!({"kind": "agent.discovered", "project_id": "p"}),
            100,
        )
        .unwrap();
    let initial = store.events(&instance, events(None), 100).unwrap();
    drop(store);
    let mut store = Store::open(&path).unwrap();
    store
        .append(
            &instance,
            json!({"kind": "agent.changed", "project_id": "p"}),
            101,
        )
        .unwrap();
    let resumed = store
        .events(&instance, events(cursor(&initial)), 101)
        .unwrap();
    assert_eq!(resumed["events"].as_array().unwrap().len(), 1);
    assert_eq!(resumed["events"][0]["kind"], "agent.changed");
    assert_eq!(
        store
            .events(&InstanceId("other".into()), events(cursor(&initial)), 101)
            .unwrap_err()
            .code,
        ErrorCode::StaleTarget
    );
    assert_eq!(
        store
            .events(&instance, events(cursor(&initial)), 8 * 86400)
            .unwrap_err()
            .code,
        ErrorCode::StaleTarget
    );
}
#[test]
fn local_control_event_scope_advances_past_nonmatches_and_includes_gap_markers() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("events.db")).unwrap();
    let instance = InstanceId("one".into());
    for value in [
        json!({"kind": "agent.changed", "project_id": "other"}),
        json!({"kind": "collection_gap"}),
        json!({"kind": "agent.changed", "project_id": "p"}),
    ] {
        store.append(&instance, value, 100).unwrap();
    }
    let mut params = events(None);
    params.scope.projects = vec!["p".into()];
    params.limit = 1;
    let page = store.events(&instance, params.clone(), 100).unwrap();
    assert_eq!(page["events"][0]["kind"], "collection_gap");
    assert_eq!(page["has_more"], true);
    params.after = cursor(&page);
    let page = store.events(&instance, params, 100).unwrap();
    assert_eq!(page["events"][0]["project_id"], "p");
    assert_eq!(page["has_more"], false);
}
#[test]
fn local_control_inbox_ack_is_independent_atomic_idempotent_and_expiring() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("inbox.db")).unwrap();
    let a = store
        .batch("reader-a", &None, &json!({"conversation": 1}), 100)
        .unwrap();
    let racing = store
        .batch("reader-a", &None, &json!({"conversation": 2}), 100)
        .unwrap();
    let b = store
        .batch("reader-b", &None, &json!({"conversation": 3}), 100)
        .unwrap();
    assert!(
        store.get("reader:reader-a").unwrap().is_none(),
        "output without ack never consumes messages"
    );
    assert!(store.ack("reader-b", &a, 100).is_err());
    store.ack("reader-a", &a, 100).unwrap();
    store.ack("reader-a", &a, 100).unwrap();
    assert_eq!(
        store.ack("reader-a", &racing, 100).unwrap_err().code,
        ErrorCode::StaleTarget
    );
    assert!(store.get("reader:reader-b").unwrap().is_none());
    store.ack("reader-b", &b, 100).unwrap();
    assert_eq!(
        store.get("reader:reader-a").unwrap().unwrap()["conversation"],
        1
    );
    assert_eq!(
        store.get("reader:reader-b").unwrap().unwrap()["conversation"],
        3
    );
    assert!(store.ack("reader-b", &b, 701).is_err());
}
fn plan(id: &str) -> ReadPlan {
    ReadPlan {
        params: AgentReadParams {
            agent_id: id.into(),
            after: None,
            limit: 3,
            tail: true,
            role: Some(AgentRole::Assistant),
            messages_only: true,
        },
        agent: json!({"agent_id": id, "state": "turn_complete"}),
        provider: crate::agent_resume::AgentResumeProvider::Claude,
        session_id: Some("stable-conversation".into()),
        transcript_path: None,
        remote: false,
    }
}
#[test]
fn local_control_inbox_follows_conversation_moves_and_keeps_backlog_on_error() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("inbox.db")).unwrap();
    let params = AgentInboxParams {
        reader_id: "reader".into(),
        scope: AgentScope::default(),
        limit: 3,
    };
    let page = collect_inbox(&mut store, params.clone(), vec![plan("old-pane")], |plan| {
        assert!(plan.params.tail); assert_eq!(plan.params.role, Some(AgentRole::Assistant));
        Ok(json!({"agent": plan.agent, "records": [{"text": "one"}], "next_cursor": "cursor1", "older_content_omitted": true}))
    }).unwrap();
    store
        .ack("reader", page["batch_id"].as_str().unwrap(), now())
        .unwrap();
    let page = collect_inbox(
        &mut store,
        params.clone(),
        vec![plan("moved-pane")],
        |plan| {
            assert!(!plan.params.tail);
            assert_eq!(plan.params.after.as_deref(), Some("cursor1"));
            Err(ControlError::new(
                ErrorCode::StaleTarget,
                "transcript rotated",
            ))
        },
    )
    .unwrap();
    assert!(page["batch_id"].is_null());
    assert_eq!(page["items"][0]["agent"]["agent_id"], "moved-pane");
    let page = collect_inbox(&mut store, params, vec![plan("moved-pane")], |plan| {
        assert_eq!(plan.params.after.as_deref(), Some("cursor1"));
        Ok(json!({"agent": plan.agent, "records": [{"text": "two"}], "next_cursor": "cursor2", "has_more": true}))
    }).unwrap();
    assert_eq!(page["items"][0]["has_more"], true);
}

#[test]
fn local_control_acknowledged_batches_do_not_exhaust_pending_quota_and_expire_without_events() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("inbox.db")).unwrap();
    let mut before = None;
    for n in 0..120 {
        let after = json!({"cursor": n});
        let batch = store.batch("reader", &before, &after, 100).unwrap();
        store.ack("reader", &batch, 100).unwrap();
        before = Some(after);
    }
    let old = store
        .batch("reader", &before, &json!({"cursor": "old"}), 100)
        .unwrap();
    store
        .batch("reader", &before, &json!({"cursor": "new"}), 701)
        .unwrap();
    assert!(store.get(&format!("batch:{old}")).unwrap().is_none());
}
#[test]
fn local_control_section_departures_remain_in_scoped_replay() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("events.db")).unwrap();
    let instance = InstanceId("one".into());
    let before = BTreeMap::from([(
        "agent".into(),
        json!({"agent_id": "agent", "project_id": "p", "section_id": "a"}),
    )]);
    let after = BTreeMap::from([(
        "agent".into(),
        json!({"agent_id": "agent", "project_id": "p", "section_id": "b"}),
    )]);
    record_changes(&mut store, &instance, "agent", &before, &after).unwrap();
    let mut params = events(None);
    params.scope.sections = vec!["a".into()];
    let page = store.events(&instance, params, now()).unwrap();
    assert_eq!(page["events"][0]["previous_section_id"], "a");
    assert_eq!(page["events"][0]["section_id"], "b");
}
#[test]
fn local_control_end_signal_never_binds_to_replacement_conversation() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("events.db")).unwrap();
    let instance = InstanceId("one".into());
    let agents = BTreeMap::from([(
        "replacement".into(),
        json!({"terminal_id": "terminal", "conversation_id": "new", "agent_id": "replacement"}),
    )]);
    let mut previous = Observation {
        agents: agents.clone(),
        projects: BTreeMap::new(),
        signal: None,
    };
    let observation = Observation {
        agents,
        projects: BTreeMap::new(),
        signal: Some(
            json!({"kind": "agent.ended", "terminal_id": "terminal", "conversation_id": "old"}),
        ),
    };
    process(
        Command::Observe(observation),
        &mut store,
        &instance,
        &mut previous,
    )
    .unwrap();
    let page = store.events(&instance, events(None), now()).unwrap();
    let event = &page["events"][0];
    assert_eq!(event["kind"], "agent.ended");
    assert_eq!(event["conversation_id"], "old");
    assert_eq!(event["terminal_id"], "terminal");
    assert!(event["agent_id"].is_null());
}
