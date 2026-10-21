//! The GitHub token, one per computer, kept only in the OS secure store: macOS Keychain or
//! Windows Credential Manager (local persistence, so it doesn't roam with a domain
//! profile). It is read in Rust when a request needs it and never sent to the renderer,
//! written to SQLite, exports, logs, or error messages.

use crate::error::{AppError, AppResult};
use crate::paths::StorageProfile;

pub trait SecretStore: Send + Sync {
    fn read(&self) -> AppResult<Option<String>>;
    fn write(&self, token: &str) -> AppResult<()>;
    /// Removing a token that isn't there is not an error.
    fn delete(&self) -> AppResult<()>;
}

/// Fine-grained personal access tokens only (classic tokens can't be limited to one
/// repository). Messages never repeat the input.
pub fn validate_token(raw: &str) -> AppResult<String> {
    let token = raw.trim();
    if !token.starts_with("github_pat_") {
        return Err(AppError::validation(
            "token",
            "use a fine-grained personal access token (it starts with github_pat_)",
        ));
    }
    let ok = (40..=255).contains(&token.len())
        && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !ok {
        return Err(AppError::validation(
            "token",
            "that doesn't look like a complete GitHub token",
        ));
    }
    Ok(token.to_owned())
}

const ACCOUNT: &str = "github-token";

pub struct OsSecretStore {
    service: String,
}

impl OsSecretStore {
    /// Development builds use their own entry so they can never touch the real token.
    pub fn new(profile: StorageProfile) -> Self {
        Self {
            service: match profile {
                StorageProfile::Production => "MuDraft GitHub sync",
                StorageProfile::Development => "MuDraft GitHub sync (development)",
            }
            .to_owned(),
        }
    }

    /// A separate entry, so tests never touch a real token.
    #[cfg(test)]
    pub(crate) fn named(service: String) -> Self {
        Self { service }
    }

    fn entry(&self) -> AppResult<keyring_core::Entry> {
        #[cfg(target_os = "macos")]
        {
            use keyring_core::api::CredentialStoreApi as _;
            let store = apple_native_keyring_store::keychain::Store::new().map_err(describe)?;
            store.build(&self.service, ACCOUNT, None).map_err(describe)
        }
        #[cfg(target_os = "windows")]
        {
            use keyring_core::api::CredentialStoreApi as _;
            let store = windows_native_keyring_store::Store::new().map_err(describe)?;
            let local = std::collections::HashMap::from([("persistence", "Local")]);
            store
                .build(self.service, ACCOUNT, Some(&local))
                .map_err(describe)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Err(AppError::StorageUnavailable(
                "no secure credential store is available on this platform".into(),
            ))
        }
    }
}

/// Describe a store error without anything it carries (some variants hold secret bytes).
fn describe(e: keyring_core::Error) -> AppError {
    use keyring_core::Error as E;
    let what = match e {
        E::NoStorageAccess(_) => "access was denied or the store is locked",
        E::BadEncoding(_) | E::BadDataFormat(..) => {
            "the saved token is unreadable; remove it and add it again"
        }
        E::NoDefaultStore | E::NotSupportedByStore(_) | E::BadStoreFormat(_) => {
            "it isn't available"
        }
        E::TooLong(..) | E::Invalid(..) => "it refused the entry",
        E::Ambiguous(_) => "it holds more than one MuDraft token",
        _ => "it reported an error",
    };
    AppError::StorageUnavailable(format!("secure credential store: {what}"))
}

impl SecretStore for OsSecretStore {
    fn read(&self) -> AppResult<Option<String>> {
        match self.entry()?.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(describe(e)),
        }
    }

    fn write(&self, token: &str) -> AppResult<()> {
        self.entry()?.set_password(token).map_err(describe)
    }

    fn delete(&self) -> AppResult<()> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(e) => Err(describe(e)),
        }
    }
}

#[cfg(test)]
pub(crate) mod memory {
    use std::sync::Mutex;

    use super::*;

    /// In-memory stand-in for tests (the real store would touch the user's keychain).
    #[derive(Default)]
    pub struct MemorySecretStore(pub Mutex<Option<String>>);

    impl SecretStore for MemorySecretStore {
        fn read(&self) -> AppResult<Option<String>> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn write(&self, token: &str) -> AppResult<()> {
            *self.0.lock().unwrap() = Some(token.to_owned());
            Ok(())
        }
        fn delete(&self) -> AppResult<()> {
            *self.0.lock().unwrap() = None;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes, reads, replaces, and deletes a throwaway entry in the real Keychain or
    /// Credential Manager. Ignored by default because it touches the user's store.
    #[test]
    #[ignore = "uses the OS credential store; run with `cargo test -- --ignored os_secure_store`"]
    fn os_secure_store_round_trip() {
        let store = OsSecretStore::named(format!(
            "MuDraft sync test {}",
            crate::domain::ids::new_id()
        ));
        let first = format!("github_pat_{}", "A".repeat(40));
        let second = format!("github_pat_{}", "B".repeat(40));
        assert_eq!(store.read().unwrap(), None);
        store.write(&first).unwrap();
        assert_eq!(store.read().unwrap().as_deref(), Some(first.as_str()));
        store.write(&second).unwrap();
        assert_eq!(store.read().unwrap().as_deref(), Some(second.as_str()));
        store.delete().unwrap();
        assert_eq!(store.read().unwrap(), None);
        store.delete().unwrap();
    }

    #[test]
    fn only_fine_grained_tokens_are_accepted_and_never_echoed() {
        let good = format!("github_pat_{}", "A1b2_".repeat(12));
        assert_eq!(validate_token(&format!("  {good}\n")).unwrap(), good);
        for bad in [
            "ghp_classic0123456789012345678901234567890",
            "github_pat_short",
            &format!("{good} x"),
        ] {
            let err = validate_token(bad).unwrap_err();
            assert!(
                !err.to_string().contains(bad.trim()),
                "error echoed the token"
            );
        }
    }
}
