//! Browser cookie import (D8). Selected profile only; readers are
//! reimplemented here (no `rookie`). The cookie DB and its WAL are copied to a
//! private temp dir and read with rusqlite. For DB schema ≥ 24 every decrypted
//! value starts with SHA-256(host_key), which is validated and stripped.
//!
//! Chromium keys per OS:
//! * macOS: `v10`, AES-128-CBC. The "<Browser> Safe Storage" password comes
//!   from `/usr/bin/security find-generic-password -w` (one "Always Allow"
//!   survives app rebuilds because the ACL entry is `security`, not Encore).
//!   PBKDF2-SHA1, 1003 rounds.
//! * Linux: AES-128-CBC with PBKDF2-SHA1, 1 round. `v10` uses the fixed
//!   password "peanuts"; `v11` uses the password from the Secret Service
//!   (GNOME Keyring) or KWallet.
//! * Windows: `v10` is AES-256-GCM with a key from `Local State`, unwrapped
//!   with DPAPI. `v20` is app-bound encryption (Chrome 127+, current Edge):
//!   only the browser's elevated service can unwrap it, so Encore says so and
//!   points to in-app sign-in. Chromium also keeps the DB locked while it runs.
//!
//! Firefox stores cookies in plain SQLite on every OS. Safari is macOS only.

use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::time::Instant;
use std::time::Duration;

use aes::cipher::{BlockModeDecrypt, KeyIvInit, block_padding::Pkcs7};
use aes_gcm::aead::{Aead, KeyInit};
use encore_core::model::BrowserProfile;
use encore_core::service::CookieRecord;
use sha2::{Digest, Sha256};

use crate::jar::{CookieJar, keep_domain};

/// The in-app sign-in, for messages.
const USE_SIGNIN: &str = "Use “Sign in” instead.";

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("{0} is not installed or has no profiles")]
    NotFound(String),
    #[error("The Keychain did not release the {0} Safe Storage key (prompt denied or timed out).")]
    KeychainDenied(String),
    #[error("Encore could not get {0}'s cookie key from the Secret Service or KWallet. Unlock your keyring and try again, or use “Sign in”.")]
    KeyUnavailable(String),
    #[error("Windows would not unlock {0}'s cookie key. {USE_SIGNIN}")]
    DpapiFailed(String),
    #[error("macOS did not let Encore read that browser's data. Allow Encore under System Settings → Privacy & Security → Full Disk Access (or “App Data”), then try again.")]
    NeedsFullDiskAccess,
    #[error("{0} is running and keeps its cookies locked. Quit it completely (including background apps in the system tray) and try again, or use “Sign in”.")]
    BrowserRunning(String),
    #[error("{0} protects its cookies with app-bound encryption, which other apps can't read on Windows. {USE_SIGNIN}")]
    AppBound(String),
    #[error("Could not read the browser's cookie database: {0}")]
    Database(String),
    #[error("No YouTube login was found in that browser profile. Sign in to music.youtube.com there first.")]
    NoLogin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Browser {
    Edge,
    Chrome,
    Brave,
    Chromium,
    Firefox,
    Safari,
}

impl Browser {
    /// The browsers Encore can import from on this OS, in order (D8).
    pub fn all() -> Vec<Browser> {
        if cfg!(target_os = "macos") {
            vec![Browser::Edge, Browser::Chrome, Browser::Brave, Browser::Firefox, Browser::Safari]
        } else if cfg!(windows) {
            vec![Browser::Edge, Browser::Chrome, Browser::Brave, Browser::Firefox]
        } else {
            vec![Browser::Edge, Browser::Chrome, Browser::Brave, Browser::Chromium, Browser::Firefox]
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Browser::Edge => "edge",
            Browser::Chrome => "chrome",
            Browser::Brave => "brave",
            Browser::Chromium => "chromium",
            Browser::Firefox => "firefox",
            Browser::Safari => "safari",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Browser::Edge => "Microsoft Edge",
            Browser::Chrome => "Google Chrome",
            Browser::Brave => "Brave",
            Browser::Chromium => "Chromium",
            Browser::Firefox => "Firefox",
            Browser::Safari => "Safari",
        }
    }
    pub fn from_key(k: &str) -> Option<Browser> {
        Browser::all().into_iter().find(|b| b.key() == k)
    }
    fn is_chromium(self) -> bool {
        matches!(self, Browser::Edge | Browser::Chrome | Browser::Brave | Browser::Chromium)
    }
    /// The browser's "User Data" folder (holds `Local State` and profiles).
    fn chromium_dir(self) -> Option<PathBuf> {
        if !self.is_chromium() {
            return None;
        }
        #[cfg(target_os = "macos")]
        {
            let base = dirs::home_dir()?.join("Library/Application Support");
            Some(match self {
                Browser::Edge => base.join("Microsoft Edge"),
                Browser::Chrome => base.join("Google/Chrome"),
                Browser::Brave => base.join("BraveSoftware/Brave-Browser"),
                _ => base.join("Chromium"),
            })
        }
        #[cfg(windows)]
        {
            let base = dirs::data_local_dir()?;
            Some(match self {
                Browser::Edge => base.join("Microsoft/Edge/User Data"),
                Browser::Chrome => base.join("Google/Chrome/User Data"),
                Browser::Brave => base.join("BraveSoftware/Brave-Browser/User Data"),
                _ => base.join("Chromium/User Data"),
            })
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let base = dirs::config_dir()?;
            Some(match self {
                Browser::Edge => base.join("microsoft-edge"),
                Browser::Chrome => base.join("google-chrome"),
                Browser::Brave => base.join("BraveSoftware/Brave-Browser"),
                _ => base.join("chromium"),
            })
        }
    }
    /// Keychain service/account, Secret Service label, KWallet key.
    #[allow(dead_code)]
    fn safe_storage(self) -> (&'static str, &'static str) {
        match self {
            Browser::Edge => ("Microsoft Edge Safe Storage", "Microsoft Edge"),
            Browser::Chrome => ("Chrome Safe Storage", "Chrome"),
            Browser::Brave => ("Brave Safe Storage", "Brave"),
            _ => ("Chromium Safe Storage", "Chromium"),
        }
    }
    /// The `application` attribute Chromium sets on its libsecret item.
    #[allow(dead_code)]
    fn libsecret_app(self) -> &'static str {
        match self {
            Browser::Edge => "microsoft-edge",
            Browser::Chrome => "chrome",
            Browser::Brave => "brave",
            _ => "chromium",
        }
    }
}

