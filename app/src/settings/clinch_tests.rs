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
}

#[test]
fn imessage_configuration_is_private_and_local_only() {
    assert!(IMessageConfigurationSetting::is_private());
    assert_eq!(
        IMessageConfigurationSetting::sync_to_cloud(),
        SyncToCloud::Never
    );
}
