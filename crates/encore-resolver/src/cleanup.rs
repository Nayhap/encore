//! One-time removal of the D22 player components (D37).
//!
//! Encore 0.5 downloaded Python, Deno and yt-dlp into `<data>/runtimes/`
//! and let yt-dlp and Deno cache into `<cache>/yt-dlp/` and
//! `<cache>/deno/`. The native resolver needs none of it. This removes
//! exactly what 0.5 made, and nothing else:
//!
//! * under 0.5's own cross-process lock (`runtimes.lock`), taken the way
//!   0.5 takes it (created if missing, never deleted), and held until done
//!   is recorded, so an older Encore still installing is never pulled from
//!   under; while it is held or can't be opened, nothing is deleted and
//!   the next launch tries again;
//! * only entries with exactly the names 0.5 made (a component folder is
//!   `python|deno|yt-dlp-<version>-<10 hex digits>`, its fingerprint);
//!   anything else in the folder is left, and so is the folder then; in the
//!   older `resolver/` layout only its `venv/` and helper script;
//! * a symlink (or a Windows junction, which `std` reports as one) is never
//!   followed: a linked entry is left alone;
//! * a file that can't be deleted (in use on Windows) leaves the job
//!   unfinished, for the next launch;
//! * done is recorded outside the folders removed (`runtimes.removed`).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Marks that the cleanup finished (in the data folder).
const DONE: &str = "runtimes.removed";

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    /// What was deleted, relative to its folder.
    pub removed: Vec<String>,
    pub bytes: u64,
    /// Everything recognised is gone (or nothing was there).
    pub complete: bool,
    /// Left for the next launch: the lock was held or a file was in use.
    pub retry: bool,
}

/// `Component::id()` of 0.5: `<slug>-<version>-<first 10 hex digits of
/// the fingerprint>`.
fn is_component_dir(name: &str) -> bool {
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"^(python|deno|yt-dlp)-[0-9][0-9A-Za-z.+_]*-[0-9a-f]{10}$").unwrap());
    RE.is_match(name)
}

/// The names 0.5 put in `runtimes/` (`runtimes/mod.rs` layout, and
/// `write_atomic`'s `<stem>.tmp<pid>` while it wrote one).
fn owned(name: &str, is_dir: bool) -> bool {
    static TMP: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"^(active|approved|encore_resolver)\.tmp[0-9]+$").unwrap());
    if is_dir {
        is_component_dir(name) || name == ".downloads" || name == ".staging"
    } else {
        matches!(name, "active.json" | "approved" | "encore_resolver.py") || TMP.is_match(name)
    }
}

