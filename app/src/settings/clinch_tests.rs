use settings::{Setting as _, SyncToCloud};

use super::{AutoCreateWorktreesForNewTabs, IMessageConfiguration, IMessageConfigurationSetting};

#[test]
fn auto_create_worktrees_for_new_tabs_defaults_to_on() {
    assert!(AutoCreateWorktreesForNewTabs::default_value());
}

#[test]
fn imessage_configuration_defaults_to_disconnected_and_disabled() {
    assert_eq!(
        IMessageConfigurationSetting::default_value(),
        IMessageConfiguration::default()
    );
    assert!(!IMessageConfigurationSetting::default_value().enabled);
    assert!(!IMessageConfigurationSetting::default_value().setup_complete);
    assert!(IMessageConfigurationSetting::default_value().notifications_enabled_by_default);
}

#[test]
fn legacy_imessage_configuration_defaults_session_notifications_to_on() {
    let configuration: IMessageConfiguration = serde_json::from_str(
        r#"{"enabled":true,"setup_complete":true,"recipient":"+14155551212","chat_id":7,"chat_guid":"chat"}"#,
    )
    .unwrap();

    assert!(configuration.notifications_enabled_by_default);
}

#[test]
fn imessage_configuration_is_private_and_local_only() {
    assert!(IMessageConfigurationSetting::is_private());
    assert_eq!(
        IMessageConfigurationSetting::sync_to_cloud(),
        SyncToCloud::Never
    );
}
