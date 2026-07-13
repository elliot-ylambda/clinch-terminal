//! Tests for the pure [`PaneAutoContinue`] state machine. All transitions
//! take injected timestamps, so no real timers are involved: "the timer
//! fired" is simulated by calling `take_fire` with the armed generation.

use chrono::{DateTime, Duration, TimeZone, Utc};
#[cfg(target_os = "macos")]
use cli_agent_usage::{LimitWindow, PlanLimits, Provider, Severity, UsageSnapshot};
#[cfg(target_os = "macos")]
use settings::ToggleableSetting as _;
#[cfg(target_os = "macos")]
use warpui::{App, EntityId, SingletonEntity};

#[cfg(target_os = "macos")]
use super::AutoContinueModel;
use super::{
    is_auto_continue_available, ArmedAutoContinue, PaneAutoContinue,
    AUTO_CONTINUE_RESET_SLACK_SECS, AUTO_CONTINUE_USAGE_CONFIRMATION_GRACE_SECS,
};
#[cfg(target_os = "macos")]
use crate::ai::blocklist::usage::CliAgentUsageModel;
#[cfg(target_os = "macos")]
use crate::settings::CliAgentUsageSettings;
#[cfg(target_os = "macos")]
use crate::terminal::cli_agent_sessions::event::{
    CLIAgentEvent, CLIAgentEventPayload, CLIAgentEventSource, CLIAgentEventType,
};
#[cfg(target_os = "macos")]
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputState, CLIAgentSession, CLIAgentSessionContext, CLIAgentSessionStatus,
    CLIAgentSessionsModel,
};
use crate::terminal::CLIAgent;
#[cfg(target_os = "macos")]
use crate::test_util::settings::initialize_settings_for_tests;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 11, 12, 0, 0).unwrap()
}

#[test]
fn availability_supports_claude_and_codex_but_not_viewers_or_other_agents() {
    assert!(is_auto_continue_available(CLIAgent::Claude, true, false));
    assert!(!is_auto_continue_available(CLIAgent::Claude, false, false));
    assert!(!is_auto_continue_available(CLIAgent::Claude, true, true));
    assert!(is_auto_continue_available(CLIAgent::Codex, true, false));
    assert!(is_auto_continue_available(CLIAgent::Codex, false, false));
    assert!(!is_auto_continue_available(CLIAgent::Codex, false, true));
    assert!(!is_auto_continue_available(CLIAgent::Gemini, true, false));
}

/// Arms a fresh, enabled pane with a reset one hour out and returns
/// `(pane, armed)`.
fn armed_pane() -> (PaneAutoContinue, ArmedAutoContinue) {
    let mut pane = PaneAutoContinue::default();
    assert!(pane.set_enabled(true));
    let armed = pane
        .on_agent_session_stopped(
            CLIAgent::Claude,
            Some("sess-1"),
            Some(now() + Duration::hours(1)),
            now(),
        )
        .expect("arms when enabled, exhausted, and reset is known")
        .clone();
    (pane, armed)
}

#[test]
fn arms_with_slack_and_session_id_when_stopped_while_exhausted() {
    let (pane, armed) = armed_pane();
    assert_eq!(
        armed.fire_at,
        now() + Duration::hours(1) + Duration::seconds(AUTO_CONTINUE_RESET_SLACK_SECS)
    );
    assert_eq!(armed.agent, CLIAgent::Claude);
    assert_eq!(armed.session_id.as_deref(), Some("sess-1"));
    assert!(pane.is_enabled());
    assert_eq!(pane.armed(), Some(&armed));
}

#[test]
fn disabled_pane_never_arms() {
    let mut pane = PaneAutoContinue::default();
    assert!(pane
        .on_agent_session_stopped(
            CLIAgent::Claude,
            Some("sess-1"),
            Some(now() + Duration::hours(1)),
            now(),
        )
        .is_none());
    assert!(pane.armed().is_none());
}

