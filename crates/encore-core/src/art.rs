//! Artwork keys (D14). Parsers register source URLs here and hand the UI an
//! opaque key; the `art://` protocol resolves keys back to URLs. Only
//! allow-listed hosts are accepted, so the protocol can never be used as a
//! general fetch proxy.
//!
//! The registry in memory is bounded. The artwork index in SQLite (scope
//! `art`) backs it: a key not in memory is looked up there (and kept again),
//! nothing is read in bulk at launch, and keys pushed out of memory are
//! written there first, so a cover still on screen never loses its address.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use parking_lot::RwLock;
use sha2::{Digest, Sha256};

use crate::model::ArtRef;
use crate::store::Store;

#[derive(Clone, Debug)]
pub struct ArtEntry {
    /// URL without any size suffix.
    pub base: String,
    pub hue: Option<(u16, u16)>,
}

/// Keys kept in memory; past this the least recently used quarter is
/// written to the index and dropped.
const CAP: usize = 20_000;
/// How long a key written out that way is kept in the index, as the art
/// cache keeps the ones it saves.
const SPILL_TTL: i64 = 180 * 24 * 3600;

/// Key → entry, each with the tick it was last used at.
struct Registry {
    map: HashMap<String, (ArtEntry, u64)>,
    tick: u64,
    cap: usize,
}

impl Registry {
    fn new(cap: usize) -> Self {
        Registry { map: HashMap::new(), tick: 0, cap }
    }

    fn touch(&mut self, key: &str) -> Option<ArtEntry> {
        self.tick += 1;
        let t = self.tick;
        self.map.get_mut(key).map(|(e, at)| {
            *at = t;
            e.clone()
        })
    }

    /// Inserts unless there; returns the entry and what was pushed out.
    fn insert(&mut self, key: String, e: ArtEntry) -> (ArtEntry, Vec<(String, ArtEntry)>) {
        self.tick += 1;
        let t = self.tick;
        let e = self.map.entry(key).or_insert((e, t)).0.clone();
        (e, self.trim())
    }

    fn trim(&mut self) -> Vec<(String, ArtEntry)> {
        if self.map.len() <= self.cap {
            return Vec::new();
        }
        let mut ages: Vec<u64> = self.map.values().map(|v| v.1).collect();
        let cut = self.map.len() / 4;
        let (_, nth, _) = ages.select_nth_unstable(cut);
        let oldest = *nth;
        let gone: Vec<String> = self.map.iter().filter(|(_, v)| v.1 < oldest).map(|(k, _)| k.clone()).collect();
        let out = gone.into_iter().filter_map(|k| self.map.remove(&k).map(|v| (k, v.0))).collect();
        self.map.shrink_to(self.cap);
        out
    }
}

fn reg() -> &'static RwLock<Registry> {
    static R: OnceLock<RwLock<Registry>> = OnceLock::new();
    R.get_or_init(|| RwLock::new(Registry::new(CAP)))
}

fn index() -> &'static OnceLock<Arc<Store>> {
    static S: OnceLock<Arc<Store>> = OnceLock::new();
    &S
}

/// The artwork index that backs the registry. Set once at startup.
pub fn attach(store: Arc<Store>) {
    let _ = index().set(store);
}

/// A key from the index, if there is one there.
fn from_index(key: &str) -> Option<ArtEntry> {
    let store = index().get()?;
    let (base, hue) = store.get::<(String, Option<(u16, u16)>)>("art", key).ok()??;
    valid(key, &base).then_some(ArtEntry { base, hue })
}

/// The same while parsing a page: only a known hue is wanted, so it never
/// waits on a busy database (a backup) for long.
fn hue_from_index(key: &str) -> Option<(u16, u16)> {
    let store = index().get()?;
    store.get_quick::<(String, Option<(u16, u16)>)>("art", key, std::time::Duration::from_millis(20))?.1
}

fn valid(key: &str, base: &str) -> bool {
    url::Url::parse(base).is_ok_and(|u| u.scheme() == "https" && u.host_str().is_some_and(host_allowed)) && key_for(base) == key
}

/// Keep `e` under `key` in memory, writing out what that pushes out.
fn keep(key: String, e: ArtEntry) -> ArtEntry {
    let (e, gone) = reg().write().insert(key, e);
    if !gone.is_empty() {
        spill(gone);
    }
    e
}

/// Writes keys pushed out of memory to the index, off the caller's thread.
/// Rows already there (a downloaded song's cover never expires) stay as
/// they are.
fn spill(gone: Vec<(String, ArtEntry)>) {
    let Some(store) = index().get().cloned() else { return };
    let rows: Vec<(String, (String, Option<(u16, u16)>))> = gone.into_iter().map(|(k, e)| (k, (e.base, e.hue))).collect();
    let _ = std::thread::Builder::new().name("encore-art-index".into()).spawn(move || {
        if let Err(e) = store.put_absent("art", &rows, Some(SPILL_TTL)) {
            tracing::warn!(error = %e.message, "couldn't save artwork keys");
        }
    });
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
    let known = reg().write().touch(&key);
    // Not in memory: the index may know its hue already.
    let e = known.unwrap_or_else(|| {
        let hue = hue_from_index(&key);
        keep(key.clone(), ArtEntry { base, hue })
    });
    Some(ArtRef { key, h1: e.hue.map(|h| h.0), h2: e.hue.map(|h| h.1) })
}

