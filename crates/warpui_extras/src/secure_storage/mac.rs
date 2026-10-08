//! Implementations of the [`SecureStorage`] service for the macOS platform.

use std::sync::OnceLock;

use anyhow::anyhow;
use security_framework::os::macos::keychain::SecKeychain;
use security_framework::os::macos::keychain_item::SecKeychainItem;
use security_framework::os::macos::passwords::SecKeychainItemPassword;

use super::Error;

// Security.framework's errSecItemNotFound. Other failures must remain errors:
// a cancelled authorization prompt is not permission to create a replacement.
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

/// Disables login-Keychain prompts for the lifetime of this process.
///
/// The newer per-query authentication options do not suppress prompts for
/// legacy, file-based keychains. Never restore this process-wide setting:
/// scoped guards can re-enable prompts while another thread is reading.
fn disable_keychain_user_interaction() -> Result<(), Error> {
    static INITIALIZED: OnceLock<Result<(), i32>> = OnceLock::new();
    match INITIALIZED.get_or_init(|| {
        SecKeychain::disable_user_interaction()
            .map(std::mem::forget)
            .map_err(|error| error.code())
    }) {
        Ok(()) => Ok(()),
        Err(code) => Err(Error::Unknown(anyhow!(
            "Could not disable Keychain user interaction (OSStatus {code})"
        ))),
    }
}

/// Implementation of the SecureStorage service using macOS Security
/// framework keychains.
pub struct SecureStorage {
    /// The name of the service under which to store the values.
    service_name: String,
    /// A failure to establish the no-prompt policy blocks every operation.
    initialization_error: Option<String>,
}

impl SecureStorage {
    pub fn new(service_name: &str) -> Self {
        Self {
            service_name: service_name.to_owned(),
            initialization_error: None,
        }
    }

    pub fn new_noninteractive(service_name: &str) -> Self {
        Self {
            service_name: service_name.to_owned(),
            initialization_error: disable_keychain_user_interaction()
                .err()
                .map(|error| format!("{error:?}")),
        }
    }

    fn check_access(&self) -> Result<(), Error> {
        match &self.initialization_error {
            Some(error) => Err(Error::Unknown(anyhow!(
                "Noninteractive Keychain access is unavailable: {error}"
            ))),
            None => Ok(()),
        }
    }
}

impl super::SecureStorage for SecureStorage {
    fn write_value(&self, key: &str, value: &str) -> Result<(), Error> {
        self.check_access()?;
        let keychain = SecKeychain::default()?;
        update_or_add(
            keychain.find_generic_password(&self.service_name, key),
            |(_, mut item)| item.set_password(value.as_bytes()),
            || keychain.add_generic_password(&self.service_name, key, value.as_bytes()),
        )
    }

    fn read_value(&self, key: &str) -> Result<String, Error> {
        let (password, _) = self.get_password_item(key)?;
        String::from_utf8(password.as_ref().to_vec())
            .map_err(|err| Error::DecodeError(err.utf8_error()))
    }

    fn remove_value(&self, key: &str) -> Result<(), Error> {
        let (_, item) = self.get_password_item(key)?;
        item.delete();
        Ok(())
    }
}

impl SecureStorage {
    fn get_password_item(
        &self,
        key: &str,
    ) -> Result<(SecKeychainItemPassword, SecKeychainItem), Error> {
        self.check_access()?;
        let keychain = SecKeychain::default()?;
        keychain
            .find_generic_password(&self.service_name, key)
            .map_err(Into::into)
    }
}

fn update_or_add<T>(
    existing: Result<T, security_framework::base::Error>,
    update: impl FnOnce(T) -> Result<(), security_framework::base::Error>,
    add: impl FnOnce() -> Result<(), security_framework::base::Error>,
) -> Result<(), Error> {
    match existing {
        Ok(item) => update(item).map_err(Into::into),
        Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => add().map_err(Into::into),
        Err(error) => Err(error.into()),
    }
}

impl From<security_framework::base::Error> for Error {
    fn from(value: security_framework::base::Error) -> Self {
        if value.code() == ERR_SEC_ITEM_NOT_FOUND {
            Error::NotFound
        } else {
            Error::Unknown(anyhow!(value))
        }
    }
}

#[cfg(test)]
#[path = "mac_tests.rs"]
mod tests;