#[test]
fn unknown_reset_time_never_arms() {
    let mut pane = PaneAutoContinue::default();
    pane.set_enabled(true);
    assert!(pane
        .on_agent_session_stopped(CLIAgent::Claude, Some("sess-1"), None, now())
        .is_none());
    assert!(pane.armed().is_none());
}

#[test]
fn missing_session_id_never_arms_for_claude() {
    let mut pane = PaneAutoContinue::default();
    pane.set_enabled(true);
    assert!(pane
        .on_agent_session_stopped(
            CLIAgent::Claude,
            None,
            Some(now() + Duration::hours(1)),
            now(),
        )
        .is_none());
    assert!(pane.armed().is_none());
}

#[test]
fn codex_without_a_reported_session_id_uses_pane_scoped_identity() {
    let mut pane = PaneAutoContinue::default();
    pane.set_enabled(true);
    let armed = pane
        .on_agent_session_stopped(
            CLIAgent::Codex,
            None,
            Some(now() + Duration::hours(1)),
            now(),
        )
        .expect("ID-less Codex fallback can still arm within this pane");
    assert_eq!(armed.agent, CLIAgent::Codex);
    assert_eq!(armed.session_id, None);
}

#[test]
fn stale_past_reset_time_never_arms() {
    let mut pane = PaneAutoContinue::default();
    pane.set_enabled(true);
    // Reset in the past (or exactly now) means the usage data is stale.
    assert!(pane
        .on_agent_session_stopped(
            CLIAgent::Claude,
            Some("sess-1"),
            Some(now() - Duration::minutes(5)),
            now(),
        )
        .is_none());
    assert!(pane
        .on_agent_session_stopped(CLIAgent::Claude, Some("sess-1"), Some(now()), now())
        .is_none());
    assert!(pane.armed().is_none());
}

#[test]
fn an_old_normal_stop_cannot_arm_from_a_much_later_usage_limit() {
    let mut pane = PaneAutoContinue::default();
    pane.set_enabled(true);
    assert!(pane
        .on_agent_session_stopped(CLIAgent::Claude, Some("sess-1"), None, now())
        .is_none());

    let after_confirmation_grace =
        now() + Duration::seconds(AUTO_CONTINUE_USAGE_CONFIRMATION_GRACE_SECS + 1);
    assert!(pane
        .retry_pending_usage_confirmation(
            Some(after_confirmation_grace + Duration::hours(1)),
            after_confirmation_grace,
        )
        .is_none());
    assert!(pane.armed().is_none());
}

#[test]
fn fires_exactly_once_per_arm() {
    let (mut pane, armed) = armed_pane();
    let fired = pane
        .take_fire(armed.generation)
        .expect("first fire consumes the arm");
    assert_eq!(fired, armed);
    // The arm was consumed: the same timer (or a duplicate) can never fire again.
    assert!(pane.take_fire(armed.generation).is_none());
    assert!(pane.armed().is_none());
}

#[test]
fn user_activity_disarms_and_invalidates_the_scheduled_timer() {
    let (mut pane, armed) = armed_pane();
    assert!(pane.disarm());
    assert!(pane.armed().is_none());
    // The timer scheduled for the old arm wakes later: its generation no
    // longer matches, so nothing fires.
    assert!(pane.take_fire(armed.generation).is_none());
    // Disarming again reports nothing to cancel.
    assert!(!pane.disarm());
}

#[test]
fn user_activity_cancels_pending_usage_confirmation_before_it_arms() {
    let mut pane = PaneAutoContinue::default();
    pane.set_enabled(true);
    assert!(pane
        .on_agent_session_stopped(CLIAgent::Claude, Some("sess-1"), None, now())
        .is_none());
    assert!(pane.has_pending_continue());

    assert!(pane.disarm());
    assert!(!pane.has_pending_continue());
    assert!(pane
        .retry_pending_usage_confirmation(
            Some(now() + Duration::hours(1)),
            now() + Duration::seconds(1),
        )
        .is_none());
}

