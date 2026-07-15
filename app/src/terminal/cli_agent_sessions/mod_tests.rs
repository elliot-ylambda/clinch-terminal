use warp_core::channel::{Channel, ChannelConfig, ChannelState};
use warp_core::AppId;
use warpui::{App, Entity, EntityId};

use super::event::{
    parse_event, CLIAgentEvent, CLIAgentEventPayload, CLIAgentEventSource, CLIAgentEventType,
};
use super::{
    merge_loaded_and_live_history, prompt_from_trusted_event, session_context_enabled_for,
    CLIAgentInputEntrypoint, CLIAgentInputState, CLIAgentSession, CLIAgentSessionContext,
    CLIAgentSessionKey, CLIAgentSessionStatus, CLIAgentSessionsModel, CLIAgentSessionsModelEvent,
    PromptHistoryLoadState,
};
use crate::agent_resume::{AgentPrompt, AgentPromptHistory, AgentResumeProvider};
use crate::ai::blocklist::{InputConfig, InputType};
use crate::terminal::CLIAgent;

#[derive(Default)]
struct CapturedSessionEvents(Vec<CLIAgentSessionsModelEvent>);

impl Entity for CapturedSessionEvents {
    type Event = ();
}

#[test]
fn parse_stop_notification() {
    let body = r#"{"v":1,"agent":"claude","event":"stop","session_id":"abc","cwd":"/tmp/proj","project":"proj","query":"write a haiku","response":"Memory is safe","transcript_path":"/tmp/t.jsonl"}"#;
    let notif = parse_event(Some("warp://cli-agent"), body).unwrap();

    assert_eq!(notif.v, 1);
    assert_eq!(notif.agent, CLIAgent::Claude);
    assert_eq!(notif.event, CLIAgentEventType::Stop);
    assert_eq!(notif.session_id.as_deref(), Some("abc"));
    assert_eq!(notif.cwd.as_deref(), Some("/tmp/proj"));
    assert_eq!(notif.project.as_deref(), Some("proj"));
    assert_eq!(notif.payload.query.as_deref(), Some("write a haiku"));
    assert_eq!(notif.payload.response.as_deref(), Some("Memory is safe"));
    assert_eq!(
        notif.payload.transcript_path.as_deref(),
        Some("/tmp/t.jsonl")
    );
}

#[test]
fn cli_agent_session_context_title_like_text_uses_trimmed_summary() {
    let context = CLIAgentSessionContext {
        summary: Some("  Reviewing changes  ".to_string()),
        query: Some("Latest prompt".to_string()),
        ..Default::default()
    };

    assert_eq!(
        context.title_like_text(),
        Some("Reviewing changes".to_string())
    );
}

#[test]
fn cli_agent_session_context_latest_user_prompt_uses_trimmed_query() {
    let context = CLIAgentSessionContext {
        summary: Some("Reviewing changes".to_string()),
        query: Some("  Latest prompt  ".to_string()),
        ..Default::default()
    };

    assert_eq!(
        context.latest_user_prompt(),
        Some("Latest prompt".to_string())
    );
}

#[test]
fn cli_agent_session_context_title_helpers_ignore_empty_text() {
    let context = CLIAgentSessionContext {
        summary: Some("  ".to_string()),
        query: Some("".to_string()),
        ..Default::default()
    };

    assert_eq!(context.title_like_text(), None);
    assert_eq!(context.latest_user_prompt(), None);
}

#[test]
fn parse_permission_request_notification() {
    let body = r#"{"v":1,"agent":"claude","event":"permission_request","session_id":"abc","cwd":"/tmp/proj","project":"proj","summary":"Wants to run Bash: rm -rf /tmp","tool_name":"Bash","tool_input":{"command":"rm -rf /tmp"}}"#;
    let notif = parse_event(Some("warp://cli-agent"), body).unwrap();

    assert_eq!(notif.event, CLIAgentEventType::PermissionRequest);
    assert_eq!(
        notif.payload.summary.as_deref(),
        Some("Wants to run Bash: rm -rf /tmp")
    );
    assert_eq!(notif.payload.tool_name.as_deref(), Some("Bash"));
    assert_eq!(
        notif.payload.tool_input_preview.as_deref(),
        Some("rm -rf /tmp")
    );
}

#[test]
fn parse_permission_request_with_file_path() {
    let body = r#"{"v":1,"agent":"claude","event":"permission_request","session_id":"abc","cwd":"/tmp","project":"tmp","tool_name":"Write","tool_input":{"file_path":"/tmp/test.py","content":"print('hi')"}}"#;
    let notif = parse_event(Some("warp://cli-agent"), body).unwrap();

    assert_eq!(
        notif.payload.tool_input_preview.as_deref(),
        Some("/tmp/test.py")
    );
}

