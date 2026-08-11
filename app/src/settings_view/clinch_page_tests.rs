use settings::Setting as _;
#[cfg(feature = "local_fs")]
use warp_core::channel::{Channel, ChannelConfig, ChannelState};
#[cfg(feature = "local_fs")]
use warp_core::AppId;
use warpui::platform::WindowStyle;
use warpui::{App, SingletonEntity, TypedActionView};

use super::{
    quick_inserts_widget_id, remote_control_browser_url, remote_control_setup_widget_id,
    ClinchSettingsPageAction, ClinchSettingsPageView, QuickInsertsWidget, RemoteControlSetupWidget,
    CLINCH_REMOTE_CONTROL_GUIDE_URL, TAILSCALE_IOS_DOWNLOAD_URL, TAILSCALE_MAC_DOWNLOAD_URL,
};
use crate::ai::blocklist::agent_view::agent_input_footer::editor::AgentToolbarEditorMode;
use crate::ai::blocklist::agent_view::agent_input_footer::quick_insert_modal::{
    QuickInsertModalEvent, QuickInsertModalTarget,
};
use crate::ai::blocklist::agent_view::agent_input_footer::toolbar_item::AgentToolbarItemKind;
use crate::appearance::Appearance;
use crate::auth::AuthStateProvider;
#[cfg(feature = "local_fs")]
use crate::remote_control::RemoteControlService;
#[cfg(target_os = "macos")]
use crate::settings::CliAgentUsageSettings;
use crate::settings::ClinchSettings;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::settings_view::settings_page::SettingsWidget;
use crate::terminal::session_settings::{SessionSettings, ToolbarChipSelection};
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
fn quick_inserts_widget_has_stable_discovery_metadata() {
    let widget = QuickInsertsWidget::default();

    assert_eq!(quick_inserts_widget_id(), widget.widget_id());
    assert!(widget.search_terms().contains("quick insert"));
    assert!(widget.search_terms().contains("auto send"));
}

#[test]
fn quick_insert_edit_action_updates_label_text_and_send_behavior() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| AuthStateProvider::new_logged_out_for_test());
        app.add_singleton_model(|_| KeybindingChangedNotifier::new());
        app.add_singleton_model(|_| Appearance::mock());
        let (_, view) = app.add_window(WindowStyle::NotStealFocus, ClinchSettingsPageView::new);
        let location = view.read(&app, |view, ctx| {
            view.cli_quick_insert_editor
                .as_ref(ctx)
                .quick_inserts()
                .into_iter()
                .next()
                .expect("CLI defaults should include a quick insert")
                .location
        });

        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &ClinchSettingsPageAction::EditQuickInsert {
                    mode: AgentToolbarEditorMode::CLIAgent,
                    location,
                },
                ctx,
            );
            assert!(view.quick_insert_modal_open);
            view.handle_quick_insert_modal_event(
                &QuickInsertModalEvent::Save {
                    target: QuickInsertModalTarget::CLIAgent,
                    label: "Review carefully".to_owned(),
                    text: "Review this without submitting yet".to_owned(),
                    auto_send: false,
                },
                ctx,
            );
            assert!(!view.quick_insert_modal_open);
        });

        let items = SessionSettings::handle(&app).read(&app, |settings, _| {
            settings.cli_agent_footer_chip_selection.left_items()
        });
        assert!(items.iter().any(|item| {
            matches!(
                item,
                AgentToolbarItemKind::CustomInsert {
                    label,
                    text,
                    auto_send: false,
                } if label == "Review carefully" && text == "Review this without submitting yet"
            )
        }));
    });
}

#[test]
fn remote_control_browser_link_gets_a_fresh_load_marker() {
    assert_eq!(
        remote_control_browser_url("https://mac.example/clinch-remote", 1234),
        "https://mac.example/clinch-remote/?clinch_refresh=1234"
    );
    assert_eq!(
        remote_control_browser_url("https://mac.example/clinch-remote/", 1234),
        "https://mac.example/clinch-remote/?clinch_refresh=1234"
    );
    assert_eq!(
        remote_control_browser_url("https://mac.example/clinch-remote?source=settings", 1234),
        "https://mac.example/clinch-remote/?source=settings&clinch_refresh=1234"
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
