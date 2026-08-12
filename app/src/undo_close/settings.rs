use std::time::Duration;

use settings::macros::define_settings_group;
use settings::{RespectUserSyncSetting, SupportedPlatforms, SyncToCloud};

define_settings_group!(UndoCloseSettings, settings: [
    enabled: UndoCloseEnabled {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "general.undo_close.enabled",
        description: "Whether closed sessions are retained without a time limit so they can be reopened.",
    },
    // Kept so existing synced settings files remain valid. This value is no
    // longer used; undo-close history has no elapsed-time expiration.
    grace_period: UndoCloseGracePeriod {
        type: Duration,
        default: Duration::from_secs(60),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "general.undo_close.grace_period",
        description: "Deprecated compatibility setting; no longer limits how long closed sessions can be reopened.",
    },
    maximum_retained_terminal_bytes: UndoCloseMaximumRetainedTerminalBytes {
        type: usize,
        default: 256 * 1024 * 1024,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "general.undo_close.maximum_retained_terminal_bytes",
        description: "The aggregate estimated terminal-grid memory, in bytes, retained for Undo \
                      Close. Old closed sessions are permanently discarded when this budget is \
                      exceeded. Set to 0 to disable this memory limit.",
    },
]);