#[test]
fn stale_timer_generation_cannot_fire_a_newer_arm() {
    let (mut pane, first) = armed_pane();
    // User activity cancels, then a later stop re-arms.
    pane.disarm();
    let second = pane
        .on_agent_session_stopped(
            CLIAgent::Claude,
            Some("sess-1"),
            Some(now() + Duration::hours(2)),
            now(),
        )
        .expect("re-arms after a fresh stop")
        .clone();
    assert_ne!(first.generation, second.generation);
    // The first (cancelled) timer wakes: must not consume the second arm.
    assert!(pane.take_fire(first.generation).is_none());
    assert_eq!(pane.armed(), Some(&second));
    // The second timer fires normally.
    assert_eq!(pane.take_fire(second.generation), Some(second));
}

#[test]
fn a_second_stop_while_armed_keeps_the_first_arm() {
    let (mut pane, first) = armed_pane();
    assert!(pane
        .on_agent_session_stopped(
            CLIAgent::Codex,
            Some("sess-2"),
            Some(now() + Duration::hours(3)),
            now(),
        )
        .is_none());
    assert_eq!(pane.armed(), Some(&first));
}

#[test]
fn codex_arm_records_provider_identity() {
    let mut pane = PaneAutoContinue::default();
    pane.set_enabled(true);
    let armed = pane
        .on_agent_session_stopped(
            CLIAgent::Codex,
            Some("codex-sess"),
            Some(now() + Duration::hours(1)),
            now(),
        )
        .expect("Codex arms from its exhausted usage window");
    assert_eq!(armed.agent, CLIAgent::Codex);
    assert_eq!(armed.session_id.as_deref(), Some("codex-sess"));
}

#[test]
fn disabling_disarms_and_blocks_pending_fires() {
    let (mut pane, armed) = armed_pane();
    assert!(pane.set_enabled(false));
    assert!(pane.armed().is_none());
    assert!(pane.take_fire(armed.generation).is_none());
    // Re-enabling does not resurrect the old arm.
    assert!(pane.set_enabled(true));
    assert!(pane.armed().is_none());
    // Setting the same value again reports no change.
    assert!(!pane.set_enabled(true));
}

#[cfg(target_os = "macos")]
fn set_plan_limits_enabled(app: &mut App, enabled: bool) {
    let current =
        CliAgentUsageSettings::handle(app).read(app, |settings, _| *settings.show_plan_limits);
    if current == enabled {
        return;
    }
    CliAgentUsageSettings::handle(app).update(app, |settings, ctx| {
        settings
            .show_plan_limits
            .toggle_and_save_value(ctx)
            .expect("plan-limit test setting should toggle");
    });
}

#[cfg(target_os = "macos")]
fn test_session(agent: CLIAgent) -> CLIAgentSession {
    CLIAgentSession {
        agent,
        status: CLIAgentSessionStatus::InProgress,
        session_context: CLIAgentSessionContext {
            session_id: Some("sess-1".to_owned()),
            ..Default::default()
        },
        input_state: CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        plugin_version: None,
        remote_host: None,
        draft_text: None,
        custom_command_prefix: None,
        received_rich_notification: true,
    }
}

#[cfg(target_os = "macos")]
fn stop_event(agent: CLIAgent) -> CLIAgentEvent {
    CLIAgentEvent {
        source: CLIAgentEventSource::RichPlugin,
        v: 1,
        agent,
        event: CLIAgentEventType::Stop,
        session_id: Some("sess-1".to_owned()),
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload::default(),
    }
}

#[cfg(target_os = "macos")]
fn codex_osc9_stop_event() -> CLIAgentEvent {
    CLIAgentEvent {
        source: CLIAgentEventSource::CodexOsc9Fallback,
        v: 1,
        agent: CLIAgent::Codex,
        event: CLIAgentEventType::Stop,
        session_id: None,
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload::default(),
    }
}

#[cfg(target_os = "macos")]
fn exhausted_snapshot(agent: CLIAgent, reset_at: DateTime<Utc>) -> UsageSnapshot {
    let provider = Provider {
        plan: Some(PlanLimits {
            session: Some(LimitWindow {
                percent: 100.0,
                resets_at: Some(reset_at),
                severity: Severity::Critical,
            }),
            weekly: None,
            fable_weekly: None,
        }),
        ..Default::default()
    };
    match agent {
        CLIAgent::Claude => UsageSnapshot {
            claude: provider,
            ..Default::default()
        },
        CLIAgent::Codex => UsageSnapshot {
            codex: provider,
            ..Default::default()
        },
        _ => panic!("auto-continue test requires a supported provider"),
    }
}

