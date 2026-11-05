//! `art://localhost/<key>?s=<size>` (D14): opaque keys, allow-listed hosts,
//! size/MIME limits, disk LRU, one fetch per key and size at a time, dominant
//! hue worked out once per key after the image is answered. The same cached
//! file feeds macOS Now Playing. Covers of downloaded songs are kept apart,
//! out of the LRU, and answer every size when the network can't (D23).
//! Covers the user chose for folders are local art (D34): checked and
//! re-encoded once into `<app data>/covers`, served under keys of their own,
//! never fetched and never through the remote allow-list.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use encore_core::art;
use encore_core::store::Store;
use parking_lot::Mutex;

const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_PIXELS: u64 = 16_000_000;
const CACHE_LIMIT: u64 = 256 * 1024 * 1024;
const SIZES: [u32; 4] = [120, 226, 544, 1080];
/// A chosen picture may be a large photo; what is kept is one small square.
const LOCAL_MAX_BYTES: u64 = 25 * 1024 * 1024;
const LOCAL_MAX_PIXELS: u64 = 64_000_000;
const LOCAL_EDGE: u32 = 640;

type HueHook = Box<dyn Fn(&str, u16, u16) + Send + Sync>;
/// Covers decoded for their hue at once, at most: a decode of a large one
/// holds tens of MB for a moment.
const HUE_DECODES: usize = 2;

/// Fetches in flight, one per key and size: a second request for the same
/// cover waits for the first and reads its file.
type Inflight = Mutex<std::collections::HashMap<(String, u32), Arc<tokio::sync::Mutex<()>>>>;

/// The hue side of the cache, shared with the tasks that work hues out
/// after the image has been answered.
struct Hues {
    store: Arc<Store>,
    on_hue: Mutex<Option<HueHook>>,
    /// Keys whose hue was worked out (or found to have none) this run.
    done: Mutex<std::collections::HashSet<String>>,
    decodes: tokio::sync::Semaphore,
}

pub struct ArtCache {
    dir: PathBuf,
    /// `<downloads>/art`: one 544 px cover per key, kept by the downloads.
    kept: Option<PathBuf>,
    /// Offline, a kept cover answers at once, at every size (D23).
    online: AtomicBool,
    http: reqwest::Client,
    hues: Arc<Hues>,
    inflight: Inflight,
    writes: Mutex<u32>,
}

pub fn snap_size(s: u32) -> u32 {
    *SIZES.iter().find(|&&x| x >= s).unwrap_or(&1080)
}

