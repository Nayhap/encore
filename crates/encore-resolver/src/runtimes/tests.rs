//! The install state machine against fake components served from memory.

use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;

use super::archive::tests::{TarItem, tar_gz, zip_bytes};
use super::*;

const TARGET: &str = match manifest::current_target() {
    Some(t) => t,
    None => "x86_64-unknown-linux-gnu",
};

/// IDLE in the target's Python layout (left out when unpacking).
const IDLE: &str = if cfg!(windows) { "python/Lib/idlelib" } else { "python/lib/python3.13/idlelib" };

#[derive(Default)]
struct MemFetcher {
    files: Mutex<HashMap<String, Vec<u8>>>,
    opens: Mutex<HashMap<String, usize>>,
    /// Break the stream of this URL after this many bytes.
    break_after: Mutex<Option<(String, usize)>>,
    /// Pause between chunks so a test can cancel mid-download.
    slow: bool,
    fail_connect: std::sync::atomic::AtomicBool,
}

#[async_trait]
impl Fetcher for MemFetcher {
    async fn open(&self, url: &str) -> Result<(Option<u64>, BoxStream<'static, Result<Vec<u8>, SetupError>>), SetupError> {
        if self.fail_connect.load(Ordering::SeqCst) {
            return Err(SetupError::new(SetupErrorKind::Offline, "offline"));
        }
        *self.opens.lock().entry(url.to_string()).or_default() += 1;
        let data = self.files.lock().get(url).cloned().ok_or_else(|| SetupError::new(SetupErrorKind::Network, "404"))?;
        let brk = self.break_after.lock().clone().filter(|b| b.0 == url).map(|b| b.1);
        let len = data.len() as u64;
        let slow = self.slow;
        let chunks: Vec<Vec<u8>> = data.chunks(512).map(|c| c.to_vec()).collect();
        let s = futures_util::stream::iter(chunks.into_iter().enumerate()).then(move |(i, c)| async move {
            if slow {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            if brk.is_some_and(|b| i * 512 >= b) {
                return Err(SetupError::new(SetupErrorKind::Network, "connection reset"));
            }
            Ok(c)
        });
        Ok((Some(len), s.boxed()))
    }
}

fn sha(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

struct Fake {
    manifest: Manifest,
    fetcher: Arc<MemFetcher>,
}

/// A manifest of three fake components, their archives served by `fetcher`.
fn fake(yt_dlp_version: &str, fetcher: Arc<MemFetcher>) -> Fake {
    let py_exe = manifest::python_exe(TARGET);
    // Incompressible filler so downloads take a few hundred chunks.
    let mut x = 0x2545_f491_4f6c_dd1du64;
    let filler: Vec<u8> = (0..64 * 1024)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect();
    let python = tar_gz(&[
        (py_exe, TarItem::File(b"#!python".to_vec(), 0o755)),
        (format!("{IDLE}/x.py").as_str(), TarItem::File(b"skip me".to_vec(), 0o644)),
        ("python/lib/python3.13/filler.bin", TarItem::File(filler, 0o644)),
    ]);
    let deno = zip_bytes(&[(manifest::deno_exe(TARGET), b"deno binary")]);
    let whl = zip_bytes(&[("yt_dlp/__init__.py", format!("__version__ = '{yt_dlp_version}'").as_bytes()), ("yt_dlp-x.data/scripts/yt-dlp", b"#!")]);
    let ejs = zip_bytes(&[("yt_dlp_ejs/__init__.py", b"")]);
    let art = |name: &str, data: &[u8], format: Format| {
        let url = format!("https://files.pythonhosted.org/{name}");
        fetcher.files.lock().insert(url.clone(), data.to_vec());
        Artifact { name: name.into(), url, sha256: sha(data), size: data.len() as u64, format }
    };
    let manifest = Manifest {
        target: TARGET.into(),
        components: vec![
            Component { kind: Kind::Python, version: "3.13.18".into(), files: vec![art("python.tar.gz", &python, Format::TarGz)], installed: 100 },
            Component { kind: Kind::Deno, version: "2.9.8".into(), files: vec![art("deno.zip", &deno, Format::Zip)], installed: 100 },
            Component {
                kind: Kind::Packages,
                version: yt_dlp_version.into(),
                files: vec![art(&format!("yt_dlp-{yt_dlp_version}.whl"), &whl, Format::Wheel), art("ejs.whl", &ejs, Format::Wheel)],
                installed: 100,
            },
        ],
    };
    Fake { manifest, fetcher }
}

fn runtimes(dir: &Path, f: &Fake) -> Arc<Runtimes> {
    let cfg = RuntimesConfig { root: dir.join("runtimes"), lock: dir.join("runtimes.lock"), legacy: vec![dir.join("resolver")], deno_dir: dir.join("deno") };
    Runtimes::with(cfg, Ok(f.manifest.clone()), f.fetcher.clone(), false)
}

#[tokio::test]
async fn installs_marks_and_activates() {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(t.path().join("resolver/venv")).unwrap();
    let f = fake("2026.10.26", Arc::new(MemFetcher::default()));
    let rt = runtimes(t.path(), &f);
    let st = rt.status();
    assert!(!st.usable && !st.ready && !st.approved && st.supported);
    assert_eq!(st.download_bytes as u64, f.manifest.components.iter().map(|c| c.download_bytes()).sum::<u64>());

    let phases = Arc::new(Mutex::new(Vec::new()));
    let p2 = phases.clone();
    rt.on_change(move |s| {
        if let Some(p) = &s.progress {
            let mut v = p2.lock();
            if v.last() != Some(&p.phase) {
                v.push(p.phase);
            }
        }
    });
    rt.approve().unwrap();
    rt.install().await.unwrap();
    assert_eq!(*phases.lock(), vec![SetupPhase::Downloading, SetupPhase::Verifying, SetupPhase::Unpacking, SetupPhase::SettingUp, SetupPhase::Done]);

    let st = rt.status();
    assert!(st.ready && st.usable && st.approved && !st.installing);
    assert_eq!(st.download_bytes, 0.0);
    assert!(st.components.iter().all(|c| c.ready && c.installed.as_deref() == Some(c.version.as_str())));
    let set = rt.active().unwrap();
    assert!(set.python.is_file() && set.deno.is_file());
    assert!(set.site.join("yt_dlp/__init__.py").is_file());
    assert!(set.site.join("yt_dlp_ejs/__init__.py").is_file());
    // Wheel data folders and trimmed Python parts are left out.
    assert!(!set.site.join("yt_dlp-x.data").exists());
    assert!(!rt.root().join(f.manifest.components[0].id()).join(IDLE).exists());
    // Temporary folders and the v0.2 venv are gone.
    assert!(!rt.root().join(DOWNLOADS).exists());
    assert!(!rt.root().join(STAGING).exists());
    assert!(!t.path().join("resolver").exists());
    assert_eq!(rt.generation(), 1);
    assert!(rt.status_measured().used_bytes > 0.0);
}

#[tokio::test]
async fn upgrade_downloads_only_what_changed() {
    let t = tempfile::tempdir().unwrap();
    let fetcher = Arc::new(MemFetcher::default());
    let v1 = fake("2026.10.26", fetcher.clone());
    runtimes(t.path(), &v1).install().await.unwrap();
    let old_pk = v1.manifest.components[2].id();

    // A newer Encore pins a newer yt-dlp.
    let v2 = fake("2026.12.01", fetcher.clone());
    let rt = runtimes(t.path(), &v2);
    let st = rt.status();
    assert!(st.usable && !st.ready, "the old set keeps working until the update lands");
    assert_eq!(st.components.iter().find(|c| c.name == "yt-dlp").unwrap().installed.as_deref(), Some("2026.10.26"));
    assert_eq!(st.download_bytes as u64, v2.manifest.components[2].download_bytes());
    rt.install().await.unwrap();
    assert!(rt.ready());
    let opens = fetcher.opens.lock().clone();
    assert_eq!(opens.get(&v2.manifest.components[0].files[0].url), Some(&1), "Python not downloaded again");
    assert_eq!(opens.get(&v2.manifest.components[2].files[0].url), Some(&1));
    assert!(!rt.root().join(old_pk).exists(), "the old yt-dlp folder is removed");
    assert_eq!(rt.active().unwrap().yt_dlp_version, "2026.12.01");
}

#[tokio::test]
async fn checksum_mismatch_is_refused_then_recovers() {
    let t = tempfile::tempdir().unwrap();
    let f = fake("2026.10.26", Arc::new(MemFetcher::default()));
    // Serve different bytes of the same length for Deno.
    let url = f.manifest.components[1].files[0].url.clone();
    let good = f.fetcher.files.lock().get(&url).cloned().unwrap();
    let mut bad = good.clone();
    let n = bad.len() / 2;
    bad[n] ^= 0xff;
    f.fetcher.files.lock().insert(url.clone(), bad);
    let rt = runtimes(t.path(), &f);
    let e = rt.install().await.unwrap_err();
    assert_eq!(e.kind, SetupErrorKind::Corrupt);
    assert!(rt.active().is_none());
    assert!(!rt.root().join(ACTIVE).exists());
    assert_eq!(rt.status().progress.unwrap().phase, SetupPhase::Failed);
    // The bad file isn't kept; the good one is fetched on the next try.
    f.fetcher.files.lock().insert(url, good);
    rt.install().await.unwrap();
    assert!(rt.ready());
}

#[tokio::test]
async fn wrong_length_and_broken_streams_fail_cleanly() {
    let t = tempfile::tempdir().unwrap();
    let f = fake("2026.10.26", Arc::new(MemFetcher::default()));
    let url = f.manifest.components[0].files[0].url.clone();
    *f.fetcher.break_after.lock() = Some((url.clone(), 600));
    let rt = runtimes(t.path(), &f);
    let e = rt.install().await.unwrap_err();
    assert_eq!(e.kind, SetupErrorKind::Network);
    let dl = rt.root().join(DOWNLOADS);
    assert!(std::fs::read_dir(&dl).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().ends_with(".part")), "no partial file left");
    *f.fetcher.break_after.lock() = None;

    // An archive that is longer than pinned is refused before writing.
    let mut longer = f.fetcher.files.lock().get(&url).cloned().unwrap();
    longer.push(0);
    f.fetcher.files.lock().insert(url.clone(), longer);
    assert_eq!(rt.install().await.unwrap_err().kind, SetupErrorKind::Corrupt);
}

#[tokio::test]
async fn offline_is_reported() {
    let t = tempfile::tempdir().unwrap();
    let f = fake("2026.10.26", Arc::new(MemFetcher::default()));
    f.fetcher.fail_connect.store(true, Ordering::SeqCst);
    let rt = runtimes(t.path(), &f);
    let e = rt.install().await.unwrap_err();
    assert_eq!(e.kind, SetupErrorKind::Offline);
    assert!(!rt.status().usable);
    f.fetcher.fail_connect.store(false, Ordering::SeqCst);
    rt.install().await.unwrap();
}

#[tokio::test]
async fn damaged_archive_with_matching_checksum_is_refused() {
    let t = tempfile::tempdir().unwrap();
    let fetcher = Arc::new(MemFetcher::default());
    let mut f = fake("2026.10.26", fetcher.clone());
    // A "verified" file that is not a valid archive (a bad pin).
    let junk = b"this is not a tar.gz at all".repeat(10);
    let a = &mut f.manifest.components[0].files[0];
    a.sha256 = sha(&junk);
    a.size = junk.len() as u64;
    fetcher.files.lock().insert(a.url.clone(), junk);
    let rt = runtimes(t.path(), &f);
    assert_eq!(rt.install().await.unwrap_err().kind, SetupErrorKind::Corrupt);
    assert!(rt.active().is_none());
    assert!(!rt.root().join(f.manifest.components[0].id()).exists(), "no half-unpacked component");
}

#[tokio::test]
async fn cancel_mid_download_then_resume() {
    let t = tempfile::tempdir().unwrap();
    let fetcher = Arc::new(MemFetcher { slow: true, ..Default::default() });
    let f = fake("2026.10.26", fetcher.clone());
    let rt = runtimes(t.path(), &f);
    let seen = Arc::new(AtomicUsize::new(0));
    let (rt2, s2) = (rt.clone(), seen.clone());
    rt.on_change(move |s| {
        if s.progress.as_ref().is_some_and(|p| p.phase == SetupPhase::Downloading && p.downloaded > 0.0) && s2.fetch_add(1, Ordering::SeqCst) == 0 {
            rt2.cancel();
        }
    });
    let e = rt.install().await.unwrap_err();
    assert_eq!(e.kind, SetupErrorKind::Cancelled);
    let st = rt.status();
    assert!(!st.installing && !st.usable);
    assert_eq!(st.progress.unwrap().phase, SetupPhase::Cancelled);
    assert!(!rt.root().join(ACTIVE).exists());
    seen.store(100, Ordering::SeqCst);
    rt.install().await.unwrap();
    assert!(rt.ready());
}

#[tokio::test]
async fn leftovers_from_a_quit_are_cleaned_and_verified_downloads_reused() {
    let t = tempfile::tempdir().unwrap();
    let fetcher = Arc::new(MemFetcher::default());
    let f = fake("2026.10.26", fetcher.clone());
    let root = t.path().join("runtimes");
    // As if Encore quit: a verified download, a partial one, a half-unpacked
    // folder in staging, and a component folder without its marker.
    let py = &f.manifest.components[0].files[0];
    std::fs::create_dir_all(root.join(DOWNLOADS)).unwrap();
    std::fs::write(root.join(DOWNLOADS).join(&py.sha256), fetcher.files.lock().get(&py.url).unwrap()).unwrap();
    std::fs::write(root.join(DOWNLOADS).join("abc.part"), b"partial").unwrap();
    std::fs::create_dir_all(root.join(STAGING).join("deno-junk")).unwrap();
    let deno_dir = root.join(f.manifest.components[1].id());
    std::fs::create_dir_all(&deno_dir).unwrap();
    std::fs::write(deno_dir.join("deno"), b"truncated").unwrap();
    let rt = runtimes(t.path(), &f);
    assert!(!rt.status().usable);
    rt.install().await.unwrap();
    assert!(rt.ready());
    assert_eq!(fetcher.opens.lock().get(&py.url), None, "the verified download was reused");
    assert_eq!(std::fs::read(rt.active().unwrap().deno).unwrap(), b"deno binary");
}

#[tokio::test]
async fn a_damaged_component_makes_the_set_unusable() {
    let t = tempfile::tempdir().unwrap();
    let f = fake("2026.10.26", Arc::new(MemFetcher::default()));
    let rt = runtimes(t.path(), &f);
    rt.install().await.unwrap();
    std::fs::remove_file(rt.root().join(f.manifest.components[1].id()).join(MARKER)).unwrap();
    assert!(rt.active().is_none());
    assert!(!rt.ready());
    rt.install().await.unwrap();
    assert!(rt.ready());
}

#[tokio::test]
async fn a_second_instance_waits_for_the_first() {
    let t = tempfile::tempdir().unwrap();
    let f = fake("2026.10.26", Arc::new(MemFetcher::default()));
    std::fs::create_dir_all(t.path()).unwrap();
    // Another process holds the install lock.
    let held = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(t.path().join("runtimes.lock")).unwrap();
    held.lock().unwrap();
    let rt = runtimes(t.path(), &f);
    let task = tokio::spawn({
        let rt = rt.clone();
        async move { rt.install().await }
    });
    let mut waited = false;
    for _ in 0..200 {
        if rt.status().progress.is_some_and(|p| p.phase == SetupPhase::Waiting) {
            waited = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(waited);
    // Removing while another holds the lock is refused, not forced.
    let other = runtimes(t.path(), &f);
    assert_eq!(other.remove(false).await.unwrap_err().kind, SetupErrorKind::Busy);
    held.unlock().unwrap();
    task.await.unwrap().unwrap();
    assert!(rt.ready());
    // The other instance sees the finished set straight away.
    assert!(other.ready());
}

#[tokio::test]
async fn remove_frees_everything() {
    let t = tempfile::tempdir().unwrap();
    let f = fake("2026.10.26", Arc::new(MemFetcher::default()));
    let rt = runtimes(t.path(), &f);
    rt.approve().unwrap();
    rt.install().await.unwrap();
    let g = rt.generation();
    rt.remove(true).await.unwrap();
    assert!(rt.approved(), "reinstall keeps the approval");
    assert!(!rt.status().usable);
    assert!(rt.generation() > g);
    rt.install().await.unwrap();
    rt.remove(false).await.unwrap();
    let st = rt.status();
    assert!(!st.approved && !st.usable && !st.ready);
    assert!(!rt.root().exists());
}

#[tokio::test]
async fn start_runs_once_in_the_background() {
    let t = tempfile::tempdir().unwrap();
    let f = fake("2026.10.26", Arc::new(MemFetcher { slow: true, ..Default::default() }));
    let rt = runtimes(t.path(), &f);
    assert!(rt.start());
    assert!(!rt.start(), "a second start while running is ignored");
    assert!(rt.status().installing);
    for _ in 0..2000 {
        if !rt.status().installing {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(rt.ready());
}

#[tokio::test]
async fn unsupported_systems_say_so() {
    let t = tempfile::tempdir().unwrap();
    let cfg = RuntimesConfig { root: t.path().join("runtimes"), lock: t.path().join("l"), legacy: vec![], deno_dir: t.path().join("d") };
    let rt = Runtimes::with(cfg, Err("Playback isn't available on plan9 yet.".into()), Arc::new(MemFetcher::default()), false);
    let st = rt.status();
    assert!(!st.supported);
    assert!(st.message.is_some());
    assert_eq!(rt.install().await.unwrap_err().kind, SetupErrorKind::Unsupported);
}
