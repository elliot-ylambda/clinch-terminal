//! Tests for the pure [`PaneAutoContinue`] state machine. All transitions
//! take injected timestamps, so no real timers are involved: "the timer
//! fired" is simulated by calling `take_fire` with the armed generation.

use chrono::{DateTime, Duration, TimeZone, Utc};
use cli_agent_usage::ExhaustionStatus;
#[cfg(target_os = "macos")]
use cli_agent_usage::{LimitWindow, PlanLimits, Provider, Severity, UsageSnapshot};
#[cfg(target_os = "macos")]
use settings::{Setting as _, ToggleableSetting as _};
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
fn due_fire_waits_while_the_window_is_exhausted_without_a_reset_time() {
    let (mut pane, first) = armed_pane();
    let rearmed = match pane.prepare_due_fire(
        first.generation,
        Some(ExhaustionStatus::ResetUnknown),
        first.fire_at,
    ) {
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

/// Codex drops a window from its usage data once the window's reset passes
/// and reports nothing new until the next request, which only the continue
/// itself can make. Deferring on missing data deadlocked every Codex arm.
#[test]
fn due_fire_sends_when_plan_data_is_gone_after_the_armed_reset() {
    let (mut pane, armed) = armed_pane();
    assert_eq!(
        pane.prepare_due_fire(armed.generation, None, armed.fire_at),
        DueFireDecision::Fire(armed)
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
fn set_auto_continue_default(app: &mut App, enabled: bool) {
    let current = CliAgentUsageSettings::handle(app)
        .read(app, |settings, _| *settings.auto_continue_on_limit_reset);
    if current == enabled {
        return;
    }
    CliAgentUsageSettings::handle(app).update(app, |settings, ctx| {
        settings
            .auto_continue_on_limit_reset
            .toggle_and_save_value(ctx)
            .expect("auto-continue test setting should toggle");
    });
}

#[cfg(target_os = "macos")]
fn test_session(agent: CLIAgent) -> CLIAgentSession {
    CLIAgentSession {
        agent,
        status: CLIAgentSessionStatus::InProgress,
        session_context: CLIAgentSessionContext {
            session_id: Some("sess-1".to_owned()),
            turn_started_at: Some(Utc::now() - Duration::minutes(1)),
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

/// A provider-reported Claude limit stop with no stated reset.
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
            stop_reason: Some(CLIAgentStopReason::UsageLimit { stated_reset: None }),
            ..Default::default()
        },
    }
}

/// The exact StopFailure body the bundled Claude plugin sends, with a real
/// Claude Code 2.1.305 limit message whose reset is `stated_reset`.
#[cfg(target_os = "macos")]
fn claude_stop_failure_event(stated_reset: DateTime<Utc>) -> CLIAgentEvent {
    CLIAgentEvent {
        source: CLIAgentEventSource::RichPlugin,
        v: 1,
        agent: CLIAgent::Claude,
        event: CLIAgentEventType::StopFailure,
        session_id: Some("sess-1".to_owned()),
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload {
            response: Some(
                "You've hit your session limit · resets 6:50pm (America/Los_Angeles)".to_owned(),
            ),
            stop_reason: Some(CLIAgentStopReason::UsageLimit {
                stated_reset: Some(stated_reset),
            }),
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

/// What the usage scan produces right after a real Codex limit stop: the
/// rollout's 100% weekly window and its `usage_limit_exceeded` turn end.
#[cfg(target_os = "macos")]
fn codex_limit_snapshot(reset_at: DateTime<Utc>, stopped_at: DateTime<Utc>) -> UsageSnapshot {
    let mut snapshot = exhausted_snapshot(CLIAgent::Codex, reset_at);
    snapshot
        .codex
        .usage_limit_stops
        .insert("sess-1".to_owned(), stopped_at);
    snapshot
}

#[test]
#[cfg(target_os = "macos")]
fn availability_requires_a_local_session_on_the_current_plugin_but_not_plan_data() {
    for agent in [CLIAgent::Claude, CLIAgent::Codex] {
        let session = test_session(agent);
        assert_eq!(
            auto_continue_availability(&session, false),
            AutoContinueAvailability::Ready
        );
        assert_eq!(
            auto_continue_availability(&session, true),
            AutoContinueAvailability::Unsupported
        );

        let mut remote = session.clone();
        remote.remote_host = Some("host".to_owned());
        assert_eq!(
            auto_continue_availability(&remote, false),
            AutoContinueAvailability::Unsupported
        );

        let mut outdated = session;
        outdated.plugin_version = Some("0.0.1".to_owned());
        assert_eq!(
            auto_continue_availability(&outdated, false),
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
fn sessions_are_enabled_by_default() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| CliAgentUsageModel::new_for_test());
        let sessions = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let auto_continue = app.add_singleton_model(AutoContinueModel::new);
        let claude = EntityId::new();
        let codex = EntityId::new();

        sessions.update(&mut app, |model, ctx| {
            model.set_session(claude, test_session(CLIAgent::Claude), ctx);
            model.set_session(codex, test_session(CLIAgent::Codex), ctx);
        });
        auto_continue.read(&app, |model, _| {
            assert!(model.is_enabled(claude));
            assert!(model.is_enabled(codex));
        });
    });
}

/// Fresh installs keep `show_plan_limits` off (no Keychain, no network), so
/// Claude must schedule from the reset its own limit message states.
#[test]
#[cfg(target_os = "macos")]
fn claude_arms_from_the_stated_reset_without_plan_data() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| CliAgentUsageModel::new_for_test());
        let sessions = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let auto_continue = app.add_singleton_model(AutoContinueModel::new);
        let terminal_view_id = EntityId::new();
        assert!(!CliAgentUsageSettings::handle(&app)
            .read(&app, |settings, _| *settings.show_plan_limits));

        sessions.update(&mut app, |model, ctx| {
            model.set_session(terminal_view_id, test_session(CLIAgent::Claude), ctx);
        });
        let reset_at = Utc::now() + Duration::hours(2);
        sessions.update(&mut app, |model, ctx| {
            model.update_from_event(terminal_view_id, &claude_stop_failure_event(reset_at), ctx);
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
fn late_usage_snapshot_arms_a_stopped_session_without_a_stated_reset() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
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
            model.update_from_event(terminal_view_id, &stop_event(CLIAgent::Claude), ctx);
        });
        auto_continue.read(&app, |model, _| {
            assert!(model.is_enabled(terminal_view_id));
            assert!(!model.is_armed(terminal_view_id));
            assert!(model.is_waiting_for_reset(terminal_view_id));
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
        let mut ordinary_stop = stop_event(CLIAgent::Claude);
        ordinary_stop.payload.stop_reason = None;
        sessions.update(&mut app, |model, ctx| {
            model.set_session(terminal_view_id, test_session(CLIAgent::Claude), ctx);
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
fn arm_survives_restore_and_footer_off_persists_for_that_session() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| CliAgentUsageModel::new_for_test());
        let sessions = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let auto_continue = app.add_singleton_model(AutoContinueModel::new);
        let terminal_view_id = EntityId::new();
        let key = "claude:sess-1";

        sessions.update(&mut app, |model, ctx| {
            model.set_session(terminal_view_id, test_session(CLIAgent::Claude), ctx);
            model.update_from_event(
                terminal_view_id,
                &claude_stop_failure_event(Utc::now() + Duration::hours(1)),
                ctx,
            );
        });
        assert!(
            CliAgentUsageSettings::handle(&app).read(&app, |settings, _| settings
                .auto_continue_armed_sessions
                .contains_key(key))
        );

        // An app restart gives the restored pane a new session entry.
        sessions.update(&mut app, |model, ctx| {
            model.remove_session(terminal_view_id, ctx);
            model.set_session(terminal_view_id, test_session(CLIAgent::Claude), ctx);
        });
        auto_continue.read(&app, |model, _| {
            assert!(model.is_enabled(terminal_view_id));
            assert!(model.is_armed(terminal_view_id));
        });

        auto_continue.update(&mut app, |model, ctx| model.toggle(terminal_view_id, ctx));
        CliAgentUsageSettings::handle(&app).read(&app, |settings, _| {
            assert_eq!(
                settings.auto_continue_session_overrides.get(key),
                Some(&false)
            );
            assert!(!settings.auto_continue_armed_sessions.contains_key(key));
        });

        sessions.update(&mut app, |model, ctx| {
            model.remove_session(terminal_view_id, ctx);
            model.set_session(terminal_view_id, test_session(CLIAgent::Claude), ctx);
        });
        auto_continue.read(&app, |model, _| {
            assert!(!model.is_enabled(terminal_view_id));
        });

        // Turning it back on matches the default, so no override is kept.
        auto_continue.update(&mut app, |model, ctx| model.toggle(terminal_view_id, ctx));
        assert!(
            CliAgentUsageSettings::handle(&app).read(&app, |settings, _| settings
                .auto_continue_session_overrides
                .is_empty())
        );
    });
}

#[test]
#[cfg(target_os = "macos")]
fn global_default_off_disables_panes_and_footer_can_opt_one_back_in() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| CliAgentUsageModel::new_for_test());
        let sessions = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let auto_continue = app.add_singleton_model(AutoContinueModel::new);
        let terminal_view_id = EntityId::new();

        sessions.update(&mut app, |model, ctx| {
            model.set_session(terminal_view_id, test_session(CLIAgent::Codex), ctx);
        });
        set_auto_continue_default(&mut app, false);
        auto_continue.read(&app, |model, _| {
            assert!(!model.is_enabled(terminal_view_id));
        });

        auto_continue.update(&mut app, |model, ctx| model.toggle(terminal_view_id, ctx));
        auto_continue.read(&app, |model, _| {
            assert!(model.is_enabled(terminal_view_id));
        });
        assert_eq!(
            CliAgentUsageSettings::handle(&app).read(&app, |settings, _| settings
                .auto_continue_session_overrides
                .get("codex:sess-1")
                .copied()),
            Some(true)
        );

        set_auto_continue_default(&mut app, true);
        CliAgentUsageSettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .auto_continue_session_overrides
                .set_value(Default::default(), ctx)
                .expect("clear overrides");
        });
        auto_continue.read(&app, |model, _| {
            assert!(model.is_enabled(terminal_view_id));
        });
    });
}

/// End to end with real Codex behaviour: no hook fires at the limit; the
/// usage scan reports the rollout's limit stop and 100% weekly window.
#[test]
#[cfg(target_os = "macos")]
fn codex_arms_from_its_rollout_limit_stop() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        let usage = app.add_singleton_model(|_| CliAgentUsageModel::new_for_test());
        let sessions = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let auto_continue = app.add_singleton_model(AutoContinueModel::new);
        let terminal_view_id = EntityId::new();

        sessions.update(&mut app, |model, ctx| {
            model.set_session(terminal_view_id, test_session(CLIAgent::Codex), ctx);
        });
        // A Claude exhaustion must not cross-arm a Codex pane.
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

        let reset_at = Utc::now() + Duration::hours(30);
        usage.update(&mut app, |model, ctx| {
            model.update_snapshot_for_test(codex_limit_snapshot(reset_at, Utc::now()), ctx);
        });

        sessions.read(&app, |model, _| {
            let session = model.session(terminal_view_id).unwrap();
            assert!(matches!(session.status, CLIAgentSessionStatus::Success));
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