#[test]
fn parse_idle_prompt_notification() {
    let body = r#"{"v":1,"agent":"claude","event":"idle_prompt","session_id":"abc","cwd":"/tmp","project":"tmp","summary":"Claude is waiting for your input"}"#;
    let notif = parse_event(Some("warp://cli-agent"), body).unwrap();

    assert_eq!(notif.event, CLIAgentEventType::IdlePrompt);
    assert_eq!(
        notif.payload.summary.as_deref(),
        Some("Claude is waiting for your input")
    );
}

#[test]
fn parse_session_start_notification() {
    let body = r#"{"v":1,"agent":"claude","event":"session_start","session_id":"abc","cwd":"/tmp","project":"tmp","plugin_version":"1.1.0"}"#;
    let notif = parse_event(Some("warp://cli-agent"), body).unwrap();

    assert_eq!(notif.event, CLIAgentEventType::SessionStart);
    assert_eq!(notif.payload.plugin_version.as_deref(), Some("1.1.0"));
}

#[test]
fn returns_none_for_wrong_sentinel() {
    let body = r#"{"v":1,"agent":"claude","event":"stop"}"#;
    assert!(parse_event(Some("Claude Code"), body).is_none());
}

#[test]
fn returns_none_for_missing_title() {
    let body = r#"{"v":1,"agent":"claude","event":"stop"}"#;
    assert!(parse_event(None, body).is_none());
}

#[test]
fn returns_none_for_invalid_json() {
    assert!(parse_event(Some("warp://cli-agent"), "not json").is_none());
}

#[test]
fn handles_unknown_event_type() {
    let body = r#"{"v":1,"agent":"claude","event":"some_future_event"}"#;
    let notif = parse_event(Some("warp://cli-agent"), body).unwrap();
    assert_eq!(
        notif.event,
        CLIAgentEventType::Unknown("some_future_event".to_string())
    );
}

#[test]
fn handles_missing_optional_fields() {
    let body = r#"{"event":"stop"}"#;
    let notif = parse_event(Some("warp://cli-agent"), body).unwrap();

    assert_eq!(notif.v, 1);
    assert_eq!(notif.agent, CLIAgent::Unknown);
    assert_eq!(notif.event, CLIAgentEventType::Stop);
    assert!(notif.session_id.is_none());
    assert!(notif.cwd.is_none());
    assert!(notif.project.is_none());
    assert!(notif.payload.query.is_none());
}

#[test]
fn handles_special_characters_in_values() {
    let body = r#"{"v":1,"agent":"claude","event":"stop","query":"what does \"hello\" mean?","response":"It means greeting. Use: printf(\"hello\")"}"#;
    let notif = parse_event(Some("warp://cli-agent"), body).unwrap();

    assert_eq!(
        notif.payload.query.as_deref(),
        Some("what does \"hello\" mean?")
    );
    assert_eq!(
        notif.payload.response.as_deref(),
        Some("It means greeting. Use: printf(\"hello\")")
    );
}

#[test]
fn rejects_unsupported_schema_version() {
    let body = r#"{"v":2,"agent":"claude","event":"stop"}"#;
    assert!(parse_event(Some("warp://cli-agent"), body).is_none());
}

#[test]
fn defaults_to_v1_when_version_missing() {
    let body = r#"{"agent":"claude","event":"stop","query":"hi"}"#;
    let notif = parse_event(Some("warp://cli-agent"), body).unwrap();
    assert_eq!(notif.v, 1);
    assert_eq!(notif.payload.query.as_deref(), Some("hi"));
}

#[test]
fn explicit_v1_parses_correctly() {
    let body = r#"{"v":1,"agent":"claude","event":"stop","query":"test"}"#;
    let notif = parse_event(Some("warp://cli-agent"), body).unwrap();
    assert_eq!(notif.v, 1);
    assert_eq!(notif.payload.query.as_deref(), Some("test"));
}

#[test]
fn parse_prompt_submit_notification() {
    let body = r#"{"v":1,"agent":"claude","event":"prompt_submit","session_id":"abc","cwd":"/tmp/proj","project":"proj","query":"fix the bug"}"#;
    let notif = parse_event(Some("warp://cli-agent"), body).unwrap();

    assert_eq!(notif.event, CLIAgentEventType::PromptSubmit);
    assert_eq!(notif.payload.query.as_deref(), Some("fix the bug"));
}

#[test]
fn parse_tool_complete_notification() {
    let body = r#"{"v":1,"agent":"claude","event":"tool_complete","session_id":"abc","cwd":"/tmp/proj","project":"proj","tool_name":"Bash"}"#;
    let notif = parse_event(Some("warp://cli-agent"), body).unwrap();

    assert_eq!(notif.event, CLIAgentEventType::ToolComplete);
    assert_eq!(notif.payload.tool_name.as_deref(), Some("Bash"));
}

