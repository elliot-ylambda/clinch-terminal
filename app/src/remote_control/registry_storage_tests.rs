use std::cell::{Cell, RefCell};
use std::rc::Rc;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use chrono::Utc;
use clinch_companion_protocol::{Capability, DeviceId, DevicePlatform, PairingClaimId};

use super::*;
use crate::remote_control::pairing::PairedDeviceRecord;

#[derive(Default)]
struct MemoryStorage {
    value: RefCell<Option<String>>,
    unavailable: Cell<bool>,
    write_unavailable: Cell<bool>,
    reads: Cell<usize>,
    writes: Cell<usize>,
}

impl SecureStorage for MemoryStorage {
    fn read_value(&self, key: &str) -> Result<String, Error> {
        assert_eq!(key, DEVICE_REGISTRY_STORAGE_KEY);
        self.reads.set(self.reads.get() + 1);
        if self.unavailable.get() {
            return Err(Error::Unknown(anyhow::anyhow!("access denied")));
        }
        self.value.borrow().clone().ok_or(Error::NotFound)
    }

    fn write_value(&self, key: &str, value: &str) -> Result<(), Error> {
        assert_eq!(key, DEVICE_REGISTRY_STORAGE_KEY);
        self.writes.set(self.writes.get() + 1);
        if self.write_unavailable.get() {
            return Err(Error::Unknown(anyhow::anyhow!("write access denied")));
        }
        *self.value.borrow_mut() = Some(value.to_owned());
        Ok(())
    }

    fn remove_value(&self, _: &str) -> Result<(), Error> {
        panic!("registry initialization must never remove saved devices")
    }
}

struct SharedMemoryStorage(Rc<MemoryStorage>);

impl SecureStorage for SharedMemoryStorage {
    fn read_value(&self, key: &str) -> Result<String, Error> {
        self.0.read_value(key)
    }

    fn write_value(&self, key: &str, value: &str) -> Result<(), Error> {
        self.0.write_value(key, value)
    }

    fn remove_value(&self, key: &str) -> Result<(), Error> {
        self.0.remove_value(key)
    }
}

fn paired_registry() -> DeviceRegistry {
    let mut key = [0; 65];
    key[0] = 4;
    DeviceRegistry {
        devices: vec![PairedDeviceRecord {
            id: DeviceId::new(),
            name: "Fixture phone".to_owned(),
            platform: DevicePlatform::Ios,
            capabilities: vec![Capability::View],
            public_key_p256_raw: BASE64_STANDARD.encode(key),
            paired_at: Utc::now(),
            last_seen_at: None,
            revoked_at: None,
        }],
        ..DeviceRegistry::default()
    }
}

fn storage_with_registry(registry: &DeviceRegistry) -> MemoryStorage {
    MemoryStorage {
        value: RefCell::new(Some(serde_json::to_string(registry).unwrap())),
        ..MemoryStorage::default()
    }
}

#[test]
fn first_use_restores_shared_authority_once_without_rewriting_it() {
    let saved = paired_registry();
    let storage = storage_with_registry(&saved);
    let pairing = PairingManager::new(DeviceRegistry::default()).unwrap();
    let adapter_pairing = pairing.clone();
    let mut registry_storage = RegistryStorage::default();

    assert_eq!(storage.reads.get(), 0);
    assert_eq!(storage.writes.get(), 0);
    registry_storage.ensure_loaded(&pairing, &storage).unwrap();
    assert_eq!(adapter_pairing.registry_snapshot().unwrap(), saved);
    registry_storage.ensure_loaded(&pairing, &storage).unwrap();
    assert_eq!(storage.reads.get(), 1);
    assert_eq!(storage.writes.get(), 0);
}

#[test]
fn denied_read_cannot_overwrite_devices_and_can_be_retried() {
    let saved = paired_registry();
    let storage = storage_with_registry(&saved);
    let original = storage.value.borrow().clone();
    storage.unavailable.set(true);
    let pairing = PairingManager::new(DeviceRegistry::default()).unwrap();
    let mut registry_storage = RegistryStorage::default();

    assert!(registry_storage.ensure_loaded(&pairing, &storage).is_err());
    assert!(!registry_storage.loaded);
    assert!(registry_storage.persist(&pairing, &storage).is_err());
    assert_eq!(*storage.value.borrow(), original);
    assert_eq!(storage.writes.get(), 0);

    storage.unavailable.set(false);
    registry_storage.ensure_loaded(&pairing, &storage).unwrap();
    assert_eq!(pairing.registry_snapshot().unwrap(), saved);
    assert_eq!(storage.writes.get(), 0);
}

