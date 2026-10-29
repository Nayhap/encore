//! Managed player components (D22): a relocatable Python, Deno, and the
//! yt-dlp packages, downloaded into `<app data>/runtimes/` once the user
//! approves, and never taken from the system.
//!
//! Layout:
//!
//! ```text
//! runtimes/
//!   approved                 the user said yes (updates need no new approval)
//!   active.json              the component set in use, written last (atomic)
//!   python-<ver>-<fp>/       one folder per component, with `.encore-ready`
//!   deno-<ver>-<fp>/
//!   yt-dlp-<ver>-<fp>/       an import folder for yt-dlp, yt-dlp-ejs, certifi
//!   encore_resolver.py       the helper script (written by the resolver)
//!   .downloads/<sha256>      verified downloads, kept until the install ends
//!   .staging/                unpacking happens here, then an atomic rename
//! runtimes.lock              held by the one Encore that is installing
//! ```
//!
//! A component folder only ever appears complete (it is renamed into place
//! with its marker inside), and `active.json` only names folders that passed
//! the start-up check, so quitting, a full disk or a bad download at any
//! point leaves either the old set or nothing, never half of one.

pub mod archive;
pub mod manifest;

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

pub use manifest::{Artifact, Component, Format, Kind, Manifest};

const MARKER: &str = ".encore-ready";
const ACTIVE: &str = "active.json";
const APPROVED: &str = "approved";
const DOWNLOADS: &str = ".downloads";
const STAGING: &str = ".staging";

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum SetupPhase {
    /// Another Encore is installing; this one waits for it.
    Waiting,
    Downloading,
    Verifying,
    Unpacking,
    SettingUp,
    Done,
    Failed,
    Cancelled,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum SetupErrorKind {
    Offline,
    Network,
    Disk,
    Permission,
    Corrupt,
    Setup,
    Unsupported,
    Busy,
    Cancelled,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SetupError {
    pub kind: SetupErrorKind,
    /// A plain sentence for the user.
    pub message: String,
}

impl SetupError {
    pub fn new(kind: SetupErrorKind, message: impl Into<String>) -> Self {
        SetupError { kind, message: message.into() }
    }
    fn cancelled() -> Self {
        Self::new(SetupErrorKind::Cancelled, "Setup was cancelled.")
    }
    fn corrupt() -> Self {
        Self::new(SetupErrorKind::Corrupt, "A download didn't match what Encore expected, so it wasn't used. Try again.")
    }
    /// A file system error, in words.
    pub fn io(e: &io::Error, what: &Path) -> Self {
        match e.kind() {
            io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => Self::new(SetupErrorKind::Disk, "There isn't enough free space on this disk. Free up some space and try again."),
            io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem => Self::new(SetupErrorKind::Permission, format!("Encore isn't allowed to write to {}.", what.display())),
            _ => Self::new(SetupErrorKind::Disk, format!("Encore couldn't save the files ({}).", e.kind())),
        }
    }
}

impl std::fmt::Display for SetupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SetupProgress {
    pub phase: SetupPhase,
    /// Replacing components that already worked (a quiet update).
    pub update: bool,
    /// The component being worked on ("Python 3.13.18").
    pub component: Option<String>,
    pub downloaded: f64,
    pub download_total: f64,
    pub unpacked: f64,
    pub unpack_total: f64,
    /// Overall, 0 to 1.
    pub fraction: f64,
    pub error: Option<SetupError>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ComponentInfo {
    pub name: String,
    /// The version this Encore uses.
    pub version: String,
    /// The version installed and in use, if any.
    pub installed: Option<String>,
    pub ready: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    /// Encore has components for this system.
    pub supported: bool,
    /// The user agreed to the download (updates follow without asking).
    pub approved: bool,
    /// This Encore's versions are installed and in use.
    pub ready: bool,
    /// Some working set is installed, so songs can play.
    pub usable: bool,
    pub installing: bool,
    pub progress: Option<SetupProgress>,
    pub components: Vec<ComponentInfo>,
    /// Still to download for this Encore's versions.
    pub download_bytes: f64,
    /// Expected size on disk of this Encore's versions.
    pub disk_bytes: f64,
    /// Measured size of the components folder (0 when not measured).
    pub used_bytes: f64,
    pub location: String,
    /// Why components aren't available here, when they aren't.
    pub message: Option<String>,
}

/// Where a working set lives.
#[derive(Clone, Debug, PartialEq)]
pub struct ActiveSet {
    pub python: PathBuf,
    pub deno: PathBuf,
    /// The folder yt-dlp is imported from.
    pub site: PathBuf,
    pub python_version: String,
    pub deno_version: String,
    pub yt_dlp_version: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct ActiveFile {
    layout: u32,
    target: String,
    components: Vec<ActiveEntry>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct ActiveEntry {
    kind: String,
    id: String,
    version: String,
}

/// A source of bytes for a URL. The real one is HTTPS; tests use memory.
#[async_trait]
pub trait Fetcher: Send + Sync {
    /// The body as a stream of chunks, and its length when known.
    async fn open(&self, url: &str) -> Result<(Option<u64>, BoxStream<'static, Result<Vec<u8>, SetupError>>), SetupError>;
}

/// Downloads over HTTPS from the pinned hosts.
pub struct HttpFetcher {
    client: reqwest::Client,
}

impl HttpFetcher {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .https_only(true)
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::limited(5))
            .user_agent(concat!("Encore/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("http client");
        HttpFetcher { client }
    }
}

impl Default for HttpFetcher {
    fn default() -> Self {
        Self::new()
    }
}

fn net_error(e: &reqwest::Error) -> SetupError {
    if e.is_connect() {
        SetupError::new(SetupErrorKind::Offline, "Encore couldn't connect to the internet. Check your connection and try again.")
    } else if e.is_timeout() {
        SetupError::new(SetupErrorKind::Network, "The download stopped responding. Check your connection and try again.")
    } else {
        SetupError::new(SetupErrorKind::Network, "The download was interrupted. Check your connection and try again.")
    }
}

#[async_trait]
impl Fetcher for HttpFetcher {
    async fn open(&self, url: &str) -> Result<(Option<u64>, BoxStream<'static, Result<Vec<u8>, SetupError>>), SetupError> {
        let host = url.strip_prefix("https://").and_then(|r| r.split('/').next()).unwrap_or("");
        if !manifest::ALLOWED_HOSTS.contains(&host) {
            return Err(SetupError::new(SetupErrorKind::Corrupt, "Encore refused a download from an unexpected address."));
        }
        let r = self.client.get(url).send().await.map_err(|e| net_error(&e))?;
        let st = r.status();
        if !st.is_success() {
            return Err(SetupError::new(SetupErrorKind::Network, format!("The download server answered {}. Try again in a while.", st.as_u16())));
        }
        let len = r.content_length();
        let s = r.bytes_stream().map(|c| c.map(|b| b.to_vec()).map_err(|e| net_error(&e)));
        Ok((len, s.boxed()))
    }
}

pub struct RuntimesConfig {
    /// `<app data>/runtimes`
    pub root: PathBuf,
    /// `<app data>/runtimes.lock`
    pub lock: PathBuf,
    /// Folders from older Encore versions to delete once set up (the v0.2
    /// `resolver/` venv).
    pub legacy: Vec<PathBuf>,
    /// Deno's cache (`DENO_DIR`).
    pub deno_dir: PathBuf,
}

impl RuntimesConfig {
    /// Under an app data folder.
    pub fn in_data_dir(data: &Path, cache: &Path) -> Self {
        RuntimesConfig { root: data.join("runtimes"), lock: data.join("runtimes.lock"), legacy: vec![data.join("resolver")], deno_dir: cache.join("deno") }
    }
}

type Listener = Arc<dyn Fn(&RuntimeStatus) + Send + Sync>;

struct Inner {
    installing: bool,
    cancel: Option<CancellationToken>,
    progress: Option<SetupProgress>,
    last_emit: Option<Instant>,
}

pub struct Runtimes {
    cfg: RuntimesConfig,
    manifest: Result<Manifest, String>,
    fetcher: Arc<dyn Fetcher>,
    /// Run the components once before using them (off in unit tests, which
    /// install fake ones).
    smoke_test: bool,
    inner: Mutex<Inner>,
    run_lock: tokio::sync::Mutex<()>,
    generation: AtomicU64,
    listeners: Mutex<Vec<Listener>>,
}

/// Overall progress weights: download, verify, unpack, set up.
const W: [f64; 4] = [0.70, 0.05, 0.20, 0.05];

impl Runtimes {
    pub fn new(cfg: RuntimesConfig) -> Arc<Self> {
        Self::with(cfg, Manifest::pinned(), Arc::new(HttpFetcher::new()), true)
    }

    pub fn with(cfg: RuntimesConfig, manifest: Result<Manifest, String>, fetcher: Arc<dyn Fetcher>, smoke_test: bool) -> Arc<Self> {
        Arc::new(Runtimes {
            cfg,
            manifest,
            fetcher,
            smoke_test,
            inner: Mutex::new(Inner { installing: false, cancel: None, progress: None, last_emit: None }),
            run_lock: tokio::sync::Mutex::new(()),
            generation: AtomicU64::new(0),
            listeners: Mutex::new(Vec::new()),
        })
    }

    pub fn root(&self) -> &Path {
        &self.cfg.root
    }

    pub fn deno_dir(&self) -> &Path {
        &self.cfg.deno_dir
    }

    /// Bumps whenever the set in use changes (installed, updated, removed).
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    pub fn on_change(&self, f: impl Fn(&RuntimeStatus) + Send + Sync + 'static) {
        self.listeners.lock().push(Arc::new(f));
    }

    fn emit(&self, force: bool) {
        {
            let mut i = self.inner.lock();
            if !force && i.last_emit.is_some_and(|t| t.elapsed() < Duration::from_millis(100)) {
                return;
            }
            i.last_emit = Some(Instant::now());
        }
        let st = self.status();
        let ls = self.listeners.lock().clone();
        for l in ls {
            l(&st);
        }
    }

    pub fn approved(&self) -> bool {
        self.cfg.root.join(APPROVED).is_file()
    }

    /// Record the user's yes.
    pub fn approve(&self) -> Result<(), SetupError> {
        fs::create_dir_all(&self.cfg.root).map_err(|e| SetupError::io(&e, &self.cfg.root))?;
        write_atomic(&self.cfg.root.join(APPROVED), b"approved\n").map_err(|e| SetupError::io(&e, &self.cfg.root))
    }

    fn component_dir(&self, c: &Component) -> PathBuf {
        self.cfg.root.join(c.id())
    }

    fn component_ready(&self, c: &Component) -> bool {
        fs::read_to_string(self.component_dir(c).join(MARKER)).is_ok_and(|s| s.trim() == c.fingerprint())
    }

    fn read_active(&self) -> Option<ActiveFile> {
        let s = fs::read_to_string(self.cfg.root.join(ACTIVE)).ok()?;
        serde_json::from_str(&s).ok()
    }

    /// The working set in use, if there is one and its folders are intact.
    pub fn active(&self) -> Option<ActiveSet> {
        let a = self.read_active()?;
        let target = self.manifest.as_ref().ok().map(|m| m.target.as_str()).unwrap_or(&a.target);
        if a.target != target {
            return None;
        }
        let find = |k: Kind| a.components.iter().find(|e| e.kind == k.slug());
        let ok = |e: &ActiveEntry| self.cfg.root.join(&e.id).join(MARKER).is_file();
        let (py, deno, pk) = (find(Kind::Python)?, find(Kind::Deno)?, find(Kind::Packages)?);
        if !(ok(py) && ok(deno) && ok(pk)) {
            return None;
        }
        let set = ActiveSet {
            python: self.cfg.root.join(&py.id).join(manifest::python_exe(target)),
            deno: self.cfg.root.join(&deno.id).join(manifest::deno_exe(target)),
            site: self.cfg.root.join(&pk.id),
            python_version: py.version.clone(),
            deno_version: deno.version.clone(),
            yt_dlp_version: pk.version.clone(),
        };
        (set.python.is_file() && set.deno.is_file()).then_some(set)
    }

    /// This Encore's versions are installed and in use.
    pub fn ready(&self) -> bool {
        let (Ok(m), Some(a)) = (&self.manifest, self.read_active()) else { return false };
        m.components.iter().all(|c| self.component_ready(c) && a.components.iter().any(|e| e.id == c.id())) && self.active().is_some()
    }

    pub fn status(&self) -> RuntimeStatus {
        let (installing, progress) = {
            let i = self.inner.lock();
            (i.installing, i.progress.clone())
        };
        let active = self.read_active();
        let installed = |k: Kind| active.as_ref().and_then(|a| a.components.iter().find(|e| e.kind == k.slug()).map(|e| (e.id.clone(), e.version.clone())));
        let (components, download, disk) = match &self.manifest {
            Ok(m) => (
                m.components
                    .iter()
                    .map(|c| {
                        let inst = installed(c.kind);
                        ComponentInfo { name: c.kind.label().into(), version: c.version.clone(), ready: inst.as_ref().is_some_and(|(id, _)| *id == c.id()) && self.component_ready(c), installed: inst.map(|i| i.1) }
                    })
                    .collect(),
                m.components.iter().filter(|c| !self.component_ready(c)).map(|c| c.download_bytes()).sum::<u64>(),
                m.components.iter().map(|c| c.installed).sum::<u64>(),
            ),
            Err(_) => (Vec::new(), 0, 0),
        };
        RuntimeStatus {
            supported: self.manifest.is_ok(),
            approved: self.approved(),
            ready: self.ready(),
            usable: self.active().is_some(),
            installing,
            progress,
            components,
            download_bytes: download as f64,
            disk_bytes: disk as f64,
            used_bytes: 0.0,
            location: self.cfg.root.display().to_string(),
            message: self.manifest.as_ref().err().cloned(),
        }
    }

    /// `status()` plus the measured size of the folder.
    pub fn status_measured(&self) -> RuntimeStatus {
        let mut s = self.status();
        s.used_bytes = archive::disk_usage(&self.cfg.root) as f64;
        s
    }

    // ---------- installing ----------

    /// Start installing in the background unless already running. Returns
    /// false if it was already running.
    pub fn start(self: &Arc<Self>) -> bool {
        {
            let mut i = self.inner.lock();
            if i.installing {
                return false;
            }
            i.installing = true;
        }
        let me = self.clone();
        tokio::spawn(async move {
            let _ = me.install().await;
        });
        true
    }

    /// Stop an install in progress (at the next chunk or file).
    pub fn cancel(&self) {
        if let Some(t) = &self.inner.lock().cancel {
            t.cancel();
        }
    }

    /// Install this Encore's versions (needs approval first). Progress and
    /// the result go to listeners as well.
    pub async fn install(&self) -> Result<(), SetupError> {
        let _run = self.run_lock.lock().await;
        let token = CancellationToken::new();
        {
            let mut i = self.inner.lock();
            i.installing = true;
            i.cancel = Some(token.clone());
            i.progress = Some(SetupProgress { phase: SetupPhase::Downloading, update: self.active().is_some(), component: None, downloaded: 0.0, download_total: 0.0, unpacked: 0.0, unpack_total: 0.0, fraction: 0.0, error: None });
        }
        self.emit(true);
        let t0 = Instant::now();
        let r = self.install_inner(&token).await;
        {
            let mut i = self.inner.lock();
            i.installing = false;
            i.cancel = None;
            if let Some(p) = i.progress.as_mut() {
                match &r {
                    Ok(()) => {
                        p.phase = SetupPhase::Done;
                        p.fraction = 1.0;
                        p.component = None;
                    }
                    Err(e) => {
                        p.phase = if e.kind == SetupErrorKind::Cancelled { SetupPhase::Cancelled } else { SetupPhase::Failed };
                        p.error = Some(e.clone());
                    }
                }
            }
        }
        match &r {
            Ok(()) => tracing::info!(ms = t0.elapsed().as_millis() as u64, "player components ready"),
            Err(e) => tracing::warn!(kind = ?e.kind, message = %e.message, "player components setup failed"),
        }
        self.emit(true);
        r
    }

    fn progress(&self, f: impl FnOnce(&mut SetupProgress)) {
        let phase_changed = {
            let mut i = self.inner.lock();
            let Some(p) = i.progress.as_mut() else { return };
            let before = (p.phase, p.component.clone());
            f(p);
            let base: f64 = match p.phase {
                SetupPhase::Waiting | SetupPhase::Downloading => 0.0,
                SetupPhase::Verifying => W[0],
                SetupPhase::Unpacking => W[0] + W[1],
                SetupPhase::SettingUp => W[0] + W[1] + W[2],
                _ => 1.0,
            };
            let within = match p.phase {
                SetupPhase::Downloading if p.download_total > 0.0 => W[0] * (p.downloaded / p.download_total),
                SetupPhase::Unpacking if p.unpack_total > 0.0 => W[2] * (p.unpacked / p.unpack_total).min(1.0),
                _ => 0.0,
            };
            p.fraction = (base + within).clamp(0.0, 1.0);
            before != (p.phase, p.component.clone())
        };
        self.emit(phase_changed);
    }

    async fn install_inner(&self, token: &CancellationToken) -> Result<(), SetupError> {
        let m = self.manifest.clone().map_err(|e| SetupError::new(SetupErrorKind::Unsupported, e))?;
        let root = self.cfg.root.clone();
        let io_err = |e: io::Error| SetupError::io(&e, &root);
        fs::create_dir_all(&root).map_err(io_err)?;
        let _lock = self.lock(token).await?;
        // A previous run may have stopped anywhere: partial downloads and
        // half-unpacked folders go; verified downloads stay for reuse.
        let downloads = root.join(DOWNLOADS);
        let staging = root.join(STAGING);
        remove_all(&staging).map_err(io_err)?;
        fs::create_dir_all(&downloads).map_err(io_err)?;
        for e in fs::read_dir(&downloads).map_err(io_err)?.flatten() {
            if e.path().extension().is_some_and(|x| x == "part") {
                let _ = fs::remove_file(e.path());
            }
        }
        let todo: Vec<Component> = m.components.iter().filter(|c| !self.component_ready(c)).cloned().collect();
        let download_total: u64 = todo.iter().map(|c| c.download_bytes()).sum();
        let unpack_total: u64 = todo.iter().map(|c| c.installed).sum();
        self.progress(|p| {
            p.phase = SetupPhase::Downloading;
            p.download_total = download_total as f64;
            p.unpack_total = unpack_total as f64;
        });

        // 1. Download.
        let mut done = 0u64;
        for c in &todo {
            for f in &c.files {
                if token.is_cancelled() {
                    return Err(SetupError::cancelled());
                }
                let label = format!("{} {}", c.kind.label(), c.version);
                self.progress(|p| p.component = Some(label));
                let path = downloads.join(&f.sha256);
                if fs::metadata(&path).is_ok_and(|md| md.len() == f.size) {
                    done += f.size;
                    self.progress(|p| p.downloaded = done as f64);
                    continue;
                }
                self.download(f, &downloads, done, token).await?;
                done += f.size;
            }
        }

        // 2. Verify every file as it is on disk.
        self.progress(|p| {
            p.phase = SetupPhase::Verifying;
            p.component = None;
            p.downloaded = download_total as f64;
        });
        for c in &todo {
            let label = format!("{} {}", c.kind.label(), c.version);
            self.progress(|p| p.component = Some(label));
            for f in &c.files {
                let path = downloads.join(&f.sha256);
                let (want, tok) = (f.sha256.clone(), token.clone());
                let p2 = path.clone();
                let got = tokio::task::spawn_blocking(move || sha256_file(&p2, &tok)).await.map_err(|_| SetupError::cancelled())?;
                match got {
                    Ok(h) if h == want => {}
                    Ok(_) => {
                        let _ = fs::remove_file(&path);
                        tracing::warn!(file = %f.name, "checksum mismatch");
                        return Err(SetupError::corrupt());
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => return Err(SetupError::cancelled()),
                    Err(e) => return Err(io_err(e)),
                }
            }
        }

        // 3. Unpack each into staging, mark it, rename it into place.
        self.progress(|p| {
            p.phase = SetupPhase::Unpacking;
            p.component = None;
        });
        let target = m.target.clone();
        let mut base = 0u64;
        let mut installed_now = Vec::new();
        for c in &todo {
            let label = format!("{} {}", c.kind.label(), c.version);
            self.progress(|p| p.component = Some(label));
            let dir = staging.join(c.id());
            let written = self.unpack(c, &downloads, &dir, &target, base, token).await?;
            base += written.max(c.installed);
            fs::write(dir.join(MARKER), c.fingerprint()).map_err(io_err)?;
            let dest = self.component_dir(c);
            remove_all(&dest).map_err(io_err)?;
            fs::rename(&dir, &dest).map_err(io_err)?;
            installed_now.push(dest);
        }

        // 4. Check the whole set starts, then make it the one in use.
        if token.is_cancelled() {
            return Err(SetupError::cancelled());
        }
        self.progress(|p| {
            p.phase = SetupPhase::SettingUp;
            p.component = None;
        });
        let entries: Vec<ActiveEntry> = m.components.iter().map(|c| ActiveEntry { kind: c.kind.slug().into(), id: c.id(), version: c.version.clone() }).collect();
        if self.smoke_test {
            let set = ActiveSet {
                python: self.component_dir(m.get(Kind::Python).unwrap()).join(manifest::python_exe(&target)),
                deno: self.component_dir(m.get(Kind::Deno).unwrap()).join(manifest::deno_exe(&target)),
                site: self.component_dir(m.get(Kind::Packages).unwrap()),
                python_version: String::new(),
                deno_version: String::new(),
                yt_dlp_version: String::new(),
            };
            if let Err(e) = smoke_test(&set, &self.cfg.deno_dir, m.get(Kind::Packages).unwrap().version.as_str()).await {
                // Something in the new folders doesn't run: make the next
                // try fetch them again.
                for d in &installed_now {
                    let _ = remove_all(d);
                }
                return Err(e);
            }
        }
        let active = ActiveFile { layout: manifest::LAYOUT, target: target.clone(), components: entries };
        write_atomic(&root.join(ACTIVE), serde_json::to_string_pretty(&active).unwrap().as_bytes()).map_err(io_err)?;
        self.generation.fetch_add(1, Ordering::SeqCst);

        // 5. Tidy: downloads, staging, older component folders, the v0.2 venv.
        let _ = remove_all(&downloads);
        let _ = remove_all(&staging);
        self.collect_garbage(&active);
        for l in &self.cfg.legacy {
            if l.exists() {
                match remove_all(l) {
                    Ok(()) => tracing::info!(path = %l.display(), "removed the old resolver environment"),
                    Err(e) => tracing::warn!(error = %e, "could not remove the old resolver environment"),
                }
            }
        }
        Ok(())
    }

    /// Take the cross-process install lock, waiting (cancellably) if another
    /// Encore holds it.
    async fn lock(&self, token: &CancellationToken) -> Result<fs::File, SetupError> {
        let f = fs::OpenOptions::new().create(true).truncate(false).write(true).open(&self.cfg.lock).map_err(|e| SetupError::io(&e, &self.cfg.lock))?;
        match f.try_lock() {
            Ok(()) => return Ok(f),
            Err(fs::TryLockError::WouldBlock) => {}
            Err(fs::TryLockError::Error(e)) => return Err(SetupError::io(&e, &self.cfg.lock)),
        }
        tracing::info!("another Encore is setting up player components; waiting");
        self.progress(|p| p.phase = SetupPhase::Waiting);
        let h = tokio::task::spawn_blocking(move || f.lock().map(|_| f));
        tokio::select! {
            r = h => {
                let f = r.map_err(|_| SetupError::cancelled())?.map_err(|e| SetupError::io(&e, &self.cfg.lock))?;
                self.progress(|p| p.phase = SetupPhase::Downloading);
                Ok(f)
            }
            _ = token.cancelled() => Err(SetupError::cancelled()),
        }
    }

    async fn download(&self, f: &Artifact, dir: &Path, before: u64, token: &CancellationToken) -> Result<(), SetupError> {
        let part = dir.join(format!("{}.part", f.sha256));
        let dest = dir.join(&f.sha256);
        let io_err = |e: io::Error| SetupError::io(&e, dir);
        let (len, mut stream) = tokio::select! {
            r = self.fetcher.open(&f.url) => r?,
            _ = token.cancelled() => return Err(SetupError::cancelled()),
        };
        if len.is_some_and(|l| l != f.size) {
            tracing::warn!(file = %f.name, len, want = f.size, "unexpected download size");
            return Err(SetupError::corrupt());
        }
        let mut out = tokio::fs::File::create(&part).await.map_err(io_err)?;
        let mut got = 0u64;
        let r: Result<(), SetupError> = async {
            loop {
                let chunk = tokio::select! {
                    c = stream.next() => c,
                    _ = token.cancelled() => return Err(SetupError::cancelled()),
                };
                let Some(chunk) = chunk else { break };
                let chunk = chunk?;
                got += chunk.len() as u64;
                if got > f.size {
                    return Err(SetupError::corrupt());
                }
                out.write_all(&chunk).await.map_err(io_err)?;
                self.progress(|p| p.downloaded = (before + got) as f64);
            }
            out.flush().await.map_err(io_err)?;
            out.sync_all().await.map_err(io_err)?;
            if got != f.size {
                return Err(SetupError::new(SetupErrorKind::Network, "The download ended early. Check your connection and try again."));
            }
            Ok(())
        }
        .await;
        drop(out);
        if let Err(e) = r {
            let _ = fs::remove_file(&part);
            return Err(e);
        }
        fs::rename(&part, &dest).map_err(io_err)
    }

    async fn unpack(&self, c: &Component, downloads: &Path, dir: &Path, target: &str, base: u64, token: &CancellationToken) -> Result<u64, SetupError> {
        let files: Vec<(PathBuf, Format)> = c.files.iter().map(|f| (downloads.join(&f.sha256), f.format)).collect();
        let (dir2, target2, kind, tok) = (dir.to_path_buf(), target.to_string(), c.kind, token.clone());
        // Progress from the blocking thread, at most every 64 KiB chunk.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<u64>();
        let job = tokio::task::spawn_blocking(move || -> Result<u64, archive::UnpackError> {
            fs::create_dir_all(&dir2)?;
            let mut total = 0u64;
            for (i, (path, format)) in files.iter().enumerate() {
                // Wheels share one folder; each archive gets its own scratch
                // folder first so "unpack into empty" holds.
                let scratch = if i == 0 { dir2.clone() } else { dir2.parent().unwrap_or(&dir2).join(format!("{}.part{i}", dir2.file_name().unwrap_or_default().to_string_lossy())) };
                let skip_python = |p: &[&str]| manifest::python_skip(&target2, p);
                let no_skip = |_: &[&str]| false;
                let skip: &dyn Fn(&[&str]) -> bool = if kind == Kind::Python { &skip_python } else { &no_skip };
                let start = total;
                let mut prog = |n: u64| {
                    let _ = tx.send(start + n);
                    !tok.is_cancelled()
                };
                let mut hooks = archive::Hooks { skip, progress: &mut prog };
                let n = match format {
                    Format::TarGz => archive::unpack_tar_gz(path, &scratch, &mut hooks)?,
                    Format::Zip => archive::unpack_zip(path, &scratch, false, &mut hooks)?,
                    Format::Wheel => archive::unpack_zip(path, &scratch, true, &mut hooks)?,
                };
                total += n;
                if i > 0 {
                    merge_into(&scratch, &dir2)?;
                    fs::remove_dir_all(&scratch)?;
                }
            }
            if kind == Kind::Deno {
                set_executable(&dir2.join(manifest::deno_exe(&target2)))?;
            }
            archive::clear_quarantine(&dir2);
            Ok(total)
        });
        tokio::pin!(job);
        loop {
            tokio::select! {
                r = &mut job => {
                    return match r.map_err(|_| SetupError::cancelled())? {
                        Ok(n) => Ok(n),
                        Err(archive::UnpackError::Cancelled) => Err(SetupError::cancelled()),
                        Err(archive::UnpackError::Io(e)) => Err(SetupError::io(&e, dir)),
                        Err(archive::UnpackError::Bad(m)) => {
                            tracing::warn!(component = c.kind.slug(), error = %m, "archive refused");
                            Err(SetupError::corrupt())
                        }
                    };
                }
                Some(n) = rx.recv() => self.progress(|p| p.unpacked = (base + n) as f64),
            }
        }
    }

    /// Delete component folders that aren't in the set in use.
    fn collect_garbage(&self, active: &ActiveFile) {
        let Ok(rd) = fs::read_dir(&self.cfg.root) else { return };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let ours = ["python-", "deno-", "yt-dlp-"].iter().any(|p| name.starts_with(p));
            if ours && e.path().is_dir() && !active.components.iter().any(|c| c.id == name) {
                // A running older Encore may still use it; on Windows that
                // fails, and the next launch tries again.
                if let Err(err) = remove_all(&e.path()) {
                    tracing::info!(folder = %name, error = %err, "old component folder not removed yet");
                }
            }
        }
    }

    /// Delete the components (and, unless `keep_approval`, the approval).
    /// Stop the helpers using them first.
    pub async fn remove(&self, keep_approval: bool) -> Result<(), SetupError> {
        self.cancel();
        let _run = self.run_lock.lock().await;
        let lock = fs::OpenOptions::new().create(true).truncate(false).write(true).open(&self.cfg.lock).map_err(|e| SetupError::io(&e, &self.cfg.lock))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => return Err(SetupError::new(SetupErrorKind::Busy, "Another Encore window is setting up the player components. Try again when it's done.")),
            Err(fs::TryLockError::Error(e)) => return Err(SetupError::io(&e, &self.cfg.lock)),
        }
        let root = &self.cfg.root;
        let mut first_err = None;
        // The marker files go first, so a folder that can't be fully deleted
        // (a file in use on Windows) isn't mistaken for a complete one.
        let _ = fs::remove_file(root.join(ACTIVE));
        if let Ok(rd) = fs::read_dir(root) {
            for e in rd.flatten() {
                if keep_approval && e.file_name() == APPROVED {
                    continue;
                }
                let _ = fs::remove_file(e.path().join(MARKER));
                if let Err(err) = remove_all(&e.path()) {
                    first_err.get_or_insert(err);
                }
            }
        }
        if !keep_approval {
            let _ = fs::remove_dir(root);
        }
        for l in &self.cfg.legacy {
            let _ = remove_all(l);
        }
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.inner.lock().progress = None;
        self.emit(true);
        match first_err {
            None => Ok(()),
            Some(e) if e.kind() == io::ErrorKind::PermissionDenied => Err(SetupError::new(SetupErrorKind::Busy, "Some files are still in use. Quit other Encore windows and try again.")),
            Some(e) => Err(SetupError::io(&e, root)),
        }
    }
}

/// A `Command` for a helper process: no console window on Windows, a clean
/// environment (no inherited proxies, credentials or Python settings), and
/// the AppImage's library path removed.
pub fn helper_command(program: &Path, deno_dir: &Path) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(program);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    cmd.env_clear();
    const KEEP: &[&str] = if cfg!(windows) {
        &["PATH", "SYSTEMROOT", "WINDIR", "SYSTEMDRIVE", "TEMP", "TMP", "USERPROFILE", "LOCALAPPDATA", "APPDATA", "PATHEXT", "COMSPEC", "PROGRAMDATA", "NUMBER_OF_PROCESSORS", "PROCESSOR_ARCHITECTURE"]
    } else {
        &["HOME", "PATH", "LANG", "TMPDIR"]
    };
    for k in KEEP.iter().copied() {
        if let Some(v) = std::env::var_os(k) {
            cmd.env(k, v);
        }
    }
    // The AppImage launcher points LD_LIBRARY_PATH into the AppImage; the
    // helpers must not load its libraries.
    if let Some(Some(ld)) = host_library_path() {
        cmd.env("LD_LIBRARY_PATH", ld);
    }
    cmd.env("DENO_DIR", deno_dir).env("DENO_NO_UPDATE_CHECK", "1").env("DENO_NO_PROMPT", "1");
    cmd
}

/// `LD_LIBRARY_PATH` without the AppImage's own folders, when running from an
/// AppImage: `Some(None)` to leave it unset, `Some(Some(v))` to set it.
fn host_library_path() -> Option<Option<std::ffi::OsString>> {
    let appdir = PathBuf::from(std::env::var_os("APPDIR")?);
    let cur = std::env::var_os("LD_LIBRARY_PATH")?;
    let kept: Vec<PathBuf> = std::env::split_paths(&cur).filter(|p| !p.as_os_str().is_empty() && !p.starts_with(&appdir)).collect();
    if kept.is_empty() {
        return Some(None);
    }
    Some(std::env::join_paths(kept).ok())
}

/// Run the new set once: Python imports yt-dlp from its folder, Deno starts.
async fn smoke_test(set: &ActiveSet, deno_dir: &Path, yt_dlp: &str) -> Result<(), SetupError> {
    let fail = |what: &str, detail: String| {
        tracing::warn!(what, detail = %detail, "component check failed");
        SetupError::new(SetupErrorKind::Setup, format!("{what} didn't start on this computer. Try again; if it keeps happening, please report it."))
    };
    let mut py = helper_command(&set.python, deno_dir);
    py.args(["-I", "-c", "import sys; sys.path.insert(0, sys.argv[1]); import ssl, yt_dlp, yt_dlp_ejs, certifi; print(yt_dlp.version.__version__)"]).arg(&set.site);
    let out = tokio::time::timeout(Duration::from_secs(120), py.output()).await.map_err(|_| fail("Python", "timed out".into()))?.map_err(|e| fail("Python", e.to_string()))?;
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || v != yt_dlp {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(fail("Python", format!("{v} {}", err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or(""))));
    }
    let mut d = helper_command(&set.deno, deno_dir);
    d.arg("--version");
    let out = tokio::time::timeout(Duration::from_secs(60), d.output()).await.map_err(|_| fail("Deno", "timed out".into()))?.map_err(|e| fail("Deno", e.to_string()))?;
    if !out.status.success() || !String::from_utf8_lossy(&out.stdout).starts_with("deno ") {
        return Err(fail("Deno", String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or("").to_string()));
    }
    Ok(())
}

fn sha256_file(p: &Path, token: &CancellationToken) -> io::Result<String> {
    let mut f = fs::File::open(p)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        if token.is_cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// Write via a temporary file and a rename, so readers see all or nothing.
pub fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    {
        let mut f = fs::File::create(&tmp)?;
        io::Write::write_all(&mut f, data)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

fn remove_all(p: &Path) -> io::Result<()> {
    match fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => fs::remove_dir_all(p),
        Ok(_) => fs::remove_file(p),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Move everything in `from` into `to` (both ours, freshly unpacked).
fn merge_into(from: &Path, to: &Path) -> io::Result<()> {
    for e in fs::read_dir(from)?.flatten() {
        let dest = to.join(e.file_name());
        if e.file_type()?.is_dir() && dest.is_dir() {
            merge_into(&e.path(), &dest)?;
        } else {
            remove_all(&dest)?;
            fs::rename(e.path(), &dest)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn set_executable(p: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(p, fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn set_executable(_: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests;