#[test]
fn parse_auggie_stop_notification() {
    // Mirrors what the community auggie-warp plugin emits on the Stop hook.
    let body = r#"{"v":1,"agent":"auggie","event":"stop","session_id":"abc","cwd":"/tmp/proj","project":"proj","query":"write a haiku","response":"Memory is safe"}"#;
    let notif = parse_event(Some("warp://cli-agent"), body).unwrap();

    assert_eq!(notif.agent, CLIAgent::Auggie);
    assert_eq!(notif.event, CLIAgentEventType::Stop);
    assert_eq!(notif.payload.query.as_deref(), Some("write a haiku"));
    assert_eq!(notif.payload.response.as_deref(), Some("Memory is safe"));
}

#[test]
fn parse_pi_stop_notification() {
    // Mirrors what the community pi-mono plugin emits on the Stop hook —
    // matches the Auggie shape and uses `"agent":"pi"`, which `resolve_agent`
    // already maps to `CLIAgent::Pi` via `command_prefix()`.
    let body = r#"{"v":1,"agent":"pi","event":"stop","session_id":"abc","cwd":"/tmp/proj","project":"proj","query":"write a haiku","response":"Memory is safe"}"#;
    let notif = parse_event(Some("warp://cli-agent"), body).unwrap();

    assert_eq!(notif.agent, CLIAgent::Pi);
    assert_eq!(notif.event, CLIAgentEventType::Stop);
    assert_eq!(notif.payload.query.as_deref(), Some("write a haiku"));
    assert_eq!(notif.payload.response.as_deref(), Some("Memory is safe"));
}

fn idle_test_session(agent: CLIAgent) -> CLIAgentSession {
    CLIAgentSession {
        agent,
        status: CLIAgentSessionStatus::InProgress,
        session_context: CLIAgentSessionContext::default(),
        input_state: CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        remote_host: None,
        plugin_version: None,
        draft_text: None,
        custom_command_prefix: None,
        received_rich_notification: true,
        has_observed_turn_activity: false,
        prompt_history: Default::default(),
        prompt_history_load_state: Default::default(),
        prompt_history_generation: 0,
    }
}

fn activity_event(agent: CLIAgent, event: CLIAgentEventType) -> CLIAgentEvent {
    CLIAgentEvent {
        source: CLIAgentEventSource::RichPlugin,
        v: 1,
        agent,
        event,
        session_id: Some("session-id".to_owned()),
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload {
            query: Some("fix the bug".to_owned()),
            ..Default::default()
        },
    }
}

#[test]
fn open_claude_and_codex_sessions_are_not_active_turns() {
    for agent in [CLIAgent::Claude, CLIAgent::Codex] {
        let session = idle_test_session(agent);
        assert!(matches!(session.status, CLIAgentSessionStatus::InProgress));
        assert!(!session.is_actively_working());
    }
}

#[test]
fn prompt_submit_is_active_until_stop() {
    for agent in [CLIAgent::Claude, CLIAgent::Codex] {
        let mut session = idle_test_session(agent);

        session.apply_event(&activity_event(agent, CLIAgentEventType::PromptSubmit));
        assert!(session.is_actively_working());

        session.apply_event(&activity_event(agent, CLIAgentEventType::Stop));
        assert!(matches!(session.status, CLIAgentSessionStatus::Success));
        assert!(!session.is_actively_working());
    }
}

#[test]
fn idle_prompt_only_completes_an_observed_turn() {
    let mut session = idle_test_session(CLIAgent::Claude);

    assert_eq!(
        session.apply_event(&activity_event(
            CLIAgent::Claude,
            CLIAgentEventType::IdlePrompt,
        )),
        None
    );
    assert!(!session.is_actively_working());

    session.apply_event(&activity_event(
        CLIAgent::Claude,
        CLIAgentEventType::PromptSubmit,
    ));
    assert!(session.is_actively_working());

    assert_eq!(
        session.apply_event(&activity_event(
            CLIAgent::Claude,
            CLIAgentEventType::IdlePrompt,
        )),
        Some(CLIAgentSessionStatus::Success)
    );
    assert!(!session.is_actively_working());
}

#[test]
fn apply_event_preserves_input_session() {
    let input_state = CLIAgentInputState::Open {
        entrypoint: CLIAgentInputEntrypoint::CtrlG,
        previous_input_config: InputConfig {
            input_type: InputType::Shell,
            is_locked: false,
        },
        previous_was_lock_set_with_empty_buffer: true,
    };
    let mut session = CLIAgentSession {
        agent: CLIAgent::Claude,
        status: CLIAgentSessionStatus::InProgress,
        session_context: CLIAgentSessionContext::default(),
        input_state,
        should_auto_toggle_input: false,
        listener: None,
        remote_host: None,
        plugin_version: None,
        draft_text: None,
        custom_command_prefix: None,
        received_rich_notification: false,
        has_observed_turn_activity: false,
        prompt_history: Default::default(),
        prompt_history_load_state: Default::default(),
        prompt_history_generation: 0,
    };

    let event = CLIAgentEvent {
        source: CLIAgentEventSource::RichPlugin,
        v: 1,
        agent: CLIAgent::Claude,
        event: CLIAgentEventType::PermissionRequest,
        session_id: Some("abc".to_string()),
        cwd: Some("/tmp/proj".to_string()),
        project: Some("proj".to_string()),
        payload: CLIAgentEventPayload {
            summary: Some("Needs approval".to_string()),
            ..Default::default()
        },
    };

    session.apply_event(&event);

    assert_eq!(session.input_state, input_state);
}