/// The https address of a registered remote cover at 544 px, for things
/// outside Encore that show it themselves (Discord). `None` for local
/// covers and unknown keys: their files never leave this computer.
pub fn public_url(key: &str) -> Option<String> {
    let e = lookup(key)?;
    let u = url::Url::parse(&e.base).ok()?;
    (u.scheme() == "https" && u.host_str().is_some_and(host_allowed)).then(|| sized_url(&e.base, 544))
}

/// Local covers (D34): images the user chose, kept in app data and served
/// by the same `art://` protocol under keys of their own: `l-` and 32 hex
/// digits, random, so a key says nothing about the file and can't name a
/// path. They never go through the remote registry or its host allow-list.
pub const LOCAL_PREFIX: &str = "l-";

pub fn new_local_key() -> String {
    format!("{LOCAL_PREFIX}{}", uuid::Uuid::new_v4().simple())
}

pub fn is_local_key(key: &str) -> bool {
    key.strip_prefix(LOCAL_PREFIX).is_some_and(|h| h.len() == 32 && h.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}

pub fn lookup(key: &str) -> Option<ArtEntry> {
    let known = reg().write().touch(key);
    known.or_else(|| from_index(key).map(|e| keep(key.to_string(), e)))
}

pub fn set_hue(key: &str, h1: u16, h2: u16) {
    if let Some((e, _)) = reg().write().map.get_mut(key) {
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

pub fn restore(key: String, base: String, hue: Option<(u16, u16)>) {
    let known = reg().read().map.contains_key(&key);
    if valid(&key, &base) && !known {
        keep(key, ArtEntry { base, hue });
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
    fn local_keys_are_their_own() {
        let k = new_local_key();
        assert!(is_local_key(&k), "{k}");
        assert!(!is_local_key("l-../../etc/passwd"));
        assert!(!is_local_key("l-0123456789ABCDEF0123456789abcdef"));
        assert!(!is_local_key("0123456789abcdef0123"));
        assert!(lookup(&k).is_none() && public_url(&k).is_none());
        let a = register("https://lh3.googleusercontent.com/pub=w60-h60").unwrap();
        assert_eq!(public_url(&a.key).as_deref(), Some("https://lh3.googleusercontent.com/pub=w544-h544-l90-rj"));
    }

    #[test]
    fn the_least_recently_used_quarter_goes_past_the_cap() {
        let mut r = Registry::new(8);
        let e = |i: u32| ArtEntry { base: format!("https://i.ytimg.com/{i}"), hue: None };
        for i in 0..8 {
            assert!(r.insert(format!("k{i}"), e(i)).1.is_empty());
        }
        r.touch("k0");
        let (_, gone) = r.insert("k8".into(), e(8));
        let mut gone: Vec<String> = gone.into_iter().map(|g| g.0).collect();
        gone.sort();
        assert_eq!(gone, ["k1", "k2"], "the oldest, not the one just used");
        assert!(r.map.contains_key("k0") && r.map.contains_key("k8"));
    }

    #[test]
    fn keys_out_of_memory_come_back_from_the_index() {
        let store = Arc::new(Store::open_memory().unwrap());
        let base = "https://lh3.googleusercontent.com/indexed".to_string();
        let key = key_for(&base);
        store.put("art", &key, &(base.clone(), Some((10u16, 20u16))), None).unwrap();
        attach(store.clone());
        let Some(s) = index().get() else { return };
        if !Arc::ptr_eq(s, &store) {
            return; // another test attached its own first
        }
        let e = lookup(&key).unwrap();
        assert_eq!((e.base.as_str(), e.hue), (base.as_str(), Some((10, 20))));
        let a = register(&format!("{base}=w60-h60")).unwrap();
        assert_eq!((a.h1, a.h2), (Some(10), Some(20)), "registering finds the hue the index kept");
        // A row the index has is never replaced by a spill (its expiry stays).
        store.put_absent("art", &[(key.clone(), (String::from("https://i.ytimg.com/x"), None::<(u16, u16)>))], Some(5)).unwrap();
        assert_eq!(store.get::<(String, Option<(u16, u16)>)>("art", &key).unwrap().unwrap().0, base);
    }

    #[test]
    fn rejects_other_hosts() {
        assert!(register("https://evil.example.com/x.jpg").is_none());
        assert!(register("http://i.ytimg.com/vi/x/hqdefault.jpg").is_none());
        assert!(register("https://i.ytimg.com/vi/x/hqdefault.jpg?sqp=abc").is_some());
    }
}