#[test]
#[cfg(target_os = "macos")]
fn late_usage_snapshot_arms_an_enabled_stopped_session() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        set_plan_limits_enabled(&mut app, true);
        let usage = app.add_singleton_model(|_| CliAgentUsageModel::new_for_test());
        let sessions = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let auto_continue = app.add_singleton_model(AutoContinueModel::new);
        let terminal_view_id = EntityId::new();

        sessions.update(&mut app, |model, ctx| {
            model.set_session(terminal_view_id, test_session(CLIAgent::Claude), ctx);
        });
        auto_continue.update(&mut app, |model, ctx| {
            model.toggle(terminal_view_id, ctx);
        });
        sessions.update(&mut app, |model, ctx| {
            model.update_from_event(terminal_view_id, &stop_event(CLIAgent::Claude), ctx);
        });
        auto_continue.read(&app, |model, _| {
            assert!(model.is_enabled(terminal_view_id));
            assert!(!model.is_armed(terminal_view_id));
        });

        let reset_at = Utc::now() + Duration::hours(1);
        usage.update(&mut app, |model, ctx| {
            model.update_snapshot_for_test(exhausted_snapshot(CLIAgent::Claude, reset_at), ctx);
        });

        auto_continue.read(&app, |model, _| {
            assert!(model.is_armed(terminal_view_id));
            assert_eq!(
                model.armed_fire_at(terminal_view_id),
                Some(reset_at + Duration::seconds(AUTO_CONTINUE_RESET_SLACK_SECS))
            );
        });
    });
}

#[test]
#[cfg(target_os = "macos")]
fn disabling_plan_limits_cancels_and_disables_an_armed_pane() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        set_plan_limits_enabled(&mut app, true);
        let usage = app.add_singleton_model(|_| CliAgentUsageModel::new_for_test());
        let sessions = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let auto_continue = app.add_singleton_model(AutoContinueModel::new);
        let terminal_view_id = EntityId::new();

        usage.update(&mut app, |model, ctx| {
            model.update_snapshot_for_test(
                exhausted_snapshot(CLIAgent::Claude, Utc::now() + Duration::hours(1)),
                ctx,
            );
        });
        sessions.update(&mut app, |model, ctx| {
            model.set_session(terminal_view_id, test_session(CLIAgent::Claude), ctx);
        });
        auto_continue.update(&mut app, |model, ctx| {
            model.toggle(terminal_view_id, ctx);
        });
        sessions.update(&mut app, |model, ctx| {
            model.update_from_event(terminal_view_id, &stop_event(CLIAgent::Claude), ctx);
        });
        auto_continue.read(&app, |model, _| {
            assert!(model.is_armed(terminal_view_id));
        });

        set_plan_limits_enabled(&mut app, false);

        auto_continue.update(&mut app, |model, ctx| {
            assert!(!model.is_enabled(terminal_view_id));
            assert!(!model.is_armed(terminal_view_id));
            assert_eq!(model.take_due_fire(terminal_view_id, 1, ctx), None);
        });
    });
}