#[test]
fn is_remote_returns_true_when_remote_host_is_set() {
    let session = CLIAgentSession {
        agent: CLIAgent::Claude,
        status: CLIAgentSessionStatus::InProgress,
        session_context: CLIAgentSessionContext::default(),
        input_state: CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        plugin_version: None,
        draft_text: None,
        remote_host: Some("user@devbox".to_owned()),
        custom_command_prefix: None,
        received_rich_notification: false,
        has_observed_turn_activity: false,
        prompt_history: Default::default(),
        prompt_history_load_state: Default::default(),
        prompt_history_generation: 0,
    };
    assert!(session.is_remote());
}

#[test]
fn is_remote_returns_false_when_remote_host_is_none() {
    let session = CLIAgentSession {
        agent: CLIAgent::Claude,
        status: CLIAgentSessionStatus::InProgress,
        session_context: CLIAgentSessionContext::default(),
        input_state: CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        remote_host: None,
        plugin_version: None,
        draft_text: None,
        custom_command_prefix: None,
        received_rich_notification: false,
        has_observed_turn_activity: false,
        prompt_history: Default::default(),
        prompt_history_load_state: Default::default(),
        prompt_history_generation: 0,
    };
    assert!(!session.is_remote());
}

#[test]
fn local_failure_is_shared_across_local_sessions() {
    let mut model = CLIAgentSessionsModel::new();

    model.record_plugin_auto_failure(CLIAgent::Claude, None);

    assert!(model.has_plugin_auto_failed(CLIAgent::Claude, &None));
}

#[test]
fn local_failure_does_not_affect_remote_host() {
    let mut model = CLIAgentSessionsModel::new();

    model.record_plugin_auto_failure(CLIAgent::Claude, None);

    let remote = Some("user@devbox".to_owned());
    assert!(!model.has_plugin_auto_failed(CLIAgent::Claude, &remote));
}

#[test]
fn remote_failure_does_not_affect_local() {
    let mut model = CLIAgentSessionsModel::new();

    model.record_plugin_auto_failure(CLIAgent::Claude, Some("user@devbox".to_owned()));

    assert!(!model.has_plugin_auto_failed(CLIAgent::Claude, &None));
}

#[test]
fn remote_failures_are_independent_per_host() {
    let mut model = CLIAgentSessionsModel::new();

    let host_a = Some("user@host-a".to_owned());
    let host_b = Some("user@host-b".to_owned());

    model.record_plugin_auto_failure(CLIAgent::Claude, host_a.clone());

    assert!(model.has_plugin_auto_failed(CLIAgent::Claude, &host_a));
    assert!(!model.has_plugin_auto_failed(CLIAgent::Claude, &host_b));
}

#[test]
fn failure_tracking_is_independent_per_agent() {
    let mut model = CLIAgentSessionsModel::new();

    model.record_plugin_auto_failure(CLIAgent::Claude, None);

    assert!(model.has_plugin_auto_failed(CLIAgent::Claude, &None));
    assert!(!model.has_plugin_auto_failed(CLIAgent::Gemini, &None));
}

#[test]
fn session_start_sets_plugin_version() {
    let mut session = CLIAgentSession {
        agent: CLIAgent::Claude,
        status: CLIAgentSessionStatus::InProgress,
        session_context: CLIAgentSessionContext::default(),
        input_state: CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        plugin_version: None,
        draft_text: None,
        remote_host: None,
        custom_command_prefix: None,
        received_rich_notification: false,
        has_observed_turn_activity: false,
        prompt_history: Default::default(),
        prompt_history_load_state: Default::default(),
        prompt_history_generation: 0,
    };

    let event = CLIAgentEvent {
        source: CLIAgentEventSource::RichPlugin,
        v: 1,
        agent: CLIAgent::Claude,
        event: CLIAgentEventType::SessionStart,
        session_id: Some("abc".to_owned()),
        cwd: Some("/tmp".to_owned()),
        project: Some("proj".to_owned()),
        payload: CLIAgentEventPayload {
            plugin_version: Some("1.5.0".to_owned()),
            ..Default::default()
        },
    };

    session.apply_event(&event);
    assert_eq!(session.plugin_version.as_deref(), Some("1.5.0"));
}

