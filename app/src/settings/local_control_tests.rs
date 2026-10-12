use std::collections::HashMap;
use std::sync::Mutex;

use settings::{PrivatePreferences, PublicPreferences, Setting as _, SettingsManager, SyncToCloud};
use warp_core::channel::{Channel, ChannelState};
use warpui::SingletonEntity as _;
use warpui_extras::secure_storage::{self, AppContextExt as _};
use warpui_extras::user_preferences;

use super::{
    default_mode_for_app, LocalControlMode, LocalControlModeSetting, LocalControlSettings,
};

#[derive(Default)]
struct InMemorySecureStorage {
    values: Mutex<HashMap<String, String>>,
}

impl secure_storage::SecureStorage for InMemorySecureStorage {
    fn write_value(&self, key: &str, value: &str) -> Result<(), secure_storage::Error> {
        match self.values.lock() {
            Ok(mut values) => {
                values.insert(key.to_owned(), value.to_owned());
                Ok(())
            }
            Err(err) => Err(secure_storage::Error::Unknown(anyhow::anyhow!(
                err.to_string()
            ))),
        }
    }

    fn read_value(&self, key: &str) -> Result<String, secure_storage::Error> {
        match self.values.lock() {
            Ok(values) => values
                .get(key)
                .cloned()
                .ok_or(secure_storage::Error::NotFound),
            Err(err) => Err(secure_storage::Error::Unknown(anyhow::anyhow!(
                err.to_string()
            ))),
        }
    }

    fn remove_value(&self, key: &str) -> Result<(), secure_storage::Error> {
        match self.values.lock() {
            Ok(mut values) => {
                values.remove(key);
                Ok(())
            }
            Err(err) => Err(secure_storage::Error::Unknown(anyhow::anyhow!(
                err.to_string()
            ))),
        }
    }
}

fn default_settings() -> LocalControlSettings {
    LocalControlSettings {
        local_control_mode: LocalControlModeSetting::new(None),
    }
}

#[test]
fn default_mode_is_enabled_for_clinch_and_dogfood_builds() {
    assert_eq!(
        default_mode_for_app(Channel::Stable, "sh.clinch.Clinch"),
        LocalControlMode::Enabled
    );
    assert_eq!(
        default_mode_for_app(Channel::Local, "sh.clinch.ClinchDev"),
        LocalControlMode::Enabled
    );
    assert_eq!(
        default_mode_for_app(Channel::Dev, "dev.warp.Warp-Dev"),
        LocalControlMode::Enabled
    );
    assert_eq!(
        default_mode_for_app(Channel::Local, "dev.warp.Warp-Local"),
        LocalControlMode::Enabled
    );
    for channel in [
        Channel::Stable,
        Channel::Preview,
        Channel::Oss,
        Channel::Integration,
    ] {
        assert_eq!(
            default_mode_for_app(channel, "dev.warp.Warp"),
            LocalControlMode::Disabled,
            "non-Clinch {channel} must retain its existing default"
        );
    }
}

#[test]
fn unset_mode_follows_channel_default() {
    let settings = default_settings();

    assert_eq!(LocalControlMode::default(), LocalControlMode::Disabled);
    assert_eq!(
        settings.mode(),
        default_mode_for_app(ChannelState::channel(), &ChannelState::app_id().to_string())
    );
}

#[test]
fn mode_is_persisted_to_secure_storage() {
    warpui::App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| {
                PublicPreferences::new(
                    Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
                )
            });
            ctx.add_singleton_model(|_| {
                PrivatePreferences::new(
                    Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
                )
            });
            ctx.add_singleton_model(|_| SettingsManager::default());
            ctx.add_singleton_model(|_| -> secure_storage::Model {
                Box::<InMemorySecureStorage>::default()
            });
            LocalControlSettings::register(ctx);
        });

        app.update(|ctx| {
            LocalControlSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .local_control_mode
                    .set_value(LocalControlMode::Enabled, ctx)
            })
        })
        .expect("setting update should succeed");

        app.read(|ctx| {
            let stored = ctx
                .secure_storage()
                .read_value(LocalControlModeSetting::storage_key())
                .expect("local-control mode should be stored securely");
            let mode = serde_json::from_str::<LocalControlMode>(&stored)
                .expect("stored local-control mode should deserialize");
            assert_eq!(mode, LocalControlMode::Enabled);

            let private_value = LocalControlModeSetting::preferences_for_setting(ctx)
                .read_value(LocalControlModeSetting::storage_key())
                .expect("private preferences should be readable");
            assert!(private_value.is_none());
        });
    });
}

