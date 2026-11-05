//! Solved challenges, per player, and preprocessed players on disk (D37).
//!
//! Upstream: yt-dlp `_video.py` `_load_player_data_from_cache` /
//! `_store_player_data_to_cache` (2208-2227): `n` answers per challenge,
//! signature permutations (`sigfuncs`) per signature length, keyed by the
//! player. Here the key also holds the player's content hash and the
//! solver's and engine's versions, so a new ejs or QuickJS never reuses an
//! old answer. Kept in memory only, for the last few players, bounded.
//!
//! The preprocessed player (`PreprocessedDisk`) is kept on disk too, beside
//! the player scripts, so the first song after a launch doesn't parse the
//! player again. Only that text is written: no bytecode, no challenge or
//! answer, no cookie, URL or account.

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::path::PathBuf;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

const PLAYERS_KEPT: usize = 3;
const N_PER_PLAYER: usize = 512;

#[derive(Default)]
struct Entry {
    n: HashMap<String, String>,
    n_order: VecDeque<String>,
    sig: HashMap<usize, Vec<usize>>,
}

#[derive(Default)]
pub struct SolvedCache {
    players: Mutex<VecDeque<(String, Entry)>>,
}

pub fn key(player_id: &str, code: &str) -> String {
    format!("{player_id}:{:016x}:ejs-{}:{}", crate::fxhash(code), super::ejs::EJS_VERSION, super::quickjs::ENGINE)
}

impl SolvedCache {
    fn with<R>(&self, key: &str, f: impl FnOnce(&mut Entry) -> R) -> R {
        let mut q = self.players.lock();
        if let Some(i) = q.iter().position(|(k, _)| k == key) {
            let e = q.remove(i).expect("entry");
            q.push_back(e);
        } else {
            q.push_back((key.to_string(), Entry::default()));
            while q.len() > PLAYERS_KEPT {
                q.pop_front();
            }
        }
        f(&mut q.back_mut().expect("entry").1)
    }

    pub fn n(&self, key: &str, challenge: &str) -> Option<String> {
        self.with(key, |e| e.n.get(challenge).cloned())
    }

    pub fn put_n(&self, key: &str, challenge: &str, answer: &str) {
        self.with(key, |e| {
            if e.n.insert(challenge.to_string(), answer.to_string()).is_none() {
                e.n_order.push_back(challenge.to_string());
                while e.n_order.len() > N_PER_PLAYER {
                    if let Some(old) = e.n_order.pop_front() {
                        e.n.remove(&old);
                    }
                }
            }
        })
    }

    pub fn sig(&self, key: &str, len: usize) -> Option<Vec<usize>> {
        self.with(key, |e| e.sig.get(&len).cloned())
    }

    pub fn put_sig(&self, key: &str, len: usize, perm: Vec<usize>) {
        self.with(key, |e| {
            e.sig.insert(len, perm);
        })
    }

    pub fn clear(&self) {
        self.players.lock().clear();
    }
}

/// The layout of a preprocessed file; another one is a miss.
const SCHEMA: u32 = 1;
/// Preprocessed players kept on disk, newest first.
pub const PREPROCESSED_FILES: usize = 3;
/// A preprocessed player is about 4.2 MB for the players YouTube serves
/// now, 3.7 MB at most in the solver corpus (tests/ejs_players.rs prints
/// it); one larger than this isn't written or read.
pub const PREPROCESSED_MAX_BYTES: u64 = 8 << 20;
/// All of them together.
const PREPROCESSED_TOTAL_BYTES: u64 = 24 << 20;
const SUFFIX: &str = ".pre";
/// The header line is short; a longer one isn't ours.
const HEADER_MAX: u64 = 1024;

/// What a preprocessed file was made from: it is used only for exactly that
/// player, ejs and engine.
#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Header {
    schema: u32,
    player: String,
    sha3_256: String,
    ejs: String,
    engine: String,
    bytes: u64,
}