#[test]
fn session_start_without_plugin_version_leaves_none() {
    let mut session = CLIAgentSession {
        agent: CLIAgent::Claude,
        status: CLIAgentSessionStatus::InProgress,
        session_context: CLIAgentSessionContext::default(),
        input_state: CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        plugin_version: None,
        draft_text: None,
        remote_host: None,
        custom_command_prefix: None,
        received_rich_notification: false,
        has_observed_turn_activity: false,
        prompt_history: Default::default(),
        prompt_history_load_state: Default::default(),
        prompt_history_generation: 0,
    };

    let event = CLIAgentEvent {
        source: CLIAgentEventSource::RichPlugin,
        v: 1,
        agent: CLIAgent::Claude,
        event: CLIAgentEventType::SessionStart,
        session_id: Some("abc".to_owned()),
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload::default(),
    };

    session.apply_event(&event);
    assert_eq!(session.plugin_version, None);
}

#[test]
fn codex_session_not_rich_until_rich_notification() {
    // Codex's OSC 9 fallback never sets `received_rich_notification`, so the
    // session must not claim rich status even when a fallback listener exists.
    let mut session = CLIAgentSession {
        agent: CLIAgent::Codex,
        status: CLIAgentSessionStatus::InProgress,
        session_context: CLIAgentSessionContext::default(),
        input_state: CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        plugin_version: None,
        remote_host: None,
        draft_text: None,
        custom_command_prefix: None,
        received_rich_notification: false,
        has_observed_turn_activity: false,
        prompt_history: Default::default(),
        prompt_history_load_state: Default::default(),
        prompt_history_generation: 0,
    };
    assert!(!session.supports_rich_status());

    // A structured OSC 777 notification latches the flag -> rich status.
    session.received_rich_notification = true;
    assert!(session.supports_rich_status());
}

#[test]
fn non_codex_session_rich_after_rich_notification() {
    let mut session = CLIAgentSession {
        agent: CLIAgent::Claude,
        status: CLIAgentSessionStatus::InProgress,
        session_context: CLIAgentSessionContext::default(),
        input_state: CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        plugin_version: None,
        remote_host: None,
        draft_text: None,
        custom_command_prefix: None,
        received_rich_notification: false,
        has_observed_turn_activity: false,
        prompt_history: Default::default(),
        prompt_history_load_state: Default::default(),
        prompt_history_generation: 0,
    };
    // No listener and no rich notification yet.
    assert!(!session.supports_rich_status());

    session.received_rich_notification = true;
    assert!(session.supports_rich_status());
}

/// Constructs a session with permission-scoped state already populated, as if
/// a `PermissionRequest` had just been received and the agent is now Blocked.
/// Used by the GH-9525 regression tests below.
fn blocked_claude_session_with_permission_state() -> CLIAgentSession {
    CLIAgentSession {
        agent: CLIAgent::Claude,
        status: CLIAgentSessionStatus::Blocked {
            message: Some("Wants to run bash: rm -rf /tmp".to_owned()),
        },
        session_context: CLIAgentSessionContext {
            summary: Some("Wants to run bash: rm -rf /tmp".to_owned()),
            tool_name: Some("Bash".to_owned()),
            tool_input_preview: Some("rm -rf /tmp".to_owned()),
            ..Default::default()
        },
        input_state: CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        plugin_version: None,
        draft_text: None,
        remote_host: None,
        custom_command_prefix: None,
        received_rich_notification: false,
        has_observed_turn_activity: false,
        prompt_history: Default::default(),
        prompt_history_load_state: Default::default(),
        prompt_history_generation: 0,
    }
}

#[test]
fn stop_clears_permission_scoped_state() {
    // GH-9525: after a PermissionRequest sets `summary`, the Stop event must
    // clear it. Otherwise the tab title falls back to the stale permission
    // text instead of reflecting the now-completed session.
    let mut session = blocked_claude_session_with_permission_state();

    let event = CLIAgentEvent {
        source: CLIAgentEventSource::RichPlugin,
        v: 1,
        agent: CLIAgent::Claude,
        event: CLIAgentEventType::Stop,
        session_id: Some("abc".to_owned()),
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload {
            query: Some("write a haiku".to_owned()),
            response: Some("Memory is safe".to_owned()),
            ..Default::default()
        },
    };

    session.apply_event(&event);

    assert_eq!(session.session_context.summary, None);
    assert_eq!(session.session_context.tool_name, None);
    assert_eq!(session.session_context.tool_input_preview, None);
    assert_eq!(
        session.session_context.query.as_deref(),
        Some("write a haiku"),
    );
    assert_eq!(
        session.session_context.response.as_deref(),
        Some("Memory is safe"),
    );
    assert!(matches!(session.status, CLIAgentSessionStatus::Success));
}

