//! Disk LRU for fully downloaded audio files.
//!
//! Only complete, verified downloads are installed (atomic rename from a
//! `.part` file). Partial downloads are discarded when their source is dropped.
//! Eviction removes the least recently used files until the cache is under its
//! byte limit. Files that are currently open are never evicted.
//!
//! Which videos have a file is kept in memory (one folder read, then
//! followed through installs, eviction and clearing), so looking a song up
//! before playing it costs no folder read. A file gone from under it (the
//! system purging caches) is noticed when it is looked up; the folder is
//! read again on a miss at most every few minutes, for files another
//! Encore (the CLI) put there.
//!
//! A song's loudness (D5) is kept beside its file, as `<videoId>.loud`: it
//! comes with the stream's address, which a song played from the cache
//! never asks for, so without it a cached song played louder than it did the
//! first time.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use parking_lot::Mutex;

pub const DEFAULT_LIMIT: u64 = 1024 * 1024 * 1024; // 1 GiB (D5)

#[derive(Clone)]
pub struct AudioCache {
    inner: Arc<Inner>,
}

struct Inner {
    dir: PathBuf,
    limit: u64,
    open: Mutex<HashSet<String>>,
    index: Mutex<Index>,
}

/// How often a miss may read the folder again.
const RESCAN: Duration = Duration::from_secs(300);

/// Complete files by video id: the format ids each has.
#[derive(Default)]
struct Index {
    videos: HashMap<String, Vec<u32>>,
    read_at: Option<Instant>,
}

impl Index {
    fn add(&mut self, key: &str) {
        if let Some((vid, itag)) = split_key(key) {
            let v = self.videos.entry(vid.to_string()).or_default();
            if !v.contains(&itag) {
                v.push(itag);
            }
        }
    }

    fn remove(&mut self, key: &str) {
        if let Some((vid, itag)) = split_key(key)
            && let Some(v) = self.videos.get_mut(vid)
        {
            v.retain(|&i| i != itag);
            if v.is_empty() {
                self.videos.remove(vid);
            }
        }
    }
}

/// `<videoId>-<itag>`, the key of a song's file.
fn split_key(key: &str) -> Option<(&str, u32)> {
    let (vid, itag) = key.rsplit_once('-')?;
    Some((vid, itag.parse().ok()?))
}

/// Cache keys are built from the video id and format id; anything else is
/// rejected so a key can never escape the cache directory.
pub fn sanitize_key(key: &str) -> Option<String> {
    if key.is_empty() || key.len() > 128 {
        return None;
    }
    if key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.') && !key.starts_with('.') {
        Some(key.to_string())
    } else {
        None
    }
}

impl AudioCache {
    pub fn new(dir: impl Into<PathBuf>, limit: u64) -> std::io::Result<Self> {
        let dir = dir.into();
        make_dir(&dir)?;
        // Crash cleanup: partial files are never valid media, and a
        // loudness note is of no use without its song's file.
        if let Ok(rd) = fs::read_dir(&dir) {
            let names: Vec<String> = rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
            let have: HashSet<&str> = names.iter().filter_map(|n| split_key(n.strip_suffix(".audio")?)).map(|k| k.0).collect();
            for n in &names {
                if n.ends_with(".part") || n.strip_suffix(".loud").is_some_and(|v| !have.contains(v)) {
                    let _ = fs::remove_file(dir.join(n));
                }
            }
        }
        Ok(Self { inner: Arc::new(Inner { dir, limit, open: Mutex::new(HashSet::new()), index: Mutex::new(Index::default()) }) })
    }

    pub fn dir(&self) -> &Path {
        &self.inner.dir
    }

    fn complete_path(&self, key: &str) -> PathBuf {
        self.inner.dir.join(format!("{key}.audio"))
    }

    /// Where a new download goes. The folder is made again if it went away
    /// while Encore ran: macOS deletes `~/Library/Caches` folders when the
    /// disk runs low, and cleaner apps do too, after which every song failed
    /// with "No such file or directory" until a restart.
    pub fn part_path(&self, key: &str) -> PathBuf {
        if !self.inner.dir.is_dir() {
            let _ = make_dir(&self.inner.dir);
        }
        self.inner.dir.join(format!("{key}.{}.part", std::process::id()))
    }

    /// Returns the path of a complete cached file and marks it recently used.
    pub fn lookup(&self, key: &str) -> Option<PathBuf> {
        let key = sanitize_key(key)?;
        let p = self.complete_path(&key);
        if p.is_file() {
            touch(&p);
            Some(p)
        } else {
            None
        }
    }

