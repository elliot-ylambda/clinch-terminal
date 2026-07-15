use settings::Setting as _;
use warpui::platform::WindowStyle;
use warpui::{App, SingletonEntity, TypedActionView};

#[cfg(target_os = "macos")]
use super::{imessage_setup_stage, IMessageSetupStage};
use super::{ClinchSettingsPageAction, ClinchSettingsPageView};
use crate::appearance::Appearance;
use crate::auth::AuthStateProvider;
#[cfg(target_os = "macos")]
use crate::imessage::{IMessageConnectionStatus, IMessageCoordinator, IMessagePermission};
#[cfg(target_os = "macos")]
use crate::settings::CliAgentUsageSettings;
use crate::settings::ClinchSettings;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
#[cfg(target_os = "macos")]
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::session_settings::SessionSettings;
use crate::test_util::settings::initialize_settings_for_tests;

#[test]
#[cfg(target_os = "macos")]
fn imessage_setup_stage_tracks_the_submitted_number_and_permission_flow() {
    assert_eq!(
        imessage_setup_stage(false, true, &IMessageConnectionStatus::SetupRequired),
        IMessageSetupStage::EnterNumber
    );
    assert_eq!(
        imessage_setup_stage(false, false, &IMessageConnectionStatus::Connecting),
        IMessageSetupStage::Connecting
    );
    assert_eq!(
        imessage_setup_stage(
            false,
            false,
            &IMessageConnectionStatus::Paused(IMessagePermission::FullDiskAccess),
        ),
        IMessageSetupStage::FullDiskAccess
    );
    assert_eq!(
        imessage_setup_stage(
            false,
            false,
            &IMessageConnectionStatus::AwaitingCalibrationReply,
        ),
        IMessageSetupStage::AwaitingReply
    );
    assert_eq!(
        imessage_setup_stage(true, false, &IMessageConnectionStatus::Connected),
        IMessageSetupStage::Connected
    );
    assert_eq!(
        imessage_setup_stage(true, false, &IMessageConnectionStatus::Disabled),
        IMessageSetupStage::Connected
    );
}

#[test]
fn agent_status_action_toggles_only_the_clinch_badge_setting() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        #[cfg(target_os = "macos")]
        app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        #[cfg(target_os = "macos")]
        app.add_singleton_model(IMessageCoordinator::new);
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
        #[cfg(target_os = "macos")]
        app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        #[cfg(target_os = "macos")]
        app.add_singleton_model(IMessageCoordinator::new);
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
        app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        app.add_singleton_model(IMessageCoordinator::new);
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

#[test]
#[cfg(target_os = "macos")]
fn imessage_default_action_changes_only_the_session_notification_default() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        app.add_singleton_model(IMessageCoordinator::new);
        app.add_singleton_model(|_| AuthStateProvider::new_logged_out_for_test());
        app.add_singleton_model(|_| KeybindingChangedNotifier::new());
        app.add_singleton_model(|_| Appearance::mock());
        let before =
            ClinchSettings::handle(&app).read(&app, |settings, _| settings.imessage().clone());
        let (_, view) = app.add_window(WindowStyle::NotStealFocus, ClinchSettingsPageView::new);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&ClinchSettingsPageAction::IMessageNotificationsDefault, ctx);
        });

        let after =
            ClinchSettings::handle(&app).read(&app, |settings, _| settings.imessage().clone());
        let mut expected = before;
        expected.notifications_enabled_by_default = !expected.notifications_enabled_by_default;
        assert_eq!(after, expected);
    });
}
