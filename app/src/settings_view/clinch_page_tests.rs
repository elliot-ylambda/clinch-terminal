use settings::Setting as _;
#[cfg(feature = "local_fs")]
use warp_core::channel::{Channel, ChannelConfig, ChannelState};
#[cfg(feature = "local_fs")]
use warp_core::AppId;
use warpui::platform::WindowStyle;
use warpui::{App, SingletonEntity, TypedActionView};

#[cfg(feature = "local_fs")]
use super::{countdown, device_activity_label, pairing_code};
use super::{
    remote_control_setup_widget_id, ClinchSettingsPageAction, ClinchSettingsPageView,
    RemoteControlSetupWidget, CLINCH_REMOTE_CONTROL_GUIDE_URL, TAILSCALE_IOS_DOWNLOAD_URL,
    TAILSCALE_MAC_DOWNLOAD_URL,
};
use crate::appearance::Appearance;
use crate::auth::AuthStateProvider;
#[cfg(feature = "local_fs")]
use crate::remote_control::RemoteControlService;
#[cfg(target_os = "macos")]
use crate::settings::CliAgentUsageSettings;
use crate::settings::ClinchSettings;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::settings_view::settings_page::SettingsWidget;
use crate::terminal::session_settings::SessionSettings;
use crate::test_util::settings::initialize_settings_for_tests;

#[test]
fn remote_control_setup_widget_has_stable_discovery_metadata() {
    let widget = RemoteControlSetupWidget::default();

    assert_eq!(remote_control_setup_widget_id(), widget.widget_id());
    assert!(widget.search_terms().contains("remote control"));
    assert!(widget.search_terms().contains("tailscale"));
    assert_eq!(
        TAILSCALE_MAC_DOWNLOAD_URL,
        "https://tailscale.com/download/mac"
    );
    assert_eq!(
        TAILSCALE_IOS_DOWNLOAD_URL,
        "https://tailscale.com/download/ios"
    );
    assert_eq!(
        CLINCH_REMOTE_CONTROL_GUIDE_URL,
        "https://clinch.sh/remote-control"
    );
}

#[test]
#[cfg(feature = "local_fs")]
fn pairing_code_matches_the_phone_format() {
    assert_eq!(pairing_code(&"0123456789abcdef".repeat(4)), "0123-4567");
    assert_eq!(pairing_code("abc"), "ABC");
}

#[test]
#[cfg(feature = "local_fs")]
fn pairing_countdowns_and_activity_read_naturally() {
    let now = chrono::Utc::now();
    assert_eq!(countdown(now + chrono::Duration::seconds(125), now), "2:05");
    assert_eq!(countdown(now - chrono::Duration::seconds(5), now), "0:00");
    assert_eq!(device_activity_label(true, None, now), "Connected now");
    assert_eq!(device_activity_label(false, None, now), "Not connected yet");
    assert_eq!(
        device_activity_label(false, Some(now - chrono::Duration::minutes(12)), now),
        "Last seen 12 min ago"
    );
    assert_eq!(
        device_activity_label(false, Some(now - chrono::Duration::hours(3)), now),
        "Last seen 3 h ago"
    );
}

/// This test mutates the process-global channel, so it relies on the repository's required
/// process-per-test nextest runner for isolation.
#[test]
#[cfg(feature = "local_fs")]
fn remote_control_model_notifications_redraw_the_visible_settings_page() {
    App::test((), |mut app| async move {
        ChannelState::set(ChannelState::new(
            Channel::Local,
            ChannelConfig::no_backend(AppId::new("test", "warp", "WarpTest"), "warp-test.log"),
        ));
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| AuthStateProvider::new_logged_out_for_test());
        app.add_singleton_model(|_| KeybindingChangedNotifier::new());
        app.add_singleton_model(|_| Appearance::mock());
        app.update(crate::remote_control::register);

        let (window_id, view) =
            app.add_window(WindowStyle::NotStealFocus, ClinchSettingsPageView::new);
        view.update(&mut app, |_, ctx| ctx.notify());
        let frames_before = app.read(|ctx| {
            let presenter = ctx
                .presenter(window_id)
                .expect("settings window should have a presenter");
            let frame_count = presenter.borrow().frame_count();
            frame_count
        });

        app.update(|ctx| {
            RemoteControlService::handle(ctx).update(ctx, |_, ctx| ctx.notify());
        });

        let frames_after = app.read(|ctx| {
            let presenter = ctx
                .presenter(window_id)
                .expect("settings window should have a presenter");
            let frame_count = presenter.borrow().frame_count();
            frame_count
        });
        assert!(
            frames_after > frames_before,
            "a background Remote Control notification must redraw the visible settings page"
        );
    });
}

