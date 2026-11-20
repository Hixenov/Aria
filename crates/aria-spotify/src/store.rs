//! Where the refresh token is kept: the system's protected store (Windows Credential Manager,
//! macOS Keychain, the Secret Service on Linux), or memory only when there is none.

use std::sync::{Mutex, PoisonError};

use crate::{Secret, SpotifyError};

const SERVICE: &str = "Aria";
const ACCOUNT: &str = "spotify";

pub trait TokenStore: Send {
    fn load(&self) -> Result<Option<Secret>, SpotifyError>;
    fn save(&self, token: &Secret) -> Result<(), SpotifyError>;
    fn delete(&self) -> Result<(), SpotifyError>;
    /// Whether a saved token outlives Aria.
    fn persistent(&self) -> bool;
}

/// The system keyring, under service `Aria` and account `spotify`.
pub struct KeyringStore;

impl KeyringStore {
    fn entry() -> Result<keyring::Entry, SpotifyError> {
        keyring::Entry::new(SERVICE, ACCOUNT).map_err(|e| store_error(&e))
    }
}

/// Names a keyring failure by its kind; the error never carries the secret.
fn store_error(error: &keyring::Error) -> SpotifyError {
    let kind = match error {
        keyring::Error::PlatformFailure(_) => "platform failure",
        keyring::Error::NoStorageAccess(_) => "no storage access",
        keyring::Error::NoEntry => "no entry",
        keyring::Error::BadEncoding(_) => "bad encoding",
        keyring::Error::TooLong(..) => "too long",
        keyring::Error::Invalid(..) => "invalid",
        keyring::Error::Ambiguous(_) => "ambiguous",
        _ => "keyring error",
    };
    SpotifyError::Store(kind.to_owned())
}

impl TokenStore for KeyringStore {
    fn load(&self) -> Result<Option<Secret>, SpotifyError> {
        match KeyringStore::entry()?.get_password() {
            Ok(token) => Ok(Some(Secret::new(token))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(store_error(&e)),
        }
    }

    fn save(&self, token: &Secret) -> Result<(), SpotifyError> {
        KeyringStore::entry()?
            .set_password(token.expose())
            .map_err(|e| store_error(&e))
    }

    fn delete(&self) -> Result<(), SpotifyError> {
        match KeyringStore::entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(store_error(&e)),
        }
    }

    fn persistent(&self) -> bool {
        true
    }
}

/// Keeps the token for this run only.
#[derive(Default)]
pub struct MemoryStore {
    token: Mutex<Option<Secret>>,
}

impl MemoryStore {
    pub fn new() -> MemoryStore {
        MemoryStore::default()
    }

    /// A store already holding `token` (for tests).
    pub fn holding(token: &str) -> MemoryStore {
        MemoryStore {
            token: Mutex::new(Some(Secret::new(token.to_owned()))),
        }
    }
}

impl TokenStore for MemoryStore {
    fn load(&self) -> Result<Option<Secret>, SpotifyError> {
        Ok(self
            .token
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone())
    }

    fn save(&self, token: &Secret) -> Result<(), SpotifyError> {
        *self.token.lock().unwrap_or_else(PoisonError::into_inner) = Some(token.clone());
        Ok(())
    }

    fn delete(&self) -> Result<(), SpotifyError> {
        *self.token.lock().unwrap_or_else(PoisonError::into_inner) = None;
        Ok(())
    }

    fn persistent(&self) -> bool {
        false
    }
}

/// The keyring if it answers, else memory. Reading a missing entry counts as working.
pub fn system_store() -> Box<dyn TokenStore> {
    let probe = KeyringStore::entry().and_then(|entry| match entry.get_password() {
        Ok(_) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(store_error(&e)),
    });
    match probe {
        Ok(()) => Box::new(KeyringStore),
        Err(e) => {
            tracing::warn!("no system keyring for the Spotify sign-in: {e}");
            Box::new(MemoryStore::new())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_round_trips_and_deletes() {
        let store = MemoryStore::new();
        assert_eq!(store.load().unwrap(), None);
        store.save(&Secret::new("r1".to_owned())).unwrap();
        assert_eq!(store.load().unwrap().unwrap().expose(), "r1");
        store.delete().unwrap();
        assert_eq!(store.load().unwrap(), None);
        store.delete().unwrap();
        assert_eq!(
            MemoryStore::holding("r2").load().unwrap().unwrap().expose(),
            "r2"
        );
    }

    #[test]
    fn memory_store_is_not_persistent() {
        assert!(!MemoryStore::new().persistent());
        assert!(KeyringStore.persistent());
    }
}
