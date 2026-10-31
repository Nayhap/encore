//! `art://localhost/<key>?s=<size>` (D14): opaque keys, allow-listed hosts,
//! size/MIME limits, disk LRU, dominant hue computed once. The same cached
//! file feeds macOS Now Playing. Covers of downloaded songs are kept apart,
//! out of the LRU, and answer every size when the network can't (D23).

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

type HueHook = Box<dyn Fn(&str, u16, u16) + Send + Sync>;

pub struct ArtCache {
    dir: PathBuf,
    /// `<downloads>/art`: one 544 px cover per key, kept by the downloads.
    kept: Option<PathBuf>,
    /// Offline, a kept cover answers at once, at every size (D23).
    online: AtomicBool,
    store: Arc<Store>,
    http: reqwest::Client,
    on_hue: Mutex<Option<HueHook>>,
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
        Self { dir, kept, online: AtomicBool::new(true), store, http, on_hue: Mutex::new(None), writes: Mutex::new(0) }
    }

    pub fn on_hue(&self, f: impl Fn(&str, u16, u16) + Send + Sync + 'static) {
        *self.on_hue.lock() = Some(Box::new(f));
    }

    fn valid_key(key: &str) -> bool {
        key.len() == 20 && key.chars().all(|c| c.is_ascii_hexdigit())
    }

    /// The file for a key: the cached one, else a downloaded song's cover.
    pub fn path_for(&self, key: &str, size: u32) -> Option<PathBuf> {
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
        if !Self::valid_key(key) {
            return Err("bad art key".into());
        }
        let size = snap_size(size);
        let path = self.dir.join(format!("{key}-{size}"));
        if let Ok(b) = tokio::fs::read(&path).await {
            if let Some(m) = mime_of(&b) {
                if let Ok(f) = std::fs::OpenOptions::new().append(true).open(&path) {
                    let _ = f.set_modified(SystemTime::now());
                }
                return Ok((b, m));
            }
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
        if tokio::fs::write(&tmp, &b).await.is_ok() {
            let _ = tokio::fs::rename(&tmp, path).await;
        }
        // Hue once per key; persist the key → URL mapping and hue.
        if entry.hue.is_none() {
            let bytes = b.to_vec();
            let h = tokio::task::spawn_blocking(move || hues(&bytes)).await.ok().flatten();
            if let Some((h1, h2)) = h {
                art::set_hue(key, h1, h2);
                if let Some(f) = self.on_hue.lock().as_ref() {
                    f(key, h1, h2);
                }
            }
            let ttl = if self.is_kept(key) { None } else { Some(180 * 24 * 3600) };
            let _ = self.store.put("art", key, &(entry.base.clone(), h), ttl);
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
    fn rejects_bad_keys() {
        assert!(!ArtCache::valid_key("../../etc/passwd"));
        assert!(ArtCache::valid_key("0123456789abcdef0123"));
    }
}
