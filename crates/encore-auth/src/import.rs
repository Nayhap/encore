//! Browser cookie import (D8): Edge, then Chrome, Brave, Firefox and Safari.
//! Selected profile only. Readers are reimplemented here (no `rookie`).
//!
//! Chromium on macOS: the "<Browser> Safe Storage" password comes from
//! `/usr/bin/security find-generic-password -w` (one "Always Allow" survives
//! app rebuilds because the ACL entry is `security`, not Encore). The cookie
//! DB and its WAL are copied to a private temp dir and read with rusqlite.
//! `v10` values are AES-128-CBC; for DB schema ≥ 24 the plaintext starts with
//! SHA-256(host_key), which is validated and stripped.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use aes::cipher::{BlockModeDecrypt, KeyIvInit, block_padding::Pkcs7};
use encore_core::model::BrowserProfile;
use encore_core::service::CookieRecord;
use sha2::{Digest, Sha256};

use crate::jar::{CookieJar, keep_domain};

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("{0} is not installed or has no profiles")]
    NotFound(String),
    #[error("The Keychain did not release the {0} Safe Storage key (prompt denied or timed out).")]
    KeychainDenied(String),
    #[error("macOS did not let Encore read that browser's data. Allow Encore under System Settings → Privacy & Security → Full Disk Access (or “App Data”), then try again.")]
    NeedsFullDiskAccess,
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
    Firefox,
    Safari,
}

impl Browser {
    pub fn all() -> [Browser; 5] {
        [Browser::Edge, Browser::Chrome, Browser::Brave, Browser::Firefox, Browser::Safari]
    }
    pub fn key(self) -> &'static str {
        match self {
            Browser::Edge => "edge",
            Browser::Chrome => "chrome",
            Browser::Brave => "brave",
            Browser::Firefox => "firefox",
            Browser::Safari => "safari",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Browser::Edge => "Microsoft Edge",
            Browser::Chrome => "Google Chrome",
            Browser::Brave => "Brave",
            Browser::Firefox => "Firefox",
            Browser::Safari => "Safari",
        }
    }
    pub fn from_key(k: &str) -> Option<Browser> {
        Browser::all().into_iter().find(|b| b.key() == k)
    }
    fn chromium_dir(self) -> Option<PathBuf> {
        let base = dirs::home_dir()?.join("Library/Application Support");
        Some(match self {
            Browser::Edge => base.join("Microsoft Edge"),
            Browser::Chrome => base.join("Google/Chrome"),
            Browser::Brave => base.join("BraveSoftware/Brave-Browser"),
            _ => return None,
        })
    }
    /// (service, account) of the Safe Storage Keychain item.
    fn safe_storage(self) -> Option<(&'static str, &'static str)> {
        match self {
            Browser::Edge => Some(("Microsoft Edge Safe Storage", "Microsoft Edge")),
            Browser::Chrome => Some(("Chrome Safe Storage", "Chrome")),
            Browser::Brave => Some(("Brave Safe Storage", "Brave")),
            _ => None,
        }
    }
}

