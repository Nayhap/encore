//! The downloads folder (D23), `<app data>/downloads/`:
//!
//! ```text
//! audio/<videoId>-<itag>.<m4a|webm>         finished songs
//! audio/<videoId>-<itag>.<ext>.json         what each one is (for recovery)
//! audio/<videoId>-<itag>.<ext>.part         being downloaded
//! art/<artKey>                              the 544 px cover
//! worker.lock                               held by the Encore that downloads
//! ```
//!
//! Names are ids, never titles. Folders are 0700 and files 0600 on Unix.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use encore_core::downloads::DownloadRow;
use encore_core::downloads::RowState;
use encore_core::model::Track;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug)]
pub struct Layout {
    pub root: PathBuf,
    pub audio: PathBuf,
    pub art: PathBuf,
}

fn private_dir(p: &Path) -> io::Result<()> {
    fs::create_dir_all(p)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(p, fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

/// Only this user can read a finished file.
pub fn private(p: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(p, fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = p;
}

impl Layout {
    pub fn create(root: PathBuf) -> io::Result<Self> {
        let l = Layout { audio: root.join("audio"), art: root.join("art"), root };
        private_dir(&l.root)?;
        private_dir(&l.audio)?;
        private_dir(&l.art)?;
        Ok(l)
    }

    pub fn file(&self, name: &str) -> PathBuf {
        self.audio.join(name)
    }

    pub fn part(&self, name: &str) -> PathBuf {
        self.audio.join(format!("{name}.part"))
    }

    pub fn sidecar(&self, name: &str) -> PathBuf {
        self.audio.join(format!("{name}.json"))
    }

    pub fn cover(&self, key: &str) -> PathBuf {
        self.art.join(key)
    }

    /// Partial files of a song, whatever format they were.
    pub fn parts_of(&self, video_id: &str) -> Vec<PathBuf> {
        let prefix = format!("{video_id}-");
        fs::read_dir(&self.audio)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".part")))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Every partial file, with the song it belongs to.
    pub fn all_parts(&self) -> Vec<(String, PathBuf)> {
        fs::read_dir(&self.audio)
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| {
                        let n = e.file_name().to_str()?.to_string();
                        if !n.ends_with(".part") {
                            return None;
                        }
                        let (vid, _) = n.rsplit_once('-')?;
                        Some((vid.to_string(), e.path()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// The container for a format: what YouTube serves for these itags (D5).
pub fn ext_for(itag: u32, mime: &str, codec: &str) -> &'static str {
    match itag {
        139 | 140 | 141 => "m4a",
        249 | 250 | 251 | 774 => "webm",
        _ if mime.contains("webm") || codec.starts_with("opus") => "webm",
        _ => "m4a",
    }
}

/// The container a file starts with (MP4 `ftyp` or WebM EBML).
pub fn sniff(p: &Path) -> Option<&'static str> {
    let mut head = [0u8; 8];
    File::open(p).ok()?.read_exact(&mut head).ok()?;
    if &head[4..8] == b"ftyp" {
        Some("m4a")
    } else if head[..4] == [0x1a, 0x45, 0xdf, 0xa3] {
        Some("webm")
    } else {
        None
    }
}

pub fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    {
        let mut f = File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    private(&tmp);
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

pub fn remove(p: &Path) -> bool {
    match fs::remove_file(p) {
        Ok(()) => true,
        Err(e) => e.kind() == io::ErrorKind::NotFound,
    }
}

pub fn sha256_file(p: &Path) -> io::Result<String> {
    let mut f = File::open(p)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// Copy a file the playback cache already has. The source is opened first,
/// so its eviction meanwhile doesn't matter on Unix; on Windows an open
/// file can't be evicted.
pub fn copy_from(src: &Path, dst: &Path) -> io::Result<u64> {
    let mut from = File::open(src)?;
    let mut to = OpenOptions::new().create(true).write(true).truncate(true).open(dst)?;
    let n = io::copy(&mut from, &mut to)?;
    to.sync_all()?;
    Ok(n)
}

pub fn dir_size(p: &Path) -> u64 {
    fs::read_dir(p).map(|rd| rd.flatten().filter_map(|e| e.metadata().ok()).filter(|m| m.is_file()).map(|m| m.len()).sum()).unwrap_or(0)
}

/// Bytes free for this user on the disk holding `dir`, when the system says.
#[cfg(unix)]
pub fn free_space(dir: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    // SAFETY: `c` is a valid C string and `st` a properly sized out value.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    Some((st.f_bavail as u64).saturating_mul(st.f_frsize as u64))
}

#[cfg(windows)]
pub fn free_space(dir: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut free: u64 = 0;
    // SAFETY: `wide` is NUL-terminated; the other outputs may be null.
    let ok = unsafe { windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut()) };
    if ok != 0 { Some(free) } else { None }
}

#[cfg(not(any(unix, windows)))]
pub fn free_space(_dir: &Path) -> Option<u64> {
    None
}

/// The Encore that downloads holds this, so the app and `encore-cli`
/// never fetch at the same time. The OS releases it if the process dies.
pub struct WorkerLock {
    _file: File,
}

impl WorkerLock {
    pub fn try_acquire(root: &Path) -> io::Result<Option<Self>> {
        let f = OpenOptions::new().create(true).truncate(false).write(true).open(root.join("worker.lock"))?;
        match f.try_lock() {
            Ok(()) => Ok(Some(WorkerLock { _file: f })),
            Err(fs::TryLockError::WouldBlock) => Ok(None),
            Err(fs::TryLockError::Error(e)) => Err(e),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SidecarArt {
    pub key: String,
    pub base: String,
    pub hue: Option<(u16, u16)>,
}

/// What a finished file is, written next to it. If the index is ever lost
/// (`Store::open` sets a damaged database aside), the songs are found again
/// from these.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sidecar {
    pub track: Track,
    pub itag: u32,
    pub ext: String,
    pub codec: String,
    pub kbps: Option<u32>,
    pub bytes: u64,
    pub sha256: String,
    pub loudness: Option<f32>,
    pub duration: Option<f64>,
    pub art: Option<SidecarArt>,
    pub added: i64,
}

impl Sidecar {
    pub fn of(r: &DownloadRow) -> Option<Self> {
        let art = match (&r.art_key, &r.art_base) {
            (Some(k), Some(b)) => Some(SidecarArt { key: k.clone(), base: b.clone(), hue: r.art_hue }),
            _ => None,
        };
        Some(Sidecar {
            track: r.track.clone(),
            itag: r.itag?,
            ext: r.ext.clone()?,
            codec: r.codec.clone().unwrap_or_default(),
            kbps: r.kbps,
            bytes: r.bytes,
            sha256: r.sha256.clone()?,
            loudness: r.loudness,
            duration: r.duration,
            art,
            added: r.added,
        })
    }

    pub fn row(&self) -> DownloadRow {
        DownloadRow {
            video_id: self.track.id.remote().to_string(),
            track: self.track.clone(),
            state: RowState::Ready,
            itag: Some(self.itag),
            ext: Some(self.ext.clone()),
            codec: Some(self.codec.clone()),
            kbps: self.kbps,
            total: Some(self.bytes),
            bytes: self.bytes,
            sha256: Some(self.sha256.clone()),
            loudness: self.loudness,
            duration: self.duration,
            art_key: self.art.as_ref().map(|a| a.key.clone()),
            art_base: self.art.as_ref().map(|a| a.base.clone()),
            art_hue: self.art.as_ref().and_then(|a| a.hue),
            art_pinned: false,
            error: None,
            attempts: 0,
            attempt: 0,
            version: 0,
            added: self.added,
            done_at: None,
            retry_at: None,
        }
    }
}

/// Sidecars whose file is there, complete.
pub fn scan_sidecars(audio: &Path) -> Vec<Sidecar> {
    let Ok(rd) = fs::read_dir(audio) else { return vec![] };
    rd.flatten()
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            let media = name.strip_suffix(".json")?;
            let s: Sidecar = serde_json::from_slice(&fs::read(e.path()).ok()?).ok()?;
            let expected = format!("{}-{}.{}", s.track.id.remote(), s.itag, s.ext);
            let len = fs::metadata(audio.join(media)).ok()?.len();
            (media == expected && len == s.bytes).then_some(s)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lock_is_held_once() {
        let dir = std::env::temp_dir().join(format!("encore-dl-lock-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let a = WorkerLock::try_acquire(&dir).unwrap();
        assert!(a.is_some());
        assert!(WorkerLock::try_acquire(&dir).unwrap().is_none(), "a second worker waits");
        drop(a);
        assert!(WorkerLock::try_acquire(&dir).unwrap().is_some());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn containers_follow_the_format() {
        assert_eq!(ext_for(140, "audio/mp4", "aac"), "m4a");
        assert_eq!(ext_for(251, "audio/webm", "opus"), "webm");
        assert_eq!(ext_for(9999, "audio/webm", "opus"), "webm");
    }
}