/// The cookie DB of a Chromium profile (newer builds keep it under `Network`).
fn chromium_cookie_db(profile_dir: &Path) -> PathBuf {
    let n = profile_dir.join("Network").join("Cookies");
    if n.is_file() { n } else { profile_dir.join("Cookies") }
}

/// List profiles (metadata only; no cookie store is opened).
pub fn list_profiles() -> Vec<BrowserProfile> {
    let mut out = Vec::new();
    for b in Browser::all() {
        match b {
            Browser::Firefox => {
                for p in firefox_profiles() {
                    out.push(BrowserProfile { id: format!("firefox:{}", p.id), browser: b.name().into(), name: p.name });
                }
            }
            Browser::Safari => {
                if safari_cookie_path().is_some_and(|p| p.parent().is_some_and(|d| d.exists())) {
                    out.push(BrowserProfile { id: "safari:default".into(), browser: b.name().into(), name: "Default".into() });
                }
            }
            _ => {
                let Some(dir) = b.chromium_dir() else { continue };
                let ls = match std::fs::read_to_string(dir.join("Local State")) {
                    Ok(s) => s,
                    // Installed, but macOS privacy settings block reading it:
                    // still offer the default profile so the import can explain.
                    Err(e) if cfg!(target_os = "macos") && (e.kind() == std::io::ErrorKind::PermissionDenied || (dir.exists() && e.kind() != std::io::ErrorKind::NotFound)) => {
                        out.push(BrowserProfile { id: format!("{}:Default", b.key()), browser: b.name().into(), name: "Default (macOS may ask for access)".into() });
                        continue;
                    }
                    Err(_) => continue,
                };
                let Ok(v) = serde_json::from_str::<serde_json::Value>(&ls) else { continue };
                if let Some(m) = v.pointer("/profile/info_cache").and_then(|m| m.as_object()) {
                    let mut keys: Vec<&String> = m.keys().collect();
                    keys.sort();
                    for k in keys {
                        if chromium_cookie_db(&dir.join(k)).is_file() {
                            let name = m[k].get("name").and_then(|n| n.as_str()).unwrap_or(k).to_string();
                            out.push(BrowserProfile { id: format!("{}:{k}", b.key()), browser: b.name().into(), name });
                        }
                    }
                }
            }
        }
    }
    out
}

/// Import the cookies Encore needs from one profile ("edge:Default").
pub fn import(profile_id: &str, keychain_timeout: Duration) -> Result<CookieJar, ImportError> {
    let (bk, prof) = profile_id.split_once(':').ok_or_else(|| ImportError::NotFound(profile_id.into()))?;
    let b = Browser::from_key(bk).ok_or_else(|| ImportError::NotFound(bk.into()))?;
    let cookies = match b {
        Browser::Firefox => {
            let p = firefox_profiles().into_iter().find(|p| p.id == prof).ok_or_else(|| ImportError::NotFound("Firefox profile".into()))?;
            read_firefox(&p.dir.join("cookies.sqlite"))?
        }
        Browser::Safari => {
            let p = safari_cookie_path().ok_or(ImportError::NotFound("Safari".into()))?;
            let data = std::fs::read(&p).map_err(|e| if e.kind() == std::io::ErrorKind::PermissionDenied { ImportError::NeedsFullDiskAccess } else { ImportError::NotFound("Safari cookies".into()) })?;
            parse_binarycookies(&data).map_err(ImportError::Database)?
        }
        _ => {
            let dir = b.chromium_dir().ok_or_else(|| ImportError::NotFound(b.name().into()))?;
            import_chromium(b, &dir, prof, keychain_timeout)?
        }
    };
    let mut jar = CookieJar::new(cookies);
    jar.retain_needed();
    if !jar.has_login() {
        return Err(ImportError::NoLogin);
    }
    Ok(jar)
}

/// Read one Chromium profile under `user_data` (the folder with `Local State`).
pub fn import_chromium(b: Browser, user_data: &Path, profile: &str, timeout: Duration) -> Result<Vec<CookieRecord>, ImportError> {
    if profile.is_empty() || profile.contains('/') || profile.contains('\\') || profile.contains("..") {
        return Err(ImportError::NotFound(profile.into()));
    }
    let db = chromium_cookie_db(&user_data.join(profile));
    match std::fs::metadata(&db) {
        Ok(m) if m.is_file() => {}
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && cfg!(target_os = "macos") => return Err(ImportError::NeedsFullDiskAccess),
        _ => return Err(ImportError::NotFound(format!("{} profile {profile}", b.name()))),
    }
    // Copy first: on Windows a running browser keeps the DB locked, and that
    // should be reported before any key prompt.
    let (_guard, copy) = copy_db(&db, b.name())?;
    let keys = chromium_keys(b, user_data, timeout)?;
    let (cookies, stats) = read_chromium_copy(&copy, &keys)?;
    if stats.app_bound > 0 && !CookieJar::new(cookies.clone()).has_login() {
        return Err(ImportError::AppBound(b.name().into()));
    }
    if stats.no_key > 0 && !CookieJar::new(cookies.clone()).has_login() {
        return Err(ImportError::KeyUnavailable(b.name().into()));
    }
    Ok(cookies)
}

