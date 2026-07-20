//! Tests for the pure [`PaneAutoContinue`] state machine. All transitions
//! take injected timestamps, so no real timers are involved: "the timer
//! fired" is simulated by calling `take_fire` with the armed generation.

use chrono::{DateTime, Duration, TimeZone, Utc};
use cli_agent_usage::ExhaustionStatus;
#[cfg(target_os = "macos")]
use cli_agent_usage::{LimitWindow, PlanLimits, Provider, Severity, UsageSnapshot};
#[cfg(target_os = "macos")]
use settings::ToggleableSetting as _;
#[cfg(target_os = "macos")]
use warpui::{App, EntityId, SingletonEntity};

#[cfg(target_os = "macos")]
use super::AutoContinueModel;
#[cfg(target_os = "macos")]
use super::{auto_continue_availability, reset_for_causal_limit_stop, AutoContinueAvailability};
use super::{ArmedAutoContinue, DueFireDecision, PaneAutoContinue, AUTO_CONTINUE_RESET_SLACK_SECS};
#[cfg(target_os = "macos")]
use crate::ai::blocklist::usage::CliAgentUsageModel;
#[cfg(target_os = "macos")]
use crate::settings::CliAgentUsageSettings;
#[cfg(target_os = "macos")]
use crate::terminal::cli_agent_sessions::event::{
    CLIAgentEvent, CLIAgentEventPayload, CLIAgentEventSource, CLIAgentEventType, CLIAgentStopReason,
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
fn codex_without_a_reported_session_id_never_arms() {
    let mut pane = PaneAutoContinue::default();
    pane.set_enabled(true);
    assert!(pane
        .on_agent_session_stopped(
            CLIAgent::Codex,
            None,
            Some(now() + Duration::hours(1)),
            now(),
        )
        .is_none());
    assert!(pane.armed().is_none());
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
fn causal_limit_stop_survives_an_extended_provider_retry_after() {
    let mut pane = PaneAutoContinue::default();
    pane.set_enabled(true);
    assert!(pane
        .on_agent_session_stopped(CLIAgent::Claude, Some("sess-1"), None, now())
        .is_none());

    let much_later = now() + Duration::hours(2);
    assert!(pane
        .retry_pending_usage_confirmation(Some(much_later + Duration::hours(1)), much_later,)
        .is_some());
    assert!(pane.armed().is_some());
}

#[test]
fn claude_confirmation_window_covers_more_than_one_five_minute_cache_interval() {
    let mut pane = PaneAutoContinue::default();
    pane.set_enabled(true);
    assert!(pane
        .on_agent_session_stopped(CLIAgent::Claude, Some("sess-1"), None, now())
        .is_none());

    let refreshed_at = now() + Duration::minutes(6);
    assert!(pane
        .retry_pending_usage_confirmation(Some(refreshed_at + Duration::hours(1)), refreshed_at,)
        .is_some());
}

#[test]
fn later_provider_reset_rearms_and_invalidates_the_old_timer() {
    let (mut pane, first) = armed_pane();
    let later_reset = now() + Duration::hours(3);
    let rearmed = pane
        .reconcile_reset(Some(later_reset), now())
        .expect("a moved reset should replace the timer")
        .clone();

    assert_ne!(rearmed.generation, first.generation);
    assert_eq!(
        rearmed.fire_at,
        later_reset + Duration::seconds(AUTO_CONTINUE_RESET_SLACK_SECS)
    );
    assert!(pane.take_fire(first.generation).is_none());
}

#[test]
fn due_fire_waits_when_usage_data_is_unknown_and_fires_after_reset_clears() {
    let (mut pane, first) = armed_pane();
    let rearmed = match pane.prepare_due_fire(first.generation, None, first.fire_at) {
        DueFireDecision::Rearmed(armed) => armed,
        other => panic!("expected a deferred fire, got {other:?}"),
    };
    assert_ne!(rearmed.generation, first.generation);

    assert_eq!(
        pane.prepare_due_fire(
            rearmed.generation,
            Some(ExhaustionStatus::NotExhausted),
            rearmed.fire_at,
        ),
        DueFireDecision::Fire(rearmed)
    );
}

#[test]
fn failed_delivery_is_retried_three_times_then_left_visibly_failed() {
    let (mut pane, armed) = armed_pane();
    let mut fired = match pane.prepare_due_fire(
        armed.generation,
        Some(ExhaustionStatus::NotExhausted),
        armed.fire_at,
    ) {
        DueFireDecision::Fire(fired) => fired,
        other => panic!("expected a due fire, got {other:?}"),
    };

    for attempt in 1..=3 {
        let rearmed = pane
            .rearm_after_delivery_failure(fired, now(), "PTY unavailable".to_owned())
            .expect("the bounded retry should remain armed")
            .clone();
        assert_eq!(rearmed.delivery_attempts, attempt);
        fired = match pane.prepare_due_fire(
            rearmed.generation,
            Some(ExhaustionStatus::NotExhausted),
            rearmed.fire_at,
        ) {
            DueFireDecision::Fire(fired) => fired,
            other => panic!("expected retry fire, got {other:?}"),
        };
    }

    assert!(pane
        .rearm_after_delivery_failure(fired, now(), "PTY unavailable".to_owned())
        .is_none());
    assert!(pane.armed().is_none());
    assert_eq!(pane.delivery_error(), Some("PTY unavailable"));
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
        plugin_version: Some(
            match agent {
                CLIAgent::Claude => "2.3.0",
                CLIAgent::Codex => "0.5.0",
                _ => "0.0.0",
            }
            .to_owned(),
        ),
        remote_host: None,
        draft_text: None,
        custom_command_prefix: None,
        received_rich_notification: true,
        has_observed_turn_activity: true,
        turn_interrupted_by_user: false,
        prompt_history: Default::default(),
        prompt_history_load_state: Default::default(),
        prompt_history_generation: 0,
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
        payload: CLIAgentEventPayload {
            stop_reason: Some(CLIAgentStopReason::UsageLimit),
            ..Default::default()
        },
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
fn plan_snapshot(agent: CLIAgent, percent: f64, reset_at: DateTime<Utc>) -> UsageSnapshot {
    let provider = Provider {
        plan: Some(PlanLimits {
            session: Some(LimitWindow {
                percent,
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

#[cfg(target_os = "macos")]
fn exhausted_snapshot(agent: CLIAgent, reset_at: DateTime<Utc>) -> UsageSnapshot {
    plan_snapshot(agent, 100.0, reset_at)
}

#[test]
#[cfg(target_os = "macos")]
fn availability_requires_local_current_plugin_identity_and_plan_data() {
    let reset = Utc::now() + Duration::hours(1);
    for (agent, show_plan_limits) in [(CLIAgent::Claude, true), (CLIAgent::Codex, false)] {
        let session = test_session(agent);
        let snapshot = plan_snapshot(agent, 50.0, reset);
        assert_eq!(
            auto_continue_availability(&session, &snapshot, show_plan_limits, false),
            AutoContinueAvailability::Ready
        );
        assert_eq!(
            auto_continue_availability(
                &session,
                &UsageSnapshot::default(),
                show_plan_limits,
                false,
            ),
            AutoContinueAvailability::WaitingForUsageData
        );
        assert_eq!(
            auto_continue_availability(&session, &snapshot, show_plan_limits, true),
            AutoContinueAvailability::Unsupported
        );

        let mut remote = session.clone();
        remote.remote_host = Some("host".to_owned());
        assert_eq!(
            auto_continue_availability(&remote, &snapshot, show_plan_limits, false),
            AutoContinueAvailability::Unsupported
        );

        let mut outdated = session;
        outdated.plugin_version = Some("0.0.1".to_owned());
        assert_eq!(
            auto_continue_availability(&outdated, &snapshot, show_plan_limits, false),
            AutoContinueAvailability::Unsupported
        );
    }
}

#[test]
#[cfg(target_os = "macos")]
fn causal_stop_can_use_a_known_reset_from_a_rounded_near_full_snapshot() {
    let reset = Utc::now() + Duration::hours(1);
    let snapshot = plan_snapshot(CLIAgent::Claude, 99.0, reset);
    assert_eq!(
        reset_for_causal_limit_stop(&snapshot, CLIAgent::Claude),
        Some(reset)
    );
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

        usage.update(&mut app, |model, ctx| {
            model.update_snapshot_for_test(
                plan_snapshot(CLIAgent::Claude, 98.0, Utc::now() + Duration::hours(1)),
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
fn normal_success_does_not_arm_even_when_the_account_is_exhausted() {
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
        auto_continue.update(&mut app, |model, ctx| model.toggle(terminal_view_id, ctx));
        let mut ordinary_stop = stop_event(CLIAgent::Claude);
        ordinary_stop.payload.stop_reason = None;
        sessions.update(&mut app, |model, ctx| {
            model.update_from_event(terminal_view_id, &ordinary_stop, ctx);
        });

        auto_continue.read(&app, |model, _| {
            assert!(model.is_enabled(terminal_view_id));
            assert!(!model.is_armed(terminal_view_id));
        });
    });
}

#[test]
#[cfg(target_os = "macos")]
fn explicit_session_opt_in_restores_and_explicit_off_removes_it() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        set_plan_limits_enabled(&mut app, true);
        let usage = app.add_singleton_model(|_| CliAgentUsageModel::new_for_test());
        let sessions = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let auto_continue = app.add_singleton_model(AutoContinueModel::new);
        let terminal_view_id = EntityId::new();
        let key = "claude:sess-1";

        usage.update(&mut app, |model, ctx| {
            model.update_snapshot_for_test(
                plan_snapshot(CLIAgent::Claude, 50.0, Utc::now() + Duration::hours(1)),
                ctx,
            );
        });
        sessions.update(&mut app, |model, ctx| {
            model.set_session(terminal_view_id, test_session(CLIAgent::Claude), ctx);
        });
        auto_continue.update(&mut app, |model, ctx| model.toggle(terminal_view_id, ctx));
        assert!(
            CliAgentUsageSettings::handle(&app).read(&app, |settings, _| settings
                .auto_continue_sessions
                .contains_key(key))
        );

        usage.update(&mut app, |model, ctx| {
            model.update_snapshot_for_test(
                exhausted_snapshot(CLIAgent::Claude, Utc::now() + Duration::hours(1)),
                ctx,
            );
        });
        sessions.update(&mut app, |model, ctx| {
            model.update_from_event(terminal_view_id, &stop_event(CLIAgent::Claude), ctx);
        });
        assert!(
            CliAgentUsageSettings::handle(&app).read(&app, |settings, _| settings
                .auto_continue_armed_sessions
                .contains_key(key))
        );

        sessions.update(&mut app, |model, ctx| {
            model.remove_session(terminal_view_id, ctx);
            model.set_session(terminal_view_id, test_session(CLIAgent::Claude), ctx);
        });
        auto_continue.read(&app, |model, _| {
            assert!(model.is_enabled(terminal_view_id));
            assert!(model.is_armed(terminal_view_id));
        });

        auto_continue.update(&mut app, |model, ctx| model.toggle(terminal_view_id, ctx));
        assert!(
            !CliAgentUsageSettings::handle(&app).read(&app, |settings, _| settings
                .auto_continue_sessions
                .contains_key(key))
        );
        assert!(
            !CliAgentUsageSettings::handle(&app).read(&app, |settings, _| settings
                .auto_continue_armed_sessions
                .contains_key(key))
        );
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

        auto_continue.update(&mut app, |model, _ctx| {
            assert!(!model.is_enabled(terminal_view_id));
            assert!(!model.is_armed(terminal_view_id));
            assert!(model.armed_for_generation(terminal_view_id, 1).is_none());
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

        usage.update(&mut app, |model, ctx| {
            model.update_snapshot_for_test(
                plan_snapshot(CLIAgent::Codex, 98.0, Utc::now() + Duration::hours(2)),
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
fn codex_osc9_fallback_without_identity_is_not_offerable() {
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
            assert!(!model.is_enabled(terminal_view_id));
            assert!(!model.is_armed(terminal_view_id));
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
