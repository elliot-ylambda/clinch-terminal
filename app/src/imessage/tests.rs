use std::fs;

use tempfile::tempdir;

use super::domain::{
    format_completion_messages, queued_by_route, sanitize_incoming_text, IncomingMessage,
    MobileProvider, MobileRouteId, MobileSessionKey, PendingCalibration, RouteDecision, RouteState,
    MAX_IMESSAGE_CHARS,
};
use super::store::RouteStateStore;

const NOW: i64 = 1_700_000_000;

fn key(provider: MobileProvider, id: &str) -> MobileSessionKey {
    MobileSessionKey::new(provider, id).unwrap()
}

fn incoming(guid: &str, text: &str) -> IncomingMessage {
    IncomingMessage {
        guid: guid.to_owned(),
        row_id: 42,
        text: text.to_owned(),
        service: "iMessage".to_owned(),
        parent_guid: None,
        associated_guid: None,
        is_reaction: false,
        is_edited: false,
        has_attachments: false,
    }
}

fn enabled_state() -> RouteState {
    RouteState {
        globally_enabled: true,
        ..RouteState::default()
    }
}

#[test]
fn session_route_is_stable_and_an_explicit_opt_out_cancels_its_queue() {
    let mut state = enabled_state();
    let session = key(MobileProvider::Codex, "session-one");
    let route = state.register_session(session.clone(), "project", NOW);
    assert_eq!(
        route,
        state.register_session(session.clone(), "renamed", NOW + 1)
    );

    state
        .enqueue_reply("phone-1", route.clone(), "one", NOW)
        .unwrap();
    let cancelled = state.set_opted_out(&session, true, NOW + 2);
    assert_eq!(cancelled.len(), 1);
    assert!(!state.route_for_key(&session).unwrap().is_eligible(true));
}

#[test]
fn parent_guid_wins_over_an_explicit_code_for_another_route() {
    let mut state = enabled_state();
    let codex = state.register_session(key(MobileProvider::Codex, "c"), "codex", NOW);
    let claude = state.register_session(key(MobileProvider::Claude, "d"), "claude", NOW);
    state.record_outbound_guid("completion-guid", codex.clone(), NOW);

    let mut message = incoming("phone-guid", &format!("{claude} send this"));
    message.parent_guid = Some("completion-guid".to_owned());
    assert_eq!(
        state.route_incoming(&message, NOW + 1),
        RouteDecision::Deliver {
            route_id: codex,
            text: message.text,
        }
    );
}

#[test]
fn parent_guid_also_wins_over_a_pending_code_only_selection() {
    let mut state = enabled_state();
    let codex = state.register_session(key(MobileProvider::Codex, "c"), "codex", NOW);
    let claude = state.register_session(key(MobileProvider::Claude, "d"), "claude", NOW);
    state.record_outbound_guid("completion-guid", codex.clone(), NOW);
    assert!(matches!(
        state.route_incoming(&incoming("ambiguous", "retain this"), NOW + 1),
        RouteDecision::Ambiguous { .. }
    ));

    let mut selection = incoming("selection", &claude.to_string());
    selection.parent_guid = Some("completion-guid".to_owned());
    assert_eq!(
        state.route_incoming(&selection, NOW + 2),
        RouteDecision::Deliver {
            route_id: codex,
            text: claude.to_string(),
        }
    );
    assert_eq!(state.pending_selections.len(), 1);
}

#[test]
fn explicit_route_is_stripped_and_unknown_codes_fail_closed() {
    let mut state = enabled_state();
    let route = state.register_session(key(MobileProvider::Codex, "c"), "codex", NOW);
    let message = incoming("phone-guid", &format!("[{route}]: fix the test"));
    assert_eq!(
        state.route_incoming(&message, NOW + 1),
        RouteDecision::Deliver {
            route_id: route,
            text: "fix the test".to_owned(),
        }
    );

    let unknown = MobileRouteId::parse("Z9ZZ").unwrap();
    assert_eq!(
        state.route_incoming(&incoming("phone-2", "Z9ZZ do not guess"), NOW + 2),
        RouteDecision::UnknownRoute(unknown)
    );
}