impl Header {
    fn of(player_id: &str, code: &str, bytes: u64) -> Self {
        use sha3::Digest;
        Header { schema: SCHEMA, player: player_id.to_string(), sha3_256: hex::encode(sha3::Sha3_256::digest(code.as_bytes())), ejs: super::ejs::EJS_VERSION.to_string(), engine: super::quickjs::ENGINE.to_string(), bytes }
    }
}

/// Preprocessed players in `<cache>/player/`, one file per player and
/// variant (`<id>-<variant>.pre`): a header line naming what it was made
/// from, then the text. Blocking: call these off the async runtime.
pub struct PreprocessedDisk {
    dir: PathBuf,
}

impl PreprocessedDisk {
    pub fn new(dir: PathBuf) -> Self {
        PreprocessedDisk { dir }
    }

    /// `9fcf08e8/main` → `9fcf08e8-main.pre`; nothing for an id that
    /// isn't a plain one.
    fn file(&self, player_id: &str) -> Option<PathBuf> {
        let name: String = player_id.chars().map(|c| if c == '/' { '-' } else { c }).collect();
        let plain = !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        plain.then(|| self.dir.join(format!("{name}{SUFFIX}")))
    }

    /// The preprocessed text for this player's `code`, if a file made from
    /// exactly it, this ejs and this engine is there and whole.
    pub fn read(&self, player_id: &str, code: &str) -> Option<String> {
        let path = self.file(player_id)?;
        let meta = std::fs::symlink_metadata(&path).ok()?;
        if !meta.is_file() || meta.len() > PREPROCESSED_MAX_BYTES + HEADER_MAX {
            return None;
        }
        let mut raw = Vec::with_capacity(meta.len() as usize);
        std::fs::File::open(&path).ok()?.take(PREPROCESSED_MAX_BYTES + HEADER_MAX).read_to_end(&mut raw).ok()?;
        let nl = raw.iter().take(HEADER_MAX as usize).position(|&b| b == b'\n')?;
        let header: Header = serde_json::from_slice(&raw[..nl]).ok()?;
        let body = &raw[nl + 1..];
        if header != Header::of(player_id, code, body.len() as u64) {
            return None;
        }
        String::from_utf8(body.to_vec()).ok()
    }

    /// Keep `pre` for this player's `code`: by temp file and rename, then
    /// only the newest few, within the total. Too large is not kept.
    pub fn write(&self, player_id: &str, code: &str, pre: &str) {
        let Some(path) = self.file(player_id) else { return };
        if pre.len() as u64 > PREPROCESSED_MAX_BYTES || std::fs::create_dir_all(&self.dir).is_err() {
            return;
        }
        let Ok(mut header) = serde_json::to_vec(&Header::of(player_id, code, pre.len() as u64)) else { return };
        header.push(b'\n');
        let tmp = path.with_extension("pre-part");
        let written = std::fs::File::create(&tmp).and_then(|mut f| {
            f.write_all(&header)?;
            f.write_all(pre.as_bytes())?;
            f.sync_all()
        });
        if written.is_err() || std::fs::rename(&tmp, &path).is_err() {
            let _ = std::fs::remove_file(&tmp);
            return;
        }
        self.prune();
    }

    /// A copy that failed to solve with: it goes, and the next solve is cold.
    pub fn remove(&self, player_id: &str) {
        if let Some(p) = self.file(player_id) {
            let _ = std::fs::remove_file(p);
        }
    }

    /// Settings → Reset player cache, with any write a crash left half done.
    pub fn clear(&self) {
        for (_, _, p) in self.files() {
            let _ = std::fs::remove_file(p);
        }
        for e in std::fs::read_dir(&self.dir).into_iter().flatten().flatten() {
            if e.file_name().to_string_lossy().ends_with(".pre-part") && e.file_type().is_ok_and(|t| t.is_file()) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }

    /// Our files, newest first.
    fn files(&self) -> Vec<(std::time::SystemTime, u64, PathBuf)> {
        let mut v: Vec<_> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(SUFFIX) && e.file_type().is_ok_and(|t| t.is_file()))
            .filter_map(|e| {
                let m = e.metadata().ok()?;
                Some((m.modified().ok()?, m.len(), e.path()))
            })
            .collect();
        v.sort_by_key(|f| std::cmp::Reverse(f.0));
        v
    }

