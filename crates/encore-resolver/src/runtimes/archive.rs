//! Safe unpacking of verified archives (D22).
//!
//! Archives are unpacked into a fresh, empty folder. Every entry path must be
//! relative and made only of normal components (no `..`, no root, no drive
//! prefix). Symbolic links may only point down (relative, no `..`), so no
//! chain of links can lead outside the folder; hard links must name an entry
//! already unpacked. Devices, FIFOs and anything else are refused. Sizes and
//! entry counts are capped so a hostile archive can't fill the disk.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

/// More than any pinned archive unpacks to.
const MAX_BYTES: u64 = 1 << 30;
const MAX_ENTRIES: usize = 50_000;

#[derive(Debug)]
pub enum UnpackError {
    /// The archive is malformed or tries something unsafe.
    Bad(String),
    Io(io::Error),
    Cancelled,
}

impl From<io::Error> for UnpackError {
    fn from(e: io::Error) -> Self {
        UnpackError::Io(e)
    }
}

impl std::fmt::Display for UnpackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnpackError::Bad(m) => write!(f, "{m}"),
            UnpackError::Io(e) => write!(f, "{e}"),
            UnpackError::Cancelled => write!(f, "cancelled"),
        }
    }
}

/// What the caller wants done while unpacking.
pub struct Hooks<'a> {
    /// Return true to leave an entry out (its path's components).
    pub skip: &'a dyn Fn(&[&str]) -> bool,
    /// Called with bytes written so far; return false to stop.
    pub progress: &'a mut dyn FnMut(u64) -> bool,
}

