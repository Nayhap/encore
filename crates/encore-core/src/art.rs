//! Artwork keys (D14). Parsers register source URLs here and hand the UI an
//! opaque key; the `art://` protocol resolves keys back to URLs. Only
//! allow-listed hosts are accepted, so the protocol can never be used as a
//! general fetch proxy.

use std::collections::HashMap;
use std::sync::OnceLock;

use parking_lot::RwLock;
use sha2::{Digest, Sha256};

use crate::model::ArtRef;

#[derive(Clone, Debug)]
pub struct ArtEntry {
    /// URL without any size suffix.
    pub base: String,
    pub hue: Option<(u16, u16)>,
}

fn reg() -> &'static RwLock<HashMap<String, ArtEntry>> {
    static R: OnceLock<RwLock<HashMap<String, ArtEntry>>> = OnceLock::new();
    R.get_or_init(|| RwLock::new(HashMap::new()))
}

pub fn host_allowed(host: &str) -> bool {
    let h = host.to_ascii_lowercase();
    h == "i.ytimg.com"
        || h.ends_with(".ytimg.com")
        || h == "lh3.googleusercontent.com"
        || h.ends_with(".googleusercontent.com")
        || h.ends_with(".ggpht.com")
}

/// Remove googleusercontent size parameters (`=w60-h60-l90-rj`, `=s120`).
pub fn strip_size(url: &str) -> String {
    if let Some(i) = url.rfind('=') {
        let tail = &url[i + 1..];
        if (tail.starts_with('w') || tail.starts_with('s')) && tail[1..].chars().next().is_some_and(|c| c.is_ascii_digit()) && !url[..i].contains('?') {
            return url[..i].to_string();
        }
    }
    url.to_string()
}

/// URL to fetch for a given edge size.
pub fn sized_url(base: &str, size: u32) -> String {
    let host = url::Url::parse(base).ok().and_then(|u| u.host_str().map(|h| h.to_string())).unwrap_or_default();
    if host.ends_with("googleusercontent.com") || host.ends_with("ggpht.com") {
        format!("{base}=w{size}-h{size}-l90-rj")
    } else {
        base.to_string()
    }
}

pub fn key_for(base: &str) -> String {
    let d = Sha256::digest(base.as_bytes());
    hex::encode(&d[..10])
}

/// Register an artwork URL and return its reference, or `None` for hosts that
/// are not allowed.
pub fn register(url: &str) -> Option<ArtRef> {
    let url = if url.starts_with("//") { format!("https:{url}") } else { url.to_string() };
    let parsed = url::Url::parse(&url).ok()?;
    if parsed.scheme() != "https" || !host_allowed(parsed.host_str()?) {
        return None;
    }
    let base = strip_size(&url);
    let key = key_for(&base);
    let mut r = reg().write();
    let e = r.entry(key.clone()).or_insert(ArtEntry { base, hue: None });
    Some(ArtRef { key, h1: e.hue.map(|h| h.0), h2: e.hue.map(|h| h.1) })
}

pub fn lookup(key: &str) -> Option<ArtEntry> {
    reg().read().get(key).cloned()
}

pub fn set_hue(key: &str, h1: u16, h2: u16) {
    if let Some(e) = reg().write().get_mut(key) {
        e.hue = Some((h1, h2));
    }
}

/// Fill in hues that were computed after the reference was created.
pub fn refresh(a: &mut ArtRef) {
    if a.h1.is_none() {
        if let Some(h) = lookup(&a.key).and_then(|e| e.hue) {
            a.h1 = Some(h.0);
            a.h2 = Some(h.1);
        }
    }
}

/// For persistence across restarts (artwork index in SQLite).
pub fn snapshot() -> Vec<(String, ArtEntry)> {
    reg().read().iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

pub fn restore(key: String, base: String, hue: Option<(u16, u16)>) {
    let u = match url::Url::parse(&base) {
        Ok(u) => u,
        Err(_) => return,
    };
    if u.host_str().is_some_and(host_allowed) && key_for(&base) == key {
        reg().write().entry(key).or_insert(ArtEntry { base, hue });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_sizes_and_registers() {
        let u = "https://lh3.googleusercontent.com/abcDEF=w60-h60-l90-rj";
        assert_eq!(strip_size(u), "https://lh3.googleusercontent.com/abcDEF");
        let a = register(u).unwrap();
        let b = register("https://lh3.googleusercontent.com/abcDEF=w544-h544-l90-rj").unwrap();
        assert_eq!(a.key, b.key);
        assert_eq!(lookup(&a.key).unwrap().base, "https://lh3.googleusercontent.com/abcDEF");
        assert_eq!(sized_url("https://lh3.googleusercontent.com/abcDEF", 320), "https://lh3.googleusercontent.com/abcDEF=w320-h320-l90-rj");
    }

    #[test]
    fn rejects_other_hosts() {
        assert!(register("https://evil.example.com/x.jpg").is_none());
        assert!(register("http://i.ytimg.com/vi/x/hqdefault.jpg").is_none());
        assert!(register("https://i.ytimg.com/vi/x/hqdefault.jpg?sqp=abc").is_some());
    }
}