#[test]
fn malformed_and_unsupported_registries_are_preserved() {
    let mut unsupported = paired_registry();
    unsupported.version += 1;
    for value in [
        "not json".to_owned(),
        serde_json::to_string(&unsupported).unwrap(),
    ] {
        let storage = MemoryStorage {
            value: RefCell::new(Some(value.clone())),
            ..MemoryStorage::default()
        };
        let pairing = PairingManager::new(DeviceRegistry::default()).unwrap();
        let mut registry_storage = RegistryStorage::default();
        assert!(registry_storage.ensure_loaded(&pairing, &storage).is_err());
        assert!(registry_storage.persist(&pairing, &storage).is_err());
        assert_eq!(storage.value.borrow().as_deref(), Some(value.as_str()));
        assert_eq!(storage.writes.get(), 0);
    }
}

#[test]
fn first_enable_saves_route_once_and_relaunch_restores_it_without_paired_devices() {
    let storage = MemoryStorage::default();
    let pairing = PairingManager::new(DeviceRegistry::default()).unwrap();
    let initial = pairing.registry_snapshot().unwrap();
    let mut registry_storage = RegistryStorage::default();
    assert!(storage.value.borrow().is_none());
    assert_eq!(storage.reads.get(), 0);
    assert_eq!(storage.writes.get(), 0);

    registry_storage.ensure_loaded(&pairing, &storage).unwrap();
    assert_eq!(pairing.registry_snapshot().unwrap(), initial);
    assert_eq!(storage.writes.get(), 1);
    registry_storage.ensure_loaded(&pairing, &storage).unwrap();
    assert_eq!(storage.writes.get(), 1);

    // A new app process begins with a new in-memory route, but must restore
    // the saved route before it can publish another persistent mount.
    let reopened_pairing = PairingManager::new(DeviceRegistry::default()).unwrap();
    let mut reopened_storage = RegistryStorage::default();
    reopened_storage
        .ensure_loaded(&reopened_pairing, &storage)
        .unwrap();
    assert_eq!(reopened_pairing.registry_snapshot().unwrap(), initial);
    assert_eq!(storage.writes.get(), 1);
}

#[test]
fn failed_initial_save_blocks_startup_until_a_successful_retry() {
    let storage = MemoryStorage::default();
    storage.write_unavailable.set(true);
    let pairing = PairingManager::new(DeviceRegistry::default()).unwrap();
    let initial = pairing.registry_snapshot().unwrap();
    let mut registry_storage = RegistryStorage::default();

    // RemoteControlService::start returns before configuring a gateway/route
    // whenever ensure_loaded fails. The state remains retryable, not loaded.
    assert!(registry_storage.ensure_loaded(&pairing, &storage).is_err());
    assert!(!registry_storage.loaded);
    assert!(registry_storage.persist(&pairing, &storage).is_err());
    assert!(storage.value.borrow().is_none());
    assert_eq!(storage.writes.get(), 1);

    storage.write_unavailable.set(false);
    registry_storage.ensure_loaded(&pairing, &storage).unwrap();
    assert!(registry_storage.loaded);
    assert_eq!(pairing.registry_snapshot().unwrap(), initial);
    assert_eq!(
        serde_json::from_str::<DeviceRegistry>(storage.value.borrow().as_deref().unwrap()).unwrap(),
        initial
    );
    assert_eq!(storage.writes.get(), 2);
}

#[test]
fn loading_cannot_replace_an_active_pairing_flow() {
    let saved = paired_registry();
    let storage = storage_with_registry(&saved);
    let pairing = PairingManager::new(DeviceRegistry::default()).unwrap();
    let initial = pairing.registry_snapshot().unwrap();
    pairing
        .create_invitation("https://fixture", Utc::now())
        .unwrap();
    let mut registry_storage = RegistryStorage::default();
    assert!(registry_storage.ensure_loaded(&pairing, &storage).is_err());
    assert_eq!(pairing.registry_snapshot().unwrap(), initial);
    assert_eq!(storage.writes.get(), 0);
}

