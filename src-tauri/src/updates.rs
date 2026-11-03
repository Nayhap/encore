//! In-app updates (D24). Encore asks the latest published GitHub release
//! whether a newer version is out, downloads it in the background (or when
//! asked) and installs it only when the user restarts from the UI or quits
//! Encore; it never restarts by itself. `tauri-plugin-updater`, driven from
//! here only, checks the minisign signature and does the platform's install;
//! the policy (when to check, skip, what installs when) is this module's.
//!
//! The rules are in `Machine`, which is pure so they are tested without the
//! network. Every check and download carries a generation: a result from an
//! older one never overwrites newer state. Every change bumps the revision
//! the UI compares (D13).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use encore_core::error::{AppError, ErrorCode, Result};
use parking_lot::Mutex;
use semver::Version;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;
use tauri_specta::Event;
use tokio::sync::Notify;

use crate::core::{AUTO_UPDATE, Core, LAST_SEEN_VERSION, SETTINGS, UPDATE_CHECKED_AT, UPDATE_SKIPPED};
use crate::events::UpdateEvent;

const FIRST_CHECK: Duration = Duration::from_secs(20);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);
/// A reconnect right after a check doesn't check again.
const COALESCE: Duration = Duration::from_secs(15 * 60);
/// After failed checks or downloads: 30 minutes, 2 hours, then 6 hours.
const BACKOFF: [Duration; 3] = [Duration::from_secs(30 * 60), Duration::from_secs(2 * 3600), Duration::from_secs(6 * 3600)];
/// No release of Encore comes close; a bigger answer isn't one.
const MAX_BYTES: u64 = 300 * 1024 * 1024;
const STALL: Duration = Duration::from_secs(60);
const RELEASES: &str = "https://github.com/Nayhap/encore/releases";

const DEV: &str = "Updates are off in development builds.";
const DEB: &str = "You installed Encore from the .deb package, which Encore can't update itself. Download the new .deb from the releases page.";
const OTHER: &str = "This copy of Encore wasn't installed by its installer, so it can't update itself. Download the new version from the releases page.";
const TRANSLOCATED: &str = "Move Encore to your Applications folder, then open it from there to let it update itself.";
const DISK_IMAGE: &str = "Encore is running from its disk image. Drag it to your Applications folder, then open it from there to let it update itself.";

/// How this copy of Encore was installed, which decides how it updates.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum InstallKind {
    /// macOS `.app`: the new bundle replaces it.
    App,
    /// Windows, per user: the new NSIS installer runs.
    Nsis,
    /// Windows, per machine: the new MSI runs (Windows may ask for an administrator).
    Msi,
    /// Linux AppImage: replaced in place.
    AppImage,
    /// Linux `.deb`: never updated by Encore, which links to the release instead.
    Deb,
    /// A development build.
    Dev,
    /// Not from an installer Encore knows.
    Other,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum UpdatePhase {
    Idle,
    Checking,
    UpToDate,
    Available,
    Downloading,
    /// Downloaded and verified: installs on restart or quit.
    Ready,
    Installing,
    Failed,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    /// RFC 3339, as the release says.
    pub date: Option<String>,
    /// The release notes as plain sentences, one per bullet (never HTML).
    pub notes: Vec<String>,
    /// The release on GitHub.
    pub page: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    /// Bumps on every change; the UI ignores older ones.
    pub revision: String,
    pub current: String,
    pub install: InstallKind,
    /// Encore can install updates itself here; otherwise `reason` says why.
    pub can_install: bool,
    pub reason: Option<String>,
    /// Settings → Updates → "Install updates automatically".
    pub auto: bool,
    pub phase: UpdatePhase,
    pub available: Option<UpdateInfo>,
    /// While downloading: bytes so far, and the size when known.
    pub received: Option<f64>,
    pub total: Option<f64>,
    /// The last check, download or install failed: why.
    pub error: Option<String>,
    /// When GitHub last answered a check (Unix seconds).
    pub checked_at: Option<f64>,
    /// The version the user skipped.
    pub skipped: Option<String>,
    /// The ready update installs when Encore quits.
    pub at_quit: bool,
    /// Encore was updated to this version since it last ran (shown once).
    pub updated: Option<UpdateInfo>,
}

/// A newer release, as far as the rules care.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub version: String,
    pub date: Option<String>,
    pub notes: Option<String>,
}

impl Found {
    fn info(&self) -> UpdateInfo {
        info(&self.version, self.date.clone(), self.notes.as_deref())
    }
}

fn info(version: &str, date: Option<String>, notes: Option<&str>) -> UpdateInfo {
    UpdateInfo { version: version.to_string(), date, notes: notes.map(bullets).unwrap_or_default(), page: format!("{RELEASES}/tag/v{version}") }
}

/// What the caller does after a transition.
#[derive(Debug, Default, PartialEq)]
pub struct Next {
    /// Start downloading `found`.
    pub download: bool,
    /// Delete the staged package: it is no longer wanted.
    pub discard: bool,
}

fn newer(a: &str, b: &str) -> bool {
    match (Version::parse(a), Version::parse(b)) {
        (Ok(a), Ok(b)) => a > b,
        _ => false,
    }
}

/// The update rules, without I/O.
pub struct Machine {
    current: String,
    can_install: bool,
    auto: bool,
    skipped: Option<String>,
    revision: u64,
    generation: u64,
    phase: UpdatePhase,
    found: Option<Found>,
    /// The verified package's version, while `Ready` (or installing).
    staged: Option<String>,
    /// The user asked for this download: it installs at quit even with
    /// automatic updates off.
    requested: bool,
    received: u64,
    total: Option<u64>,
    error: Option<String>,
    checked_at: Option<i64>,
    failures: usize,
}

impl Machine {
    pub fn new(current: &str, can_install: bool, auto: bool, skipped: Option<String>, checked_at: Option<i64>) -> Self {
        Machine { current: current.into(), can_install, auto, skipped, revision: 1, generation: 0, phase: UpdatePhase::Idle, found: None, staged: None, requested: false, received: 0, total: None, error: None, checked_at, failures: 0 }
    }

    fn bump(&mut self) {
        self.revision += 1;
    }

