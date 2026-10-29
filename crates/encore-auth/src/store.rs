//! Session persistence (D9): the whole session (structured cookie jar plus
//! channel selection) is one item in the macOS Keychain under service
//! `app.encore.desktop`. No file fallback.

use parking_lot::Mutex;

use crate::session::Session;

pub const SERVICE: &str = "app.encore.desktop";
pub const SESSION_ACCOUNT: &str = "youtube-music-session";
/// Reserved for a later Last.fm integration.
pub const LASTFM_ACCOUNT: &str = "lastfm-session";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("The Keychain is locked or refused access.")]
    Denied,
    #[error("Keychain error: {0}")]
    Other(String),
}

pub trait SecretStore: Send + Sync {
    fn load(&self) -> Result<Option<String>, StoreError>;
    fn save(&self, value: &str) -> Result<(), StoreError>;
    fn delete(&self) -> Result<(), StoreError>;
}

pub struct KeychainStore {
    account: String,
}

impl KeychainStore {
    pub fn session() -> Self {
        KeychainStore { account: SESSION_ACCOUNT.into() }
    }
    pub fn named(account: &str) -> Self {
        KeychainStore { account: account.into() }
    }
    fn entry(&self) -> Result<keyring::Entry, StoreError> {
        keyring::Entry::new(SERVICE, &self.account).map_err(|e| StoreError::Other(e.to_string()))
    }
}

fn map(e: keyring::Error) -> StoreError {
    match e {
        keyring::Error::NoStorageAccess(_) => StoreError::Denied,
        other => StoreError::Other(encore_core::redact::redact(&other.to_string())),
    }
}

impl SecretStore for KeychainStore {
    fn load(&self) -> Result<Option<String>, StoreError> {
        match self.entry()?.get_password() {
            Ok(s) => Ok(Some(s)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(map(e)),
        }
    }
    fn save(&self, value: &str) -> Result<(), StoreError> {
        let e = self.entry()?;
        e.set_password(value).map_err(map)?;
        // Read back so a silently failing store is noticed.
        match e.get_password() {
            Ok(v) if v == value => Ok(()),
            Ok(_) => Err(StoreError::Other("the Keychain kept a different value".into())),
            Err(e) => Err(map(e)),
        }
    }
    fn delete(&self) -> Result<(), StoreError> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(map(e)),
        }
    }
}

/// In-memory store for tests and `--ephemeral` sessions.
#[derive(Default)]
pub struct MemoryStore(Mutex<Option<String>>);

impl SecretStore for MemoryStore {
    fn load(&self) -> Result<Option<String>, StoreError> {
        Ok(self.0.lock().clone())
    }
    fn save(&self, value: &str) -> Result<(), StoreError> {
        *self.0.lock() = Some(value.to_string());
        Ok(())
    }
    fn delete(&self) -> Result<(), StoreError> {
        *self.0.lock() = None;
        Ok(())
    }
}

pub fn load_session(store: &dyn SecretStore) -> Result<Option<Session>, StoreError> {
    Ok(store.load()?.and_then(|s| serde_json::from_str(&s).ok()))
}

pub fn save_session(store: &dyn SecretStore, s: &Session) -> Result<(), StoreError> {
    let v = serde_json::to_string(s).map_err(|e| StoreError::Other(e.to_string()))?;
    store.save(&v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jar::CookieJar;
    use encore_core::service::CookieRecord;

    #[test]
    fn session_roundtrip_in_memory() {
        let st = MemoryStore::default();
        let jar = CookieJar::new(vec![CookieRecord { name: "SAPISID".into(), value: "x".into(), domain: ".youtube.com".into(), path: "/".into(), secure: true, http_only: false, expires: None }]);
        let s = Session::new(jar, "edge:Default");
        save_session(&st, &s).unwrap();
        let back = load_session(&st).unwrap().unwrap();
        assert_eq!(back, s);
        st.delete().unwrap();
        assert!(load_session(&st).unwrap().is_none());
    }
}