    fn prune(&self) {
        let mut total = 0;
        for (i, (_, len, p)) in self.files().into_iter().enumerate() {
            total += len;
            if i >= PREPROCESSED_FILES || total > PREPROCESSED_TOTAL_BYTES {
                let _ = std::fs::remove_file(p);
            }
        }
    }

    #[cfg(test)]
    fn dir(&self) -> &std::path::Path {
        &self.dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preprocessed_player_is_used_only_for_what_it_was_made_from() {
        let tmp = tempfile::tempdir().unwrap();
        let d = PreprocessedDisk::new(tmp.path().join("player"));
        assert_eq!(d.read("9fcf08e8/main", "code"), None);
        d.write("9fcf08e8/main", "code", "pre");
        assert!(d.dir().join("9fcf08e8-main.pre").is_file());
        assert_eq!(d.read("9fcf08e8/main", "code").as_deref(), Some("pre"));
        assert_eq!(d.read("9fcf08e8/main", "other code"), None, "another player under the same id");
        assert_eq!(d.read("9fcf08e8/tv", "code"), None);
        // Another ejs or engine, or a cut-off or altered file, is a miss.
        let path = d.dir().join("9fcf08e8-main.pre");
        let raw = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, raw.replace(super::super::ejs::EJS_VERSION, "0.0.1")).unwrap();
        assert_eq!(d.read("9fcf08e8/main", "code"), None);
        std::fs::write(&path, &raw[..raw.len() - 1]).unwrap();
        assert_eq!(d.read("9fcf08e8/main", "code"), None);
        std::fs::write(&path, "not a header").unwrap();
        assert_eq!(d.read("9fcf08e8/main", "code"), None);
        // An id that isn't plain never names a file.
        d.write("../../x", "code", "pre");
        assert_eq!(d.read("../../x", "code"), None);
        assert!(!tmp.path().join("x.pre").exists());
        d.remove("9fcf08e8/main");
        assert!(!path.exists());
    }

    #[test]
    fn few_and_small_are_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let d = PreprocessedDisk::new(tmp.path().to_path_buf());
        // A player script beside them is never touched.
        std::fs::write(tmp.path().join("9fcf08e8-main.js"), "player").unwrap();
        for i in 0..5 {
            d.write(&format!("{i:08x}/main"), "code", "pre");
            // Distinct times, oldest first.
            let f = std::fs::File::options().append(true).open(tmp.path().join(format!("{i:08x}-main.pre"))).unwrap();
            f.set_modified(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000 + i)).unwrap();
        }
        d.prune();
        let kept: Vec<_> = d.files().into_iter().map(|f| f.2.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert_eq!(kept, ["00000004-main.pre", "00000003-main.pre", "00000002-main.pre"]);
        let big = "x".repeat(PREPROCESSED_MAX_BYTES as usize + 1);
        d.write("0000000a/main", "code", &big);
        assert_eq!(d.read("0000000a/main", "code"), None, "too large isn't kept");
        d.clear();
        assert!(d.files().is_empty());
        assert!(tmp.path().join("9fcf08e8-main.js").exists());
    }

    #[test]
    fn answers_are_kept_per_player_and_bounded() {
        let c = SolvedCache::default();
        c.put_n("a", "x", "y");
        c.put_sig("a", 3, vec![2, 1, 0]);
        assert_eq!(c.n("a", "x").as_deref(), Some("y"));
        assert_eq!(c.sig("a", 3), Some(vec![2, 1, 0]));
        assert_eq!(c.n("b", "x"), None);
        for i in 0..N_PER_PLAYER + 10 {
            c.put_n("a", &i.to_string(), "z");
        }
        assert_eq!(c.n("a", "x"), None, "the oldest answer went");
        for p in ["c", "d", "e"] {
            c.n(p, "x");
        }
        assert_eq!(c.sig("a", 3), None, "the oldest player went");
        assert!(key("9fcf08e8/main", "code").contains("ejs-0.8.0:quickjs-ng"));
    }
}