/// A path inside the destination, or `None` if it isn't a plain relative one.
pub fn safe_relative(p: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::Normal(s) => {
                let s = s.to_str()?;
                // Windows would read these as a drive or stream, or trim them.
                if s.contains(':') || s.contains('\\') || s.ends_with('.') || s.ends_with(' ') {
                    return None;
                }
                out.push(s);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// A symlink target that can only point at or below the link's own folder.
fn safe_link_target(t: &Path) -> bool {
    !t.as_os_str().is_empty() && t.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

fn parts(p: &Path) -> Vec<&str> {
    p.components().filter_map(|c| c.as_os_str().to_str()).collect()
}

fn ensure_empty_dir(dest: &Path) -> Result<(), UnpackError> {
    fs::create_dir_all(dest)?;
    if fs::read_dir(dest)?.next().is_some() {
        return Err(UnpackError::Bad("destination is not empty".into()));
    }
    Ok(())
}

/// Create `path`'s parent folders and remove whatever is at `path` already
/// (a duplicate entry), without following links.
fn prepare(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Ok(m) = fs::symlink_metadata(path) {
        if m.is_dir() && !m.file_type().is_symlink() {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

/// Copy at most `limit` bytes; more means the archive lied about the size.
fn copy_capped(r: &mut dyn Read, w: &mut dyn Write, limit: u64, written: &mut u64, hooks: &mut Hooks) -> Result<(), UnpackError> {
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        *written += n as u64;
        if *written > limit {
            return Err(UnpackError::Bad("archive unpacks to more than expected".into()));
        }
        w.write_all(&buf[..n])?;
        if !(hooks.progress)(*written) {
            return Err(UnpackError::Cancelled);
        }
    }
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    // Keep the executable bits; never setuid, setgid, sticky or world-writable.
    let mode = (mode & 0o755) | 0o600;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_: &Path, _: u32) -> io::Result<()> {
    Ok(())
}

/// Unpack a `.tar.gz` into the empty folder `dest`. Returns bytes written.
pub fn unpack_tar_gz(archive: &Path, dest: &Path, hooks: &mut Hooks) -> Result<u64, UnpackError> {
    ensure_empty_dir(dest)?;
    let gz = flate2::read::GzDecoder::new(io::BufReader::new(File::open(archive)?));
    let mut tar = tar::Archive::new(gz);
    let mut written = 0u64;
    let mut count = 0usize;
    let entries = tar.entries().map_err(|e| UnpackError::Bad(format!("not a tar archive: {e}")))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| UnpackError::Bad(format!("damaged archive: {e}")))?;
        count += 1;
        if count > MAX_ENTRIES {
            return Err(UnpackError::Bad("archive has too many entries".into()));
        }
        let raw = entry.path().map_err(|_| UnpackError::Bad("unreadable entry name".into()))?.into_owned();
        let Some(rel) = safe_relative(&raw) else { return Err(UnpackError::Bad(format!("unsafe path in archive: {}", raw.display()))) };
        if (hooks.skip)(&parts(&rel)) {
            continue;
        }
        let out = dest.join(&rel);
        let kind = entry.header().entry_type();
        match kind {
            tar::EntryType::Directory => fs::create_dir_all(&out)?,
            tar::EntryType::Regular | tar::EntryType::Continuous => {
                prepare(&out)?;
                let mode = entry.header().mode().unwrap_or(0o644);
                let mut f = File::create(&out)?;
                copy_capped(&mut entry, &mut f, MAX_BYTES, &mut written, hooks)?;
                f.flush()?;
                drop(f);
                set_mode(&out, mode)?;
            }
            tar::EntryType::Symlink => {
                let target = entry.link_name().ok().flatten().map(|t| t.into_owned()).ok_or_else(|| UnpackError::Bad("symlink without a target".into()))?;
                if !safe_link_target(&target) {
                    return Err(UnpackError::Bad(format!("unsafe link in archive: {} -> {}", rel.display(), target.display())));
                }
                prepare(&out)?;
                #[cfg(unix)]
                std::os::unix::fs::symlink(&target, &out)?;
                // Windows: none of the pinned archives has links; leave it out.
            }
            tar::EntryType::Link => {
                let target = entry.link_name().ok().flatten().map(|t| t.into_owned()).ok_or_else(|| UnpackError::Bad("hard link without a target".into()))?;
                let Some(trel) = safe_relative(&target) else { return Err(UnpackError::Bad(format!("unsafe hard link: {}", target.display()))) };
                let src = dest.join(&trel);
                let m = fs::symlink_metadata(&src).map_err(|_| UnpackError::Bad(format!("hard link to a missing entry: {}", trel.display())))?;
                if !m.is_file() {
                    return Err(UnpackError::Bad("hard link to something that is not a file".into()));
                }
                prepare(&out)?;
                fs::copy(&src, &out)?;
                written += m.len();
                if written > MAX_BYTES {
                    return Err(UnpackError::Bad("archive unpacks to more than expected".into()));
                }
            }
            // Metadata records the tar crate already applied to the next entry.
            tar::EntryType::XGlobalHeader | tar::EntryType::XHeader | tar::EntryType::GNULongName | tar::EntryType::GNULongLink => {}
            other => return Err(UnpackError::Bad(format!("unsupported entry type in archive: {other:?}"))),
        }
    }
    Ok(written)
}

/// Unpack a zip (or a wheel, skipping its `*.data/` folder) into the empty
/// folder `dest`. Links are refused.
pub fn unpack_zip(archive: &Path, dest: &Path, wheel: bool, hooks: &mut Hooks) -> Result<u64, UnpackError> {
    ensure_empty_dir(dest)?;
    let mut zip = zip::ZipArchive::new(io::BufReader::new(File::open(archive)?)).map_err(|e| UnpackError::Bad(format!("not a zip archive: {e}")))?;
    if zip.len() > MAX_ENTRIES {
        return Err(UnpackError::Bad("archive has too many entries".into()));
    }
    let mut written = 0u64;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|e| UnpackError::Bad(format!("damaged archive: {e}")))?;
        let name = f.name().map_err(|e| UnpackError::Bad(format!("unreadable entry name: {e}")))?.to_string();
        let Some(rel) = safe_relative(Path::new(&name)) else { return Err(UnpackError::Bad(format!("unsafe path in archive: {name}"))) };
        let ps = parts(&rel);
        if wheel && ps.first().is_some_and(|p| p.ends_with(".data")) {
            continue;
        }
        if (hooks.skip)(&ps) {
            continue;
        }
        if f.is_symlink() {
            return Err(UnpackError::Bad(format!("links are not allowed: {name}")));
        }
        let out = dest.join(&rel);
        if f.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        prepare(&out)?;
        let mode = f.unix_mode();
        let mut w = File::create(&out)?;
        copy_capped(&mut f, &mut w, MAX_BYTES, &mut written, hooks)?;
        w.flush()?;
        drop(w);
        if let Some(m) = mode {
            set_mode(&out, m)?;
        }
    }
    Ok(written)
}

/// Remove macOS's quarantine flag from everything under `root`, so the
/// unpacked programs run without a Gatekeeper prompt. Encore's own
/// downloads don't get the flag, but a copied or restored folder might.
#[cfg(target_os = "macos")]
pub fn clear_quarantine(root: &Path) {
    use std::os::unix::ffi::OsStrExt;
    fn walk(p: &Path) {
        let Ok(c) = std::ffi::CString::new(p.as_os_str().as_bytes()) else { return };
        // SAFETY: both arguments are valid NUL-terminated strings.
        unsafe {
            libc::removexattr(c.as_ptr(), c"com.apple.quarantine".as_ptr(), libc::XATTR_NOFOLLOW);
        }
        if let Ok(m) = fs::symlink_metadata(p) {
            if m.is_dir() {
                if let Ok(rd) = fs::read_dir(p) {
                    for e in rd.flatten() {
                        walk(&e.path());
                    }
                }
            }
        }
    }
    walk(root);
}

#[cfg(not(target_os = "macos"))]
pub fn clear_quarantine(_: &Path) {}

/// Bytes used by everything under `p` (links not followed).
pub fn disk_usage(p: &Path) -> u64 {
    let Ok(m) = fs::symlink_metadata(p) else { return 0 };
    if m.is_dir() {
        fs::read_dir(p).map(|rd| rd.flatten().map(|e| disk_usage(&e.path())).sum()).unwrap_or(0)
    } else {
        m.len()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn tar_gz(entries: &[(&str, TarItem)]) -> Vec<u8> {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut b = tar::Builder::new(gz);
        for (name, item) in entries {
            let mut h = tar::Header::new_gnu();
            match item {
                TarItem::File(data, mode) => {
                    h.set_entry_type(tar::EntryType::Regular);
                    h.set_size(data.len() as u64);
                    h.set_mode(*mode);
                    set_raw_path(&mut h, name);
                    h.set_cksum();
                    b.append(&h, &data[..]).unwrap();
                }
                TarItem::Dir => {
                    h.set_entry_type(tar::EntryType::Directory);
                    h.set_size(0);
                    h.set_mode(0o755);
                    set_raw_path(&mut h, name);
                    h.set_cksum();
                    b.append(&h, &[][..]).unwrap();
                }
                TarItem::Symlink(t) | TarItem::Hardlink(t) => {
                    h.set_entry_type(if matches!(item, TarItem::Symlink(_)) { tar::EntryType::Symlink } else { tar::EntryType::Link });
                    h.set_size(0);
                    h.set_mode(0o777);
                    set_raw_path(&mut h, name);
                    // The builder refuses `..` in link names; write the bytes.
                    let gnu = h.as_gnu_mut().unwrap();
                    gnu.linkname = [0; 100];
                    gnu.linkname[..t.len()].copy_from_slice(t.as_bytes());
                    h.set_cksum();
                    b.append(&h, &[][..]).unwrap();
                }
            }
        }
        b.into_inner().unwrap().finish().unwrap()
    }

    /// Write a name as-is (the builder would reject `..` and absolute paths).
    fn set_raw_path(h: &mut tar::Header, name: &str) {
        let gnu = h.as_gnu_mut().unwrap();
        gnu.name = [0; 100];
        gnu.name[..name.len()].copy_from_slice(name.as_bytes());
    }

    pub enum TarItem {
        File(Vec<u8>, u32),
        Dir,
        Symlink(String),
        Hardlink(String),
    }

    pub fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
        for (name, data) in entries {
            let opts = zip::write::SimpleFileOptions::default().unix_permissions(0o755);
            w.start_file(*name, opts).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    fn no_skip(_: &[&str]) -> bool {
        false
    }

    fn run_tar(bytes: Vec<u8>) -> (tempfile::TempDir, Result<u64, UnpackError>) {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("a.tar.gz");
        fs::write(&a, bytes).unwrap();
        let dest = t.path().join("out");
        let mut prog = |_| true;
        let r = unpack_tar_gz(&a, &dest, &mut Hooks { skip: &no_skip, progress: &mut prog });
        (t, r)
    }

    #[test]
    fn safe_relative_paths() {
        assert_eq!(safe_relative(Path::new("python/bin/python3")), Some(PathBuf::from("python/bin/python3")));
        assert_eq!(safe_relative(Path::new("./a/./b")), Some(PathBuf::from("a/b")));
        assert!(safe_relative(Path::new("../x")).is_none());
        assert!(safe_relative(Path::new("a/../../x")).is_none());
        assert!(safe_relative(Path::new("/etc/passwd")).is_none());
        assert!(safe_relative(Path::new("")).is_none());
        assert!(safe_relative(Path::new("C:x")).is_none());
        assert!(safe_relative(Path::new("a\\..\\b")).is_none());
        assert!(safe_relative(Path::new("trailing.")).is_none());
    }

    #[test]
    fn tar_unpacks_files_dirs_links_and_modes() {
        let (t, r) = run_tar(tar_gz(&[
            ("python/", TarItem::Dir),
            ("python/bin/python3.13", TarItem::File(b"#!bin".to_vec(), 0o4755)),
            ("python/bin/python3", TarItem::Symlink("python3.13".into())),
            ("python/lib/x.txt", TarItem::File(b"hello".to_vec(), 0o666)),
            ("python/lib/y.txt", TarItem::Hardlink("python/lib/x.txt".into())),
        ]));
        assert_eq!(r.unwrap(), 15);
        let out = t.path().join("out/python");
        assert_eq!(fs::read(out.join("lib/y.txt")).unwrap(), b"hello");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::read_link(out.join("bin/python3")).unwrap(), PathBuf::from("python3.13"));
            // setuid dropped, executable kept; world-writable dropped.
            assert_eq!(fs::metadata(out.join("bin/python3.13")).unwrap().permissions().mode() & 0o7777, 0o755);
            assert_eq!(fs::metadata(out.join("lib/x.txt")).unwrap().permissions().mode() & 0o7777, 0o644);
        }
    }

    #[test]
    fn tar_refuses_traversal() {
        for bad in ["../evil", "a/../../evil", "/abs/evil"] {
            let (t, r) = run_tar(tar_gz(&[(bad, TarItem::File(b"x".to_vec(), 0o644))]));
            assert!(matches!(r, Err(UnpackError::Bad(_))), "{bad}");
            assert!(!t.path().join("evil").exists());
        }
    }

    #[test]
    fn tar_refuses_links_that_escape() {
        for target in ["../../outside", "/etc/passwd", "a/../.."] {
            let (_t, r) = run_tar(tar_gz(&[("link", TarItem::Symlink(target.into()))]));
            assert!(matches!(r, Err(UnpackError::Bad(_))), "{target}");
        }
        // A file written "through" a link would need the link to point up.
        let (t, r) = run_tar(tar_gz(&[("d", TarItem::Symlink("..".into())), ("d/escape", TarItem::File(b"x".to_vec(), 0o644))]));
        assert!(r.is_err());
        assert!(!t.path().join("escape").exists());
        let (_t, r) = run_tar(tar_gz(&[("h", TarItem::Hardlink("../x".into()))]));
        assert!(r.is_err());
        let (_t, r) = run_tar(tar_gz(&[("h", TarItem::Hardlink("missing".into()))]));
        assert!(r.is_err());
    }

    #[test]
    fn tar_needs_an_empty_destination() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("a.tar.gz");
        fs::write(&a, tar_gz(&[("f", TarItem::File(b"x".to_vec(), 0o644))])).unwrap();
        fs::create_dir_all(t.path().join("out")).unwrap();
        fs::write(t.path().join("out/old"), b"").unwrap();
        let mut prog = |_| true;
        assert!(unpack_tar_gz(&a, &t.path().join("out"), &mut Hooks { skip: &no_skip, progress: &mut prog }).is_err());
    }

    #[test]
    fn corrupt_archives_fail_cleanly() {
        let (_t, r) = run_tar(b"definitely not gzip".to_vec());
        assert!(r.is_err());
        let mut good = tar_gz(&[("f", TarItem::File(vec![7; 100_000], 0o644))]);
        good.truncate(good.len() / 2);
        let (_t, r) = run_tar(good);
        assert!(r.is_err());
    }

    #[test]
    fn skip_and_cancel() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("a.tar.gz");
        fs::write(&a, tar_gz(&[("keep", TarItem::File(b"1".to_vec(), 0o644)), ("drop/x", TarItem::File(b"2".to_vec(), 0o644))])).unwrap();
        let skip = |p: &[&str]| p.first() == Some(&"drop");
        let mut prog = |_| true;
        unpack_tar_gz(&a, &t.path().join("o1"), &mut Hooks { skip: &skip, progress: &mut prog }).unwrap();
        assert!(t.path().join("o1/keep").exists());
        assert!(!t.path().join("o1/drop").exists());
        let mut stop = |_| false;
        let r = unpack_tar_gz(&a, &t.path().join("o2"), &mut Hooks { skip: &no_skip, progress: &mut stop });
        assert!(matches!(r, Err(UnpackError::Cancelled)));
    }

    #[test]
    fn zip_and_wheel() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("w.whl");
        fs::write(&a, zip_bytes(&[("pkg/__init__.py", b"x = 1"), ("pkg-1.0.data/scripts/tool", b"#!"), ("pkg-1.0.dist-info/METADATA", b"m")])).unwrap();
        let mut prog = |_| true;
        unpack_zip(&a, &t.path().join("o"), true, &mut Hooks { skip: &no_skip, progress: &mut prog }).unwrap();
        assert!(t.path().join("o/pkg/__init__.py").exists());
        assert!(t.path().join("o/pkg-1.0.dist-info/METADATA").exists());
        assert!(!t.path().join("o/pkg-1.0.data").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(t.path().join("o/pkg/__init__.py")).unwrap().permissions().mode() & 0o777, 0o755);
        }
        let b = t.path().join("bad.zip");
        fs::write(&b, zip_bytes(&[("../escape", b"x")])).unwrap();
        let mut prog = |_| true;
        assert!(unpack_zip(&b, &t.path().join("o2"), false, &mut Hooks { skip: &no_skip, progress: &mut prog }).is_err());
        assert!(!t.path().join("escape").exists());
    }
}