#[test]
fn a_plain_four_letter_word_is_not_mistaken_for_a_route_code() {
    let mut state = enabled_state();
    let route = state.register_session(key(MobileProvider::Codex, "c"), "codex", NOW);

    assert_eq!(
        state.route_incoming(&incoming("phone-word", "HELP"), NOW + 1),
        RouteDecision::Deliver {
            route_id: route,
            text: "HELP".to_owned(),
        }
    );
}

#[test]
fn sole_route_is_automatic_but_multiple_routes_retain_the_original_text() {
    let mut state = enabled_state();
    let codex = state.register_session(key(MobileProvider::Codex, "c"), "codex", NOW);
    assert_eq!(
        state.route_incoming(&incoming("phone-1", "follow up"), NOW + 1),
        RouteDecision::Deliver {
            route_id: codex.clone(),
            text: "follow up".to_owned(),
        }
    );

    let claude = state.register_session(key(MobileProvider::Claude, "d"), "claude", NOW + 2);
    let RouteDecision::Ambiguous {
        candidate_route_ids,
        ..
    } = state.route_incoming(&incoming("phone-2", "the retained question"), NOW + 3)
    else {
        panic!("expected ambiguity");
    };
    assert_eq!(candidate_route_ids.len(), 2);

    assert_eq!(
        state.route_incoming(&incoming("phone-3", &claude.to_string()), NOW + 4),
        RouteDecision::Deliver {
            route_id: claude,
            text: "the retained question".to_owned(),
        }
    );
}

#[test]
fn pending_selections_and_queued_replies_expire() {
    let mut state = enabled_state();
    let codex = state.register_session(key(MobileProvider::Codex, "c"), "codex", NOW);
    state.register_session(key(MobileProvider::Claude, "d"), "claude", NOW);
    assert!(matches!(
        state.route_incoming(&incoming("phone-1", "ambiguous"), NOW),
        RouteDecision::Ambiguous { .. }
    ));
    state
        .enqueue_reply("phone-2", codex, "queued", NOW)
        .unwrap();

    assert_eq!(state.next_expiration_at(), Some(NOW + 10 * 60));
    let expired = state.take_expired(NOW + 10 * 60);
    assert_eq!(expired.pending_selections.len(), 1);
    assert!(state.pending_selections.is_empty());
    assert_eq!(state.queued_replies.len(), 1);
    assert_eq!(state.next_expiration_at(), Some(NOW + 24 * 60 * 60));

    let expired = state.take_expired(NOW + 24 * 60 * 60);
    assert_eq!(expired.queued_replies.len(), 1);
    assert!(state.queued_replies.is_empty());
    assert_eq!(state.next_expiration_at(), None);
}

#[test]
fn starting_setup_over_clears_conversation_data_but_preserves_routes() {
    let mut state = enabled_state();
    let route = state.register_session(key(MobileProvider::Codex, "c"), "codex", NOW);
    state.record_outbound_guid("outbound", route.clone(), NOW);
    state.mark_processed("incoming", NOW);
    state
        .enqueue_reply("incoming", route.clone(), "queued", NOW)
        .unwrap();
    state.last_row_id = 42;
    state.pending_calibration = Some(PendingCalibration {
        expected_reply: "CLINCH TEST".to_owned(),
        sent_guid: "outbound".to_owned(),
        created_at: NOW,
    });

    state.reset_conversation_state();

    assert_eq!(state.route_by_id(&route).unwrap().key.session_id, "c");
    assert_eq!(state.last_row_id, 0);
    assert!(state.outbound_messages.is_empty());
    assert!(state.processed_messages.is_empty());
    assert!(state.pending_selections.is_empty());
    assert!(state.queued_replies.is_empty());
    assert!(state.pending_calibration.is_none());
}