    /// A complete cached file for a video, whatever format it was: returns
    /// (path, format id).
    pub fn find_video(&self, video_id: &str) -> Option<(PathBuf, u32)> {
        let vid = sanitize_key(video_id)?;
        let mut ix = self.inner.index.lock();
        if ix.read_at.is_none_or(|t| t.elapsed() > RESCAN && !ix.videos.contains_key(&vid)) {
            self.read_index(&mut ix);
        }
        let itags = ix.videos.get(&vid)?.clone();
        for itag in itags {
            let p = self.complete_path(&format!("{vid}-{itag}"));
            if p.is_file() {
                touch(&p);
                return Some((p, itag));
            }
            ix.remove(&format!("{vid}-{itag}"));
        }
        None
    }

    /// The index from the folder.
    fn read_index(&self, ix: &mut Index) {
        ix.videos.clear();
        ix.read_at = Some(Instant::now());
        if let Ok(rd) = fs::read_dir(&self.inner.dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if let Some(key) = name.strip_suffix(".audio") {
                    ix.add(key);
                }
            }
        }
    }

    fn loud_path(&self, vid: &str) -> PathBuf {
        self.inner.dir.join(format!("{vid}.loud"))
    }

    /// Keep what the service said of a song's loudness (`None`: it gave
    /// none), for when the song plays from its cached file.
    pub fn set_loudness(&self, video_id: &str, db: Option<f32>) {
        let Some(vid) = sanitize_key(video_id) else { return };
        let text = db.filter(|d| d.is_finite()).map(|d| d.to_string()).unwrap_or_else(|| "none".into());
        let p = self.loud_path(&vid);
        if fs::read_to_string(&p).is_ok_and(|t| t == text) {
            return;
        }
        if !self.inner.dir.is_dir() {
            let _ = make_dir(&self.inner.dir);
        }
        let tmp = self.inner.dir.join(format!("{vid}.{}.loud.part", std::process::id()));
        if fs::write(&tmp, text).is_ok() && fs::rename(&tmp, &p).is_err() {
            let _ = fs::remove_file(&tmp);
        }
    }

    /// The loudness kept for a song: `None` when none was kept (a file
    /// cached before Encore kept it), `Some(None)` when the service gave none.
    pub fn loudness(&self, video_id: &str) -> Option<Option<f32>> {
        let text = fs::read_to_string(self.loud_path(&sanitize_key(video_id)?)).ok()?;
        match text.trim() {
            "none" => Some(None),
            t => t.parse::<f32>().ok().filter(|d| d.is_finite()).map(Some),
        }
    }

    /// A file left the cache: its song's loudness goes with its last file.
    fn removed(&self, key: &str) {
        let mut ix = self.inner.index.lock();
        ix.remove(key);
        if let Some((vid, _)) = split_key(key)
            && !ix.videos.contains_key(vid)
        {
            let _ = fs::remove_file(self.loud_path(vid));
        }
    }

    pub fn mark_open(&self, key: &str) {
        self.inner.open.lock().insert(key.to_string());
    }
    pub fn mark_closed(&self, key: &str) {
        self.inner.open.lock().remove(key);
    }

    /// Install a finished part file as the complete entry, then evict.
    pub fn install(&self, key: &str, part: &Path) -> std::io::Result<PathBuf> {
        let key = sanitize_key(key).ok_or_else(|| std::io::Error::other("bad cache key"))?;
        let dst = self.complete_path(&key);
        fs::rename(part, &dst)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&dst, fs::Permissions::from_mode(0o600));
        }
        self.inner.index.lock().add(&key);
        self.evict();
        Ok(dst)
    }

    pub fn total_bytes(&self) -> u64 {
        self.entries().iter().map(|e| e.1).sum()
    }

    fn entries(&self) -> Vec<(PathBuf, u64, SystemTime)> {
        let mut out = Vec::new();
        if let Ok(rd) = fs::read_dir(&self.inner.dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "audio") {
                    if let Ok(md) = e.metadata() {
                        out.push((p, md.len(), md.modified().unwrap_or(SystemTime::UNIX_EPOCH)));
                    }
                }
            }
        }
        out
    }

    pub fn evict(&self) {
        let mut es = self.entries();
        let mut total: u64 = es.iter().map(|e| e.1).sum();
        if total <= self.inner.limit {
            return;
        }
        es.sort_by_key(|e| e.2);
        let open = self.inner.open.lock().clone();
        for (p, len, _) in es {
            if total <= self.inner.limit {
                break;
            }
            let key = p.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
            if open.contains(&key) {
                continue;
            }
            if fs::remove_file(&p).is_ok() {
                total = total.saturating_sub(len);
                self.removed(&key);
            }
        }
    }

    pub fn clear(&self) {
        let open = self.inner.open.lock().clone();
        for (p, _, _) in self.entries() {
            let key = p.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
            if !open.contains(&key) && fs::remove_file(p).is_ok() {
                self.removed(&key);
            }
        }
    }
}