    pub fn phase(&self) -> UpdatePhase {
        self.phase
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    #[cfg(test)]
    pub fn found(&self) -> Option<&Found> {
        self.found.as_ref()
    }

    /// A check starts. Automatic checks wait while one runs, and nothing
    /// checks during a download or an install. While an update is ready the
    /// check runs quietly: it only matters if something newer is out.
    pub fn begin_check(&mut self, manual: bool) -> Option<u64> {
        match self.phase {
            UpdatePhase::Downloading | UpdatePhase::Installing => return None,
            UpdatePhase::Checking if !manual => return None,
            _ => {}
        }
        self.generation += 1;
        if self.phase != UpdatePhase::Ready {
            self.phase = UpdatePhase::Checking;
        }
        self.bump();
        Some(self.generation)
    }

    /// What GitHub said. `Ok(None)`: nothing newer than this Encore.
    pub fn check_done(&mut self, generation: u64, manual: bool, r: std::result::Result<Option<Found>, String>, now: i64) -> Next {
        if generation != self.generation {
            return Next::default();
        }
        self.bump();
        let f = match r {
            Err(e) => {
                self.failures += 1;
                self.error = Some(e);
                if self.phase == UpdatePhase::Checking {
                    self.phase = if self.found.is_some() { UpdatePhase::Available } else { UpdatePhase::Failed };
                }
                return Next::default();
            }
            Ok(f) => f,
        };
        self.failures = 0;
        self.error = None;
        self.checked_at = Some(now);
        let mut next = Next::default();
        let Some(f) = f.filter(|f| newer(&f.version, &self.current)) else {
            // A ready update whose release was pulled isn't installed.
            next.discard = self.staged.take().is_some();
            self.found = None;
            self.requested = false;
            self.phase = UpdatePhase::UpToDate;
            return next;
        };
        if let Some(s) = &self.staged {
            if *s == f.version {
                self.found = Some(f);
                return next;
            }
            // Something newer, or the staged release was pulled: start over.
            self.staged = None;
            next.discard = true;
        }
        let skipped = self.skipped.as_deref() == Some(f.version.as_str());
        self.found = Some(f);
        self.phase = UpdatePhase::Available;
        self.requested = false;
        // A manual check shows a skipped version, but doesn't fetch it.
        next.download = self.can_install && self.auto && !skipped;
        let _ = manual;
        next
    }

    /// Start downloading what was found: automatically, or because the
    /// user asked (`requested`).
    pub fn begin_download(&mut self, requested: bool) -> Option<u64> {
        if !self.can_install || self.found.is_none() || !matches!(self.phase, UpdatePhase::Available | UpdatePhase::Failed) {
            return None;
        }
        self.generation += 1;
        self.phase = UpdatePhase::Downloading;
        self.received = 0;
        self.total = None;
        self.error = None;
        self.requested = requested;
        self.bump();
        Some(self.generation)
    }

    pub fn progress(&mut self, generation: u64, received: u64, total: Option<u64>) -> bool {
        if generation != self.generation || self.phase != UpdatePhase::Downloading || (self.received, self.total) == (received, total) {
            return false;
        }
        self.received = received;
        self.total = total;
        self.bump();
        true
    }

    /// The download ended: staged and verified, or failed. False when it
    /// was cancelled meanwhile (the caller deletes what it staged).
    pub fn download_done(&mut self, generation: u64, r: std::result::Result<(), String>) -> bool {
        if generation != self.generation || self.phase != UpdatePhase::Downloading {
            return false;
        }
        self.bump();
        match r {
            Ok(()) => {
                self.phase = UpdatePhase::Ready;
                self.staged = self.found.as_ref().map(|f| f.version.clone());
            }
            Err(e) => {
                self.failures += 1;
                self.phase = UpdatePhase::Failed;
                self.error = Some(e);
            }
        }
        true
    }

    /// A package staged by an earlier run, checked again at launch.
    pub fn restore_ready(&mut self, f: Found, requested: bool) {
        self.staged = Some(f.version.clone());
        self.found = Some(f);
        self.requested = requested;
        self.phase = UpdatePhase::Ready;
        self.bump();
    }

    /// Skip a version: automatic checks leave it alone from now on. A
    /// download of it stops, and a staged copy goes.
    pub fn skip(&mut self, version: &str) -> Next {
        self.skipped = Some(version.to_string());
        self.bump();
        let mut next = Next::default();
        if self.found.as_ref().is_some_and(|f| f.version == version) {
            match self.phase {
                UpdatePhase::Downloading => {
                    self.generation += 1;
                    self.phase = UpdatePhase::Available;
                }
                UpdatePhase::Ready => {
                    self.staged = None;
                    self.phase = UpdatePhase::Available;
                    next.discard = true;
                }
                _ => {}
            }
            self.requested = false;
        }
        next
    }

    pub fn unskip(&mut self) -> Next {
        self.skipped = None;
        self.bump();
        Next { download: self.wants_download(), discard: false }
    }

    /// Off: an automatic download stops, and an update it downloaded no
    /// longer installs at quit. On: what was found is fetched.
    pub fn set_auto(&mut self, on: bool) -> Next {
        self.auto = on;
        self.bump();
        if !on && self.phase == UpdatePhase::Downloading && !self.requested {
            self.generation += 1;
            self.phase = UpdatePhase::Available;
        }
        Next { download: on && self.wants_download(), discard: false }
    }

    fn wants_download(&self) -> bool {
        self.auto && self.can_install && self.phase == UpdatePhase::Available && self.found.as_ref().is_some_and(|f| self.skipped.as_deref() != Some(f.version.as_str()))
    }

    /// Quitting now installs the ready update.
    pub fn at_quit(&self) -> bool {
        self.phase == UpdatePhase::Ready && (self.auto || self.requested)
    }

    pub fn begin_install(&mut self) -> bool {
        if self.phase != UpdatePhase::Ready {
            return false;
        }
        self.phase = UpdatePhase::Installing;
        self.error = None;
        self.bump();
        true
    }

    /// The install failed; the package stays for another try.
    pub fn install_failed(&mut self, e: String) {
        self.phase = UpdatePhase::Ready;
        self.error = Some(e);
        self.bump();
    }

    pub fn set_can_install(&mut self, on: bool) {
        if self.can_install != on {
            self.can_install = on;
            self.bump();
        }
    }

    /// How long until the next automatic check.
    pub fn next_wait(&self) -> Duration {
        match self.failures {
            0 => CHECK_EVERY,
            n => BACKOFF[(n - 1).min(BACKOFF.len() - 1)],
        }
    }

    pub fn checked_at(&self) -> Option<i64> {
        self.checked_at
    }

    pub fn status(&self, install: InstallKind, reason: Option<String>, updated: Option<UpdateInfo>) -> UpdateStatus {
        let downloading = self.phase == UpdatePhase::Downloading;
        UpdateStatus {
            revision: self.revision.to_string(),
            current: self.current.clone(),
            install,
            can_install: self.can_install,
            reason,
            auto: self.auto,
            phase: self.phase,
            available: self.found.as_ref().map(Found::info),
            received: downloading.then_some(self.received as f64),
            total: if downloading { self.total.map(|t| t as f64) } else { None },
            error: self.error.clone(),
            checked_at: self.checked_at.map(|t| t as f64),
            skipped: self.skipped.clone(),
            at_quit: self.at_quit(),
            updated,
        }
    }
}

/// Release notes (Markdown) as plain bullets: each `- ` item with its
/// continuation lines, without emphasis, `<kbd>`, tags or link targets.
/// Headings and other paragraphs are left out.
pub fn bullets(md: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur: Option<String> = None;
    for line in md.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
            out.extend(cur.take());
            cur = Some(rest.to_string());
        } else if t.is_empty() || t.starts_with('#') {
            out.extend(cur.take());
        } else if let Some(c) = cur.as_mut() {
            c.push(' ');
            c.push_str(t);
        }
    }
    out.extend(cur);
    out.iter().map(|b| plain(b)).filter(|b| !b.is_empty()).collect()
}