/// The keys one Chromium profile needs on this OS.
pub enum ChromiumKeys {
    /// AES-128-CBC. macOS: `v10` only. Linux: `v10` ("peanuts") and `v11`.
    Cbc { v10: [u8; 16], v11: Option<[u8; 16]> },
    /// Windows: AES-256-GCM for `v10`/`v11`.
    Gcm([u8; 32]),
}

#[allow(unused_variables)]
fn chromium_keys(b: Browser, user_data: &Path, timeout: Duration) -> Result<ChromiumKeys, ImportError> {
    #[cfg(target_os = "macos")]
    {
        let (svc, acct) = b.safe_storage();
        let password = safe_storage_password(svc, acct, timeout).ok_or_else(|| ImportError::KeychainDenied(b.name().into()))?;
        Ok(ChromiumKeys::Cbc { v10: derive_key(password.as_bytes(), 1003), v11: None })
    }
    #[cfg(windows)]
    {
        let ls = std::fs::read_to_string(user_data.join("Local State")).map_err(|_| ImportError::NotFound(format!("{} Local State", b.name())))?;
        let v: serde_json::Value = serde_json::from_str(&ls).map_err(|e| ImportError::Database(e.to_string()))?;
        let enc = v.pointer("/os_crypt/encrypted_key").and_then(|k| k.as_str()).ok_or_else(|| ImportError::DpapiFailed(b.name().into()))?;
        use base64::Engine;
        let raw = base64::engine::general_purpose::STANDARD.decode(enc).map_err(|_| ImportError::DpapiFailed(b.name().into()))?;
        let wrapped = raw.strip_prefix(b"DPAPI").ok_or_else(|| ImportError::DpapiFailed(b.name().into()))?;
        let key = dpapi::unprotect(wrapped).ok_or_else(|| ImportError::DpapiFailed(b.name().into()))?;
        let key: [u8; 32] = key.try_into().map_err(|_| ImportError::DpapiFailed(b.name().into()))?;
        Ok(ChromiumKeys::Gcm(key))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let v11 = linux_keys::password(b).map(|p| derive_key(&p, 1));
        Ok(ChromiumKeys::Cbc { v10: derive_key(b"peanuts", 1), v11 })
    }
}

/// Read the Safe Storage password with `/usr/bin/security`. The child is
/// killed if the user does not answer the Keychain prompt in time.
#[cfg(target_os = "macos")]
fn safe_storage_password(service: &str, account: &str, timeout: Duration) -> Option<String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let mut child = Command::new("/usr/bin/security")
        .args(["find-generic-password", "-w", "-s", service, "-a", account])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let t0 = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                let mut s = String::new();
                child.stdout.take()?.read_to_string(&mut s).ok()?;
                let s = s.trim_end_matches(['\n', '\r']).to_string();
                return (!s.is_empty()).then_some(s);
            }
            Ok(None) if t0.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(_) => return None,
        }
    }
}

/// Linux: the Chromium Safe Storage password from the Secret Service, then
/// KWallet (KDE). Chromium started with `--password-store=basic` has none and
/// writes only `v10` values.
#[cfg(all(unix, not(target_os = "macos")))]
mod linux_keys {
    use super::Browser;
    use std::collections::HashMap;
    use std::time::Duration;

    pub fn password(b: Browser) -> Option<Vec<u8>> {
        secret_service(b).or_else(|| kwallet(b))
    }

    fn secret_service(b: Browser) -> Option<Vec<u8>> {
        use dbus_secret_service::{EncryptionType, SecretService};
        let ss = SecretService::connect(EncryptionType::Dh).ok()?;
        let label = b.safe_storage().0;
        let found = ss.search_items(HashMap::from([("application", b.libsecret_app())])).ok()?;
        let mut items: Vec<_> = found.unlocked.into_iter().chain(found.locked).collect();
        if items.is_empty() {
            // Older Chromium items carry only the label.
            if let Ok(c) = ss.get_default_collection() {
                items = c.get_all_items().ok()?.into_iter().filter(|i| i.get_label().ok().as_deref() == Some(label)).collect();
            }
        }
        for it in items {
            if it.ensure_unlocked().is_err() {
                continue;
            }
            if let Ok(s) = it.get_secret() {
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }
        None
    }

    fn kwallet(b: Browser) -> Option<Vec<u8>> {
        use dbus::blocking::Connection;
        let (key, app) = b.safe_storage();
        let folder = format!("{app} Keys");
        let conn = Connection::new_session().ok()?;
        for (svc, path) in [("org.kde.kwalletd6", "/modules/kwalletd6"), ("org.kde.kwalletd5", "/modules/kwalletd5")] {
            let p = conn.with_proxy(svc, path, Duration::from_secs(30));
            let Ok((wallet,)) = p.method_call::<(String,), _, _, _>("org.kde.KWallet", "networkWallet", ()) else { continue };
            let Ok((handle,)) = p.method_call::<(i32,), _, _, _>("org.kde.KWallet", "open", (wallet, 0i64, "Encore")) else { continue };
            if handle < 0 {
                continue;
            }
            let r = p.method_call::<(String,), _, _, _>("org.kde.KWallet", "readPassword", (handle, folder.as_str(), key, "Encore"));
            let _ = p.method_call::<(i32,), _, _, _>("org.kde.KWallet", "close", (handle, false, "Encore"));
            if let Ok((pw,)) = r {
                if !pw.is_empty() {
                    return Some(pw.into_bytes());
                }
            }
        }
        None
    }
}

/// Windows DPAPI (current user).
#[cfg(windows)]
pub mod dpapi {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptProtectData, CryptUnprotectData};

    fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        let v = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
        unsafe { LocalFree(out.pbData as _) };
        v
    }

    pub fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        let ok = unsafe { CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0, &mut out) };
        (ok != 0).then(|| take(out))
    }

    /// Used by tests to build synthetic `Local State` files.
    pub fn protect(data: &[u8]) -> Option<Vec<u8>> {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        let ok = unsafe { CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0, &mut out) };
        (ok != 0).then(|| take(out))
    }
}

/// PBKDF2-SHA1 with Chromium's salt (macOS: 1003 rounds; Linux: 1).
pub fn derive_key(password: &[u8], rounds: u32) -> [u8; 16] {
    let mut key = [0u8; 16];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password, b"saltysalt", rounds, &mut key);
    key
}

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

#[derive(Debug, PartialEq, Eq)]
pub enum Decrypted {
    Value(String),
    /// `v20`: app-bound (Windows).
    AppBound,
    /// `v11` without a keyring key (Linux).
    NoKey,
    Invalid,
}

/// Validate and strip the SHA-256(host_key) binding (schema ≥ 24).
fn unbind(pt: &[u8], host_key: &str, db_version: i64) -> Option<String> {
    let pt = if db_version >= 24 {
        if pt.len() < 32 || pt[..32] != Sha256::digest(host_key.as_bytes())[..] {
            return None;
        }
        &pt[32..]
    } else {
        pt
    };
    String::from_utf8(pt.to_vec()).ok()
}

fn cbc_decrypt(body: &[u8], key: &[u8; 16]) -> Option<Vec<u8>> {
    if body.is_empty() || body.len() % 16 != 0 {
        return None;
    }
    let iv = [b' '; 16];
    let mut buf = body.to_vec();
    Aes128CbcDec::new(key.into(), &iv.into()).decrypt_padded::<Pkcs7>(&mut buf).ok().map(|p| p.to_vec())
}

fn gcm_decrypt(body: &[u8], key: &[u8; 32]) -> Option<Vec<u8>> {
    if body.len() < 12 + 16 {
        return None;
    }
    let (nonce, ct) = body.split_at(12);
    let nonce: [u8; 12] = nonce.try_into().ok()?;
    aes_gcm::Aes256Gcm::new_from_slice(key).ok()?.decrypt(&aes_gcm::Nonce::from(nonce), ct).ok()
}

/// Decrypt one Chromium cookie value.
pub fn decrypt_value(enc: &[u8], keys: &ChromiumKeys, host_key: &str, db_version: i64) -> Decrypted {
    let (prefix, body) = if enc.len() >= 3 { enc.split_at(3) } else { (enc, &[][..]) };
    let pt = match (keys, prefix) {
        (_, b"v20") => return Decrypted::AppBound,
        (ChromiumKeys::Cbc { v10, .. }, b"v10") => cbc_decrypt(body, v10),
        (ChromiumKeys::Cbc { v11: Some(k), .. }, b"v11") => cbc_decrypt(body, k),
        (ChromiumKeys::Cbc { v11: None, .. }, b"v11") => return Decrypted::NoKey,
        (ChromiumKeys::Gcm(k), b"v10" | b"v11") => gcm_decrypt(body, k),
        // Values from before Chrome 80 on Windows are bare DPAPI blobs.
        #[cfg(windows)]
        (ChromiumKeys::Gcm(_), _) => dpapi::unprotect(enc),
        _ => None,
    };
    match pt.and_then(|p| unbind(&p, host_key, db_version)) {
        Some(v) => Decrypted::Value(v),
        None => Decrypted::Invalid,
    }
}