#[test]
fn explicitly_disabled_mode_survives_registration() {
    warpui::App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| {
                PublicPreferences::new(
                    Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
                )
            });
            ctx.add_singleton_model(|_| {
                PrivatePreferences::new(
                    Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
                )
            });
            ctx.add_singleton_model(|_| SettingsManager::default());
            ctx.add_singleton_model(|_| -> secure_storage::Model {
                Box::<InMemorySecureStorage>::default()
            });
            ctx.secure_storage()
                .write_value(
                    LocalControlModeSetting::storage_key(),
                    &serde_json::to_string(&LocalControlMode::Disabled).expect("mode serializes"),
                )
                .expect("disabled mode should be writable");
            LocalControlSettings::register(ctx);
        });

        app.read(|ctx| {
            let settings = LocalControlSettings::as_ref(ctx);
            assert_eq!(settings.mode(), LocalControlMode::Disabled);
            assert!(settings.local_control_mode.is_value_explicitly_set());
        });
    });
}

#[test]
fn malformed_secure_value_fails_closed() {
    warpui::App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| {
                PublicPreferences::new(
                    Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
                )
            });
            ctx.add_singleton_model(|_| {
                PrivatePreferences::new(
                    Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
                )
            });
            ctx.add_singleton_model(|_| SettingsManager::default());
            ctx.add_singleton_model(|_| -> secure_storage::Model {
                Box::<InMemorySecureStorage>::default()
            });
            ctx.secure_storage()
                .write_value(LocalControlModeSetting::storage_key(), "not-json")
                .expect("malformed fixture should be writable");
            LocalControlSettings::register(ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                LocalControlSettings::as_ref(ctx).mode(),
                LocalControlMode::Disabled
            );
        });
    });
}

#[test]
fn unavailable_secure_value_disables_control_without_overwriting_storage() {
    struct LockedStorage;

    impl secure_storage::SecureStorage for LockedStorage {
        fn read_value(&self, key: &str) -> Result<String, secure_storage::Error> {
            assert_eq!(key, LocalControlModeSetting::storage_key());
            Err(secure_storage::Error::Unknown(anyhow::anyhow!(
                "interaction is not allowed"
            )))
        }

        fn write_value(&self, _: &str, _: &str) -> Result<(), secure_storage::Error> {
            panic!("an unavailable setting must not be replaced")
        }

        fn remove_value(&self, _: &str) -> Result<(), secure_storage::Error> {
            panic!("an unavailable setting must not be removed")
        }
    }

    warpui::App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| -> secure_storage::Model { Box::new(LockedStorage) });
            let setting = LocalControlModeSetting::new_from_storage(ctx);
            assert_eq!(*setting.value(), LocalControlMode::Disabled);
            assert!(setting.is_value_explicitly_set());
        });
    });
}

#[test]
fn mode_does_not_migrate_from_private_preferences() {
    warpui::App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| {
                PublicPreferences::new(
                    Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
                )
            });
            ctx.add_singleton_model(|_| {
                PrivatePreferences::new(
                    Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
                )
            });
            ctx.add_singleton_model(|_| SettingsManager::default());
            ctx.add_singleton_model(|_| -> secure_storage::Model {
                Box::<InMemorySecureStorage>::default()
            });
            LocalControlModeSetting::preferences_for_setting(ctx)
                .write_value(
                    LocalControlModeSetting::storage_key(),
                    serde_json::to_string(&LocalControlMode::Enabled).expect("mode serializes"),
                )
                .expect("private preference is writable");
            LocalControlSettings::register(ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                LocalControlSettings::as_ref(ctx).mode(),
                default_mode_for_app(ChannelState::channel(), &ChannelState::app_id().to_string())
            );
            let private_value = LocalControlModeSetting::preferences_for_setting(ctx)
                .read_value(LocalControlModeSetting::storage_key())
                .expect("private preference is readable");
            assert!(private_value.is_some());
        });
    });
}
#[test]
fn mode_is_private_and_never_cloud_synced() {
    assert_eq!(LocalControlModeSetting::sync_to_cloud(), SyncToCloud::Never);
    assert!(LocalControlModeSetting::is_private());
}