#[test]
#[cfg(target_os = "macos")]
fn codex_arms_from_codex_usage_even_when_claude_plan_limits_are_disabled() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        set_plan_limits_enabled(&mut app, false);
        let usage = app.add_singleton_model(|_| CliAgentUsageModel::new_for_test());
        let sessions = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let auto_continue = app.add_singleton_model(AutoContinueModel::new);
        let terminal_view_id = EntityId::new();

        sessions.update(&mut app, |model, ctx| {
            model.set_session(terminal_view_id, test_session(CLIAgent::Codex), ctx);
        });
        auto_continue.update(&mut app, |model, ctx| {
            model.toggle(terminal_view_id, ctx);
        });
        sessions.update(&mut app, |model, ctx| {
            model.update_from_event(terminal_view_id, &stop_event(CLIAgent::Codex), ctx);
        });

        // An exhausted Claude window must not cross-arm a Codex pane.
        usage.update(&mut app, |model, ctx| {
            model.update_snapshot_for_test(
                exhausted_snapshot(CLIAgent::Claude, Utc::now() + Duration::hours(1)),
                ctx,
            );
        });
        auto_continue.read(&app, |model, _| {
            assert!(model.is_enabled(terminal_view_id));
            assert!(!model.is_armed(terminal_view_id));
        });

        let reset_at = Utc::now() + Duration::hours(2);
        usage.update(&mut app, |model, ctx| {
            model.update_snapshot_for_test(exhausted_snapshot(CLIAgent::Codex, reset_at), ctx);
        });
        auto_continue.read(&app, |model, _| {
            assert!(model.is_armed(terminal_view_id));
            assert_eq!(
                model.armed_fire_at(terminal_view_id),
                Some(reset_at + Duration::seconds(AUTO_CONTINUE_RESET_SLACK_SECS))
            );
        });
    });
}

#[test]
#[cfg(target_os = "macos")]
fn codex_osc9_fallback_arms_without_a_reported_session_id() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        set_plan_limits_enabled(&mut app, false);
        let usage = app.add_singleton_model(|_| CliAgentUsageModel::new_for_test());
        let sessions = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let auto_continue = app.add_singleton_model(AutoContinueModel::new);
        let terminal_view_id = EntityId::new();

        let reset_at = Utc::now() + Duration::hours(1);
        usage.update(&mut app, |model, ctx| {
            model.update_snapshot_for_test(exhausted_snapshot(CLIAgent::Codex, reset_at), ctx);
        });
        sessions.update(&mut app, |model, ctx| {
            let mut session = test_session(CLIAgent::Codex);
            session.session_context.session_id = None;
            session.received_rich_notification = false;
            model.set_session(terminal_view_id, session, ctx);
        });
        auto_continue.update(&mut app, |model, ctx| {
            model.toggle(terminal_view_id, ctx);
        });
        sessions.update(&mut app, |model, ctx| {
            model.update_from_event(terminal_view_id, &codex_osc9_stop_event(), ctx);
        });

        auto_continue.read(&app, |model, _| {
            assert!(model.is_enabled(terminal_view_id));
            assert!(model.is_armed(terminal_view_id));
            assert_eq!(
                model.armed_fire_at(terminal_view_id),
                Some(reset_at + Duration::seconds(AUTO_CONTINUE_RESET_SLACK_SECS))
            );
        });
    });
}

#[test]
#[cfg(target_os = "macos")]
fn disabling_claude_plan_limits_does_not_cancel_an_armed_codex_pane() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        set_plan_limits_enabled(&mut app, true);
        let usage = app.add_singleton_model(|_| CliAgentUsageModel::new_for_test());
        let sessions = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let auto_continue = app.add_singleton_model(AutoContinueModel::new);
        let terminal_view_id = EntityId::new();

        usage.update(&mut app, |model, ctx| {
            model.update_snapshot_for_test(
                exhausted_snapshot(CLIAgent::Codex, Utc::now() + Duration::hours(1)),
                ctx,
            );
        });
        sessions.update(&mut app, |model, ctx| {
            model.set_session(terminal_view_id, test_session(CLIAgent::Codex), ctx);
        });
        auto_continue.update(&mut app, |model, ctx| {
            model.toggle(terminal_view_id, ctx);
        });
        sessions.update(&mut app, |model, ctx| {
            model.update_from_event(terminal_view_id, &stop_event(CLIAgent::Codex), ctx);
        });
        auto_continue.read(&app, |model, _| {
            assert!(model.is_armed(terminal_view_id));
        });

        set_plan_limits_enabled(&mut app, false);

        auto_continue.read(&app, |model, _| {
            assert!(model.is_enabled(terminal_view_id));
            assert!(model.is_armed(terminal_view_id));
        });
    });
}