fn mime_of(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if b.starts_with(b"\x89PNG") {
        Some("image/png")
    } else if b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// Two dominant hues (0..360) weighted by saturation and brightness.
pub fn hues(bytes: &[u8]) -> Option<(u16, u16)> {
    let img = image::load_from_memory(bytes).ok()?;
    let small = img.thumbnail(32, 32).to_rgb8();
    let mut bins = [0f64; 36];
    for p in small.pixels() {
        let (r, g, b) = (p[0] as f64 / 255.0, p[1] as f64 / 255.0, p[2] as f64 / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let d = max - min;
        if d < 0.08 || max < 0.15 {
            continue; // greys and near-black carry no hue
        }
        let h = if max == r {
            60.0 * (((g - b) / d).rem_euclid(6.0))
        } else if max == g {
            60.0 * ((b - r) / d + 2.0)
        } else {
            60.0 * ((r - g) / d + 4.0)
        };
        let s = d / max;
        bins[((h / 10.0) as usize).min(35)] += s * max;
    }
    let total: f64 = bins.iter().sum();
    if total < 1.0 {
        return None;
    }
    let first = (0..36).max_by(|&a, &b| bins[a].partial_cmp(&bins[b]).unwrap()).unwrap();
    let dist = |a: usize, b: usize| {
        let d = (a as i32 - b as i32).unsigned_abs() as usize;
        d.min(36 - d)
    };
    let second = (0..36).filter(|&i| dist(i, first) >= 4).max_by(|&a, &b| bins[a].partial_cmp(&bins[b]).unwrap()).filter(|&i| bins[i] > total * 0.05).unwrap_or((first + 4) % 36);
    Some((first as u16 * 10 + 5, second as u16 * 10 + 5))
}

impl ArtCache {
    pub fn new(dir: PathBuf, store: Arc<Store>, kept: Option<PathBuf>) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        let http = reqwest::Client::builder().connect_timeout(Duration::from_secs(8)).timeout(Duration::from_secs(20)).redirect(reqwest::redirect::Policy::custom(|a| {
            // Follow only redirects that stay on allow-listed hosts.
            if a.previous().len() < 3 && a.url().scheme() == "https" && a.url().host_str().is_some_and(art::host_allowed) { a.follow() } else { a.stop() }
        })).build().expect("http");
        let hues = Arc::new(Hues { store, on_hue: Mutex::new(None), done: Mutex::new(Default::default()), decodes: tokio::sync::Semaphore::new(HUE_DECODES) });
        Self { dir, kept, online: AtomicBool::new(true), http, hues, inflight: Mutex::new(Default::default()), writes: Mutex::new(0) }
    }

    pub fn on_hue(&self, f: impl Fn(&str, u16, u16) + Send + Sync + 'static) {
        *self.hues.on_hue.lock() = Some(Box::new(f));
    }

    fn valid_key(key: &str) -> bool {
        key.len() == 20 && key.chars().all(|c| c.is_ascii_hexdigit())
    }

    /// The file for a key: the cached one, else a downloaded song's cover.
    pub fn path_for(&self, key: &str, size: u32) -> Option<PathBuf> {
        if art::is_local_key(key) {
            return Some(local_path(&local_dir(), key));
        }
        if !Self::valid_key(key) {
            return None;
        }
        let cached = self.dir.join(format!("{key}-{}", snap_size(size)));
        if cached.exists() {
            return Some(cached);
        }
        Some(self.kept_path(key).filter(|p| p.exists()).unwrap_or(cached))
    }

    fn kept_path(&self, key: &str) -> Option<PathBuf> {
        self.kept.as_ref().map(|d| d.join(key))
    }

    pub fn set_online(&self, on: bool) {
        self.online.store(on, Ordering::Relaxed);
    }

    /// A downloaded song uses this cover: its address is kept with the
    /// download, so it gets no expiry in the art index.
    pub fn is_kept(&self, key: &str) -> bool {
        Self::valid_key(key) && self.kept_path(key).is_some_and(|p| p.is_file())
    }

    /// Bytes and MIME type for an art key at (at least) `size` pixels.
    pub async fn get(&self, key: &str, size: u32) -> Result<(Vec<u8>, &'static str), String> {
        if art::is_local_key(key) {
            return self.get_local(key).await;
        }
        if !Self::valid_key(key) {
            return Err("bad art key".into());
        }
        let size = snap_size(size);
        let path = self.dir.join(format!("{key}-{size}"));
        if let Some(hit) = Self::read_cached(&path).await {
            return Ok(hit);
        }
        // One fetch per cover and size: the others wait, then read its file.
        let flight = Flight::join(&self.inflight, (key.to_string(), size));
        let _one = flight.lock.lock().await;
        if let Some(hit) = Self::read_cached(&path).await {
            return Ok(hit);
        }
        // A downloaded song's cover: enough for anything up to 544 px, and
        // for every size when the network can't answer.
        let kept = match self.kept_path(key) {
            Some(p) => tokio::fs::read(&p).await.ok().and_then(|b| mime_of(&b).map(|m| (b, m))),
            None => None,
        };
        if size <= 544 || !self.online.load(Ordering::Relaxed) {
            if let Some(k) = kept {
                return Ok(k);
            }
            return self.fetch(key, size, &path).await;
        }
        match self.fetch(key, size, &path).await {
            Ok(x) => Ok(x),
            Err(e) => kept.ok_or(e),
        }
    }

    /// A cached file that is an image, its time touched for the LRU.
    async fn read_cached(path: &std::path::Path) -> Option<(Vec<u8>, &'static str)> {
        let b = tokio::fs::read(path).await.ok()?;
        let m = mime_of(&b)?;
        if let Ok(f) = std::fs::OpenOptions::new().append(true).open(path) {
            let _ = f.set_modified(SystemTime::now());
        }
        Some((b, m))
    }

    async fn fetch(&self, key: &str, size: u32, path: &std::path::Path) -> Result<(Vec<u8>, &'static str), String> {
        let entry = art::lookup(key).ok_or("unknown art key")?;
        let url = art::sized_url(&entry.base, size);
        let u = url::Url::parse(&url).map_err(|_| "bad art url")?;
        if u.scheme() != "https" || !u.host_str().is_some_and(art::host_allowed) {
            return Err("art host not allowed".into());
        }
        let r = self.http.get(u).send().await.map_err(|_| "artwork download failed".to_string())?;
        if !r.status().is_success() {
            return Err(format!("artwork server answered {}", r.status().as_u16()));
        }
        if r.content_length().is_some_and(|l| l as usize > MAX_BYTES) {
            return Err("artwork too large".into());
        }
        let b = r.bytes().await.map_err(|_| "artwork download interrupted".to_string())?;
        if b.len() > MAX_BYTES {
            return Err("artwork too large".into());
        }
        let mime = mime_of(&b).ok_or("artwork is not an image")?;
        let dims_ok = image::ImageReader::new(std::io::Cursor::new(&b)).with_guessed_format().ok().and_then(|r| r.into_dimensions().ok()).is_some_and(|(w, h)| (w as u64) * (h as u64) <= MAX_PIXELS);
        if !dims_ok {
            return Err("artwork dimensions not allowed".into());
        }
        let tmp = path.with_extension("tmp");
        // The folder may have gone while Encore ran (macOS purges caches).
        if let Some(dir) = path.parent() {
            let _ = tokio::fs::create_dir_all(dir).await;
        }
        if tokio::fs::write(&tmp, &b).await.is_ok() {
            let _ = tokio::fs::rename(&tmp, path).await;
        }
        // Hue once per key (none found counts too), worked out after the
        // image is answered; then the key → URL mapping and hue are saved.
        if entry.hue.is_none() && self.hues.done.lock().insert(key.to_string()) {
            let (hs, key, base, bytes) = (self.hues.clone(), key.to_string(), entry.base.clone(), b.clone());
            let ttl = if self.is_kept(&key) { None } else { Some(180 * 24 * 3600) };
            tauri::async_runtime::spawn(async move {
                let h = hs.work_out(bytes).await;
                if let Some((h1, h2)) = h {
                    art::set_hue(&key, h1, h2);
                    hs.tell(&key, h1, h2);
                }
                let _ = hs.store.put("art", &key, &(base, h), ttl);
            });
        }
        let evict = {
            let mut w = self.writes.lock();
            *w += 1;
            *w % 50 == 0
        };
        if evict {
            self.evict();
        }
        Ok((b.to_vec(), mime))
    }

    /// A local cover: one size for every request, its hue sent once a run.
    async fn get_local(&self, key: &str) -> Result<(Vec<u8>, &'static str), String> {
        let b = tokio::fs::read(local_path(&local_dir(), key)).await.map_err(|_| "no such cover".to_string())?;
        let mime = mime_of(&b).ok_or("cover is not an image")?;
        if self.hues.done.lock().insert(key.to_string()) {
            let (hs, key, bytes) = (self.hues.clone(), key.to_string(), b.clone());
            tauri::async_runtime::spawn(async move {
                if let Some((h1, h2)) = hs.work_out(bytes).await {
                    hs.tell(&key, h1, h2);
                }
            });
        }
        Ok((b, mime))
    }

    /// Make a local cover from a picture the user chose: checked to be a
    /// JPEG, PNG or WebP of a sane size, cropped to a square and saved as a
    /// new JPEG (nothing else of the original is kept). Returns its key.
    /// Blocking: call it off the async runtime.
    pub fn import_local(&self, file: &std::path::Path) -> Result<String, String> {
        import_local_into(&local_dir(), file)
    }

    /// Delete a local cover's file (its folder no longer uses it).
    pub fn remove_local(&self, key: &str) {
        if art::is_local_key(key) {
            let _ = std::fs::remove_file(local_path(&local_dir(), key));
        }
    }

    fn evict(&self) {
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return };
        let mut v: Vec<(PathBuf, u64, SystemTime)> = rd.flatten().filter_map(|e| e.metadata().ok().map(|m| (e.path(), m.len(), m.modified().unwrap_or(SystemTime::UNIX_EPOCH)))).collect();
        let mut total: u64 = v.iter().map(|x| x.1).sum();
        v.sort_by_key(|x| x.2);
        for (p, l, _) in v {
            if total <= CACHE_LIMIT {
                break;
            }
            if std::fs::remove_file(&p).is_ok() {
                total -= l;
            }
        }
    }

    pub fn total_bytes(&self) -> u64 {
        std::fs::read_dir(&self.dir).map(|rd| rd.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum()).unwrap_or(0)
    }

    pub fn clear(&self) {
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

impl Hues {
    /// A cover's hues, decoded off the async runtime, a few at a time.
    async fn work_out(&self, bytes: impl AsRef<[u8]> + Send + 'static) -> Option<(u16, u16)> {
        let _turn = self.decodes.acquire().await.ok()?;
        tokio::task::spawn_blocking(move || hues(bytes.as_ref())).await.ok().flatten()
    }

    fn tell(&self, key: &str, h1: u16, h2: u16) {
        if let Some(f) = self.on_hue.lock().as_ref() {
            f(key, h1, h2);
        }
    }
}

/// One request's share of a fetch in flight; the last one out removes it.
struct Flight<'a> {
    map: &'a Inflight,
    key: (String, u32),
    lock: Arc<tokio::sync::Mutex<()>>,
}

impl<'a> Flight<'a> {
    fn join(map: &'a Inflight, key: (String, u32)) -> Self {
        let lock = map.lock().entry(key.clone()).or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))).clone();
        Flight { map, key, lock }
    }
}