#[test]
fn permission_replied_clears_permission_scoped_state() {
    // When the user replies to a permission prompt the agent transitions back
    // to InProgress; the now-stale summary/tool fields must be cleared so they
    // don't leak into UI surfaces during the next turn.
    let mut session = blocked_claude_session_with_permission_state();

    let event = CLIAgentEvent {
        source: CLIAgentEventSource::RichPlugin,
        v: 1,
        agent: CLIAgent::Claude,
        event: CLIAgentEventType::PermissionReplied,
        session_id: Some("abc".to_owned()),
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload::default(),
    };

    session.apply_event(&event);

    assert_eq!(session.session_context.summary, None);
    assert_eq!(session.session_context.tool_name, None);
    assert_eq!(session.session_context.tool_input_preview, None);
    assert!(matches!(session.status, CLIAgentSessionStatus::InProgress));
}

#[test]
fn prompt_submit_clears_permission_scoped_state() {
    // PromptSubmit already clears `response`; clearing the permission-scoped
    // fields keeps the same hygiene if the user manages to start a new turn
    // while permission state is still populated (e.g. an abandoned permission
    // flow that was not closed by an explicit PermissionReplied).
    let mut session = blocked_claude_session_with_permission_state();
    session.session_context.response = Some("stale response".to_owned());

    let event = CLIAgentEvent {
        source: CLIAgentEventSource::RichPlugin,
        v: 1,
        agent: CLIAgent::Claude,
        event: CLIAgentEventType::PromptSubmit,
        session_id: Some("abc".to_owned()),
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload {
            query: Some("next prompt".to_owned()),
            ..Default::default()
        },
    };

    session.apply_event(&event);

    assert_eq!(session.session_context.summary, None);
    assert_eq!(session.session_context.tool_name, None);
    assert_eq!(session.session_context.tool_input_preview, None);
    assert_eq!(session.session_context.response, None);
    assert_eq!(
        session.session_context.query.as_deref(),
        Some("next prompt")
    );
    assert!(matches!(session.status, CLIAgentSessionStatus::InProgress));
}

#[test]
fn permission_request_still_populates_summary_and_tool_fields() {
    // Sanity: clearing permission-scoped state on Stop/Reply/Submit must not
    // also break the PermissionRequest path that initially populates them.
    let mut session = CLIAgentSession {
        agent: CLIAgent::Claude,
        status: CLIAgentSessionStatus::InProgress,
        session_context: CLIAgentSessionContext::default(),
        input_state: CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        plugin_version: None,
        draft_text: None,
        remote_host: None,
        custom_command_prefix: None,
        received_rich_notification: false,
        has_observed_turn_activity: false,
        prompt_history: Default::default(),
        prompt_history_load_state: Default::default(),
        prompt_history_generation: 0,
    };

    let event = CLIAgentEvent {
        source: CLIAgentEventSource::RichPlugin,
        v: 1,
        agent: CLIAgent::Claude,
        event: CLIAgentEventType::PermissionRequest,
        session_id: Some("abc".to_owned()),
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload {
            summary: Some("Wants to run bash: rm -rf /tmp".to_owned()),
            tool_name: Some("Bash".to_owned()),
            tool_input_preview: Some("rm -rf /tmp".to_owned()),
            ..Default::default()
        },
    };

    session.apply_event(&event);

    assert_eq!(
        session.session_context.summary.as_deref(),
        Some("Wants to run bash: rm -rf /tmp"),
    );
    assert_eq!(session.session_context.tool_name.as_deref(), Some("Bash"));
    assert_eq!(
        session.session_context.tool_input_preview.as_deref(),
        Some("rm -rf /tmp"),
    );
    assert!(matches!(
        session.status,
        CLIAgentSessionStatus::Blocked { .. },
    ));
}

#[test]
fn prompt_history_merge_preserves_repeated_turns_by_occurrence() {
    let prompt = |text: &str| AgentPrompt {
        timestamp: None,
        text: text.to_owned(),
    };
    let loaded = AgentPromptHistory {
        prompts: vec![prompt("repeat"), prompt("repeat")],
        is_partial: true,
    };
    let live = AgentPromptHistory {
        prompts: vec![
            prompt("repeat"),
            prompt("repeat"),
            prompt("repeat"),
            prompt("follow up"),
        ],
        is_partial: false,
    };

    let merged = merge_loaded_and_live_history(loaded, live);

    assert_eq!(
        merged
            .prompts
            .iter()
            .map(|prompt| prompt.text.as_str())
            .collect::<Vec<_>>(),
        vec!["repeat", "repeat", "repeat", "follow up"]
    );
    assert!(merged.is_partial);
}