#[test]
fn cloud_sync_cannot_disable_local_control() {
    warpui::App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| {
                PublicPreferences::new(
                    Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
                )
            });
            ctx.add_singleton_model(|_| {
                PrivatePreferences::new(
                    Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
                )
            });
            ctx.add_singleton_model(|_| SettingsManager::default());
            ctx.add_singleton_model(|_| -> secure_storage::Model {
                Box::<InMemorySecureStorage>::default()
            });
            LocalControlSettings::register(ctx);
        });

        app.update(|ctx| {
            LocalControlSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .local_control_mode
                    .set_value(LocalControlMode::Enabled, ctx)
            })
        })
        .expect("local control should enable");

        app.update(|ctx| {
            LocalControlSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .local_control_mode
                    .set_value_from_cloud_sync(LocalControlMode::Disabled, ctx)
            })
        })
        .expect("cloud sync update should be ignored without error");

        app.read(|ctx| {
            let settings = LocalControlSettings::as_ref(ctx);
            assert_eq!(settings.mode(), LocalControlMode::Enabled);
            let stored = ctx
                .secure_storage()
                .read_value(LocalControlModeSetting::storage_key())
                .expect("explicitly enabled mode should remain stored securely");
            let mode = serde_json::from_str::<LocalControlMode>(&stored)
                .expect("stored local-control mode should deserialize");
            assert_eq!(mode, LocalControlMode::Enabled);
        });
    });
}

struct UnavailableLegacyStorage;

impl secure_storage::SecureStorage for UnavailableLegacyStorage {
    fn read_value(&self, _: &str) -> Result<String, secure_storage::Error> {
        Err(secure_storage::Error::Unknown(anyhow::anyhow!(
            "interaction is not allowed (OSStatus -25308)"
        )))
    }

    fn write_value(&self, _: &str, _: &str) -> Result<(), secure_storage::Error> {
        panic!("legacy storage must remain untouched")
    }

    fn remove_value(&self, _: &str) -> Result<(), secure_storage::Error> {
        panic!("legacy storage must remain untouched")
    }
}

#[test]
fn clinch_migrates_both_legacy_choices_and_survives_keychain_failure() {
    for mode in LocalControlMode::ALL {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("local-control-settings/mode.json");
        let storage = InMemorySecureStorage::default();
        secure_storage::SecureStorage::write_value(
            &storage,
            LocalControlModeSetting::storage_key(),
            &serde_json::to_string(&mode).unwrap(),
        )
        .unwrap();
        let migrated = LocalControlModeSetting::from_storage(&storage, Some(path.clone()));
        assert_eq!(*migrated.value(), mode);
        assert!(migrated.load_error().is_none());
        assert_eq!(super::read_local_mode(&path).unwrap(), Some(mode));
        let restarted =
            LocalControlModeSetting::from_storage(&UnavailableLegacyStorage, Some(path));
        assert_eq!(*restarted.value(), mode);
        assert!(restarted.load_error().is_none());
    }
}

#[test]
fn clinch_missing_legacy_preference_uses_default_without_writing() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("local-control-settings/mode.json");
    let setting = LocalControlModeSetting::from_storage(
        &InMemorySecureStorage::default(),
        Some(path.clone()),
    );
    assert_eq!(*setting.value(), LocalControlModeSetting::default_value());
    assert!(!setting.is_value_explicitly_set());
    assert!(setting.load_error().is_none());
    assert!(!path.exists());
}

#[test]
fn clinch_unavailable_legacy_preference_automatically_enables_and_persists() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("local-control-settings/mode.json");
    let setting =
        LocalControlModeSetting::from_storage(&UnavailableLegacyStorage, Some(path.clone()));
    assert_eq!(*setting.value(), LocalControlMode::Enabled);
    assert!(setting.load_error().is_none());
    assert_eq!(
        super::read_local_mode(&path).unwrap(),
        Some(LocalControlMode::Enabled)
    );
    let restarted = LocalControlModeSetting::from_storage(&UnavailableLegacyStorage, Some(path));
    assert_eq!(*restarted.value(), LocalControlMode::Enabled);
    assert!(restarted.load_error().is_none());
}

