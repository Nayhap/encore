//! Disk LRU for fully downloaded audio files.
//!
//! Only complete, verified downloads are installed (atomic rename from a
//! `.part` file). Partial downloads are discarded when their source is dropped.
//! Eviction removes the least recently used files until the cache is under its
//! byte limit. Files that are currently open are never evicted.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

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
        fs::create_dir_all(&dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
        }
        // Crash cleanup: partial files are never valid media.
        if let Ok(rd) = fs::read_dir(&dir) {
            for e in rd.flatten() {
                if e.path().extension().is_some_and(|x| x == "part") {
                    let _ = fs::remove_file(e.path());
                }
            }
        }
        Ok(Self { inner: Arc::new(Inner { dir, limit, open: Mutex::new(HashSet::new()) }) })
    }

    pub fn dir(&self) -> &Path {
        &self.inner.dir
    }

    fn complete_path(&self, key: &str) -> PathBuf {
        self.inner.dir.join(format!("{key}.audio"))
    }

    pub fn part_path(&self, key: &str) -> PathBuf {
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
        let prefix = format!("{}-", sanitize_key(video_id)?);
        let rd = fs::read_dir(&self.inner.dir).ok()?;
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if let Some(rest) = name.strip_prefix(&prefix).and_then(|r| r.strip_suffix(".audio")) {
                if let Ok(itag) = rest.parse::<u32>() {
                    let p = e.path();
                    touch(&p);
                    return Some((p, itag));
                }
            }
        }
        None
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
            }
        }
    }

    pub fn clear(&self) {
        let open = self.inner.open.lock().clone();
        for (p, _, _) in self.entries() {
            let key = p.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
            if !open.contains(&key) {
                let _ = fs::remove_file(p);
            }
        }
    }
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
