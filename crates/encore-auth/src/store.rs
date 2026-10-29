//! Session persistence (D9). The session (structured cookie jar plus channel
//! selection) lives in the OS credential store under service
//! `app.encore.desktop`:
//!
//! * macOS: one Keychain item.
//! * Linux: one Secret Service item (GNOME Keyring, KWallet). If no Secret
//!   Service is running, saving fails with a clear message; there is no
//!   plaintext fallback.
//! * Windows: Credential Manager caps a secret at 2,560 bytes, which a session
//!   outgrows. A random 256-bit key is kept there instead, and the session is
//!   sealed with it (AES-256-GCM) into `session.enc` in the app's local data
//!   folder. The file is useless without the credential.

use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use parking_lot::Mutex;

use crate::session::Session;

pub const SERVICE: &str = "app.encore.desktop";
pub const SESSION_ACCOUNT: &str = "youtube-music-session";
/// Reserved for a later Last.fm integration.
pub const LASTFM_ACCOUNT: &str = "lastfm-session";

/// What the OS calls its credential store, for messages.
pub const STORE_NAME: &str = if cfg!(target_os = "macos") {
    "the Keychain"
} else if cfg!(windows) {
    "Windows Credential Manager"
} else {
    "the Secret Service"
};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("{} is locked or refused access.", cap(STORE_NAME))]
    Denied,
    #[error("No Secret Service is running, so Encore can't keep you signed in safely. Install or unlock GNOME Keyring or KWallet, then sign in again.")]
    Unavailable,
    #[error("{}", other_message(.0))]
    Other(String),
}

fn other_message(m: &str) -> String {
    format!("{} error: {m}", cap(STORE_NAME))
}

fn cap(s: &str) -> String {
    let s = s.strip_prefix("the ").unwrap_or(s);
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

pub trait SecretStore: Send + Sync {
    fn load(&self) -> Result<Option<String>, StoreError>;
    fn save(&self, value: &str) -> Result<(), StoreError>;
    fn delete(&self) -> Result<(), StoreError>;
}

/// The session store for this OS (see the module docs).
pub fn session_store() -> std::sync::Arc<dyn SecretStore> {
    #[cfg(windows)]
    {
        let dir = dirs::data_local_dir().unwrap_or_else(std::env::temp_dir).join(SERVICE);
        std::sync::Arc::new(SealedStore::new(OsStore::named(&format!("{SESSION_ACCOUNT}-key")), dir.join("session.enc"), SESSION_ACCOUNT))
    }
    #[cfg(not(windows))]
    {
        std::sync::Arc::new(OsStore::session())
    }
}

/// One item in the OS credential store, through `keyring`.
pub struct OsStore {
    account: String,
}

impl OsStore {
    pub fn session() -> Self {
        OsStore { account: SESSION_ACCOUNT.into() }
    }
    pub fn named(account: &str) -> Self {
        OsStore { account: account.into() }
    }
    fn entry(&self) -> Result<keyring::Entry, StoreError> {
        keyring::Entry::new(SERVICE, &self.account).map_err(map)
    }
}

fn map(e: keyring::Error) -> StoreError {
    let text = e.to_string();
    // Linux: no Secret Service on the session bus (or no session bus at all).
    if cfg!(all(unix, not(target_os = "macos"))) {
        let t = text.to_ascii_lowercase();
        if ["serviceunknown", "org.freedesktop.secrets", "was not provided", "failed to connect", "noreply", "dbus_session_bus_address", "autolaunch", "not activatable"].iter().any(|k| t.contains(k)) {
            return StoreError::Unavailable;
        }
    }
    match e {
        keyring::Error::NoStorageAccess(_) => StoreError::Denied,
        _ => StoreError::Other(encore_core::redact::redact(&text)),
    }
}

impl SecretStore for OsStore {
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
            Ok(_) => Err(StoreError::Other("the store kept a different value".into())),
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

const SEAL_MAGIC: &[u8; 4] = b"ENC1";

/// A value sealed with AES-256-GCM into a file, with the key in another
/// `SecretStore` (Windows: Credential Manager). The account name is bound as
/// associated data so a file can't be swapped between slots.
pub struct SealedStore<K: SecretStore> {
    key: K,
    path: PathBuf,
    aad: String,
}

impl<K: SecretStore> SealedStore<K> {
    pub fn new(key: K, path: PathBuf, aad: &str) -> Self {
        SealedStore { key, path, aad: aad.into() }
    }

    fn cipher(hex_key: &str) -> Option<Aes256Gcm> {
        let k = hex::decode(hex_key.trim()).ok()?;
        Aes256Gcm::new_from_slice(&k).ok()
    }

    pub fn seal(&self, hex_key: &str, value: &str) -> Result<Vec<u8>, StoreError> {
        let c = Self::cipher(hex_key).ok_or_else(|| StoreError::Other("bad session key".into()))?;
        let mut nonce = [0u8; 12];
        getrandom::fill(&mut nonce).map_err(|e| StoreError::Other(e.to_string()))?;
        let ct = c.encrypt(&Nonce::from(nonce), Payload { msg: value.as_bytes(), aad: self.aad.as_bytes() }).map_err(|_| StoreError::Other("could not seal the session".into()))?;
        let mut out = Vec::with_capacity(4 + 12 + ct.len());
        out.extend_from_slice(SEAL_MAGIC);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        Ok(out)
    }

    pub fn open(&self, hex_key: &str, data: &[u8]) -> Option<String> {
        let c = Self::cipher(hex_key)?;
        let rest = data.strip_prefix(SEAL_MAGIC)?;
        if rest.len() < 12 + 16 {
            return None;
        }
        let (nonce, ct) = rest.split_at(12);
        let nonce: [u8; 12] = nonce.try_into().ok()?;
        let pt = c.decrypt(&Nonce::from(nonce), Payload { msg: ct, aad: self.aad.as_bytes() }).ok()?;
        String::from_utf8(pt).ok()
    }
}

fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path)
}