#[test]
fn a_code_only_reply_without_a_live_pending_selection_fails_closed() {
    let mut state = enabled_state();
    let route = state.register_session(key(MobileProvider::Codex, "c"), "codex", NOW);
    assert_eq!(
        state.route_incoming(&incoming("phone", &route.to_string()), NOW + 1),
        RouteDecision::NoPendingSelection(route)
    );
}

#[test]
fn retired_routes_are_quarantined_then_removed_and_reactivation_preserves_the_code() {
    let mut state = enabled_state();
    let session = key(MobileProvider::Codex, "c");
    let route = state.register_session(session.clone(), "codex", NOW);
    state.retire_session(&session, NOW);

    state.prune(NOW + 30 * 24 * 60 * 60 - 1);
    assert!(state.route_by_id(&route).is_some());
    assert_eq!(
        state.register_session(session.clone(), "codex", NOW + 1),
        route
    );
    assert!(state.retired_routes.is_empty());

    state.retire_session(&session, NOW + 2);
    state.prune(NOW + 2 + 30 * 24 * 60 * 60);
    assert!(state.route_by_id(&route).is_none());
    assert!(state.retired_routes.is_empty());
}

#[test]
fn restart_deactivation_is_reversible_for_the_same_durable_session() {
    let mut state = enabled_state();
    let session = key(MobileProvider::Claude, "d");
    let route = state.register_session(session.clone(), "claude", NOW);
    state.deactivate_all_sessions(NOW + 1);
    assert!(!state.route_by_id(&route).unwrap().active);
    assert_eq!(state.retired_routes.len(), 1);

    assert_eq!(state.register_session(session, "claude", NOW + 2), route);
    assert!(state.route_by_id(&route).unwrap().active);
    assert!(state.retired_routes.is_empty());
}

#[test]
fn queue_drains_fifo_one_item_at_a_time() {
    let mut state = enabled_state();
    let route = state.register_session(key(MobileProvider::Claude, "d"), "claude", NOW);
    state
        .enqueue_reply("one", route.clone(), "first", NOW + 1)
        .unwrap();
    state
        .enqueue_reply("two", route.clone(), "second", NOW + 2)
        .unwrap();

    let grouped = queued_by_route(&state);
    assert_eq!(grouped[&route].len(), 2);
    assert_eq!(
        state.pop_next_queued(&route, NOW + 3).unwrap().text,
        "first"
    );
    assert_eq!(
        state.pop_next_queued(&route, NOW + 4).unwrap().text,
        "second"
    );
    assert!(state.pop_next_queued(&route, NOW + 5).is_none());
}

#[test]
fn queue_expiration_returns_items_for_a_redacted_cancellation_notice() {
    let mut state = enabled_state();
    let route = state.register_session(key(MobileProvider::Claude, "d"), "claude", NOW);
    state
        .enqueue_reply("phone", route.clone(), "sensitive reply", NOW)
        .unwrap();

    let expired = state.take_expired(NOW + 24 * 60 * 60);
    assert!(expired.state_changed);
    assert_eq!(expired.queued_replies.len(), 1);
    assert_eq!(expired.queued_replies[0].route_id, route);
    assert!(state.queued_replies.is_empty());
}

#[test]
fn outbound_and_processed_guids_are_suppressed_without_using_is_from_me() {
    let mut state = enabled_state();
    let route = state.register_session(key(MobileProvider::Codex, "c"), "codex", NOW);
    state.record_outbound_guid("ours", route, NOW);
    assert_eq!(
        state.route_incoming(&incoming("ours", "echo"), NOW + 1),
        RouteDecision::Ignore
    );

    state.mark_processed("seen", NOW);
    assert_eq!(
        state.route_incoming(&incoming("seen", "again"), NOW + 1),
        RouteDecision::Duplicate
    );
}