impl Drop for Flight<'_> {
    fn drop(&mut self) {
        let mut m = self.map.lock();
        // Copies are taken under this lock: the map's and this one left
        // means no one else waits.
        if Arc::strong_count(&self.lock) == 2 && m.get(&self.key).is_some_and(|l| Arc::ptr_eq(l, &self.lock)) {
            m.remove(&self.key);
        }
    }
}

/// `<app data>/covers`, beside the database: durable, unlike the art cache.
fn local_dir() -> PathBuf {
    static D: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    D.get_or_init(|| crate::core::Paths::new().data.join("covers")).clone()
}

fn local_path(dir: &std::path::Path, key: &str) -> PathBuf {
    dir.join(format!("{key}.jpg"))
}

fn import_local_into(dir: &std::path::Path, file: &std::path::Path) -> Result<String, String> {
    const NOT_IMAGE: &str = "That file isn't a picture Encore can use. Choose a JPEG, PNG or WebP image.";
    const UNREADABLE: &str = "Encore couldn't open that file.";
    const UNSAVED: &str = "Encore couldn't save that picture. Check that the disk isn't full.";
    let len = std::fs::metadata(file).map_err(|_| UNREADABLE.to_string())?.len();
    if len > LOCAL_MAX_BYTES {
        return Err("That picture is too large. Choose one under 25 MB.".into());
    }
    let b = std::fs::read(file).map_err(|_| UNREADABLE.to_string())?;
    mime_of(&b).ok_or(NOT_IMAGE)?;
    let reader = || image::ImageReader::new(std::io::Cursor::new(&b)).with_guessed_format().map_err(|_| NOT_IMAGE.to_string());
    let (w, h) = reader()?.into_dimensions().map_err(|_| NOT_IMAGE.to_string())?;
    if (w as u64) * (h as u64) > LOCAL_MAX_PIXELS || w < 16 || h < 16 {
        return Err("That picture is too large or too small. Choose another one.".into());
    }
    let img = reader()?.decode().map_err(|_| NOT_IMAGE.to_string())?;
    let side = w.min(h);
    let square = img.crop_imm((w - side) / 2, (h - side) / 2, side, side);
    let edge = side.min(LOCAL_EDGE);
    let rgb = square.resize_exact(edge, edge, image::imageops::FilterType::Lanczos3).to_rgb8();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 88).encode_image(&rgb).map_err(|_| UNSAVED.to_string())?;
    std::fs::create_dir_all(dir).map_err(|_| UNSAVED.to_string())?;
    let key = art::new_local_key();
    let path = local_path(dir, &key);
    let tmp = path.with_extension("part");
    std::fs::write(&tmp, &out).map_err(|_| UNSAVED.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, &path).map_err(|_| UNSAVED.to_string())?;
    Ok(key)
}

