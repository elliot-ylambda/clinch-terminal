//! Lazy persistence for the shared Remote Control authority.

use anyhow::{Context as _, Result};
use warpui_extras::secure_storage::{Error, SecureStorage};

use super::pairing::{DeviceRegistry, PairingManager};
use super::DEVICE_REGISTRY_STORAGE_KEY;

#[derive(Default)]
pub(super) struct RegistryStorage {
    loaded: bool,
}

impl RegistryStorage {
    pub(super) fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub(super) fn ensure_loaded(
        &mut self,
        pairing: &PairingManager,
        storage: &dyn SecureStorage,
    ) -> Result<()> {
        if self.loaded {
            return Ok(());
        }
        match storage.read_value(DEVICE_REGISTRY_STORAGE_KEY) {
            Ok(json) => {
                let registry = serde_json::from_str::<DeviceRegistry>(&json)
                    .context("Could not decode saved Remote Control devices")?;
                pairing
                    .restore_registry(registry)
                    .context("Could not restore saved Remote Control devices")?;
            }
            // The in-memory default is safe only when the item is truly missing.
            // Save its route before start can publish a persistent Tailscale mount,
            // even if the user quits before approving their first device.
            Err(Error::NotFound) => {
                write_registry(pairing, storage)
                    .context("Could not save the new Remote Control device registry")?;
            }
            Err(error) => {
                return Err(error).context("Could not read saved Remote Control devices");
            }
        }
        self.loaded = true;
        Ok(())
    }

    pub(super) fn persist(
        &self,
        pairing: &PairingManager,
        storage: &dyn SecureStorage,
    ) -> Result<()> {
        anyhow::ensure!(
            self.loaded,
            "Remote Control device registry has not been loaded"
        );
        write_registry(pairing, storage)
    }
}

fn write_registry(pairing: &PairingManager, storage: &dyn SecureStorage) -> Result<()> {
    let registry = pairing.registry_snapshot()?;
    let json = serde_json::to_string(&registry)?;
    storage.write_value_with_owner_only_fallback(DEVICE_REGISTRY_STORAGE_KEY, &json)?;
    Ok(())
}

#[cfg(test)]
#[path = "registry_storage_tests.rs"]
mod tests;
