//! Secure storage for Microsoft authentication tokens.
//!
//! Tokens (`MicrosoftAuth`) are long-lived credentials to the user's Microsoft
//! account, so we keep them out of the plaintext `config.json` and in the OS
//! keyring instead: Credential Manager on Windows, Secret Service on Linux,
//! Keychain on macOS. Each account's tokens are serialized to JSON and stored
//! under one keyring entry keyed by the account UUID.
//!
//! When no keyring backend is available (e.g. a minimal Linux box with no
//! running Secret Service daemon), [`available`] reports `false` and callers in
//! `core::config` fall back to writing tokens into `config.json` as before.

use std::sync::OnceLock;

use keyring::Entry;

use crate::core::config::MicrosoftAuth;

const SERVICE: &str = "pyrite-launcher";

/// Build the keyring entry for an account's tokens.
fn entry(uuid: &str) -> Result<Entry, keyring::Error> {
    Entry::new(SERVICE, uuid)
}

/// Whether a usable keyring backend exists on this system. Probed once and
/// cached: we try a round-trip on a throwaway entry, since `Entry::new`
/// succeeding doesn't guarantee the backend actually works at call time.
pub fn available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let probe = match entry("__pyrite_probe__") {
            Ok(e) => e,
            Err(_) => return false,
        };
        // A set + delete round-trip confirms the backend is writable. We treat
        // a "not found" delete as success so a leftover probe never blocks us.
        match probe.set_password("probe") {
            Ok(()) => {
                let _ = probe.delete_credential();
                true
            }
            Err(_) => false,
        }
    })
}

/// Store an account's tokens in the keyring. Returns `false` (and leaves the
/// caller to fall back to config-file storage) if anything goes wrong.
pub fn store(uuid: &str, auth: &MicrosoftAuth) -> bool {
    let json = match serde_json::to_string(auth) {
        Ok(j) => j,
        Err(_) => return false,
    };
    match entry(uuid) {
        Ok(e) => e.set_password(&json).is_ok(),
        Err(_) => false,
    }
}

/// Load an account's tokens from the keyring, or `None` if absent/unreadable.
pub fn load(uuid: &str) -> Option<MicrosoftAuth> {
    let e = entry(uuid).ok()?;
    let json = match e.get_password() {
        Ok(j) => j,
        Err(keyring::Error::NoEntry) => return None,
        Err(_) => return None,
    };
    serde_json::from_str(&json).ok()
}

/// Remove an account's tokens from the keyring. A missing entry is not an error.
pub fn delete(uuid: &str) {
    if let Ok(e) = entry(uuid) {
        let _ = e.delete_credential();
    }
}