#[test]
fn only_rich_prompt_submit_is_trusted_as_user_history() {
    let event = |source, event, query: Option<&str>| CLIAgentEvent {
        source,
        v: 1,
        agent: CLIAgent::Codex,
        event,
        session_id: Some("session".to_owned()),
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload {
            query: query.map(str::to_owned),
            ..Default::default()
        },
    };

    assert_eq!(
        prompt_from_trusted_event(&event(
            CLIAgentEventSource::RichPlugin,
            CLIAgentEventType::PromptSubmit,
            Some("exact\nmessage"),
        ))
        .map(|prompt| prompt.text),
        Some("exact\nmessage".to_owned())
    );
    assert!(prompt_from_trusted_event(&event(
        CLIAgentEventSource::RichPlugin,
        CLIAgentEventType::Stop,
        Some("repeated by stop"),
    ))
    .is_none());
    assert!(prompt_from_trusted_event(&event(
        CLIAgentEventSource::CodexOsc9Fallback,
        CLIAgentEventType::PromptSubmit,
        Some("opaque OSC 9 body"),
    ))
    .is_none());
    assert!(prompt_from_trusted_event(&event(
        CLIAgentEventSource::RichPlugin,
        CLIAgentEventType::PromptSubmit,
        Some("  \n"),
    ))
    .is_none());
}

/// This test mutates the process-global channel state. The repository's required nextest runner
/// isolates tests by process, so the Clinch app ID cannot leak into another test.
#[test]
fn appending_prompt_without_session_id_emits_session_updated() {
    ChannelState::set(ChannelState::new(
        Channel::Local,
        ChannelConfig::no_backend(AppId::new("sh", "clinch", "ClinchDev"), "clinch-test.log"),
    ));

    App::test((), |mut app| async move {
        let sessions = app.add_model(|_| CLIAgentSessionsModel::new());
        let captured_events = app.add_model(|_| CapturedSessionEvents::default());
        captured_events.update(&mut app, |_, ctx| {
            ctx.subscribe_to_model(&sessions, |captured, _, event, _| {
                captured.0.push(event.clone());
            });
        });

        let terminal_view_id = EntityId::new();
        sessions.update(&mut app, |model, ctx| {
            model.set_session(terminal_view_id, idle_test_session(CLIAgent::Claude), ctx);
        });
        captured_events.update(&mut app, |captured, _| captured.0.clear());

        sessions.update(&mut app, |model, ctx| {
            model.update_from_event(
                terminal_view_id,
                &CLIAgentEvent {
                    source: CLIAgentEventSource::RichPlugin,
                    v: 1,
                    agent: CLIAgent::Claude,
                    event: CLIAgentEventType::PromptSubmit,
                    session_id: None,
                    cwd: None,
                    project: None,
                    payload: CLIAgentEventPayload {
                        query: Some("hi cc".to_owned()),
                        ..Default::default()
                    },
                },
                ctx,
            );
        });

        sessions.read(&app, |model, _| {
            let session = model.session(terminal_view_id).expect("session exists");
            assert_eq!(session.prompt_count(), 1);
            assert_eq!(session.latest_prompt().unwrap().text, "hi cc");
        });
        captured_events.read(&app, |captured, _| {
            assert!(captured.0.iter().any(|event| matches!(
                event,
                CLIAgentSessionsModelEvent::SessionUpdated {
                    terminal_view_id: updated_view_id,
                    agent: CLIAgent::Claude,
                } if *updated_view_id == terminal_view_id
            )));
        });
    });
}

#[test]
fn history_load_key_rejects_stale_session_or_generation() {
    let key = CLIAgentSessionKey {
        provider: AgentResumeProvider::Claude,
        session_id: "one".to_owned(),
    };
    let other_key = CLIAgentSessionKey {
        provider: AgentResumeProvider::Claude,
        session_id: "two".to_owned(),
    };
    let state = PromptHistoryLoadState::Loading {
        key: key.clone(),
        generation: 7,
    };

    assert!(state.matches_loading(&key, 7));
    assert!(!state.matches_loading(&key, 6));
    assert!(!state.matches_loading(&other_key, 7));
}

#[test]
fn listener_events_cannot_replace_the_outer_session_identity() {
    let mut session = CLIAgentSession {
        agent: CLIAgent::Claude,
        status: CLIAgentSessionStatus::InProgress,
        session_context: CLIAgentSessionContext {
            session_id: Some("outer".to_owned()),
            ..Default::default()
        },
        input_state: CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        plugin_version: None,
        draft_text: None,
        remote_host: None,
        custom_command_prefix: None,
        received_rich_notification: true,
        has_observed_turn_activity: true,
        prompt_history: AgentPromptHistory {
            prompts: vec![AgentPrompt {
                timestamp: None,
                text: "outer prompt".to_owned(),
            }],
            is_partial: false,
        },
        prompt_history_load_state: PromptHistoryLoadState::Ready,
        prompt_history_generation: 3,
    };
    let event = |event| CLIAgentEvent {
        source: CLIAgentEventSource::RichPlugin,
        v: 1,
        agent: CLIAgent::Claude,
        event,
        session_id: Some("nested-or-new".to_owned()),
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload::default(),
    };

    assert!(!session.prepare_for_event_identity(&event(CLIAgentEventType::PromptSubmit)));
    assert_eq!(session.prompt_count(), 1);
    assert_eq!(session.prompt_history_generation, 3);

    assert!(!session.prepare_for_event_identity(&event(CLIAgentEventType::SessionStart)));
    assert_eq!(session.prompt_count(), 1);
    assert_eq!(session.prompt_history_generation, 3);

    // Explicit registration/seeding owns identity changes and resets the old context.
    session.reset_identity_scoped_state();
    assert_eq!(session.prompt_count(), 0);
    assert_eq!(
        session.prompt_history_load_state,
        PromptHistoryLoadState::NotRequested
    );
    assert_eq!(session.prompt_history_generation, 4);
}