fn size(p: &Path) -> u64 {
    match fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => fs::read_dir(p).into_iter().flatten().flatten().map(|e| size(&e.path())).sum(),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

/// Delete `p` if it is a real directory or file (never through a link).
fn remove(p: &Path, label: String, report: &mut Report) -> io::Result<()> {
    let m = match fs::symlink_metadata(p) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if m.file_type().is_symlink() {
        tracing::info!(path = %label, "runtime cleanup: left a link alone");
        return Ok(());
    }
    let bytes = size(p);
    // The marker first, so a half-deleted folder never reads as complete.
    if m.is_dir() {
        let _ = fs::remove_file(p.join(".encore-ready"));
        fs::remove_dir_all(p)?;
    } else {
        fs::remove_file(p)?;
    }
    report.bytes += bytes;
    report.removed.push(label);
    Ok(())
}

pub fn run(data_dir: &Path, cache_dir: &Path) -> Report {
    let mut report = Report::default();
    let done = data_dir.join(DONE);
    if done.exists() {
        report.complete = true;
        return report;
    }
    let root = data_dir.join("runtimes");
    let legacy = data_dir.join("resolver");
    let lock_path = data_dir.join("runtimes.lock");
    let caches = [cache_dir.join("yt-dlp"), cache_dir.join("deno")];
    let anything = [&root, &legacy, &lock_path].into_iter().chain(caches.iter()).any(|p| fs::symlink_metadata(p).is_ok());
    // Never ran 0.5 (0.5 makes the lock before anything else): nothing to do.
    if !anything {
        report.complete = fs::write(&done, b"").is_ok();
        return report;
    }
    // 0.5's lock, opened as 0.5 opens it, and held until the end.
    let lock = match fs::OpenOptions::new().create(true).truncate(false).write(true).open(&lock_path) {
        Ok(f) => f,
        Err(e) => {
            tracing::info!(error = %e.kind(), "runtime cleanup: couldn't open the install lock; trying at the next launch");
            report.retry = true;
            return report;
        }
    };
    if lock.try_lock().is_err() {
        tracing::info!("runtime cleanup: an older Encore holds the install lock; trying at the next launch");
        report.retry = true;
        return report;
    }
    let mut failed = false;
    let mut note = |r: io::Result<()>, what: &str| {
        if let Err(e) = r {
            tracing::warn!(path = what, error = %e.kind(), "runtime cleanup: could not remove");
            failed = true;
        }
    };
    let sweep = |dir: &Path, label: &str, keep: &dyn Fn(&str, bool) -> bool, report: &mut Report, note: &mut dyn FnMut(io::Result<()>, &str)| {
        match fs::symlink_metadata(dir) {
            Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {
                let mut entries: Vec<(String, PathBuf)> = Vec::new();
                for e in fs::read_dir(dir).into_iter().flatten().flatten() {
                    let name = e.file_name().to_string_lossy().to_string();
                    let Ok(ft) = e.file_type() else { continue };
                    if !ft.is_symlink() && keep(&name, ft.is_dir()) {
                        entries.push((name, e.path()));
                    }
                }
                // active.json goes first: nothing then names a set that is
                // half gone.
                entries.sort_by_key(|(n, _)| n != "active.json");
                for (name, path) in entries {
                    let r = remove(&path, format!("{label}/{name}"), report);
                    note(r, &name);
                }
                // Only once empty: unknown files keep the folder.
                if fs::remove_dir(dir).is_ok() {
                    report.removed.push(label.into());
                }
            }
            Ok(_) => tracing::info!(path = label, "runtime cleanup: not a plain folder; left alone"),
            Err(_) => {}
        }
    };
    sweep(&root, "runtimes", &owned, &mut report, &mut note);
    // The layout before D22: a venv and the helper script.
    sweep(&legacy, "resolver", &|n: &str, d: bool| (d && n == "venv") || (!d && n == "encore_resolver.py"), &mut report, &mut note);
    // yt-dlp's and Deno's caches: Encore's own folders, with nothing else in them.
    for (p, name) in caches.iter().zip(["yt-dlp", "deno"]) {
        let r = remove(p, format!("cache/{name}"), &mut report);
        note(r, name);
    }
    if failed {
        report.retry = true;
    } else {
        // Recorded while the lock is still held; the lock file stays, as 0.5
        // never deletes it either.
        report.complete = fs::write(&done, b"").is_ok();
    }
    drop(lock);
    if !report.removed.is_empty() {
        tracing::info!(removed = ?report.removed, mb = report.bytes / 1_000_000, complete = report.complete, "runtime cleanup: removed the old player components");
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(base: &Path) -> (PathBuf, PathBuf) {
        let data = base.join("data");
        let cache = base.join("cache");
        let rt = data.join("runtimes");
        for d in ["python-3.13.18-3ed3c1ed12/bin", "deno-2.9.8-1729cc1d06", "yt-dlp-2026.10.26-a7e6f45711/yt_dlp", ".staging/x", ".downloads"] {
            fs::create_dir_all(rt.join(d)).unwrap();
        }
        fs::write(rt.join("python-3.13.18-3ed3c1ed12/.encore-ready"), "fp").unwrap();
        fs::write(rt.join("python-3.13.18-3ed3c1ed12/bin/python3"), vec![0u8; 1000]).unwrap();
        fs::write(rt.join("active.json"), "{}").unwrap();
        fs::write(rt.join("approved"), "").unwrap();
        fs::write(rt.join("encore_resolver.py"), "#").unwrap();
        fs::write(data.join("runtimes.lock"), "").unwrap();
        fs::create_dir_all(cache.join("yt-dlp/youtube-sigfuncs")).unwrap();
        fs::create_dir_all(cache.join("deno")).unwrap();
        // Not ours: kept.
        fs::create_dir_all(cache.join("audio")).unwrap();
        fs::write(data.join("encore.sqlite"), "db").unwrap();
        fs::create_dir_all(data.join("downloads")).unwrap();
        (data, cache)
    }

    #[test]
    fn removes_what_0_5_made_and_nothing_else() {
        let t = tempfile::tempdir().unwrap();
        let (data, cache) = tree(t.path());
        let r = run(&data, &cache);
        assert!(r.complete && !r.retry, "{r:?}");
        assert!(r.bytes >= 1000);
        assert!(!data.join("runtimes").exists());
        assert!(data.join("runtimes.lock").exists(), "the lock file is never deleted");
        assert!(!cache.join("yt-dlp").exists() && !cache.join("deno").exists());
        assert!(cache.join("audio").exists() && data.join("encore.sqlite").exists() && data.join("downloads").exists());
        assert!(data.join(DONE).exists());
        // Once done, never again.
        fs::create_dir_all(cache.join("deno")).unwrap();
        assert!(run(&data, &cache).removed.is_empty());
        assert!(cache.join("deno").exists());
    }

    #[test]
    fn names_0_5_never_made_are_left() {
        let t = tempfile::tempdir().unwrap();
        let (data, cache) = tree(t.path());
        for d in ["python-backup", "yt-dlp-notes", "deno-2.9.8-XYZ"] {
            fs::create_dir_all(data.join("runtimes").join(d)).unwrap();
        }
        fs::write(data.join("runtimes/mine.tmp"), "x").unwrap();
        fs::write(data.join("runtimes/active.tmp4242"), "x").unwrap();
        fs::create_dir_all(data.join("resolver/venv/bin")).unwrap();
        fs::write(data.join("resolver/encore_resolver.py"), "#").unwrap();
        fs::write(data.join("resolver/keep.txt"), "mine").unwrap();
        let r = run(&data, &cache);
        assert!(r.complete, "{r:?}");
        for d in ["python-backup", "yt-dlp-notes", "deno-2.9.8-XYZ", "mine.tmp"] {
            assert!(data.join("runtimes").join(d).exists(), "{d} was removed");
        }
        assert!(!data.join("runtimes/active.tmp4242").exists());
        assert!(!data.join("resolver/venv").exists() && !data.join("resolver/encore_resolver.py").exists());
        assert!(data.join("resolver/keep.txt").exists());
    }

    #[test]
    fn a_missing_lock_is_made_and_taken_first() {
        let t = tempfile::tempdir().unwrap();
        let (data, cache) = tree(t.path());
        fs::remove_file(data.join("runtimes.lock")).unwrap();
        let r = run(&data, &cache);
        assert!(r.complete && !data.join("runtimes").exists());
        assert!(data.join("runtimes.lock").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_wont_go_leaves_the_job_for_the_next_launch() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let (data, cache) = tree(t.path());
        // Like a file Windows keeps open: its folder can't be emptied.
        let stuck = data.join("runtimes/deno-2.9.8-1729cc1d06");
        fs::write(stuck.join("deno"), "x").unwrap();
        fs::set_permissions(&stuck, fs::Permissions::from_mode(0o500)).unwrap();
        let r = run(&data, &cache);
        fs::set_permissions(&stuck, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(r.retry && !r.complete, "{r:?}");
        assert!(!data.join(DONE).exists());
        let r = run(&data, &cache);
        assert!(r.complete && !data.join("runtimes").exists(), "{r:?}");
    }

    #[test]
    fn unknown_files_keep_the_folder() {
        let t = tempfile::tempdir().unwrap();
        let (data, cache) = tree(t.path());
        fs::write(data.join("runtimes/notes.txt"), "mine").unwrap();
        let r = run(&data, &cache);
        assert!(r.complete);
        assert!(data.join("runtimes/notes.txt").exists());
        assert!(!data.join("runtimes/active.json").exists() && !data.join("runtimes/deno-2.9.8-1729cc1d06").exists());
    }

    #[cfg(unix)]
    #[test]
    fn links_are_never_followed() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("data");
        let cache = t.path().join("cache");
        let elsewhere = t.path().join("elsewhere");
        fs::create_dir_all(elsewhere.join("python-3.13-x")).unwrap();
        fs::create_dir_all(&data).unwrap();
        fs::create_dir_all(&cache).unwrap();
        std::os::unix::fs::symlink(&elsewhere, data.join("runtimes")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, cache.join("deno")).unwrap();
        let r = run(&data, &cache);
        assert!(r.complete);
        assert!(elsewhere.join("python-3.13-x").exists());
    }

    #[test]
    fn a_held_lock_waits_for_the_next_launch() {
        let t = tempfile::tempdir().unwrap();
        let (data, cache) = tree(t.path());
        let held = fs::OpenOptions::new().write(true).open(data.join("runtimes.lock")).unwrap();
        held.lock().unwrap();
        let r = run(&data, &cache);
        assert!(r.retry && !r.complete && r.removed.is_empty());
        assert!(data.join("runtimes/active.json").exists());
        drop(held);
        assert!(run(&data, &cache).complete);
    }

    #[test]
    fn nothing_there_is_done() {
        let t = tempfile::tempdir().unwrap();
        let r = run(t.path(), t.path());
        assert!(r.complete && r.removed.is_empty());
        assert!(!t.path().join("runtimes.lock").exists(), "a fresh install gets no lock file");
    }
}
