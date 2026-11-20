//! Connection passwords in the operating system keychain.
//!
//! Windows Credential Manager, macOS Keychain, or the Secret Service on
//! Linux. Nothing here ever writes a password to a file.

use keyring::{Entry, Error};
use secrecy::{ExposeSecret, SecretString};

/// Keychain entries are filed under this service name, one per profile id.
const SERVICE: &str = "sqltool";

fn entry(profile_id: &str) -> Result<Entry, String> {
    Entry::new(SERVICE, profile_id).map_err(|e| e.to_string())
}

pub fn save(profile_id: &str, password: &SecretString) -> Result<(), String> {
    entry(profile_id)?.set_password(password.expose_secret()).map_err(|e| e.to_string())
}

/// `Ok(None)` when no password is stored for this profile.
pub fn load(profile_id: &str) -> Result<Option<SecretString>, String> {
    match entry(profile_id)?.get_password() {
        Ok(password) => Ok(Some(password.into())),
        Err(Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Removing a password that is not there is not an error.
pub fn delete(profile_id: &str) -> Result<(), String> {
    match entry(profile_id)?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}