#[test]
fn durable_identity_seed_preserves_live_session_state_and_invalidates_old_history() {
    let mut session = CLIAgentSession {
        agent: CLIAgent::Codex,
        status: CLIAgentSessionStatus::Success,
        session_context: CLIAgentSessionContext {
            session_id: None,
            query: Some("opaque fallback text".to_owned()),
            ..Default::default()
        },
        input_state: CLIAgentInputState::Closed,
        should_auto_toggle_input: true,
        listener: None,
        plugin_version: None,
        draft_text: Some("unfinished follow-up".to_owned()),
        remote_host: None,
        custom_command_prefix: None,
        received_rich_notification: false,
        has_observed_turn_activity: false,
        prompt_history: AgentPromptHistory {
            prompts: vec![AgentPrompt {
                timestamp: None,
                text: "stale prompt".to_owned(),
            }],
            is_partial: false,
        },
        prompt_history_load_state: PromptHistoryLoadState::Ready,
        prompt_history_generation: 4,
    };

    assert!(session.seed_session_identity(CLIAgent::Codex, "durable-id".to_owned()));
    assert_eq!(session.status, CLIAgentSessionStatus::Success);
    assert!(session.should_auto_toggle_input);
    assert_eq!(session.draft_text.as_deref(), Some("unfinished follow-up"));
    assert_eq!(
        session.session_context.session_id.as_deref(),
        Some("durable-id")
    );
    assert_eq!(session.prompt_count(), 0);
    assert_eq!(
        session.prompt_history_load_state,
        PromptHistoryLoadState::NotRequested
    );
    assert_eq!(session.prompt_history_generation, 5);

    assert!(!session.seed_session_identity(CLIAgent::Claude, "other-id".to_owned()));
    assert_eq!(
        session.session_context.session_id.as_deref(),
        Some("durable-id")
    );
}

#[test]
fn native_codex_osc9_query_is_not_used_as_a_prompt_title() {
    let mut session = CLIAgentSession {
        agent: CLIAgent::Codex,
        status: CLIAgentSessionStatus::Success,
        session_context: CLIAgentSessionContext {
            query: Some("opaque completion notice".to_owned()),
            ..Default::default()
        },
        input_state: CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        plugin_version: None,
        draft_text: None,
        remote_host: None,
        custom_command_prefix: None,
        received_rich_notification: false,
        has_observed_turn_activity: false,
        prompt_history: AgentPromptHistory::default(),
        prompt_history_load_state: PromptHistoryLoadState::Unavailable,
        prompt_history_generation: 1,
    };

    assert_eq!(session.latest_user_prompt_for_chrome(), None);
    assert_eq!(session.title_for_tab(true), None);

    session.received_rich_notification = true;
    assert_eq!(
        session.latest_user_prompt_for_chrome().as_deref(),
        Some("opaque completion notice")
    );
}

#[test]
fn default_title_is_stable_first_prompt_and_explicit_preference_uses_latest() {
    let mut session = CLIAgentSession {
        agent: CLIAgent::Claude,
        status: CLIAgentSessionStatus::InProgress,
        session_context: CLIAgentSessionContext {
            summary: Some("fallback summary".to_owned()),
            query: Some("legacy latest".to_owned()),
            ..Default::default()
        },
        input_state: CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        plugin_version: None,
        draft_text: None,
        remote_host: None,
        custom_command_prefix: None,
        received_rich_notification: true,
        has_observed_turn_activity: true,
        prompt_history: AgentPromptHistory::default(),
        prompt_history_load_state: PromptHistoryLoadState::Ready,
        prompt_history_generation: 1,
    };
    session.prompt_history.prompts = vec![
        AgentPrompt {
            timestamp: None,
            text: "Plan the implementation. Include tests after this sentence.".to_owned(),
        },
        AgentPrompt {
            timestamp: None,
            text: "Now open a pull request".to_owned(),
        },
    ];

    assert_eq!(
        session.title_for_tab(false).as_deref(),
        Some("Plan the implementation.")
    );
    assert_eq!(
        session.title_for_tab(true).as_deref(),
        Some("Now open a pull request")
    );
}

#[test]
fn session_context_is_clinch_only() {
    assert!(session_context_enabled_for("sh.clinch.Clinch"));
    assert!(session_context_enabled_for("sh.clinch.ClinchDev"));
    assert!(!session_context_enabled_for("dev.warp.Warp-Stable"));
}