/// Parse `art://localhost/<key>?s=<size>` (or `http://art.localhost/…`).
pub fn parse_request(uri: &str) -> Option<(String, u32)> {
    let u = url::Url::parse(uri).ok()?;
    let key = u.path().trim_start_matches('/').to_string();
    let size = u.query_pairs().find(|(k, _)| k == "s").and_then(|(_, v)| v.parse().ok()).unwrap_or(544);
    Some((key, size))
}

/// Downloads keep the cover the app shows (D23).
#[async_trait::async_trait]
impl encore_downloads::ArtSource for ArtCache {
    async fn cover(&self, key: &str) -> Option<Vec<u8>> {
        self.get(key, 544).await.ok().map(|(b, _)| b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_requests() {
        assert_eq!(parse_request("art://localhost/0123456789abcdef0123?s=226"), Some(("0123456789abcdef0123".into(), 226)));
        assert_eq!(snap_size(100), 120);
        assert_eq!(snap_size(300), 544);
        assert_eq!(snap_size(5000), 1080);
    }

    #[test]
    fn hue_of_solid_colour() {
        let mut img = image::RgbImage::new(8, 8);
        for p in img.pixels_mut() {
            *p = image::Rgb([20, 80, 230]);
        }
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        let (h1, _) = hues(buf.get_ref()).unwrap();
        assert!((215..=235).contains(&h1), "{h1}");
    }

    #[test]
    fn local_covers_are_checked_and_re_encoded() {
        let dir = std::env::temp_dir().join(format!("encore-covers-{}", std::process::id()));
        let src = dir.join("in");
        std::fs::create_dir_all(&src).unwrap();
        let mut img = image::RgbImage::new(300, 200);
        for p in img.pixels_mut() {
            *p = image::Rgb([200, 40, 90]);
        }
        let png = src.join("a.png");
        img.save(&png).unwrap();
        let key = import_local_into(&dir, &png).unwrap();
        assert!(art::is_local_key(&key));
        let out = std::fs::read(local_path(&dir, &key)).unwrap();
        assert_eq!(mime_of(&out), Some("image/jpeg"));
        let back = image::load_from_memory(&out).unwrap();
        assert_eq!((back.width(), back.height()), (200, 200), "cropped to a square, never enlarged");
        let fake = src.join("b.png");
        std::fs::write(&fake, b"\x89PNG not really").unwrap();
        assert!(import_local_into(&dir, &fake).is_err());
        let text = src.join("c.txt");
        std::fs::write(&text, b"hello").unwrap();
        assert!(import_local_into(&dir, &text).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn one_fetch_per_cover_and_size() {
        let map: Inflight = Mutex::new(Default::default());
        let a = Flight::join(&map, ("k".into(), 120));
        let b = Flight::join(&map, ("k".into(), 120));
        let other = Flight::join(&map, ("k".into(), 544));
        assert!(Arc::ptr_eq(&a.lock, &b.lock));
        assert!(!Arc::ptr_eq(&a.lock, &other.lock), "another size is another fetch");
        let g = a.lock.lock().await;
        assert!(b.lock.try_lock().is_err(), "the second waits for the first");
        drop(g);
        drop((a, b, other));
        assert!(map.lock().is_empty());
    }

    #[tokio::test]
    async fn grey_covers_have_no_hue_and_decodes_wait_their_turn() {
        let store = Arc::new(Store::open_memory().unwrap());
        let hs = Hues { store, on_hue: Mutex::new(None), done: Mutex::new(Default::default()), decodes: tokio::sync::Semaphore::new(1) };
        let grey = image::RgbImage::from_pixel(8, 8, image::Rgb([120, 120, 120]));
        let mut buf = std::io::Cursor::new(Vec::new());
        grey.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        assert_eq!(hs.work_out(buf.into_inner()).await, None);
        let held = hs.decodes.acquire().await.unwrap();
        let waiting = hs.work_out(vec![0u8; 4]);
        assert!(tokio::time::timeout(Duration::from_millis(20), waiting).await.is_err());
        drop(held);
    }

    #[test]
    fn rejects_bad_keys() {
        assert!(!ArtCache::valid_key("../../etc/passwd"));
        assert!(ArtCache::valid_key("0123456789abcdef0123"));
    }
}