impl<K: SecretStore> SecretStore for SealedStore<K> {
    fn load(&self) -> Result<Option<String>, StoreError> {
        let Some(k) = self.key.load()? else { return Ok(None) };
        let data = match std::fs::read(&self.path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(StoreError::Other(format!("cannot read the saved session: {}", e.kind()))),
        };
        // A file that doesn't open with this key is treated as no session.
        Ok(self.open(&k, &data))
    }
    fn save(&self, value: &str) -> Result<(), StoreError> {
        let k = match self.key.load()? {
            Some(k) if Self::cipher(&k).is_some() => k,
            _ => {
                let mut raw = [0u8; 32];
                getrandom::fill(&mut raw).map_err(|e| StoreError::Other(e.to_string()))?;
                let k = hex::encode(raw);
                self.key.save(&k)?;
                k
            }
        };
        let sealed = self.seal(&k, value)?;
        write_private(&self.path, &sealed).map_err(|e| StoreError::Other(format!("cannot write the saved session: {}", e.kind())))?;
        // Read back so a silently failing store is noticed.
        match self.load()? {
            Some(v) if v == value => Ok(()),
            _ => Err(StoreError::Other("the saved session did not read back".into())),
        }
    }
    fn delete(&self) -> Result<(), StoreError> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(StoreError::Other(format!("cannot remove the saved session: {}", e.kind()))),
        }
        self.key.delete()
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

impl<T: SecretStore + ?Sized> SecretStore for std::sync::Arc<T> {
    fn load(&self) -> Result<Option<String>, StoreError> {
        (**self).load()
    }
    fn save(&self, value: &str) -> Result<(), StoreError> {
        (**self).save(value)
    }
    fn delete(&self) -> Result<(), StoreError> {
        (**self).delete()
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
    use std::sync::Arc;

    fn big_session() -> Session {
        // Larger than Credential Manager's 2,560-byte limit.
        let cookies = (0..40).map(|i| CookieRecord { name: format!("C{i}"), value: "x".repeat(120), domain: ".youtube.com".into(), path: "/".into(), secure: true, http_only: false, expires: None }).collect();
        Session::new(CookieJar::new(cookies), "edge:Default")
    }

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

    #[test]
    fn sealed_store_roundtrip_and_tamper() {
        let dir = std::env::temp_dir().join(format!("encore-sealed-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let key = Arc::new(MemoryStore::default());
        let st = SealedStore::new(key.clone(), dir.join("session.enc"), SESSION_ACCOUNT);
        assert!(load_session(&st).unwrap().is_none());
        let s = big_session();
        save_session(&st, &s).unwrap();
        // Only a 64-character hex key sits in the credential store.
        assert_eq!(key.load().unwrap().unwrap().len(), 64);
        let raw = std::fs::read(dir.join("session.enc")).unwrap();
        assert!(raw.len() > 2560);
        assert!(!raw.windows(7).any(|w| w == b"SAPISID" || w == b"xxxxxxx"), "sealed file must not contain plaintext");
        assert_eq!(load_session(&st).unwrap().unwrap(), s);
        // A second save reuses the key and still reads back.
        save_session(&st, &s).unwrap();
        assert_eq!(load_session(&st).unwrap().unwrap(), s);
        // Flipped byte: rejected, reads as no session.
        let mut bad = raw.clone();
        let n = bad.len() - 5;
        bad[n] ^= 1;
        std::fs::write(dir.join("session.enc"), &bad).unwrap();
        assert!(load_session(&st).unwrap().is_none());
        // Another slot's associated data: rejected.
        std::fs::write(dir.join("session.enc"), &raw).unwrap();
        let other = SealedStore::new(key.clone(), dir.join("session.enc"), LASTFM_ACCOUNT);
        assert!(other.load().unwrap().is_none());
        // Delete removes both the file and the key.
        st.delete().unwrap();
        assert!(!dir.join("session.enc").exists());
        assert!(key.load().unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sealed_store_without_key_is_empty() {
        let dir = std::env::temp_dir().join(format!("encore-sealed-nokey-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("session.enc"), b"ENC1garbagegarbagegarbagegarbage").unwrap();
        let st = SealedStore::new(MemoryStore::default(), dir.join("session.enc"), SESSION_ACCOUNT);
        assert!(st.load().unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Live round trip through this OS's real store. Opt in with
    /// `ENCORE_TEST_OS_STORE=1` (it writes and deletes a test item).
    #[test]
    fn os_store_live_roundtrip() {
        if std::env::var_os("ENCORE_TEST_OS_STORE").is_none() {
            return;
        }
        let account = format!("encore-test-{}", std::process::id());
        #[cfg(windows)]
        let st: Box<dyn SecretStore> = Box::new(SealedStore::new(OsStore::named(&format!("{account}-key")), std::env::temp_dir().join(format!("{account}.enc")), &account));
        #[cfg(not(windows))]
        let st: Box<dyn SecretStore> = Box::new(OsStore::named(&account));
        let v = serde_json::to_string(&big_session()).unwrap();
        match st.save(&v) {
            Ok(()) => {}
            Err(e) => panic!("save failed: {e}"),
        }
        assert_eq!(st.load().unwrap().as_deref(), Some(v.as_str()));
        st.delete().unwrap();
        assert!(st.load().unwrap().is_none());
    }
}
