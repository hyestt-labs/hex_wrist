//! Device secrets in the user's login Keychain, where Wristper kept them:
//! generic-password items, service `dev.wristper.mac.device`, account the
//! watch's device id, value `{"token":…,"e2ee_key":…}` JSON
//! (`KeychainSecretStore.swift`). Reading and writing never show a dialog
//! and never block: user interaction is turned off for the process while a
//! store exists, so a locked Keychain fails in milliseconds and the caller
//! answers `secrets-unavailable` and retries later.
//!
//! Items Wristper.app created trust only its code signature. Hex ships as
//! the same app (`dev.wristper.mac`, same team), so they stay readable; if
//! the signature ever differs, reading one is refused without a dialog:
//! that watch's requests answer 503, Settings shows it needs pairing again,
//! and pairing it again deletes the item and adds a new one.
//!
//! The migration marker lives beside the device items under
//! `<service>.state`, account `migration`, value `done`, so no device id can
//! collide with it.

use security_framework::base::Error;
use security_framework::os::macos::keychain::{KeychainUserInteractionLock, SecKeychain};
use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};

use super::{DeviceSecret, SecretStore};

/// `errSecItemNotFound`.
const ITEM_NOT_FOUND: i32 = -25300;

pub(crate) const DEFAULT_SERVICE: &str = "dev.wristper.mac.device";
/// `Wristper Test.app` keeps its own items, as it keeps its own data folder:
/// its pairings and migration marker never reach the installed app's.
const TEST_BUILD_SERVICE: &str = "dev.wristper.mac.test.device";

fn service_for(test_build: bool) -> &'static str {
    if test_build {
        TEST_BUILD_SERVICE
    } else {
        DEFAULT_SERVICE
    }
}
const MIGRATION_ACCOUNT: &str = "migration";
const MIGRATION_DONE: &[u8] = b"done";

pub(crate) struct KeychainSecretStore {
    service: String,
    _no_dialogs: Option<KeychainUserInteractionLock>,
}

impl KeychainSecretStore {
    pub(crate) fn new() -> Self {
        Self::with_service(service_for(crate::app_paths::is_macos_test_build()))
    }

    pub(crate) fn with_service(service: &str) -> Self {
        Self {
            service: service.to_owned(),
            _no_dialogs: SecKeychain::disable_user_interaction().ok(),
        }
    }

    fn state_service(&self) -> String {
        format!("{}.state", self.service)
    }
}

fn describe(operation: &str, error: &Error) -> String {
    format!("keychain {operation} failed: {error} ({})", error.code())
}

impl SecretStore for KeychainSecretStore {
    fn load(&self, device_id: &str) -> Result<Option<DeviceSecret>, String> {
        let data = match get_generic_password(&self.service, device_id) {
            Ok(data) => data,
            Err(error) if error.code() == ITEM_NOT_FOUND => return Ok(None),
            Err(error) => return Err(describe("read", &error)),
        };
        let secret: DeviceSecret = serde_json::from_slice(&data)
            .map_err(|error| format!("keychain item is not Wristper's JSON: {error}"))?;
        if secret.token.is_empty() {
            return Err("keychain item has no token".to_owned());
        }
        Ok(Some(DeviceSecret {
            e2ee_key: secret.e2ee_key.filter(|key| !key.is_empty()),
            ..secret
        }))
    }

    fn save(&self, device_id: &str, secret: &DeviceSecret) -> Result<(), String> {
        let data = serde_json::to_vec(secret).map_err(|error| error.to_string())?;
        set_generic_password(&self.service, device_id, &data)
            .map_err(|error| describe("write", &error))
    }

    fn migration_completed(&self) -> Result<bool, String> {
        match get_generic_password(&self.state_service(), MIGRATION_ACCOUNT) {
            Ok(data) => Ok(data == MIGRATION_DONE),
            Err(error) if error.code() == ITEM_NOT_FOUND => Ok(false),
            Err(error) => Err(describe("migration marker read", &error)),
        }
    }

    fn mark_migration_completed(&self) -> Result<(), String> {
        set_generic_password(&self.state_service(), MIGRATION_ACCOUNT, MIGRATION_DONE)
            .map_err(|error| describe("migration marker write", &error))
    }

    fn delete(&self, device_id: &str) -> Result<(), String> {
        match delete_generic_password(&self.service, device_id) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == ITEM_NOT_FOUND => Ok(()),
            Err(error) => Err(describe("delete", &error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::service_for;

    #[test]
    fn a_test_build_never_shares_the_installed_apps_keychain_items() {
        assert_eq!(service_for(false), "dev.wristper.mac.device");
        assert_eq!(service_for(true), "dev.wristper.mac.test.device");
        assert_ne!(
            format!("{}.state", service_for(true)),
            format!("{}.state", service_for(false))
        );
    }
}