fn make_dir(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

fn touch(p: &Path) {
    if let Ok(f) = fs::OpenOptions::new().append(true).open(p) {
        let _ = f.set_modified(SystemTime::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_deleted_folder_is_made_again() {
        let base = std::env::temp_dir().join(format!("encore-cache-gone-{}", std::process::id()));
        let c = AudioCache::new(base.join("audio"), 1 << 30).unwrap();
        fs::remove_dir_all(&base).unwrap();
        let part = c.part_path("abc-140");
        fs::write(&part, b"x").unwrap();
        assert!(c.install("abc-140", &part).is_ok());
        assert!(c.lookup("abc-140").is_some());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn songs_are_found_without_reading_the_folder() {
        let dir = std::env::temp_dir().join(format!("encore-cache-index-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let c = AudioCache::new(&dir, 1 << 30).unwrap();
        // A file there before: found by the first lookup's folder read.
        fs::write(dir.join("aaaaaaaaaaa-140.audio"), b"x").unwrap();
        assert_eq!(c.find_video("aaaaaaaaaaa").map(|f| f.1), Some(140));
        // Installed later: found from the index.
        let part = c.part_path("bbbbbbbbbbb-251");
        fs::write(&part, b"y").unwrap();
        c.install("bbbbbbbbbbb-251", &part).unwrap();
        assert_eq!(c.find_video("bbbbbbbbbbb").map(|f| f.1), Some(251));
        // A file purged from under it is not answered.
        fs::remove_file(dir.join("bbbbbbbbbbb-251.audio")).unwrap();
        assert!(c.find_video("bbbbbbbbbbb").is_none());
        // Cleared: gone from the index too.
        c.clear();
        assert!(c.find_video("aaaaaaaaaaa").is_none());
        assert!(c.inner.index.lock().videos.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn loudness_stays_with_the_file() {
        let dir = std::env::temp_dir().join(format!("encore-cache-loud-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let c = AudioCache::new(&dir, 1 << 30).unwrap();
        assert_eq!(c.loudness("aaaaaaaaaaa"), None);
        c.set_loudness("aaaaaaaaaaa", Some(4.25));
        c.set_loudness("bbbbbbbbbbb", None);
        c.set_loudness("ccccccccccc", Some(-1.5));
        assert_eq!(c.loudness("aaaaaaaaaaa"), Some(Some(4.25)));
        assert_eq!(c.loudness("bbbbbbbbbbb"), Some(None));
        for k in ["aaaaaaaaaaa-140", "bbbbbbbbbbb-251"] {
            let part = c.part_path(k);
            fs::write(&part, b"x").unwrap();
            c.install(k, &part).unwrap();
        }
        // A new launch drops the note of a song that has no file.
        let c = AudioCache::new(&dir, 1 << 30).unwrap();
        assert_eq!(c.loudness("aaaaaaaaaaa"), Some(Some(4.25)));
        assert_eq!(c.loudness("ccccccccccc"), None);
        // And it goes with the file.
        c.find_video("aaaaaaaaaaa").unwrap();
        c.clear();
        assert_eq!(c.loudness("aaaaaaaaaaa"), None);
        assert_eq!(c.loudness("bbbbbbbbbbb"), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn keys_cannot_escape() {
        assert!(sanitize_key("../etc").is_none());
        assert!(sanitize_key("a/b").is_none());
        assert!(sanitize_key(".hidden").is_none());
        assert_eq!(sanitize_key("dQw4w9WgXcQ-251").as_deref(), Some("dQw4w9WgXcQ-251"));
    }

    #[test]
    fn evicts_lru_over_limit() {
        let dir = std::env::temp_dir().join(format!("encore-cache-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let c = AudioCache::new(&dir, 10).unwrap();
        for (i, k) in ["a", "b", "c"].iter().enumerate() {
            let part = c.part_path(k);
            fs::write(&part, vec![0u8; 6]).unwrap();
            c.install(k, &part).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20 + i as u64));
        }
        // limit 10 bytes, each file 6: only the newest survives
        assert!(c.lookup("c").is_some());
        assert!(c.lookup("a").is_none());
        assert!(c.total_bytes() <= 10);
        let _ = fs::remove_dir_all(&dir);
    }
}