struct TempCopy(PathBuf);
impl Drop for TempCopy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Copy a SQLite DB with its WAL/SHM to a private temp dir (the browser keeps
/// the original locked while running).
fn copy_db(db: &Path, browser: &str) -> Result<(TempCopy, PathBuf), ImportError> {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("encore-import-{}-{}-{n}", std::process::id(), encore_core::store::now()));
    std::fs::create_dir_all(&dir).map_err(|e| ImportError::Database(e.kind().to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    let guard = TempCopy(dir.clone());
    let name = db.file_name().unwrap();
    let dst = dir.join(name);
    std::fs::copy(db, &dst).map_err(|e| {
        // ERROR_SHARING_VIOLATION / ERROR_LOCK_VIOLATION: the browser is running.
        if cfg!(windows) && matches!(e.raw_os_error(), Some(32) | Some(33)) {
            ImportError::BrowserRunning(browser.into())
        } else if e.kind() == std::io::ErrorKind::PermissionDenied && cfg!(target_os = "macos") {
            ImportError::NeedsFullDiskAccess
        } else {
            ImportError::Database(e.kind().to_string())
        }
    })?;
    for ext in ["-wal", "-shm"] {
        let src = PathBuf::from(format!("{}{ext}", db.display()));
        if src.is_file() {
            let _ = std::fs::copy(&src, dir.join(format!("{}{ext}", name.to_string_lossy())));
        }
    }
    Ok((guard, dst))
}

const CHROME_EPOCH_OFFSET: i64 = 11_644_473_600; // seconds 1601→1970

#[derive(Default, Debug)]
pub struct ReadStats {
    pub app_bound: usize,
    pub no_key: usize,
}

pub fn read_chromium(db: &Path, keys: &ChromiumKeys) -> Result<(Vec<CookieRecord>, ReadStats), ImportError> {
    let (_guard, copy) = copy_db(db, "The browser")?;
    read_chromium_copy(&copy, keys)
}

fn read_chromium_copy(copy: &Path, keys: &ChromiumKeys) -> Result<(Vec<CookieRecord>, ReadStats), ImportError> {
    let conn = rusqlite::Connection::open_with_flags(copy, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE).map_err(|e| ImportError::Database(e.to_string()))?;
    let version: i64 = conn.query_row("SELECT value FROM meta WHERE key = 'version'", [], |r| r.get::<_, String>(0)).ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut st = conn
        .prepare("SELECT host_key, name, value, encrypted_value, path, expires_utc, is_secure, is_httponly FROM cookies WHERE host_key LIKE '%youtube.com' OR host_key LIKE '%google.com'")
        .map_err(|e| ImportError::Database(e.to_string()))?;
    let rows = st
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, Vec<u8>>(3)?, r.get::<_, String>(4)?, r.get::<_, i64>(5)?, r.get::<_, i64>(6)?, r.get::<_, i64>(7)?))
        })
        .map_err(|e| ImportError::Database(e.to_string()))?;
    let mut out = Vec::new();
    let mut stats = ReadStats::default();
    for row in rows.flatten() {
        let (host, name, value, enc, path, exp, secure, httponly) = row;
        // Exact domain boundaries, not substring matches (D8).
        if !keep_domain(&host) {
            continue;
        }
        let value = if !value.is_empty() {
            value
        } else {
            match decrypt_value(&enc, keys, &host, version) {
                Decrypted::Value(v) => v,
                Decrypted::AppBound => {
                    stats.app_bound += 1;
                    continue;
                }
                Decrypted::NoKey => {
                    stats.no_key += 1;
                    continue;
                }
                Decrypted::Invalid => continue,
            }
        };
        let expires = (exp > 0).then(|| exp / 1_000_000 - CHROME_EPOCH_OFFSET);
        out.push(CookieRecord { name, value, domain: host, path, secure: secure != 0, http_only: httponly != 0, expires });
    }
    Ok((out, stats))
}

/// Firefox's profile folders on this OS (Linux also checks Snap and Flatpak).
fn firefox_roots() -> Vec<PathBuf> {
    let mut v = Vec::new();
    #[cfg(target_os = "macos")]
    if let Some(h) = dirs::home_dir() {
        v.push(h.join("Library/Application Support/Firefox"));
    }
    #[cfg(windows)]
    if let Some(a) = dirs::data_dir() {
        v.push(a.join("Mozilla/Firefox"));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(h) = dirs::home_dir() {
        v.push(h.join(".mozilla/firefox"));
        v.push(h.join("snap/firefox/common/.mozilla/firefox"));
        v.push(h.join(".var/app/org.mozilla.firefox/.mozilla/firefox"));
    }
    v
}

struct FirefoxProfile {
    id: String,
    name: String,
    dir: PathBuf,
}

fn firefox_profiles() -> Vec<FirefoxProfile> {
    let mut out: Vec<FirefoxProfile> = Vec::new();
    for root in firefox_roots() {
        for p in firefox_profiles_in(&root) {
            if !out.iter().any(|o| o.id == p.id) {
                out.push(p);
            }
        }
    }
    out
}

fn firefox_profiles_in(root: &Path) -> Vec<FirefoxProfile> {
    let Ok(ini) = std::fs::read_to_string(root.join("profiles.ini")) else { return vec![] };
    let mut out = Vec::new();
    let (mut name, mut path, mut relative) = (None::<String>, None::<String>, true);
    let flush = |name: &mut Option<String>, path: &mut Option<String>, relative: &mut bool, out: &mut Vec<FirefoxProfile>| {
        if let Some(p) = path.take() {
            let dir = if *relative { root.join(&p) } else { PathBuf::from(&p) };
            if dir.join("cookies.sqlite").is_file() {
                out.push(FirefoxProfile { id: p.clone(), name: name.take().unwrap_or(p), dir });
            }
        }
        *name = None;
        *relative = true;
    };
    for line in ini.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            flush(&mut name, &mut path, &mut relative, &mut out);
        } else if let Some(v) = l.strip_prefix("Name=") {
            name = Some(v.to_string());
        } else if let Some(v) = l.strip_prefix("Path=") {
            path = Some(v.to_string());
        } else if let Some(v) = l.strip_prefix("IsRelative=") {
            relative = v != "0";
        }
    }
    flush(&mut name, &mut path, &mut relative, &mut out);
    out
}

pub fn read_firefox(db: &Path) -> Result<Vec<CookieRecord>, ImportError> {
    let (_guard, copy) = copy_db(db, "Firefox")?;
    let conn = rusqlite::Connection::open(&copy).map_err(|e| ImportError::Database(e.to_string()))?;
    let mut st = conn.prepare("SELECT host, name, value, path, expiry, isSecure, isHttpOnly FROM moz_cookies").map_err(|e| ImportError::Database(e.to_string()))?;
    let rows = st
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, i64>(4)?, r.get::<_, i64>(5)?, r.get::<_, i64>(6)?)))
        .map_err(|e| ImportError::Database(e.to_string()))?;
    Ok(rows
        .flatten()
        .filter(|r| keep_domain(&r.0))
        .map(|(host, name, value, path, exp, sec, ho)| {
            // Firefox stores expiry in seconds (older) or milliseconds (newer).
            let exp = if exp > 100_000_000_000 { exp / 1000 } else { exp };
            CookieRecord { name, value, domain: host, path, secure: sec != 0, http_only: ho != 0, expires: (exp > 0).then_some(exp) }
        })
        .collect())
}