/// Changes the process-global channel; run with nextest's per-test isolation.
#[test]
fn disabled_device_management_loads_retries_and_revokes_without_starting() {
    use warp_core::channel::{Channel, ChannelConfig, ChannelState};
    use warp_core::AppId;
    use warpui::{App, SingletonEntity as _};
    use warpui_extras::secure_storage;

    use crate::remote_control::{register, RemoteControlService, RemoteControlStatus};
    use crate::settings::RemoteControlSettings;
    use crate::test_util::settings::initialize_settings_for_tests;

    App::test((), |mut app| async move {
        ChannelState::set(ChannelState::new(
            Channel::Local,
            ChannelConfig::no_backend(AppId::new("test", "warp", "WarpTest"), "warp-test.log"),
        ));
        initialize_settings_for_tests(&mut app);
        let saved = paired_registry();
        let device_id = saved.devices[0].id;
        let storage = Rc::new(storage_with_registry(&saved));
        storage.unavailable.set(true);
        app.update(|ctx| {
            secure_storage::Model::handle(ctx).update(ctx, |provider, _| {
                *provider = Box::new(SharedMemoryStorage(storage.clone()));
            });
            register(ctx);
        });
        assert_eq!(storage.reads.get(), 0);
        assert_eq!(storage.writes.get(), 0);

        app.update(|ctx| {
            RemoteControlService::handle(ctx).update(ctx, |service, ctx| {
                service.show_paired_phones(ctx);
                assert!(!service.paired_phones_loaded());
                assert!(matches!(
                    service.view_state().status,
                    RemoteControlStatus::Error {
                        retryable: true,
                        ..
                    }
                ));
                service.approve_pairing(PairingClaimId::new(), ctx);
                assert!(service
                    .view_state()
                    .pairing_error
                    .as_deref()
                    .unwrap()
                    .contains("access denied"));
                service.dismiss_pairing_error(ctx);
                service.revoke_device(device_id, ctx);
                assert!(service
                    .view_state()
                    .pairing_error
                    .as_deref()
                    .unwrap()
                    .contains("access denied"));
                service.dismiss_pairing_error(ctx);
                service.revoke_all_devices(ctx);
                assert!(service
                    .view_state()
                    .pairing_error
                    .as_deref()
                    .unwrap()
                    .contains("access denied"));
                service.dismiss_pairing_error(ctx);
                assert!(!service.paired_phones_loaded());
                assert!(service.runtime.is_none());
                assert!(service.gateway.is_none());
                assert!(!RemoteControlSettings::as_ref(ctx).is_enabled());
            });
        });
        assert_eq!(storage.writes.get(), 0);
        assert_eq!(
            serde_json::from_str::<DeviceRegistry>(storage.value.borrow().as_deref().unwrap())
                .unwrap(),
            saved
        );

        storage.unavailable.set(false);
        app.update(|ctx| {
            RemoteControlService::handle(ctx).update(ctx, |service, ctx| {
                service.retry(ctx);
                assert!(service.paired_phones_loaded());
                assert_eq!(service.view_state().paired_devices.len(), 1);
                assert_eq!(service.view_state().paired_devices[0].id, device_id);
                assert_eq!(service.view_state().status, RemoteControlStatus::Disabled);
                assert!(service.runtime.is_none());
                assert!(service.gateway.is_none());
                assert!(!RemoteControlSettings::as_ref(ctx).is_enabled());
                service.revoke_device(device_id, ctx);
                assert!(service.view_state().pairing_error.is_none());
                assert!(service.view_state().paired_devices.is_empty());
                assert!(service.runtime.is_none());
                assert!(service.gateway.is_none());
                assert!(!RemoteControlSettings::as_ref(ctx).is_enabled());
            });
        });
        assert_eq!(storage.reads.get(), 5);
        assert_eq!(storage.writes.get(), 1);
    });
}