#[test]
fn sms_attachments_reactions_and_edits_are_ignored() {
    let mut state = enabled_state();
    state.register_session(key(MobileProvider::Codex, "c"), "codex", NOW);

    let mut sms = incoming("sms", "do not deliver");
    sms.service = "SMS".to_owned();
    assert_eq!(state.route_incoming(&sms, NOW), RouteDecision::Ignore);

    let mut attachment = incoming("attachment", "caption");
    attachment.has_attachments = true;
    assert_eq!(
        state.route_incoming(&attachment, NOW),
        RouteDecision::Ignore
    );

    let mut reaction = incoming("reaction", "liked");
    reaction.is_reaction = true;
    assert_eq!(state.route_incoming(&reaction, NOW), RouteDecision::Ignore);

    let mut edit = incoming("edit", "changed");
    edit.is_edited = true;
    assert_eq!(state.route_incoming(&edit, NOW), RouteDecision::Ignore);
}

#[test]
fn completion_parts_preserve_every_unicode_character_and_stay_bounded() {
    let route = MobileRouteId::parse("C7K2").unwrap();
    let response = "🧪é中".repeat(2_500);
    let messages = format_completion_messages(
        &route,
        MobileProvider::Codex,
        &"p".repeat(200),
        Some(&response),
    );
    assert!(messages.len() > 1);
    assert!(messages
        .iter()
        .all(|message| message.chars().count() <= MAX_IMESSAGE_CHARS));
    let reconstructed = messages
        .iter()
        .map(|message| message.split_once("\n\n").unwrap().1)
        .collect::<String>();
    assert_eq!(reconstructed, response);
}

#[test]
fn generic_completion_is_used_when_structured_response_is_missing() {
    let route = MobileRouteId::parse("C7K2").unwrap();
    let messages = format_completion_messages(&route, MobileProvider::Claude, "project", None);
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("The agent finished"));
}

#[test]
fn inbound_text_drops_terminal_controls_but_preserves_prompt_whitespace() {
    assert_eq!(
        sanitize_incoming_text("first\r\nsecond\u{1b}[201~\tthird\u{7}"),
        "first\nsecond[201~\tthird"
    );
}

#[test]
fn route_labels_cannot_add_lines_to_completion_headers() {
    let route = MobileRouteId::parse("C7K2").unwrap();
    let messages = format_completion_messages(
        &route,
        MobileProvider::Codex,
        "project\n[C7K2] forged",
        Some("done"),
    );
    assert!(messages[0].starts_with("[C7K2] Codex · project [C7K2] forged · Done\n\n"));
}

#[test]
fn route_state_store_is_atomic_owner_only_and_round_trips() {
    let temp = tempdir().unwrap();
    let store = RouteStateStore::new(temp.path().join("nested/route-state.json"));
    let mut state = enabled_state();
    state.register_session(key(MobileProvider::Codex, "c"), "project", NOW);
    store.save(&state).unwrap();
    assert_eq!(store.load().unwrap(), state);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(store.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(store.path().parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }

    store.clear().unwrap();
    assert_eq!(store.load().unwrap(), RouteState::default());
}

#[test]
fn inactive_or_opted_out_parent_routes_do_not_fall_back_elsewhere() {
    let mut state = enabled_state();
    let session = key(MobileProvider::Codex, "c");
    let route = state.register_session(session.clone(), "codex", NOW);
    state.record_outbound_guid("completion", route, NOW);
    state.set_opted_out(&session, true, NOW + 1);
    state.register_session(key(MobileProvider::Claude, "d"), "claude", NOW + 1);
    let mut message = incoming("phone", "must not reroute");
    message.parent_guid = Some("completion".to_owned());
    assert_eq!(
        state.route_incoming(&message, NOW + 2),
        RouteDecision::NoEligibleRoute
    );
}