#[test]
fn clinch_explicit_disable_recovers_and_clear_does_not_revisit_legacy() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("local-control-settings/mode.json");
    let setting =
        LocalControlModeSetting::from_storage(&UnavailableLegacyStorage, Some(path.clone()));
    let test_path = path.clone();
    warpui::App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| -> secure_storage::Model {
                Box::new(UnavailableLegacyStorage)
            });
            ctx.add_singleton_model(|_| LocalControlSettings {
                local_control_mode: setting,
            });
            LocalControlSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .local_control_mode
                    .set_value(LocalControlMode::Disabled, ctx)
                    .unwrap();
                assert!(settings.load_error().is_none());
                assert_eq!(
                    super::read_local_mode(&test_path).unwrap(),
                    Some(LocalControlMode::Disabled)
                );
                settings.local_control_mode.clear_value(ctx).unwrap();
                assert!(settings.load_error().is_none());
                assert!(!settings.local_control_mode.is_value_explicitly_set());
            });
        });
    });
    let restarted = LocalControlModeSetting::from_storage(&UnavailableLegacyStorage, Some(path));
    assert_eq!(*restarted.value(), LocalControlModeSetting::default_value());
    assert!(restarted.load_error().is_none());
}

#[test]
fn clinch_malformed_legacy_preference_is_preserved_and_uses_enabled_default() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("local-control-settings/mode.json");
    let storage = InMemorySecureStorage::default();
    secure_storage::SecureStorage::write_value(
        &storage,
        LocalControlModeSetting::storage_key(),
        "broken",
    )
    .unwrap();
    let setting = LocalControlModeSetting::from_storage(&storage, Some(path.clone()));
    assert_eq!(*setting.value(), LocalControlMode::Enabled);
    assert!(setting.load_error().is_none());
    assert_eq!(
        super::read_local_mode(&path).unwrap(),
        Some(LocalControlMode::Enabled)
    );
    assert_eq!(
        secure_storage::SecureStorage::read_value(&storage, LocalControlModeSetting::storage_key())
            .unwrap(),
        "broken"
    );
}

#[test]
fn clinch_corrupt_local_preference_does_not_fall_back_to_enabled_legacy() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("local-control-settings/mode.json");
    super::write_local_mode(&path, LocalControlMode::Disabled).unwrap();
    std::fs::write(&path, "broken").unwrap();
    let storage = InMemorySecureStorage::default();
    secure_storage::SecureStorage::write_value(
        &storage,
        LocalControlModeSetting::storage_key(),
        "\"Enabled\"",
    )
    .unwrap();
    let setting = LocalControlModeSetting::from_storage(&storage, Some(path));
    assert_eq!(*setting.value(), LocalControlMode::Disabled);
    assert!(setting.load_error().is_some());
}

#[cfg(unix)]
#[test]
fn clinch_preference_is_owner_only_and_rejects_symlinks() {
    use std::os::unix::fs::{symlink, PermissionsExt as _};

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("local-control-settings/mode.json");
    super::write_local_mode(&path, LocalControlMode::Enabled).unwrap();
    assert_eq!(
        std::fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let target = directory.path().join("unrelated.json");
    std::fs::write(&target, "do not change").unwrap();
    std::fs::remove_file(&path).unwrap();
    symlink(&target, &path).unwrap();
    assert!(super::read_local_mode(&path).is_err());
    assert!(super::write_local_mode(&path, LocalControlMode::Disabled).is_err());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "do not change");
}

#[cfg(unix)]
#[test]
fn clinch_preference_rejects_shared_permissions_and_symlinked_directory() {
    use std::os::unix::fs::{symlink, PermissionsExt as _};

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("local-control-settings/mode.json");
    super::write_local_mode(&path, LocalControlMode::Enabled).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
    let setting =
        LocalControlModeSetting::from_storage(&UnavailableLegacyStorage, Some(path.clone()));
    assert_eq!(*setting.value(), LocalControlMode::Disabled);
    assert!(setting.load_error().is_some());
    assert!(super::write_local_mode(&path, LocalControlMode::Enabled).is_err());
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    let target = directory.path().join("unrelated-directory");
    std::fs::create_dir(&target).unwrap();
    symlink(&target, path.parent().unwrap()).unwrap();
    assert!(super::read_local_mode(&path).is_err());
    assert!(super::write_local_mode(&path, LocalControlMode::Enabled).is_err());
    assert!(!target.join("mode.json").exists());
}