/// Mutates the process-global channel; isolated by the process-per-test nextest runner.
#[test]
#[cfg(feature = "local_fs")]
fn removing_a_paired_phone_takes_a_second_click() {
    App::test((), |mut app| async move {
        ChannelState::set(ChannelState::new(
            Channel::Local,
            ChannelConfig::no_backend(AppId::new("test", "warp", "WarpTest"), "warp-test.log"),
        ));
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| AuthStateProvider::new_logged_out_for_test());
        app.add_singleton_model(|_| KeybindingChangedNotifier::new());
        app.add_singleton_model(|_| Appearance::mock());
        app.update(crate::remote_control::register);
        let (_, view) = app.add_window(WindowStyle::NotStealFocus, ClinchSettingsPageView::new);
        let device_id = clinch_companion_protocol::DeviceId::new();
        let revoke = ClinchSettingsPageAction::RemoteControlRevoke(device_id);

        view.update(&mut app, |view, ctx| view.handle_action(&revoke, ctx));
        view.read(&app, |view, _| {
            assert_eq!(
                view.armed_removal,
                Some(super::ArmedRemoval::Device(device_id))
            );
        });

        view.update(&mut app, |view, ctx| view.handle_action(&revoke, ctx));
        view.read(&app, |view, _| assert_eq!(view.armed_removal, None));
        // The unknown device surfaces an error instead of failing silently.
        RemoteControlService::handle(&app).read(&app, |service, _| {
            assert!(service.view_state().pairing_error.is_some());
        });
    });
}

#[test]
fn agent_status_action_toggles_only_the_clinch_badge_setting() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| AuthStateProvider::new_logged_out_for_test());
        app.add_singleton_model(|_| KeybindingChangedNotifier::new());
        app.add_singleton_model(|_| Appearance::mock());
        let before = SessionSettings::handle(&app)
            .read(&app, |settings, _| settings.notifications.value().clone());
        let (_, view) = app.add_window(WindowStyle::NotStealFocus, ClinchSettingsPageView::new);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&ClinchSettingsPageAction::AgentStatusOnTabs, ctx);
        });

        let after = SessionSettings::handle(&app)
            .read(&app, |settings, _| settings.notifications.value().clone());
        let mut expected = before;
        expected.show_agent_status_on_tabs = !expected.show_agent_status_on_tabs;
        assert_eq!(after, expected);
    });
}

#[test]
fn auto_worktree_action_toggles_the_local_clinch_setting() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| AuthStateProvider::new_logged_out_for_test());
        app.add_singleton_model(|_| KeybindingChangedNotifier::new());
        app.add_singleton_model(|_| Appearance::mock());
        let before = ClinchSettings::handle(&app).read(&app, |settings, _| {
            *settings.auto_create_worktrees_for_new_tabs
        });
        let (_, view) = app.add_window(WindowStyle::NotStealFocus, ClinchSettingsPageView::new);

        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &ClinchSettingsPageAction::AutoCreateWorktreesForNewTabs,
                ctx,
            );
        });

        let after = ClinchSettings::handle(&app).read(&app, |settings, _| {
            *settings.auto_create_worktrees_for_new_tabs
        });
        assert_eq!(after, !before);
    });
}

#[test]
#[cfg(target_os = "macos")]
fn plan_limits_action_toggles_the_existing_opt_in_setting() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| AuthStateProvider::new_logged_out_for_test());
        app.add_singleton_model(|_| KeybindingChangedNotifier::new());
        app.add_singleton_model(|_| Appearance::mock());
        let before = CliAgentUsageSettings::handle(&app)
            .read(&app, |settings, _| *settings.show_plan_limits);
        let (_, view) = app.add_window(WindowStyle::NotStealFocus, ClinchSettingsPageView::new);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&ClinchSettingsPageAction::CliAgentPlanLimits, ctx);
        });

        let after = CliAgentUsageSettings::handle(&app)
            .read(&app, |settings, _| *settings.show_plan_limits);
        assert_eq!(after, !before);
    });
}