fn safari_cookie_path() -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let h = dirs::home_dir()?;
    let a = h.join("Library/Containers/com.apple.Safari/Data/Library/Cookies/Cookies.binarycookies");
    if a.parent().is_some_and(|p| p.exists()) { Some(a) } else { Some(h.join("Library/Cookies/Cookies.binarycookies")) }
}

/// Parse Safari's `Cookies.binarycookies`.
pub fn parse_binarycookies(d: &[u8]) -> Result<Vec<CookieRecord>, String> {
    let be32 = |o: usize| -> Option<u32> { d.get(o..o + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]])) };
    if d.get(..4) != Some(b"cook") {
        return Err("not a Safari cookie file".into());
    }
    let n = be32(4).ok_or("truncated")? as usize;
    let sizes: Vec<usize> = (0..n).map(|i| be32(8 + i * 4).map(|v| v as usize)).collect::<Option<_>>().ok_or("truncated")?;
    let mut off = 8 + n * 4;
    let mut out = Vec::new();
    for sz in sizes {
        let page = d.get(off..off + sz).ok_or("truncated page")?;
        off += sz;
        let le32 = |o: usize| -> Option<u32> { page.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])) };
        let count = le32(4).ok_or("bad page")? as usize;
        for i in 0..count {
            let co = le32(8 + i * 4).ok_or("bad page")? as usize;
            let c = page.get(co..).ok_or("bad cookie")?;
            let cle = |o: usize| -> Option<u32> { c.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])) };
            let flags = cle(8).unwrap_or(0);
            let (url_o, name_o, path_o, val_o) = (cle(16).unwrap_or(0) as usize, cle(20).unwrap_or(0) as usize, cle(24).unwrap_or(0) as usize, cle(28).unwrap_or(0) as usize);
            let exp = c.get(40..48).map(|b| f64::from_le_bytes(b.try_into().unwrap())).unwrap_or(0.0);
            let cstr = |o: usize| -> String {
                c.get(o..).map(|s| String::from_utf8_lossy(&s[..s.iter().position(|&b| b == 0).unwrap_or(s.len())]).into_owned()).unwrap_or_default()
            };
            let domain = cstr(url_o);
            if !keep_domain(&domain) {
                continue;
            }
            // Mac absolute time: seconds since 2001-01-01.
            let expires = (exp > 0.0).then(|| exp as i64 + 978_307_200);
            out.push(CookieRecord { name: cstr(name_o), value: cstr(val_o), domain, path: cstr(path_o), secure: flags & 1 != 0, http_only: flags & 4 != 0, expires });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockModeEncrypt;

    type Enc = cbc::Encryptor<aes::Aes128>;

    fn cbc_encrypt(prefix: &[u8], pt: &[u8], key: &[u8; 16]) -> Vec<u8> {
        let iv = [b' '; 16];
        let mut buf = vec![0u8; pt.len() + 16];
        buf[..pt.len()].copy_from_slice(pt);
        let ct = Enc::new(key.into(), &iv.into()).encrypt_padded::<Pkcs7>(&mut buf, pt.len()).unwrap();
        let mut v = prefix.to_vec();
        v.extend_from_slice(ct);
        v
    }

    fn gcm_encrypt(prefix: &[u8], pt: &[u8], key: &[u8; 32]) -> Vec<u8> {
        let nonce = [7u8; 12];
        let ct = aes_gcm::Aes256Gcm::new_from_slice(key).unwrap().encrypt(&aes_gcm::Nonce::from(nonce), pt).unwrap();
        let mut v = prefix.to_vec();
        v.extend_from_slice(&nonce);
        v.extend_from_slice(&ct);
        v
    }

    fn bound(host: &str, v: &str) -> Vec<u8> {
        let mut pt = Sha256::digest(host.as_bytes()).to_vec();
        pt.extend_from_slice(v.as_bytes());
        pt
    }

    fn cbc(key: [u8; 16]) -> ChromiumKeys {
        ChromiumKeys::Cbc { v10: key, v11: None }
    }

    #[test]
    fn pbkdf2_matches_known_values() {
        // hashlib.pbkdf2_hmac('sha1', b'peanuts', b'saltysalt', 1003, 16).hex()
        assert_eq!(hex::encode(derive_key(b"peanuts", 1003)), "d9a09d499b4e1b7461f28e67972c6dbd");
        // Linux v10: hashlib.pbkdf2_hmac('sha1', b'peanuts', b'saltysalt', 1, 16).hex()
        assert_eq!(hex::encode(derive_key(b"peanuts", 1)), "fd621fe5a2b402539dfa147ca9272778");
    }

    #[test]
    fn decrypts_v10_old_schema() {
        let key = derive_key(b"synthetic-password", 1003);
        let enc = cbc_encrypt(b"v10", b"cookie-value-123", &key);
        assert_eq!(decrypt_value(&enc, &cbc(key), ".youtube.com", 23), Decrypted::Value("cookie-value-123".into()));
    }

    #[test]
    fn decrypts_and_validates_host_prefix_schema_24() {
        let key = derive_key(b"synthetic-password", 1003);
        // A value whose first bytes are valid UTF-8 (the heuristic would fail here).
        let enc = cbc_encrypt(b"v10", &bound(".youtube.com", "AAAAvalue"), &key);
        assert_eq!(decrypt_value(&enc, &cbc(key), ".youtube.com", 24), Decrypted::Value("AAAAvalue".into()));
        // Bound to a different host: rejected.
        assert_eq!(decrypt_value(&enc, &cbc(key), ".google.com", 24), Decrypted::Invalid);
        // Wrong key: rejected.
        assert_eq!(decrypt_value(&enc, &cbc(derive_key(b"other", 1003)), ".youtube.com", 24), Decrypted::Invalid);
    }

    #[test]
    fn linux_v10_peanuts_and_v11_keyring() {
        let v10 = derive_key(b"peanuts", 1);
        let v11 = derive_key(b"synthetic-keyring-password", 1);
        let a = cbc_encrypt(b"v10", &bound(".youtube.com", "from-peanuts"), &v10);
        let b = cbc_encrypt(b"v11", &bound(".youtube.com", "from-keyring"), &v11);
        let keys = ChromiumKeys::Cbc { v10, v11: Some(v11) };
        assert_eq!(decrypt_value(&a, &keys, ".youtube.com", 24), Decrypted::Value("from-peanuts".into()));
        assert_eq!(decrypt_value(&b, &keys, ".youtube.com", 24), Decrypted::Value("from-keyring".into()));
        // No keyring key: v11 is reported, not silently dropped.
        assert_eq!(decrypt_value(&b, &ChromiumKeys::Cbc { v10, v11: None }, ".youtube.com", 24), Decrypted::NoKey);
    }

    #[test]
    fn windows_gcm_and_app_bound() {
        let key = [3u8; 32];
        let enc = gcm_encrypt(b"v10", &bound(".youtube.com", "gcm-value"), &key);
        assert_eq!(decrypt_value(&enc, &ChromiumKeys::Gcm(key), ".youtube.com", 24), Decrypted::Value("gcm-value".into()));
        assert_eq!(decrypt_value(&enc, &ChromiumKeys::Gcm([4u8; 32]), ".youtube.com", 24), Decrypted::Invalid);
        let mut v20 = b"v20".to_vec();
        v20.extend_from_slice(&[0u8; 60]);
        assert_eq!(decrypt_value(&v20, &ChromiumKeys::Gcm(key), ".youtube.com", 24), Decrypted::AppBound);
    }

    /// A synthetic Chromium "User Data" folder: Local State plus one profile.
    fn synthetic_profile(tag: &str, rows: &[(&str, &str, Vec<u8>)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("encore-chromium-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let prof = dir.join("Default").join("Network");
        std::fs::create_dir_all(&prof).unwrap();
        let c = rusqlite::Connection::open(prof.join("Cookies")).unwrap();
        c.execute_batch(
            "PRAGMA journal_mode=WAL; CREATE TABLE meta(key TEXT, value TEXT); INSERT INTO meta VALUES('version','24');
             CREATE TABLE cookies(host_key TEXT, name TEXT, value TEXT, encrypted_value BLOB, path TEXT, expires_utc INTEGER, is_secure INTEGER, is_httponly INTEGER);",
        )
        .unwrap();
        for (host, name, enc) in rows {
            c.execute("INSERT INTO cookies VALUES (?1, ?2, '', ?3, '/', 13400000000000000, 1, 0)", rusqlite::params![host, name, enc]).unwrap();
        }
        dir
    }

    #[test]
    fn reads_synthetic_chromium_db_with_wal() {
        let key = derive_key(b"synthetic", 1003);
        let rows: Vec<(&str, &str, Vec<u8>)> = [(".youtube.com", "SAPISID"), (".notyoutube.com", "X"), (".google.com", "SID"), ("www.youtube.com.evil.com", "Y")]
            .iter()
            .map(|(h, n)| (*h, *n, cbc_encrypt(b"v10", &bound(h, &format!("v-{n}")), &key)))
            .collect();
        let dir = synthetic_profile("wal", &rows);
        let (got, _) = read_chromium(&dir.join("Default/Network/Cookies"), &cbc(key)).unwrap();
        let names: Vec<&str> = got.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["SAPISID", "SID"]);
        assert_eq!(got[0].value, "v-SAPISID");
        assert!(got[0].expires.unwrap() > 1_700_000_000);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn app_bound_profile_explains_itself() {
        let mut v20 = b"v20".to_vec();
        v20.extend_from_slice(&[9u8; 60]);
        let dir = synthetic_profile("v20", &[(".youtube.com", "SAPISID", v20.clone()), (".youtube.com", "__Secure-3PAPISID", v20)]);
        let (got, stats) = read_chromium(&dir.join("Default/Network/Cookies"), &ChromiumKeys::Gcm([1u8; 32])).unwrap();
        assert!(got.is_empty());
        assert_eq!(stats.app_bound, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Windows end to end: a DPAPI-wrapped key in Local State and GCM values.
    #[cfg(windows)]
    #[test]
    fn windows_synthetic_profile_with_dpapi_key() {
        use base64::Engine;
        let key = [0x5au8; 32];
        let rows: Vec<(&str, &str, Vec<u8>)> = ["SAPISID", "__Secure-3PAPISID", "SID", "HSID", "SSID", "APISID", "__Secure-1PSID", "__Secure-3PSID"]
            .iter()
            .map(|n| (".youtube.com", *n, gcm_encrypt(b"v10", &bound(".youtube.com", &format!("v-{n}")), &key)))
            .collect();
        let dir = synthetic_profile("dpapi", &rows);
        let mut wrapped = b"DPAPI".to_vec();
        wrapped.extend_from_slice(&dpapi::protect(&key).unwrap());
        let ls = serde_json::json!({ "os_crypt": { "encrypted_key": base64::engine::general_purpose::STANDARD.encode(&wrapped) } });
        std::fs::write(dir.join("Local State"), ls.to_string()).unwrap();
        let got = import_chromium(Browser::Edge, &dir, "Default", Duration::from_secs(1)).unwrap();
        assert_eq!(got.len(), rows.len());
        assert!(got.iter().all(|c| c.value == format!("v-{}", c.name)));
        assert!(CookieJar::new(got).has_login());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Windows: a DB held open without sharing, the way a running Chromium
    /// holds it, is reported as "browser running".
    #[cfg(windows)]
    #[test]
    fn windows_locked_db_says_browser_running() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = synthetic_profile("locked", &[]);
        let db = dir.join("Default/Network/Cookies");
        let hold = std::fs::OpenOptions::new().read(true).write(true).share_mode(0).open(&db).unwrap();
        let err = import_chromium(Browser::Edge, &dir, "Default", Duration::from_secs(1)).unwrap_err();
        assert!(matches!(err, ImportError::BrowserRunning(_)), "{err:?}");
        drop(hold);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Linux end to end with v10 ("peanuts") values: no keyring needed.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn linux_synthetic_profile_peanuts() {
        let key = derive_key(b"peanuts", 1);
        let rows: Vec<(&str, &str, Vec<u8>)> = ["SAPISID", "__Secure-3PAPISID", "SID", "HSID", "SSID", "APISID", "__Secure-1PSID", "__Secure-3PSID"]
            .iter()
            .map(|n| (".youtube.com", *n, cbc_encrypt(b"v10", &bound(".youtube.com", &format!("v-{n}")), &key)))
            .collect();
        let dir = synthetic_profile("peanuts", &rows);
        let got = import_chromium(Browser::Chromium, &dir, "Default", Duration::from_secs(1)).unwrap();
        assert_eq!(got.len(), rows.len());
        assert!(CookieJar::new(got).has_login());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Linux with a v11 key in a live Secret Service. Opt in with
    /// `ENCORE_TEST_OS_STORE=1`; creates and deletes a "Chromium Safe Storage" item.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn linux_v11_key_from_secret_service() {
        if std::env::var_os("ENCORE_TEST_OS_STORE").is_none() {
            return;
        }
        use dbus_secret_service::{EncryptionType, SecretService};
        use std::collections::HashMap;
        let ss = SecretService::connect(EncryptionType::Dh).expect("secret service");
        let col = ss.get_default_collection().expect("default collection");
        let _ = col.ensure_unlocked();
        let item = col.create_item("Chromium Safe Storage", HashMap::from([("application", "chromium")]), b"synthetic-v11-password", true, "text/plain").expect("create item");
        let key = derive_key(b"synthetic-v11-password", 1);
        let rows: Vec<(&str, &str, Vec<u8>)> = ["SAPISID", "__Secure-3PAPISID", "SID", "HSID", "SSID", "APISID", "__Secure-1PSID", "__Secure-3PSID"]
            .iter()
            .map(|n| (".youtube.com", *n, cbc_encrypt(b"v11", &bound(".youtube.com", &format!("v-{n}")), &key)))
            .collect();
        let dir = synthetic_profile("v11", &rows);
        let got = import_chromium(Browser::Chromium, &dir, "Default", Duration::from_secs(1));
        let _ = item.delete();
        let got = got.unwrap();
        assert_eq!(got.len(), rows.len());
        assert!(got.iter().all(|c| c.value == format!("v-{}", c.name)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_synthetic_firefox_profile() {
        let root = std::env::temp_dir().join(format!("encore-firefox-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Profiles/abc.default-release")).unwrap();
        std::fs::write(root.join("profiles.ini"), "[Profile0]\nName=default-release\nIsRelative=1\nPath=Profiles/abc.default-release\n\n[General]\nVersion=2\n").unwrap();
        let db = root.join("Profiles/abc.default-release/cookies.sqlite");
        let c = rusqlite::Connection::open(&db).unwrap();
        c.execute_batch("CREATE TABLE moz_cookies(host TEXT, name TEXT, value TEXT, path TEXT, expiry INTEGER, isSecure INTEGER, isHttpOnly INTEGER);
            INSERT INTO moz_cookies VALUES('.youtube.com','SAPISID','v1','/',1900000000000,1,0);
            INSERT INTO moz_cookies VALUES('.example.com','X','v2','/',1900000000,1,0);").unwrap();
        let ps = firefox_profiles_in(&root);
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].name, "default-release");
        let got = read_firefox(&ps[0].dir.join("cookies.sqlite")).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].expires, Some(1_900_000_000));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn parses_binarycookies() {
        // Build a one-page file with one cookie.
        let mut cookie = vec![0u8; 56];
        let strs = [".youtube.com\0", "SAPISID\0", "/\0", "val\0"];
        let mut offs = vec![];
        for s in strs {
            offs.push(cookie.len() as u32);
            cookie.extend_from_slice(s.as_bytes());
        }
        let size = cookie.len() as u32;
        cookie[0..4].copy_from_slice(&size.to_le_bytes());
        cookie[8..12].copy_from_slice(&1u32.to_le_bytes());
        for (i, o) in offs.iter().enumerate() {
            cookie[16 + i * 4..20 + i * 4].copy_from_slice(&o.to_le_bytes());
        }
        cookie[40..48].copy_from_slice(&(800_000_000f64).to_le_bytes());
        let mut page = vec![0, 0, 1, 0];
        page.extend_from_slice(&1u32.to_le_bytes());
        page.extend_from_slice(&16u32.to_le_bytes());
        page.extend_from_slice(&[0, 0, 0, 0]);
        page.extend_from_slice(&cookie);
        let mut f = b"cook".to_vec();
        f.extend_from_slice(&1u32.to_be_bytes());
        f.extend_from_slice(&(page.len() as u32).to_be_bytes());
        f.extend_from_slice(&page);
        let got = parse_binarycookies(&f).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "SAPISID");
        assert_eq!(got[0].value, "val");
        assert!(got[0].secure);
        assert_eq!(got[0].expires, Some(800_000_000 + 978_307_200));
    }
}