/// List profiles (metadata only; no cookie store is opened).
pub fn list_profiles() -> Vec<BrowserProfile> {
    let mut out = Vec::new();
    for b in Browser::all() {
        match b {
            Browser::Edge | Browser::Chrome | Browser::Brave => {
                let Some(dir) = b.chromium_dir() else { continue };
                let ls = match std::fs::read_to_string(dir.join("Local State")) {
                    Ok(s) => s,
                    // Installed, but macOS privacy settings block reading it:
                    // still offer the default profile so the import can explain.
                    Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied || (dir.exists() && e.kind() != std::io::ErrorKind::NotFound) => {
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
                        if dir.join(k).join("Cookies").is_file() {
                            let name = m[k].get("name").and_then(|n| n.as_str()).unwrap_or(k).to_string();
                            out.push(BrowserProfile { id: format!("{}:{k}", b.key()), browser: b.name().into(), name });
                        }
                    }
                }
            }
            Browser::Firefox => {
                for (id, name) in firefox_profiles() {
                    out.push(BrowserProfile { id: format!("firefox:{id}"), browser: b.name().into(), name });
                }
            }
            Browser::Safari => {
                if safari_cookie_path().is_some_and(|p| p.parent().is_some_and(|d| d.exists())) {
                    out.push(BrowserProfile { id: "safari:default".into(), browser: b.name().into(), name: "Default".into() });
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
        Browser::Edge | Browser::Chrome | Browser::Brave => {
            let dir = b.chromium_dir().ok_or_else(|| ImportError::NotFound(b.name().into()))?;
            if prof.contains('/') || prof.contains("..") {
                return Err(ImportError::NotFound(prof.into()));
            }
            let db = dir.join(prof).join("Cookies");
            match std::fs::metadata(&db) {
                Ok(m) if m.is_file() => {}
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => return Err(ImportError::NeedsFullDiskAccess),
                _ => return Err(ImportError::NotFound(format!("{} profile {prof}", b.name()))),
            }
            let (svc, acct) = b.safe_storage().unwrap();
            let password = safe_storage_password(svc, acct, keychain_timeout).ok_or_else(|| ImportError::KeychainDenied(b.name().into()))?;
            let key = derive_key(password.as_bytes());
            read_chromium(&db, &key)?
        }
        Browser::Firefox => {
            let p = firefox_profiles().into_iter().find(|(id, _)| id == prof).ok_or_else(|| ImportError::NotFound("Firefox profile".into()))?;
            read_firefox(&firefox_root().unwrap().join(&p.0).join("cookies.sqlite"))?
        }
        Browser::Safari => {
            let p = safari_cookie_path().ok_or(ImportError::NotFound("Safari".into()))?;
            let data = std::fs::read(&p).map_err(|e| if e.kind() == std::io::ErrorKind::PermissionDenied { ImportError::NeedsFullDiskAccess } else { ImportError::NotFound("Safari cookies".into()) })?;
            parse_binarycookies(&data).map_err(ImportError::Database)?
        }
    };
    let mut jar = CookieJar::new(cookies);
    jar.retain_needed();
    if !jar.has_login() {
        return Err(ImportError::NoLogin);
    }
    Ok(jar)
}

/// Read the Safe Storage password with `/usr/bin/security`. The child is
/// killed if the user does not answer the Keychain prompt in time.
fn safe_storage_password(service: &str, account: &str, timeout: Duration) -> Option<String> {
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

pub fn derive_key(password: &[u8]) -> [u8; 16] {
    let mut key = [0u8; 16];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password, b"saltysalt", 1003, &mut key);
    key
}

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

/// Decrypt one Chromium cookie value (macOS `v10`).
pub fn decrypt_value(enc: &[u8], key: &[u8; 16], host_key: &str, db_version: i64) -> Option<String> {
    let body = enc.strip_prefix(b"v10")?;
    if body.is_empty() || body.len() % 16 != 0 {
        return None;
    }
    let iv = [b' '; 16];
    let mut buf = body.to_vec();
    let pt = Aes128CbcDec::new(key.into(), &iv.into()).decrypt_padded::<Pkcs7>(&mut buf).ok()?;
    let pt = if db_version >= 24 {
        // Validate the SHA-256(host_key) binding, then strip it.
        if pt.len() < 32 {
            return None;
        }
        let h = Sha256::digest(host_key.as_bytes());
        if pt[..32] != h[..] {
            return None;
        }
        &pt[32..]
    } else {
        pt
    };
    String::from_utf8(pt.to_vec()).ok()
}

struct TempCopy(PathBuf);
impl Drop for TempCopy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Copy a SQLite DB with its WAL/SHM to a private temp dir (the browser keeps
/// the original locked while running).
fn copy_db(db: &Path) -> Result<(TempCopy, PathBuf), ImportError> {
    let dir = std::env::temp_dir().join(format!("encore-import-{}-{}", std::process::id(), encore_core::store::now()));
    std::fs::create_dir_all(&dir).map_err(|e| ImportError::Database(e.kind().to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    let guard = TempCopy(dir.clone());
    let name = db.file_name().unwrap();
    let dst = dir.join(name);
    std::fs::copy(db, &dst).map_err(|e| if e.kind() == std::io::ErrorKind::PermissionDenied { ImportError::NeedsFullDiskAccess } else { ImportError::Database(e.kind().to_string()) })?;
    for ext in ["-wal", "-shm"] {
        let src = PathBuf::from(format!("{}{ext}", db.display()));
        if src.is_file() {
            let _ = std::fs::copy(&src, dir.join(format!("{}{ext}", name.to_string_lossy())));
        }
    }
    Ok((guard, dst))
}

const CHROME_EPOCH_OFFSET: i64 = 11_644_473_600; // seconds 1601→1970

pub fn read_chromium(db: &Path, key: &[u8; 16]) -> Result<Vec<CookieRecord>, ImportError> {
    let (_guard, copy) = copy_db(db)?;
    let conn = rusqlite::Connection::open_with_flags(&copy, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE).map_err(|e| ImportError::Database(e.to_string()))?;
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
    for row in rows.flatten() {
        let (host, name, value, enc, path, exp, secure, httponly) = row;
        // Exact domain boundaries, not substring matches (D8).
        if !keep_domain(&host) {
            continue;
        }
        let value = if !value.is_empty() { Some(value) } else { decrypt_value(&enc, key, &host, version) };
        let Some(value) = value else { continue };
        let expires = (exp > 0).then(|| exp / 1_000_000 - CHROME_EPOCH_OFFSET);
        out.push(CookieRecord { name, value, domain: host, path, secure: secure != 0, http_only: httponly != 0, expires });
    }
    Ok(out)
}

fn firefox_root() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join("Library/Application Support/Firefox"))
}

fn firefox_profiles() -> Vec<(String, String)> {
    let Some(root) = firefox_root() else { return vec![] };
    let Ok(ini) = std::fs::read_to_string(root.join("profiles.ini")) else { return vec![] };
    let mut out = Vec::new();
    let (mut name, mut path) = (None::<String>, None::<String>);
    let flush = |name: &mut Option<String>, path: &mut Option<String>, out: &mut Vec<(String, String)>| {
        if let Some(p) = path.take() {
            if root.join(&p).join("cookies.sqlite").is_file() {
                out.push((p.clone(), name.take().unwrap_or(p)));
            }
        }
        *name = None;
    };
    for line in ini.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            flush(&mut name, &mut path, &mut out);
        } else if let Some(v) = l.strip_prefix("Name=") {
            name = Some(v.to_string());
        } else if let Some(v) = l.strip_prefix("Path=") {
            path = Some(v.to_string());
        }
    }
    flush(&mut name, &mut path, &mut out);
    out
}

pub fn read_firefox(db: &Path) -> Result<Vec<CookieRecord>, ImportError> {
    let (_guard, copy) = copy_db(db)?;
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

    fn encrypt(pt: &[u8], key: &[u8; 16]) -> Vec<u8> {
        let iv = [b' '; 16];
        let mut buf = vec![0u8; pt.len() + 16];
        buf[..pt.len()].copy_from_slice(pt);
        let ct = Enc::new(key.into(), &iv.into()).encrypt_padded::<Pkcs7>(&mut buf, pt.len()).unwrap();
        let mut v = b"v10".to_vec();
        v.extend_from_slice(ct);
        v
    }

    #[test]
    fn pbkdf2_matches_known_value() {
        // Computed independently: hashlib.pbkdf2_hmac('sha1', b'peanuts', b'saltysalt', 1003, 16).hex()
        assert_eq!(hex::encode(derive_key(b"peanuts")), "d9a09d499b4e1b7461f28e67972c6dbd");
    }

    #[test]
    fn decrypts_v10_old_schema() {
        let key = derive_key(b"synthetic-password");
        let enc = encrypt(b"cookie-value-123", &key);
        assert_eq!(decrypt_value(&enc, &key, ".youtube.com", 23).as_deref(), Some("cookie-value-123"));
    }

    #[test]
    fn decrypts_and_validates_host_prefix_schema_24() {
        let key = derive_key(b"synthetic-password");
        // A value whose first bytes are valid UTF-8 (the heuristic would fail here).
        let mut pt = Sha256::digest(b".youtube.com").to_vec();
        pt.extend_from_slice(b"AAAAvalue");
        let enc = encrypt(&pt, &key);
        assert_eq!(decrypt_value(&enc, &key, ".youtube.com", 24).as_deref(), Some("AAAAvalue"));
        // Bound to a different host: rejected.
        assert_eq!(decrypt_value(&enc, &key, ".google.com", 24), None);
        // Wrong key: rejected.
        assert_eq!(decrypt_value(&enc, &derive_key(b"other"), ".youtube.com", 24), None);
    }

    #[test]
    fn reads_synthetic_chromium_db_with_wal() {
        let dir = std::env::temp_dir().join(format!("encore-chromium-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("Cookies");
        let key = derive_key(b"synthetic");
        {
            let c = rusqlite::Connection::open(&db).unwrap();
            c.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE meta(key TEXT, value TEXT); INSERT INTO meta VALUES('version','24');
                CREATE TABLE cookies(host_key TEXT, name TEXT, value TEXT, encrypted_value BLOB, path TEXT, expires_utc INTEGER, is_secure INTEGER, is_httponly INTEGER);").unwrap();
            for (host, name) in [(".youtube.com", "SAPISID"), (".notyoutube.com", "X"), (".google.com", "SID"), ("www.youtube.com.evil.com", "Y")] {
                let mut pt = Sha256::digest(host.as_bytes()).to_vec();
                pt.extend_from_slice(format!("v-{name}").as_bytes());
                c.execute("INSERT INTO cookies VALUES (?1, ?2, '', ?3, '/', 13400000000000000, 1, 0)", rusqlite::params![host, name, encrypt(&pt, &key)]).unwrap();
            }
            // keep the WAL around (connection still open when copied below)
            let got = read_chromium(&db, &key).unwrap();
            let names: Vec<&str> = got.iter().map(|c| c.name.as_str()).collect();
            assert_eq!(names, vec!["SAPISID", "SID"]);
            assert_eq!(got[0].value, "v-SAPISID");
            assert!(got[0].expires.unwrap() > 1_700_000_000);
        }
        let _ = std::fs::remove_dir_all(&dir);
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