fn plain(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(c) = rest.chars().next() {
        match c {
            // [text](target) → text
            '[' => {
                if let Some((text, after)) = rest[1..].split_once("](")
                    && let Some(end) = after.find(')')
                {
                    out.push_str(text);
                    rest = &after[end + 1..];
                    continue;
                }
                out.push('[');
                rest = &rest[1..];
            }
            '<' => match rest.find('>') {
                Some(end) => rest = &rest[end + 1..],
                None => {
                    out.push('<');
                    rest = &rest[1..];
                }
            },
            '*' | '`' => rest = &rest[1..],
            _ => {
                out.push(c);
                rest = &rest[c.len_utf8()..];
            }
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------- the install ----------

/// How Encore was installed, and why it can't update itself here (if it
/// can't). Checked at startup and again right before an install.
pub fn classify() -> (InstallKind, Option<String>) {
    if cfg!(debug_assertions) {
        return (InstallKind::Dev, Some(DEV.into()));
    }
    use tauri::utils::config::BundleType;
    match tauri::utils::platform::bundle_type() {
        // Always `App` on macOS, development builds included (hence the
        // debug check above).
        Some(BundleType::App) => (InstallKind::App, std::env::current_exe().ok().and_then(|exe| app_refusal(&exe, writable))),
        Some(BundleType::Nsis) => (InstallKind::Nsis, None),
        Some(BundleType::Msi) => (InstallKind::Msi, None),
        Some(BundleType::AppImage) => match std::env::var_os("APPIMAGE") {
            Some(p) => (InstallKind::AppImage, appimage_refusal(Path::new(&p), writable)),
            None => (InstallKind::AppImage, Some(OTHER.into())),
        },
        Some(BundleType::Deb) => (InstallKind::Deb, Some(DEB.into())),
        _ => (InstallKind::Other, Some(OTHER.into())),
    }
}

/// Whether this user may write to `p`.
#[cfg(unix)]
fn writable(p: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(p.as_os_str().as_bytes()).map_err(std::io::Error::other)?;
    // SAFETY: a valid NUL-terminated path that outlives the call.
    if unsafe { libc::access(c.as_ptr(), libc::W_OK) } == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

#[cfg(not(unix))]
fn writable(_: &Path) -> std::io::Result<()> {
    Ok(())
}

/// macOS: the bundle is swapped for the new one with a rename in its
/// folder. That needs the bundle and its folder to be writable here, and
/// a real location: not a translocated copy (an app opened from where it
/// was downloaded runs from a random read-only path) and not the disk
/// image. Refusing here is also what keeps the plugin's fallback, an
/// administrator password dialog from AppleScript, from ever showing.
fn app_refusal(exe: &Path, writable: impl Fn(&Path) -> std::io::Result<()>) -> Option<String> {
    if exe.to_string_lossy().contains("/AppTranslocation/") {
        return Some(TRANSLOCATED.into());
    }
    let bundle = exe.parent()?.parent()?.parent()?;
    if bundle.extension().and_then(|e| e.to_str()) != Some("app") {
        return Some(OTHER.into());
    }
    let folder = bundle.parent()?;
    for p in [bundle, folder] {
        if let Err(e) = writable(p) {
            return Some(if e.kind() == std::io::ErrorKind::ReadOnlyFilesystem {
                DISK_IMAGE.into()
            } else {
                format!("Encore can't update itself in {}, because that folder isn't writable for you. Move Encore to your Applications folder, or download the new version from the releases page.", folder.display())
            });
        }
    }
    None
}

/// Linux: the AppImage file is replaced where it is.
fn appimage_refusal(file: &Path, writable: impl Fn(&Path) -> std::io::Result<()>) -> Option<String> {
    let folder = file.parent()?;
    if writable(file).is_err() || writable(folder).is_err() {
        return Some(format!("Encore can't replace its AppImage in {}, because that folder isn't writable for you. Move the AppImage to a folder of yours, such as ~/Applications, and open it from there.", folder.display()));
    }
    None
}

/// The package's file ending, from its download URL.
fn package_ext(url: &url::Url) -> String {
    let name = url.path_segments().and_then(|mut s| s.next_back()).unwrap_or_default();
    for e in [".app.tar.gz", ".AppImage.tar.gz", ".tar.gz", ".msi.zip", ".exe.zip"] {
        if name.ends_with(e) {
            return e[1..].to_string();
        }
    }
    Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("bin").to_string()
}

/// What failed, in words for the status line.
fn reason(e: &tauri_plugin_updater::Error) -> String {
    use tauri_plugin_updater::Error as E;
    match e {
        E::Reqwest(r) if r.is_timeout() => "GitHub took too long to answer".into(),
        E::Reqwest(_) | E::Network(_) => "GitHub didn't answer".into(),
        E::ReleaseNotFound => "GitHub has no update information for the latest release".into(),
        E::TargetNotFound(_) | E::TargetsNotFound(_) => "the new version has no download for this system".into(),
        E::Serialization(_) | E::Semver(_) | E::UrlParse(_) => "the release information couldn't be read".into(),
        E::Minisign(_) | E::Base64(_) | E::SignatureUtf8(_) | E::SignedVersionMismatch { .. } | E::MissingSignedVersion => "the download didn't match its signature".into(),
        E::Io(io) => format!("a file couldn't be written ({})", io.kind()),
        other => other.to_string(),
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// `<data>/update-endpoint`: one https URL to check instead of GitHub's
/// latest release, for testing a build end to end. http is refused.
fn endpoint_override(data: &Path) -> Option<url::Url> {
    let text = std::fs::read_to_string(data.join("update-endpoint")).ok()?;
    let line = text.trim();
    match url::Url::parse(line) {
        Ok(u) if u.scheme() == "https" => {
            tracing::warn!(endpoint = %u, "updates: checking a test endpoint instead of GitHub");
            Some(u)
        }
        _ => {
            tracing::warn!("updates: ignoring update-endpoint, which isn't one https URL");
            None
        }
    }
}

/// The staged package of an earlier run.
struct Staged {
    version: String,
    package: PathBuf,
    raw: serde_json::Value,
    requested: bool,
}

/// Files in `<cache>/updates`: `<version>.<ext>` beside `<version>.json`
/// (the raw manifest), and `<version>.requested` when the user asked for
/// it. At most one version is kept.
fn read_staged(dir: &Path, current: &str) -> (Option<Staged>, Option<serde_json::Value>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return (None, None) };
    let mut best: Option<Staged> = None;
    let mut installed = None;
    let names: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    for p in names.iter().filter(|p| p.extension().is_some_and(|e| e == "json")) {
        let Some(v) = p.file_stem().and_then(|s| s.to_str()).map(str::to_string) else { continue };
        let raw = std::fs::read(p).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
        if v == current {
            // Just installed: its notes are this version's "What's new".
            installed = raw;
            continue;
        }
        let package = names.iter().find(|q| {
            let n = q.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            n.starts_with(&format!("{v}.")) && !n.ends_with(".json") && !n.ends_with(".requested") && !n.ends_with(".part")
        });
        if let (Some(raw), Some(package), true) = (raw, package, newer(&v, current))
            && best.as_ref().is_none_or(|b| newer(&v, &b.version))
        {
            let requested = dir.join(format!("{v}.requested")).exists();
            best = Some(Staged { version: v, package: package.clone(), raw, requested });
        }
    }
    // Everything else goes: older versions, partial writes, leftovers.
    for p in &names {
        let keep = best.as_ref().is_some_and(|b| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&format!("{}.", b.version)) && !n.ends_with(".part")));
        if !keep {
            let _ = std::fs::remove_file(p);
        }
    }
    (best, installed)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("{}.part", path.extension().and_then(|e| e.to_str()).unwrap_or("")));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

// ---------- the driver ----------

pub struct Updates {
    app: AppHandle,
    core: Arc<Core>,
    kind: InstallKind,
    reason: Mutex<Option<String>>,
    m: Mutex<Machine>,
    /// The plugin's view of what was found: it verifies and installs.
    update: Mutex<Option<tauri_plugin_updater::Update>>,
    /// The verified package on disk, while ready.
    package: Mutex<Option<PathBuf>>,
    /// Staging and its generation check happen one at a time.
    staging: tokio::sync::Mutex<()>,
    dir: PathBuf,
    endpoint: Option<url::Url>,
    wake: Notify,
    last_attempt: Mutex<Option<Instant>>,
    updated: Option<UpdateInfo>,
    /// Quitting (or restarting into the update) has begun.
    quitting: AtomicBool,
}

impl Updates {
    pub fn new(app: AppHandle, core: Arc<Core>) -> Arc<Self> {
        // The version the updater compares with (tauri.conf.json's; the
        // release checks that it is Cargo's too).
        let current = app.package_info().version.to_string();
        let current = current.as_str();
        let (kind, reason) = classify();
        let store = &core.store;
        let auto = store.get::<bool>(SETTINGS, AUTO_UPDATE).ok().flatten().unwrap_or(true);
        let skipped = store.get::<String>(SETTINGS, UPDATE_SKIPPED).ok().flatten();
        let checked = store.get::<i64>(SETTINGS, UPDATE_CHECKED_AT).ok().flatten();
        let dir = core.paths.cache.join("updates");
        let (staged, installed) = read_staged(&dir, current);
        // "What's new" once, the first time a newer version runs.
        let last = store.get::<String>(SETTINGS, LAST_SEEN_VERSION).ok().flatten();
        let updated = match &last {
            Some(l) if newer(current, l) => {
                let notes = installed.as_ref().and_then(|r| r.get("notes")).and_then(|n| n.as_str());
                let date = installed.as_ref().and_then(|r| r.get("pub_date")).and_then(|d| d.as_str()).map(str::to_string);
                Some(info(current, date, notes))
            }
            _ => None,
        };
        if last.as_deref() != Some(current) {
            let _ = store.put(SETTINGS, LAST_SEEN_VERSION, &current.to_string(), None);
        }
        let endpoint = endpoint_override(&core.paths.data);
        tracing::info!(install = ?kind, can = reason.is_none(), auto, "updates");
        let me = Arc::new(Updates {
            app,
            core,
            kind,
            m: Mutex::new(Machine::new(current, reason.is_none(), auto, skipped.clone(), checked)),
            reason: Mutex::new(reason),
            update: Mutex::new(None),
            package: Mutex::new(None),
            staging: tokio::sync::Mutex::new(()),
            dir,
            endpoint,
            wake: Notify::new(),
            last_attempt: Mutex::new(None),
            updated,
            quitting: AtomicBool::new(false),
        });
        if let Some(s) = staged {
            me.restore(s, skipped.as_deref());
        }
        me
    }

    fn updater(&self) -> std::result::Result<tauri_plugin_updater::Updater, tauri_plugin_updater::Error> {
        let mut b = self.app.updater_builder().timeout(Duration::from_secs(30));
        if let Some(u) = &self.endpoint {
            b = b.endpoints(vec![u.clone()])?;
        }
        b.build()
    }

    /// A package from an earlier run: rebuilt from its saved manifest with
    /// today's key and rules, and verified again before it counts.
    fn restore(&self, s: Staged, skipped: Option<&str>) {
        let drop_it = |why: &str| {
            tracing::info!(version = %s.version, why, "updates: dropped the downloaded update");
            for e in ["json", "requested"] {
                let _ = std::fs::remove_file(self.dir.join(format!("{}.{e}", s.version)));
            }
            let _ = std::fs::remove_file(&s.package);
        };
        if skipped == Some(s.version.as_str()) {
            return drop_it("skipped");
        }
        if self.reason.lock().is_some() {
            return drop_it("can't install here");
        }
        let update = match self.updater().and_then(|u| u.restore_update(s.raw.clone())) {
            Ok(Some(u)) => u,
            Ok(None) => return drop_it("not newer"),
            Err(e) => return drop_it(&reason(&e)),
        };
        let ok = std::fs::read(&s.package).map_err(|e| e.to_string()).and_then(|b| update.verify(&b).map_err(|e| reason(&e)));
        if let Err(e) = ok {
            return drop_it(&e);
        }
        let found = Found { version: update.version.clone(), date: update.raw_json.get("pub_date").and_then(|d| d.as_str()).map(str::to_string), notes: update.body.clone() };
        tracing::info!(version = %found.version, "updates: a downloaded update is ready");
        self.m.lock().restore_ready(found, s.requested);
        *self.update.lock() = Some(update);
        *self.package.lock() = Some(s.package);
    }

    pub fn status(&self) -> UpdateStatus {
        self.m.lock().status(self.kind, self.reason.lock().clone(), self.updated.clone())
    }

    fn emit(&self) {
        let _ = UpdateEvent(self.status()).emit(&self.app);
    }

    /// Automatic checks: 20 s after launch, then every 6 hours (sooner
    /// after a failure), and on reconnect when the last check is old.
    /// Offline, they wait; development builds never check.
    pub fn start(self: &Arc<Self>) {
        if self.kind == InstallKind::Dev {
            return;
        }
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(FIRST_CHECK).await;
            loop {
                if me.core.connectivity.online() {
                    me.check(false).await;
                }
                let wait = me.m.lock().next_wait();
                tokio::select! {
                    _ = tokio::time::sleep(wait) => {}
                    _ = me.wake.notified() => {}
                }
            }
        });
        let me = Arc::downgrade(self);
        self.core.connectivity.on_change(move |on| {
            if let (true, Some(me)) = (on, me.upgrade()) {
                me.reconnected();
            }
        });
    }

    /// Back online: check if the last answer is older than 6 hours, unless
    /// a check was just tried (launch and reconnect coalesce).
    fn reconnected(&self) {
        let stale = self.m.lock().checked_at().is_none_or(|t| unix_now() - t > CHECK_EVERY.as_secs() as i64);
        let recent = self.last_attempt.lock().is_some_and(|t| t.elapsed() < COALESCE);
        if stale && !recent {
            // Only a waiting loop is woken: one still in its first wait checks anyway.
            self.wake.notify_waiters();
        }
    }

    /// Ask GitHub. A manual check also shows a skipped version, and runs
    /// even when YouTube is unreachable (that says nothing about GitHub).
    /// Failures here never touch Encore's online state (D23).
    pub async fn check(self: &Arc<Self>, manual: bool) -> UpdateStatus {
        if self.kind == InstallKind::Dev {
            return self.status();
        }
        let Some(generation) = self.m.lock().begin_check(manual) else { return self.status() };
        *self.last_attempt.lock() = Some(Instant::now());
        self.emit();
        let t0 = Instant::now();
        let r = match self.updater() {
            Ok(u) => u.check().await,
            Err(e) => Err(e),
        };
        let (found, update) = match r {
            Ok(Some(u)) => {
                let f = Found { version: u.version.clone(), date: u.raw_json.get("pub_date").and_then(|d| d.as_str()).map(str::to_string), notes: u.body.clone() };
                (Ok(Some(f)), Some(u))
            }
            Ok(None) => (Ok(None), None),
            Err(e) => (Err(reason(&e)), None),
        };
        match &found {
            Ok(f) => tracing::info!(manual, latest = f.as_ref().map(|f| f.version.as_str()).unwrap_or("none newer"), ms = t0.elapsed().as_millis() as u64, "updates: checked"),
            Err(e) => tracing::info!(manual, error = %e, "updates: check failed"),
        }
        let now = unix_now();
        let next = {
            let mut m = self.m.lock();
            let fresh = m.generation() == generation;
            let next = m.check_done(generation, manual, found, now);
            if fresh && update.is_some() {
                let same = self.update.lock().as_ref().map(|u| u.version.clone()) == update.as_ref().map(|u| u.version.clone());
                // A ready update keeps the instance its package was verified with.
                if !(same && m.phase() == UpdatePhase::Ready) {
                    *self.update.lock() = update;
                }
            }
            next
        };
        if self.m.lock().checked_at() == Some(now) {
            let _ = self.core.store.put(SETTINGS, UPDATE_CHECKED_AT, &now, None);
        }
        self.follow(next);
        self.emit();
        self.status()
    }

    fn follow(self: &Arc<Self>, next: Next) {
        if next.discard {
            self.discard();
        }
        if next.download {
            self.download(false);
        }
    }

    /// Delete the staged package.
    fn discard(&self) {
        *self.package.lock() = None;
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }

    /// Download in the background: progress at most four times a second,
    /// stopped when it outgrows 300 MB or stalls for a minute, cancelled
    /// (the future dropped) when the generation moves on.
    pub fn download(self: &Arc<Self>, requested: bool) {
        let Some(update) = self.update.lock().clone() else { return };
        let Some(generation) = self.m.lock().begin_download(requested) else { return };
        self.emit();
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            let t0 = Instant::now();
            let received = Arc::new(AtomicU64::new(0));
            let total = Arc::new(AtomicU64::new(u64::MAX));
            let last = Arc::new(Mutex::new(Instant::now()));
            let (r2, t2, l2) = (received.clone(), total.clone(), last.clone());
            let dl = update.download(
                move |n, len| {
                    r2.fetch_add(n as u64, Ordering::Relaxed);
                    if let Some(len) = len {
                        t2.store(len, Ordering::Relaxed);
                    }
                    *l2.lock() = Instant::now();
                },
                || {},
            );
            tokio::pin!(dl);
            let mut tick = tokio::time::interval(Duration::from_millis(250));
            let r: std::result::Result<Vec<u8>, Option<String>> = loop {
                tokio::select! {
                    r = &mut dl => break r.map_err(|e| Some(reason(&e))),
                    _ = tick.tick() => {
                        if me.m.lock().generation() != generation {
                            break Err(None);
                        }
                        let (got, len) = (received.load(Ordering::Relaxed), total.load(Ordering::Relaxed));
                        let len = (len != u64::MAX).then_some(len);
                        if got > MAX_BYTES || len.is_some_and(|l| l > MAX_BYTES) {
                            break Err(Some("the download was larger than any Encore release (over 300 MB), so Encore stopped it".into()));
                        }
                        if last.lock().elapsed() > STALL {
                            break Err(Some("the download stalled".into()));
                        }
                        if me.m.lock().progress(generation, got, len) {
                            me.emit();
                        }
                    }
                }
            };
            let r = match r {
                Err(None) => return,
                Err(Some(e)) => Err(e),
                Ok(bytes) => match me.stage(generation, &update, bytes, requested).await {
                    Ok(true) => Ok(()),
                    Ok(false) => return,
                    Err(e) => Err(e),
                },
            };
            match &r {
                Ok(()) => tracing::info!(version = %update.version, bytes = received.load(Ordering::Relaxed), secs = t0.elapsed().as_secs(), "updates: downloaded and verified"),
                Err(e) => tracing::info!(version = %update.version, error = %e, "updates: download failed"),
            }
            let applied = me.m.lock().download_done(generation, r);
            if !applied {
                me.discard();
            }
            me.emit();
        });
    }

    /// Write the verified package and its manifest, each by temp file and
    /// rename, the manifest last (it is what makes the package count).
    /// `Ok(false)`: cancelled meanwhile, nothing written.
    async fn stage(&self, generation: u64, update: &tauri_plugin_updater::Update, bytes: Vec<u8>, requested: bool) -> std::result::Result<bool, String> {
        let _one = self.staging.lock().await;
        if self.m.lock().generation() != generation {
            return Ok(false);
        }
        let dir = self.dir.clone();
        let v = update.version.clone();
        let package = dir.join(format!("{v}.{}", package_ext(&update.download_url)));
        let raw = serde_json::to_vec(&update.raw_json).unwrap_or_default();
        let p2 = package.clone();
        let r = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            // At most one version: the others go first.
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for e in rd.flatten() {
                    let _ = std::fs::remove_file(e.path());
                }
            }
            std::fs::create_dir_all(&dir)?;
            write_atomic(&p2, &bytes)?;
            if requested {
                std::fs::write(dir.join(format!("{v}.requested")), b"")?;
            }
            write_atomic(&dir.join(format!("{v}.json")), &raw)
        })
        .await
        .map_err(|_| "staging stopped".to_string())?;
        r.map_err(|e| format!("Encore couldn't save the update ({})", e.kind()))?;
        *self.package.lock() = Some(package);
        Ok(true)
    }

    pub fn skip(self: &Arc<Self>, version: &str) -> Result<UpdateStatus> {
        self.core.store.put(SETTINGS, UPDATE_SKIPPED, &version.to_string(), None)?;
        let next = self.m.lock().skip(version);
        self.follow(next);
        self.emit();
        Ok(self.status())
    }

    pub fn unskip(self: &Arc<Self>) -> Result<UpdateStatus> {
        self.core.store.delete(SETTINGS, UPDATE_SKIPPED)?;
        let next = self.m.lock().unskip();
        self.follow(next);
        self.emit();
        Ok(self.status())
    }

    pub fn set_auto(self: &Arc<Self>, on: bool) -> Result<UpdateStatus> {
        self.core.store.put(SETTINGS, AUTO_UPDATE, &on, None)?;
        let next = self.m.lock().set_auto(on);
        self.follow(next);
        self.emit();
        Ok(self.status())
    }

    /// Install the ready update now: Encore pauses (the queue and position
    /// are saved and come back after the restart), settles history,
    /// downloads and the helpers, then hands over to the installer.
    /// `relaunch`: start the new version afterwards (otherwise Encore is
    /// quitting). Returns only if the install failed.
    pub async fn install(self: &Arc<Self>, relaunch: bool) -> Result<()> {
        // Looked at again now: the app may have moved since launch.
        let (_, refusal) = classify();
        if let Some(why) = refusal {
            self.m.lock().set_can_install(false);
            *self.reason.lock() = Some(why.clone());
            self.emit();
            return Err(AppError::new(ErrorCode::Unsupported, why));
        }
        if !self.m.lock().begin_install() {
            return Err(AppError::invalid("No update is ready to install. Check for updates first."));
        }
        self.emit();
        self.quitting.store(true, Ordering::SeqCst);
        let (Some(update), Some(package)) = (self.update.lock().clone(), self.package.lock().clone()) else {
            return self.install_failed("the downloaded update is missing".into()).await;
        };
        self.core.prepare_exit().await;
        let version = update.version.clone();
        tracing::info!(%version, install = ?self.kind, relaunch, "updates: installing");
        // Windows: the installer starts and this process exits inside
        // `install`, which is why everything above came first.
        let r = tokio::task::spawn_blocking(move || {
            let bytes = std::fs::read(&package).map_err(|e| format!("the downloaded update couldn't be read ({})", e.kind()))?;
            update.restart_after_install(relaunch).install(&bytes).map_err(|e| reason(&e))
        })
        .await
        .unwrap_or_else(|_| Err("the installer stopped".into()));
        match r {
            Ok(()) => {
                tracing::info!(%version, "updates: installed");
                if relaunch {
                    self.app.restart();
                }
                Ok(())
            }
            Err(e) => self.install_failed(e).await,
        }
    }

    async fn install_failed(&self, e: String) -> Result<()> {
        tracing::warn!(error = %e, "updates: install failed");
        self.quitting.store(false, Ordering::SeqCst);
        self.core.abort_exit().await;
        let text = format!("Encore couldn't install the update: {e}. Try again, or download Encore from the releases page.");
        self.m.lock().install_failed(text.clone());
        self.emit();
        Err(AppError::internal(text))
    }

    /// Quitting (macOS: Quit or the Dock; Windows and Linux: after the
    /// window closed). True the first time: the caller holds the exit,
    /// runs `quit` and exits after it.
    pub fn begin_quit(&self) -> bool {
        !self.quitting.swap(true, Ordering::SeqCst)
    }

    /// The shared exit (D24): settle everything, then install a ready
    /// update without relaunching. A failed install doesn't stop the quit.
    pub async fn quit(self: &Arc<Self>) {
        self.core.prepare_exit().await;
        if self.m.lock().at_quit() {
            let _ = self.install(false).await;
        }
        // Even after a failed install: this exit goes through, once.
        self.quitting.store(true, Ordering::SeqCst);
        self.core.prepare_exit().await;
    }

    /// Windows and Linux: the window is closing with an update to install,
    /// so it stays up for the moment that takes ("Installing the update…").
    #[cfg(not(target_os = "macos"))]
    pub fn install_on_close(&self) -> bool {
        self.m.lock().at_quit() && !self.quitting.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(v: &str) -> Found {
        Found { version: v.into(), date: None, notes: None }
    }

    fn machine(auto: bool) -> Machine {
        Machine::new("0.5.0", true, auto, None, None)
    }

    #[test]
    fn a_newer_version_downloads_when_automatic() {
        let mut m = machine(true);
        let g = m.begin_check(false).unwrap();
        assert_eq!(m.phase(), UpdatePhase::Checking);
        let n = m.check_done(g, false, Ok(Some(found("0.5.1"))), 100);
        assert_eq!(n, Next { download: true, discard: false });
        assert_eq!(m.phase(), UpdatePhase::Available);
        let d = m.begin_download(false).unwrap();
        assert!(m.progress(d, 10, Some(100)));
        assert!(!m.progress(d, 10, Some(100)), "no change, no event");
        assert!(m.download_done(d, Ok(())));
        assert_eq!(m.phase(), UpdatePhase::Ready);
        assert!(m.at_quit());
        assert_eq!(m.checked_at(), Some(100));
    }

    #[test]
    fn manual_mode_only_tells() {
        let mut m = machine(false);
        let g = m.begin_check(false).unwrap();
        assert_eq!(m.check_done(g, false, Ok(Some(found("0.5.1"))), 1), Next::default());
        assert_eq!(m.phase(), UpdatePhase::Available);
        // Asked for: it downloads, and installs at quit even though automatic is off.
        let d = m.begin_download(true).unwrap();
        m.download_done(d, Ok(()));
        assert!(m.at_quit());
    }

    #[test]
    fn same_or_older_is_up_to_date_and_no_downgrade() {
        let mut m = machine(true);
        for v in ["0.5.0", "0.4.9"] {
            let g = m.begin_check(false).unwrap();
            assert_eq!(m.check_done(g, false, Ok(Some(found(v))), 1), Next::default());
            assert_eq!(m.phase(), UpdatePhase::UpToDate);
            assert!(m.found().is_none());
        }
        // Pre-releases order below their release.
        let mut m = Machine::new("0.5.0-rc.1", true, true, None, None);
        let g = m.begin_check(false).unwrap();
        assert!(m.check_done(g, false, Ok(Some(found("0.5.0"))), 1).download);
    }

    #[test]
    fn skipped_versions_wait_for_a_manual_check_and_a_newer_one_counts() {
        let mut m = Machine::new("0.5.0", true, true, Some("0.5.1".into()), None);
        let g = m.begin_check(false).unwrap();
        assert_eq!(m.check_done(g, false, Ok(Some(found("0.5.1"))), 1), Next::default(), "automatic: ignored");
        let g = m.begin_check(true).unwrap();
        assert_eq!(m.check_done(g, true, Ok(Some(found("0.5.1"))), 1), Next::default(), "manual: shown, not fetched");
        assert_eq!(m.phase(), UpdatePhase::Available);
        assert_eq!(m.status(InstallKind::App, None, None).skipped.as_deref(), Some("0.5.1"));
        let g = m.begin_check(false).unwrap();
        assert!(m.check_done(g, false, Ok(Some(found("0.5.2"))), 1).download, "a newer version isn't skipped");
    }

    #[test]
    fn skipping_stops_a_download_and_drops_a_ready_one() {
        let mut m = machine(true);
        let g = m.begin_check(false).unwrap();
        m.check_done(g, false, Ok(Some(found("0.5.1"))), 1);
        let d = m.begin_download(false).unwrap();
        assert_eq!(m.skip("0.5.1"), Next::default());
        assert_eq!(m.phase(), UpdatePhase::Available);
        assert!(!m.download_done(d, Ok(())), "the cancelled download is stale");
        assert!(m.unskip().download);
        let d = m.begin_download(false).unwrap();
        m.download_done(d, Ok(()));
        assert_eq!(m.skip("0.5.1"), Next { download: false, discard: true });
        assert!(!m.at_quit());
    }

    #[test]
    fn newer_while_ready_starts_over() {
        let mut m = machine(true);
        let g = m.begin_check(false).unwrap();
        m.check_done(g, false, Ok(Some(found("0.5.1"))), 1);
        let d = m.begin_download(false).unwrap();
        m.download_done(d, Ok(()));
        // The same version again: still ready, nothing to do.
        let g = m.begin_check(false).unwrap();
        assert_eq!(m.phase(), UpdatePhase::Ready, "a quiet check while ready");
        assert_eq!(m.check_done(g, false, Ok(Some(found("0.5.1"))), 2), Next::default());
        assert_eq!(m.phase(), UpdatePhase::Ready);
        let g = m.begin_check(false).unwrap();
        assert_eq!(m.check_done(g, false, Ok(Some(found("0.5.2"))), 3), Next { download: true, discard: true });
        assert_eq!(m.found().unwrap().version, "0.5.2");
        // The release was pulled: nothing newer than this Encore any more.
        let d = m.begin_download(false).unwrap();
        m.download_done(d, Ok(()));
        let g = m.begin_check(false).unwrap();
        assert_eq!(m.check_done(g, false, Ok(None), 4), Next { download: false, discard: true });
        assert_eq!(m.phase(), UpdatePhase::UpToDate);
    }

    #[test]
    fn late_results_never_overwrite_newer_state() {
        let mut m = machine(true);
        let old = m.begin_check(true).unwrap();
        let new = m.begin_check(true).unwrap();
        m.check_done(new, true, Ok(None), 2);
        assert_eq!(m.check_done(old, true, Ok(Some(found("0.5.1"))), 1), Next::default());
        assert_eq!(m.phase(), UpdatePhase::UpToDate);
        assert_eq!(m.checked_at(), Some(2));
        // An automatic check waits for one in progress; nothing checks while downloading.
        m.begin_check(false).unwrap();
        assert!(m.begin_check(false).is_none());
        let g = m.begin_check(true).unwrap();
        m.check_done(g, true, Ok(Some(found("0.5.1"))), 3);
        m.begin_download(false).unwrap();
        assert!(m.begin_check(true).is_none());
        // Turning automatic off cancels the automatic download.
        let rev = m.status(InstallKind::App, None, None).revision;
        m.set_auto(false);
        assert_eq!(m.phase(), UpdatePhase::Available);
        assert_ne!(m.status(InstallKind::App, None, None).revision, rev, "every change bumps the revision");
    }

    #[test]
    fn failures_back_off_and_keep_what_was_found() {
        let mut m = machine(false);
        assert_eq!(m.next_wait(), CHECK_EVERY);
        let g = m.begin_check(false).unwrap();
        m.check_done(g, false, Err("GitHub didn't answer".into()), 1);
        assert_eq!(m.phase(), UpdatePhase::Failed);
        assert_eq!(m.checked_at(), None, "a failed check isn't an answer");
        assert_eq!(m.next_wait(), BACKOFF[0]);
        let g = m.begin_check(false).unwrap();
        m.check_done(g, false, Err("x".into()), 2);
        assert_eq!(m.next_wait(), BACKOFF[1]);
        for _ in 0..3 {
            let g = m.begin_check(false).unwrap();
            m.check_done(g, false, Err("x".into()), 3);
        }
        assert_eq!(m.next_wait(), BACKOFF[2]);
        let g = m.begin_check(false).unwrap();
        m.check_done(g, false, Ok(Some(found("0.5.1"))), 4);
        assert_eq!(m.next_wait(), CHECK_EVERY);
        let g = m.begin_check(false).unwrap();
        m.check_done(g, false, Err("x".into()), 5);
        assert_eq!(m.phase(), UpdatePhase::Available, "still says what it found");
    }

    #[test]
    fn automatic_off_disarms_install_at_quit() {
        let mut m = machine(true);
        let g = m.begin_check(false).unwrap();
        m.check_done(g, false, Ok(Some(found("0.5.1"))), 1);
        let d = m.begin_download(false).unwrap();
        m.download_done(d, Ok(()));
        assert!(m.at_quit());
        m.set_auto(false);
        assert!(!m.at_quit());
        assert_eq!(m.phase(), UpdatePhase::Ready, "Restart to update still works");
        assert!(m.begin_install());
        m.install_failed("x".into());
        assert_eq!(m.phase(), UpdatePhase::Ready);
    }

    #[test]
    fn where_it_cant_install_it_only_tells() {
        let mut m = Machine::new("0.5.0", false, true, None, None);
        let g = m.begin_check(false).unwrap();
        assert_eq!(m.check_done(g, false, Ok(Some(found("0.5.1"))), 1), Next::default());
        assert!(m.begin_download(true).is_none());
        assert_eq!(m.phase(), UpdatePhase::Available);
    }

    #[test]
    fn mac_installs_need_a_writable_real_location() {
        let ok = |_: &Path| Ok(());
        let exe = |p: &str| PathBuf::from(format!("{p}/Encore.app/Contents/MacOS/encore"));
        assert_eq!(app_refusal(&exe("/Applications"), ok), None);
        assert_eq!(app_refusal(&exe("/private/var/folders/x/AppTranslocation/1234/d"), ok).as_deref(), Some(TRANSLOCATED));
        let ro = |_: &Path| Err(std::io::Error::from(std::io::ErrorKind::ReadOnlyFilesystem));
        assert_eq!(app_refusal(&exe("/Volumes/Encore"), ro).as_deref(), Some(DISK_IMAGE));
        let denied = |p: &Path| if p == Path::new("/Applications") { Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)) } else { Ok(()) };
        assert!(app_refusal(&exe("/Applications"), denied).unwrap().contains("/Applications"));
        assert_eq!(app_refusal(Path::new("/usr/local/bin/encore"), ok).as_deref(), Some(OTHER));
        assert_eq!(appimage_refusal(Path::new("/home/u/Apps/Encore.AppImage"), ok), None);
        assert!(appimage_refusal(Path::new("/opt/Encore.AppImage"), |_: &Path| Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))).is_some());
    }

    #[test]
    fn notes_become_plain_bullets() {
        let md = "## What's new\n\n- **Download songs** from any track's menu, or press <kbd>D</kbd>\n  for the song that's playing.\n- See [the guide](https://example.com) and `encore-cli`.\n\n## Fixes\n\n- A *fix*.\n\nEncore is unofficial.\n";
        assert_eq!(bullets(md), vec!["Download songs from any track's menu, or press D for the song that's playing.", "See the guide and encore-cli.", "A fix."]);
        assert!(bullets("").is_empty());
        assert_eq!(plain("a < b [c] (d)"), "a < b [c] (d)", "stray brackets stay");
    }

    #[test]
    fn package_names() {
        let u = |s: &str| url::Url::parse(s).unwrap();
        assert_eq!(package_ext(&u("https://x/Encore_0.5.0_universal.app.tar.gz")), "app.tar.gz");
        assert_eq!(package_ext(&u("https://x/Encore_0.5.0_x64-setup.exe")), "exe");
        assert_eq!(package_ext(&u("https://x/Encore_0.5.0_x64_en-US.msi")), "msi");
        assert_eq!(package_ext(&u("https://x/Encore_0.5.0_amd64.AppImage")), "AppImage");
    }

    #[test]
    fn staged_files_keep_one_newer_version() {
        let dir = std::env::temp_dir().join(format!("encore-updates-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let w = |n: &str, b: &[u8]| std::fs::write(dir.join(n), b).unwrap();
        w("0.5.0.json", br#"{"version":"0.5.0","notes":"- Hi"}"#);
        w("0.5.0.app.tar.gz", b"old");
        w("0.5.1.json", br#"{"version":"0.5.1"}"#);
        w("0.5.1.app.tar.gz", b"one");
        w("0.5.2.json", br#"{"version":"0.5.2"}"#);
        w("0.5.2.app.tar.gz", b"two");
        w("0.5.2.requested", b"");
        w("0.5.3.app.tar.gz.part", b"partial");
        let (s, installed) = read_staged(&dir, "0.5.0");
        let s = s.unwrap();
        assert_eq!(s.version, "0.5.2");
        assert!(s.requested);
        assert_eq!(installed.unwrap()["notes"], "- Hi");
        let mut left: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, vec!["0.5.2.app.tar.gz", "0.5.2.json", "0.5.2.requested"]);
        // Once that version runs, it all goes.
        let (s, _) = read_staged(&dir, "0.5.2");
        assert!(s.is_none());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
