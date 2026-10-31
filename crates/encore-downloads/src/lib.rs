//! Downloads (D23): songs kept on this computer to play without a
//! connection, and playlists, albums and liked songs kept downloaded and up
//! to date.
//!
//! * One worker fetches one song at a time. The yt-dlp helper only resolves
//!   (D3); the bytes come over checked HTTP ranges (`fetch`).
//! * Resolves wait while playback resolves (`PlaybackGate`) and are spaced
//!   2–4 s apart. A bot check pauses everything for 10 minutes, then 30,
//!   then stops until the user retries.
//! * A file counts once it decodes and is hashed, and is installed only
//!   while nothing cancelled it (`Store::download_commit`).
//! * Songs are per device; pins and kept collections are per account. Sign
//!   out never deletes anything.
//! * Lyrics and the cover are kept alongside, best effort.

pub mod fetch;
pub mod files;
pub mod sync;
pub mod types;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use encore_audio::AudioCache;
use encore_audio::cache::sanitize_key;
use encore_core::downloads::{Commit, DownloadRow, Finished, Format, KeptRow, RowState, next_attempt};
use encore_core::error::{AppError, Result};
use encore_core::model::{Collection, Entry, ItemId, Track};
use encore_core::service::{LocalAudio, LocalFile, LoudnessSource, MusicService, PlayFailureKind, PlaybackGate, ResolveAuth, ResolvedStream, StreamResolver};
use encore_core::store::{Store, now};
use encore_lyrics::LyricsService;
use parking_lot::Mutex;
use tokio::sync::{Notify, broadcast};

pub use fetch::{FetchError, Fetched, Fetcher, HttpFetcher};
use fetch::{Outcome, Source, TransferError};
use files::{Layout, Sidecar, WorkerLock};
use sync::Traversal;
pub use types::*;

pub type AuthProvider = Arc<dyn Fn() -> Option<ResolveAuth> + Send + Sync>;

/// Pins of songs found again from their files after the index was lost.
pub const RECOVERED: &str = "recovered";
const SETTINGS: &str = "settings";
const PAUSED: &str = "downloads_paused";
/// Seconds before the next try after a failure; then the song fails.
const RETRIES: [i64; 3] = [10, 60, 300];
/// Free space kept beyond the song itself.
const HEADROOM: u64 = 64 * 1024 * 1024;
/// Kept collections are checked at most this often while nothing changes.
const SYNC_EVERY: i64 = 6 * 3600;
/// At launch and on reconnect, unless checked this recently.
const SYNC_SOON: i64 = 30 * 60;
/// When to ask again for lyrics nobody had.
const LYRICS_RETRY: i64 = 7 * 24 * 3600;
const GONE: &str = "This is no longer on YouTube Music, or it's private now. Your downloads of it stay.";
const CHECK_FAILED: &str = "Couldn't check for changes.";

/// Covers come from the app's artwork cache (D14).
#[async_trait]
pub trait ArtSource: Send + Sync {
    /// The 544 px cover for an art key.
    async fn cover(&self, key: &str) -> Option<Vec<u8>>;
}

pub struct Deps {
    pub store: Arc<Store>,
    pub service: Arc<dyn MusicService>,
    pub resolver: Arc<dyn StreamResolver>,
    pub auth: AuthProvider,
    pub gate: Arc<PlaybackGate>,
    pub fetcher: Arc<dyn Fetcher>,
    /// The playback cache: complete files there are copied, not fetched.
    pub cache: Option<AudioCache>,
    pub lyrics: Option<Arc<LyricsService>>,
    pub art: Option<Arc<dyn ArtSource>>,
    pub loudness: Option<Arc<dyn LoudnessSource>>,
    /// Pause between requests to YouTube, picked between the two.
    pub spacing: (Duration, Duration),
}

#[derive(Clone, Debug)]
pub enum DownloadsUpdate {
    Summary(DownloadsSummary),
    /// A song changed state; `revision` is the summary revision it made.
    Item { revision: u64, item: DownloadItem },
}

#[derive(Default)]
struct Breaker {
    /// Bot checks in a row: 1 waits 10 minutes, 2 waits 30, 3 stops.
    level: u8,
    until: Option<Instant>,
    stopped: bool,
}

struct Active {
    video_id: String,
    attempt: u64,
    bytes: u64,
    total: Option<u64>,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
struct Counts {
    ready: u32,
    waiting: u32,
    failed: u32,
    /// Finished songs, partial files and covers.
    bytes: u64,
}

struct Mem {
    ready: HashMap<String, LocalFile>,
    revision: u64,
    paused: bool,
    online: bool,
    setup_ready: bool,
    disk_full: bool,
    elsewhere: bool,
    breaker: Breaker,
    active: Option<Active>,
    syncing: HashSet<String>,
    counts: Counts,
    last_progress: Option<Instant>,
    last_request: Option<Instant>,
    /// Covers and lyrics that couldn't be fetched this session.
    art_tried: HashSet<String>,
    lyrics_tried: HashSet<String>,
    started: bool,
}

pub struct DownloadManager {
    d: Deps,
    layout: Layout,
    mem: Mutex<Mem>,
    tx: broadcast::Sender<DownloadsUpdate>,
    wake: Notify,
    lock: Mutex<Option<WorkerLock>>,
    on_network_error: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    rt: tokio::runtime::Handle,
}

enum JobError {
    Challenged,
    SetupNeeded,
    DiskFull,
    /// Worth another try later; counts toward the limit.
    Retry(String),
    /// About this song: it fails now.
    Fail(String),
}

fn jitter() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos() as u64).unwrap_or(0)
}

/// Playlists are kept under their `VL…` browse id, whichever form the UI has.
pub fn normalize(id: &ItemId) -> ItemId {
    let r = id.remote();
    if r.starts_with("MPRE") || r.starts_with("VL") || r.starts_with("FE") { id.clone() } else { ItemId::ytm(&format!("VL{r}")) }
}

fn item_of(r: &DownloadRow) -> DownloadItem {
    DownloadItem {
        track: r.track.clone(),
        state: match r.state {
            RowState::Queued => DownloadState::Queued,
            RowState::Active => DownloadState::Downloading,
            RowState::Ready => DownloadState::Ready,
            RowState::Failed => DownloadState::Failed,
            RowState::Deleting => DownloadState::Removed,
        },
        error: r.error.clone(),
        bytes: r.bytes as f64,
        codec: r.codec.clone(),
        kbps: r.kbps,
        itag: r.itag,
        added: r.added as f64,
        version: r.version.to_string(),
        attempt: r.attempt.to_string(),
    }
}

fn wait_of(m: &Mem) -> Option<(DownloadWait, String)> {
    if !m.online {
        return Some((DownloadWait::Offline, "You're offline. Downloads continue when you're back online.".into()));
    }
    if m.elsewhere {
        return Some((DownloadWait::Elsewhere, "Another Encore is downloading. Downloads continue here once it's closed.".into()));
    }
    if m.breaker.stopped {
        return Some((DownloadWait::Stopped, "YouTube kept asking Encore to prove it isn't a bot, so downloads have stopped. Try again later.".into()));
    }
    if let Some(u) = m.breaker.until {
        let left = u.saturating_duration_since(Instant::now());
        if !left.is_zero() {
            let mins = left.as_secs().div_ceil(60).max(1);
            return Some((DownloadWait::BotCheck, format!("YouTube asked Encore to prove it isn't a bot. Downloads continue in {mins} {}.", if mins == 1 { "minute" } else { "minutes" })));
        }
    }
    if m.disk_full {
        return Some((DownloadWait::DiskFull, "Your disk is full. Free some space, then resume downloads.".into()));
    }
    if !m.setup_ready {
        return Some((DownloadWait::SetupNeeded, "Set up playback to download songs.".into()));
    }
    None
}

/// What the summary says when downloads can't work on this computer.
pub fn unavailable_snapshot(message: &str) -> DownloadsSnapshot {
    DownloadsSnapshot {
        summary: DownloadsSummary {
            revision: "0".into(),
            available: false,
            songs: 0,
            waiting: 0,
            failed: 0,
            bytes: 0.0,
            active: None,
            paused: false,
            wait: None,
            message: Some(message.into()),
            online: true,
            syncing: vec![],
        },
        items: vec![],
        collections: vec![],
    }
}

impl DownloadManager {
    /// Open the downloads kept in `dir` (`<app data>/downloads`). Must run
    /// inside the tokio runtime. Nothing is fetched or cleaned up until
    /// `start`, so a second process (the CLI) can read safely.
    pub fn open(dir: PathBuf, d: Deps) -> Result<Arc<Self>> {
        let layout = Layout::create(dir).map_err(|e| AppError::internal(format!("Encore couldn't create its downloads folder ({}).", e.kind())))?;
        if d.store.download_count()? == 0 {
            let found = files::scan_sidecars(&layout.audio);
            if !found.is_empty() {
                tracing::warn!(songs = found.len(), "the downloads index was empty; finding the songs again from their files");
            }
            for s in &found {
                d.store.download_recover(RECOVERED, &s.row())?;
            }
        }
        let paused = d.store.get::<bool>(SETTINGS, PAUSED).ok().flatten().unwrap_or(false);
        let (tx, _) = broadcast::channel(256);
        let me = Arc::new(DownloadManager {
            d,
            layout,
            mem: Mutex::new(Mem {
                ready: HashMap::new(),
                revision: 1,
                paused,
                online: true,
                setup_ready: true,
                disk_full: false,
                elsewhere: false,
                breaker: Breaker::default(),
                active: None,
                syncing: HashSet::new(),
                counts: Counts::default(),
                last_progress: None,
                last_request: None,
                art_tried: HashSet::new(),
                lyrics_tried: HashSet::new(),
                started: false,
            }),
            tx,
            wake: Notify::new(),
            lock: Mutex::new(None),
            on_network_error: Mutex::new(None),
            rt: tokio::runtime::Handle::current(),
        });
        me.reload();
        Ok(me)
    }

    /// Run the worker (it waits for the folder's lock if another Encore
    /// holds it).
    pub fn start(self: &Arc<Self>) {
        let me = self.clone();
        self.rt.spawn(async move { me.run().await });
    }

    pub fn subscribe(&self) -> broadcast::Receiver<DownloadsUpdate> {
        self.tx.subscribe()
    }

    pub fn dir(&self) -> &std::path::Path {
        &self.layout.root
    }

    /// Called when a download fails for lack of a connection, so the app
    /// can check whether it is offline.
    pub fn on_network_error(&self, f: impl Fn() + Send + Sync + 'static) {
        *self.on_network_error.lock() = Some(Arc::new(f));
    }

    fn network_error(&self) {
        let f = self.on_network_error.lock().clone();
        if let Some(f) = f {
            f();
        }
    }

    fn local_file(&self, r: &DownloadRow) -> Option<LocalFile> {
        Some(LocalFile { path: self.layout.file(&r.file_name()?), itag: r.itag?, codec: r.codec.clone().unwrap_or_default(), kbps: r.kbps, bytes: r.bytes, duration: r.duration, loudness_db: r.loudness })
    }

    /// Read the index into memory, and bring back the cover addresses
    /// (the art registry is otherwise rebuilt only by browsing).
    fn reload(&self) {
        let rows = self.d.store.download_rows().unwrap_or_default();
        let mut ready = HashMap::new();
        for r in &rows {
            if let (Some(k), Some(b)) = (&r.art_key, &r.art_base) {
                encore_core::art::restore(k.clone(), b.clone(), r.art_hue);
            }
            if r.state == RowState::Ready {
                if let Some(f) = self.local_file(r) {
                    ready.insert(r.video_id.clone(), f);
                }
            }
        }
        self.mem.lock().ready = ready;
        self.refresh_counts();
    }

    fn refresh_counts(&self) {
        let (ready, waiting, failed, bytes) = self.d.store.download_counts().unwrap_or_default();
        let extra = files::dir_size(&self.layout.art) + self.layout.all_parts().iter().filter_map(|(_, p)| std::fs::metadata(p).ok()).map(|m| m.len()).sum::<u64>();
        self.mem.lock().counts = Counts { ready, waiting, failed, bytes: bytes + extra };
    }

    // ---------- what the UI sees ----------

    fn summary_of(&self, m: &Mem) -> DownloadsSummary {
        let (wait, message) = match wait_of(m) {
            Some((w, s)) => (Some(w), Some(s)),
            None => (None, None),
        };
        DownloadsSummary {
            revision: m.revision.to_string(),
            available: true,
            songs: m.counts.ready,
            waiting: m.counts.waiting,
            failed: m.counts.failed,
            bytes: m.counts.bytes as f64,
            active: m.active.as_ref().map(|a| ActiveDownload { track_id: ItemId::ytm(&a.video_id), attempt: a.attempt.to_string(), bytes: a.bytes as f64, total: a.total.map(|t| t as f64) }),
            paused: m.paused,
            wait,
            message,
            online: m.online,
            syncing: m.syncing.iter().map(|s| ItemId(s.clone())).collect(),
        }
    }

    pub fn summary(&self) -> DownloadsSummary {
        let m = self.mem.lock();
        self.summary_of(&m)
    }

    fn kept_of(&self, k: &KeptRow, account: &str) -> KeptCollection {
        let c = self.d.store.kept_counts(&k.account, &k.set_id).unwrap_or_default();
        KeptCollection {
            card: k.card.clone(),
            channel: k.channel.clone(),
            mine: k.account == account,
            songs: c.songs,
            ready: c.ready,
            failed: c.failed,
            bytes: c.bytes as f64,
            synced: k.synced.map(|s| s as f64),
            error: k.error.clone(),
            gone: k.gone,
            paused: k.paused,
        }
    }

    /// Everything at once. The revision is read first, so a change that
    /// lands meanwhile still arrives as a newer event.
    pub fn snapshot(&self) -> DownloadsSnapshot {
        let summary = self.summary();
        let account = self.d.service.context().account;
        let items = self.d.store.download_rows().unwrap_or_default().iter().map(item_of).collect();
        let collections = self.d.store.kept_rows().unwrap_or_default().iter().map(|k| self.kept_of(k, &account)).collect();
        DownloadsSnapshot { summary, items, collections }
    }

    fn emit_summary(&self) {
        let s = self.summary();
        let _ = self.tx.send(DownloadsUpdate::Summary(s));
    }

    /// Songs changed state: one event each, then the summary.
    fn emit_rows(&self, rows: &[DownloadRow]) {
        if rows.is_empty() {
            return;
        }
        self.refresh_counts();
        {
            let mut m = self.mem.lock();
            for r in rows {
                m.revision += 1;
                let _ = self.tx.send(DownloadsUpdate::Item { revision: m.revision, item: item_of(r) });
            }
        }
        self.emit_summary();
    }

    fn emit_row(&self, video_id: &str) {
        if let Ok(Some(r)) = self.d.store.download_row(video_id) {
            self.emit_rows(&[r]);
        }
    }

    /// Kept collections (or many songs) changed: the UI takes a snapshot.
    fn changed(&self) {
        self.refresh_counts();
        self.mem.lock().revision += 1;
        self.emit_summary();
    }

    fn progress(&self, attempt: u64, bytes: u64) {
        let send = {
            let mut m = self.mem.lock();
            match m.active.as_mut() {
                Some(a) if a.attempt == attempt => a.bytes = bytes,
                _ => return,
            }
            let due = m.last_progress.is_none_or(|t| t.elapsed() >= Duration::from_millis(250));
            if due {
                m.last_progress = Some(Instant::now());
                Some(self.summary_of(&m))
            } else {
                None
            }
        };
        if let Some(s) = send {
            let _ = self.tx.send(DownloadsUpdate::Summary(s));
        }
    }

    fn set_total(&self, attempt: u64, total: u64) {
        if let Some(a) = self.mem.lock().active.as_mut().filter(|a| a.attempt == attempt) {
            a.total = Some(total);
        }
    }

    // ---------- what the user asks for ----------

    /// Download songs. Returns how many are now waiting.
    pub fn add(&self, tracks: Vec<Track>) -> Result<usize> {
        let tracks: Vec<Track> = tracks.into_iter().filter(|t| t.playable && sanitize_key(t.id.remote()).is_some()).collect();
        if tracks.is_empty() {
            return Err(AppError::invalid("These songs can't be downloaded."));
        }
        let account = self.d.service.context().account;
        let n = self.d.store.download_request(&account, &tracks)?;
        let rows: Vec<DownloadRow> = tracks.iter().filter_map(|t| self.d.store.download_row(t.id.remote()).ok().flatten()).collect();
        self.emit_rows(&rows);
        self.wake.notify_one();
        Ok(n)
    }

    /// Remove downloaded songs, unless a kept collection still needs them.
    pub fn remove(&self, ids: &[ItemId]) -> Result<DownloadRemoved> {
        let vids: Vec<String> = ids.iter().map(|i| i.remote().to_string()).collect();
        let kept = self.d.store.download_unpin(&vids)?;
        let removed = self.release();
        Ok(DownloadRemoved {
            removed: removed as u32,
            kept: kept.into_iter().filter_map(|(v, titles)| Some(KeptBy { track_id: ItemId::ytm(&v), collection: titles.into_iter().next()? })).collect(),
        })
    }

    /// Remove every download, pin and kept collection, for every account.
    pub fn remove_all(&self) -> Result<()> {
        let rows = self.d.store.download_remove_all()?;
        self.drop_rows(&rows);
        self.changed();
        Ok(())
    }

    /// Delete the files of songs nothing wants any more. Returns how many.
    fn release(&self) -> usize {
        match self.d.store.download_release() {
            Ok(rows) => {
                self.drop_rows(&rows);
                rows.len()
            }
            Err(e) => {
                tracing::warn!(error = %e.message, "downloads: couldn't release songs");
                0
            }
        }
    }

    fn drop_rows(&self, rows: &[DownloadRow]) {
        if rows.is_empty() {
            return;
        }
        {
            let mut m = self.mem.lock();
            for r in rows {
                m.ready.remove(&r.video_id);
                if let Some(a) = m.active.as_ref().filter(|a| a.video_id == r.video_id) {
                    a.cancel.store(true, Ordering::SeqCst);
                }
            }
        }
        self.emit_rows(rows);
        for r in rows {
            if self.delete_files(r) {
                let _ = self.d.store.download_forget(&r.video_id, r.attempt);
            }
        }
        self.refresh_counts();
        self.emit_summary();
    }

    /// Remove a song's files. A file still open elsewhere (Windows) stays
    /// marked for removal and is tried again later.
    fn delete_files(&self, r: &DownloadRow) -> bool {
        let mut ok = true;
        if let Some(name) = r.file_name() {
            ok &= files::remove(&self.layout.file(&name));
            ok &= files::remove(&self.layout.sidecar(&name));
        }
        for p in self.layout.parts_of(&r.video_id) {
            ok &= files::remove(&p);
        }
        if let Some(k) = &r.art_key {
            if !self.d.store.download_art_in_use(k).unwrap_or(true) {
                files::remove(&self.layout.cover(k));
            }
        }
        ok
    }

    fn finish_removals(&self) {
        for r in self.d.store.download_deleting().unwrap_or_default() {
            if self.delete_files(&r) {
                let _ = self.d.store.download_forget(&r.video_id, r.attempt);
            }
        }
    }

    /// Partial files no queued song will continue.
    fn clean_parts(&self) {
        for (vid, p) in self.layout.all_parts() {
            let keep = matches!(self.d.store.download_row(&vid), Ok(Some(r)) if r.state == RowState::Queued);
            if !keep {
                files::remove(&p);
            }
        }
    }

    /// Pause or resume all downloads (the user's choice, kept across restarts).
    pub fn pause(&self, on: bool) {
        let _ = self.d.store.put(SETTINGS, PAUSED, &on, None);
        {
            let mut m = self.mem.lock();
            m.paused = on;
            if on {
                m.disk_full = false;
            }
        }
        self.wake.notify_one();
        self.emit_summary();
    }

    /// Try again: failed songs, a stopped bot-check wait, a full disk.
    pub fn retry(&self) {
        {
            let mut m = self.mem.lock();
            if m.breaker.stopped {
                // Another bot check waits 30 minutes, not 10.
                m.breaker = Breaker { level: 1, until: None, stopped: false };
            }
            m.disk_full = false;
            m.art_tried.clear();
            m.lyrics_tried.clear();
        }
        let _ = self.d.store.download_retry_failed();
        self.wake.notify_one();
        self.changed();
    }

    pub fn set_online(self: &Arc<Self>, on: bool) {
        let changed = std::mem::replace(&mut self.mem.lock().online, on) != on;
        if !changed {
            return;
        }
        self.wake.notify_one();
        self.emit_summary();
        if on {
            self.sync_due(SYNC_SOON);
        }
    }

    /// The player components are (or aren't) there to resolve with (D22).
    pub fn set_setup_ready(&self, on: bool) {
        let changed = std::mem::replace(&mut self.mem.lock().setup_ready, on) != on;
        if changed {
            self.wake.notify_one();
            self.emit_summary();
        }
    }

    fn set_elsewhere(&self, on: bool) {
        let changed = std::mem::replace(&mut self.mem.lock().elsewhere, on) != on;
        if changed {
            self.emit_summary();
        }
    }

    /// Settings → Downloads: measured on disk.
    pub fn storage(&self) -> DownloadsStorage {
        let bytes = files::dir_size(&self.layout.audio) + files::dir_size(&self.layout.art);
        DownloadsStorage { songs: self.mem.lock().counts.ready, bytes: bytes as f64, location: self.layout.root.display().to_string() }
    }

    // ---------- kept collections ----------

    /// Keep a collection downloaded and up to date for the account in use.
    /// `c` is its first page; `channel` the name shown as its keeper.
    pub fn keep(self: &Arc<Self>, c: Collection, channel: Option<String>) -> Result<()> {
        let account = self.d.service.context().account;
        let card = sync::card_of(&c);
        self.d.store.kept_put(&account, &card, channel.as_deref())?;
        self.changed();
        let me = self.clone();
        self.rt.spawn(async move { me.sync_set(account, card.id, Some(c)).await });
        Ok(())
    }

    /// The account keeping `id`: the one in use if it does, else any.
    fn keeper(&self, id: &ItemId) -> Result<String> {
        let account = self.d.service.context().account;
        if self.d.store.kept_row(&account, id.as_str())?.is_some() {
            return Ok(account);
        }
        self.d.store.kept_rows()?.into_iter().find(|k| k.set_id == id.as_str()).map(|k| k.account).ok_or_else(|| AppError::not_found("That isn't kept downloaded."))
    }

    /// Stop keeping a collection. Songs nothing else wants are removed.
    pub fn unkeep(&self, id: &ItemId) -> Result<DownloadRemoved> {
        let id = normalize(id);
        let owner = self.keeper(&id)?;
        self.d.store.kept_remove(&owner, id.as_str())?;
        let removed = self.release();
        self.changed();
        Ok(DownloadRemoved { removed: removed as u32, kept: vec![] })
    }

    pub fn pause_collection(&self, id: &ItemId, on: bool) -> Result<()> {
        let id = normalize(id);
        let owner = self.keeper(&id)?;
        self.d.store.kept_pause(&owner, id.as_str(), on)?;
        self.wake.notify_one();
        self.changed();
        Ok(())
    }

    /// "Check for changes": one collection, or all of this account's.
    pub fn sync(self: &Arc<Self>, id: Option<&ItemId>) {
        match id {
            Some(id) => {
                let account = self.d.service.context().account;
                let me = self.clone();
                let id = normalize(id);
                self.rt.spawn(async move { me.sync_set(account, id, None).await });
            }
            None => self.sync_due(0),
        }
    }

    /// The account in use changed (or was restored at launch): its kept
    /// collections are checked unless they were in the last half hour, and
    /// the UI learns which collections are now "mine".
    pub fn account_changed(self: &Arc<Self>) {
        self.changed();
        self.sync_due(SYNC_SOON);
    }

    /// Encore changed a playlist (D20) or liked songs: check it if kept.
    pub fn collection_changed(self: &Arc<Self>, id: &ItemId) {
        let id = normalize(id);
        let account = self.d.service.context().account;
        if matches!(self.d.store.kept_row(&account, id.as_str()), Ok(Some(_))) {
            let me = self.clone();
            self.rt.spawn(async move { me.sync_set(account, id, None).await });
        }
    }

    fn due(&self, max_age: i64) -> Vec<ItemId> {
        let account = self.d.service.context().account;
        let n = now();
        self.d.store.kept_rows().unwrap_or_default().into_iter().filter(|k| k.account == account && k.synced.is_none_or(|s| n - s >= max_age)).map(|k| ItemId(k.set_id)).collect()
    }

    fn sync_due(self: &Arc<Self>, max_age: i64) {
        let due = self.due(max_age);
        if due.is_empty() {
            return;
        }
        let account = self.d.service.context().account;
        let me = self.clone();
        self.rt.spawn(async move {
            for id in due {
                me.clone().sync_set(account.clone(), id, None).await;
            }
        });
    }

    /// Read a kept collection again and apply what changed. Only for the
    /// account in use: another account's collections wait for it.
    async fn sync_set(self: Arc<Self>, account: String, id: ItemId, first: Option<Collection>) {
        let ctx = self.d.service.context();
        if ctx.account != account || !self.mem.lock().online {
            return;
        }
        if !self.mem.lock().syncing.insert(id.0.clone()) {
            return;
        }
        self.changed();
        if let Ok(Some(k)) = self.d.store.kept_row(&account, id.as_str()) {
            let had = self.d.store.kept_has_members(&account, id.as_str()).unwrap_or(false);
            let t = sync::traverse(self.d.service.as_ref(), &ctx, &id, first, &self.d.gate).await;
            let s = &self.d.store;
            let (set, generation) = (id.as_str(), k.generation);
            let r = match &t {
                Traversal::Complete { card, members } => s.kept_apply(&account, set, generation, Some(members.as_slice()), Some(card), None, false),
                // A first read that stopped short is still better than nothing.
                Traversal::Partial { card, members } if !had => s.kept_apply(&account, set, generation, Some(members.as_slice()), Some(card), Some(CHECK_FAILED), false),
                Traversal::Partial { card, .. } => s.kept_apply(&account, set, generation, None, Some(card), Some(CHECK_FAILED), false),
                Traversal::Gone => s.kept_apply(&account, set, generation, None, None, Some(GONE), true),
                Traversal::Failed => s.kept_apply(&account, set, generation, None, None, Some(CHECK_FAILED), false),
            };
            match r {
                Ok(true) => tracing::info!(collection = %id, complete = matches!(t, Traversal::Complete { .. }), "kept collection checked"),
                Ok(false) => tracing::info!(collection = %id, "kept collection changed meanwhile; check dropped"),
                Err(e) => tracing::warn!(collection = %id, error = %e.message, "couldn't save a kept collection"),
            }
        }
        self.mem.lock().syncing.remove(&id.0);
        self.release();
        self.changed();
        self.wake.notify_one();
    }

    // ---------- the worker ----------

    async fn nap(&self, d: Duration) {
        tokio::select! {
            _ = self.wake.notified() => {}
            _ = tokio::time::sleep(d) => {}
        }
    }

    /// Take the folder's lock; the first time, tidy up after the last run.
    fn hold_lock(&self) -> bool {
        let held = {
            let mut l = self.lock.lock();
            if l.is_none() {
                *l = match WorkerLock::try_acquire(&self.layout.root) {
                    Ok(x) => x,
                    Err(e) => {
                        tracing::warn!(error = %e, "downloads: couldn't take the folder's lock");
                        None
                    }
                };
            }
            l.is_some()
        };
        self.set_elsewhere(!held);
        if held && !std::mem::replace(&mut self.mem.lock().started, true) {
            let _ = self.d.store.download_reset_active();
            self.finish_removals();
            self.clean_parts();
            self.changed();
        }
        held
    }

    /// How long to wait before looking at the queue again, if downloads
    /// are waiting on something.
    fn blocked(&self) -> Option<Duration> {
        let (d, expired) = {
            let mut m = self.mem.lock();
            let expired = m.breaker.until.is_some_and(|u| u <= Instant::now());
            if expired {
                m.breaker.until = None;
            }
            let d = if m.paused || !m.online || m.disk_full || !m.setup_ready || m.breaker.stopped {
                Some(Duration::from_secs(60))
            } else {
                m.breaker.until.map(|u| u.saturating_duration_since(Instant::now()))
            };
            (d, expired)
        };
        if expired {
            self.emit_summary();
        }
        d
    }

    fn should_stop(&self) -> bool {
        let m = self.mem.lock();
        m.paused || !m.online
    }

    async fn run(self: Arc<Self>) {
        loop {
            if !self.hold_lock() {
                self.nap(Duration::from_secs(30)).await;
                continue;
            }
            if let Some(d) = self.blocked() {
                self.nap(d).await;
                continue;
            }
            match self.d.store.download_next(now()) {
                Ok(Some(row)) => {
                    self.run_job(row).await;
                    continue;
                }
                Ok(None) => {}
                Err(e) => tracing::warn!(error = %e.message, "downloads: couldn't read the queue"),
            }
            self.idle().await;
            let wait = match self.d.store.download_next_retry() {
                Ok(Some(at)) => Duration::from_secs((at - now()).clamp(1, 60) as u64),
                _ => Duration::from_secs(60),
            };
            self.nap(wait).await;
        }
    }

    /// Nothing to fetch: finish removals, keep lyrics and covers, check
    /// kept collections that are due.
    async fn idle(self: &Arc<Self>) {
        self.finish_removals();
        if !self.mem.lock().online {
            return;
        }
        if self.d.lyrics.is_some() {
            let tried = self.mem.lock().lyrics_tried.clone();
            for r in self.d.store.lyrics_due(now(), 20).unwrap_or_default().into_iter().filter(|r| !tried.contains(&r.video_id)).take(2) {
                self.keep_lyrics(&r).await;
            }
        }
        if self.d.art.is_some() {
            let tried = self.mem.lock().art_tried.clone();
            for r in self.d.store.art_due(20).unwrap_or_default().into_iter().filter(|r| r.art_key.as_ref().is_some_and(|k| !tried.contains(k))).take(3) {
                self.keep_cover(&r).await;
            }
        }
        for id in self.due(SYNC_EVERY) {
            let account = self.d.service.context().account;
            self.clone().sync_set(account, id, None).await;
        }
    }

    fn challenged(&self) {
        let mut m = self.mem.lock();
        m.breaker.level = m.breaker.level.saturating_add(1);
        match m.breaker.level {
            1 => m.breaker.until = Some(Instant::now() + Duration::from_secs(10 * 60)),
            2 => m.breaker.until = Some(Instant::now() + Duration::from_secs(30 * 60)),
            _ => {
                m.breaker.until = None;
                m.breaker.stopped = true;
            }
        }
        tracing::warn!(level = m.breaker.level, "downloads: YouTube asked for a bot check");
    }

    async fn run_job(self: &Arc<Self>, row: DownloadRow) {
        let vid = row.video_id.clone();
        let attempt = next_attempt();
        if !matches!(self.d.store.download_start(&vid, attempt), Ok(true)) {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut m = self.mem.lock();
            m.active = Some(Active { video_id: vid.clone(), attempt, bytes: 0, total: row.total, cancel: cancel.clone() });
            m.last_progress = None;
        }
        self.emit_row(&vid);
        let end = self.fetch_song(&row, attempt, &cancel).await;
        let s = &self.d.store;
        let mut installed = None;
        match end {
            Ok(Some((name, fin))) => installed = self.install(&vid, attempt, &name, fin),
            Ok(None) => {
                let _ = s.download_requeue(&vid, attempt, None, false, None);
            }
            Err(JobError::Challenged) => {
                self.challenged();
                let _ = s.download_requeue(&vid, attempt, None, false, None);
            }
            Err(JobError::SetupNeeded) => {
                self.set_setup_ready(false);
                let _ = s.download_requeue(&vid, attempt, None, false, None);
            }
            Err(JobError::DiskFull) => {
                self.mem.lock().disk_full = true;
                let _ = s.download_requeue(&vid, attempt, None, false, None);
            }
            Err(JobError::Retry(msg)) => {
                let n = row.attempts as usize;
                tracing::info!(video = %vid, tries = n + 1, error = %msg, "download failed");
                let _ = match RETRIES.get(n) {
                    Some(secs) => s.download_requeue(&vid, attempt, Some(now() + secs), true, Some(&msg)),
                    None => s.download_fail(&vid, attempt, &msg),
                };
            }
            Err(JobError::Fail(msg)) => {
                tracing::info!(video = %vid, error = %msg, "download not possible");
                let _ = s.download_fail(&vid, attempt, &msg);
            }
        }
        self.mem.lock().active = None;
        self.emit_row(&vid);
        if let Some(done) = installed {
            self.keep_cover(&done).await;
            self.keep_lyrics(&done).await;
        }
    }

    /// Wait for playback's resolves to finish, then for the spacing.
    /// False if the job was cancelled meanwhile.
    async fn before_request(&self, cancel: &AtomicBool) -> bool {
        let idle = async {
            while self.d.gate.busy() {
                if cancel.load(Ordering::SeqCst) {
                    return false;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            true
        };
        if !idle.await {
            return false;
        }
        let (lo, hi) = self.d.spacing;
        let spacing = if hi > lo { lo + Duration::from_millis(jitter() % ((hi - lo).as_millis() as u64 + 1)) } else { lo };
        let wait = self.mem.lock().last_request.map(|t| spacing.saturating_sub(t.elapsed()));
        if let Some(w) = wait {
            tokio::time::sleep(w).await;
        }
        while self.d.gate.busy() {
            if cancel.load(Ordering::SeqCst) {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        self.mem.lock().last_request = Some(Instant::now());
        !cancel.load(Ordering::SeqCst)
    }

    async fn resolve(&self, vid: &str, cancel: &AtomicBool) -> std::result::Result<Option<ResolvedStream>, JobError> {
        if !self.before_request(cancel).await {
            return Ok(None);
        }
        let auth = (self.d.auth)();
        match self.d.resolver.resolve(vid, auth.as_ref()).await {
            Ok(s) => Ok(Some(s)),
            Err(f) => Err(match f.kind {
                // Only the extractor's bot check counts here (D23): a refused
                // URL was already resolved again by the resolver.
                PlayFailureKind::Challenged => JobError::Challenged,
                PlayFailureKind::SetupNeeded => JobError::SetupNeeded,
                PlayFailureKind::Network => {
                    self.network_error();
                    JobError::Retry(f.message)
                }
                PlayFailureKind::Unavailable | PlayFailureKind::SessionExpired => JobError::Fail(f.message),
                PlayFailureKind::Unknown => JobError::Retry(f.message),
            }),
        }
    }

    /// A complete file the playback cache already has, copied into place.
    async fn adopt(&self, row: &DownloadRow, attempt: u64) -> Option<(String, u64)> {
        let cache = self.d.cache.clone()?;
        let vid = row.video_id.clone();
        let layout = self.layout.clone();
        let (name, itag, ext, n) = tokio::task::spawn_blocking(move || {
            let (src, itag) = cache.find_video(&vid)?;
            let ext = files::sniff(&src)?;
            let name = format!("{vid}-{itag}.{ext}");
            let part = layout.part(&name);
            match files::copy_from(&src, &part) {
                Ok(n) if n > 0 => Some((name, itag, ext, n)),
                _ => {
                    files::remove(&part);
                    None
                }
            }
        })
        .await
        .ok()
        .flatten()?;
        let codec = if ext == "webm" { "opus" } else { "aac" };
        let _ = self.d.store.download_format(&row.video_id, attempt, &Format { itag, ext: ext.into(), codec: codec.into(), kbps: None, total: n });
        tracing::info!(video = %row.video_id, itag, "download copied from the playback cache");
        Some((name, n))
    }

    /// Fetch a song into its `.part` file and check it. `Ok(None)`: stopped
    /// (paused, offline, cancelled); the partial file stays.
    async fn fetch_song(self: &Arc<Self>, row: &DownloadRow, attempt: u64, cancel: &Arc<AtomicBool>) -> std::result::Result<Option<(String, Finished)>, JobError> {
        let vid = row.video_id.as_str();
        let (name, total) = match self.adopt(row, attempt).await {
            Some(x) => x,
            None => match self.fetch_stream(row, attempt, cancel).await? {
                Some(x) => x,
                None => return Ok(None),
            },
        };
        self.set_total(attempt, total);
        self.progress(attempt, total);
        let part = self.layout.part(&name);
        let p2 = part.clone();
        let checked = tokio::task::spawn_blocking(move || -> std::result::Result<(String, Option<f64>, String, u64), String> {
            let (codec, duration) = encore_audio::decode::validate_file(&p2, 48)?;
            let len = std::fs::metadata(&p2).map_err(|e| e.kind().to_string())?.len();
            let sha = files::sha256_file(&p2).map_err(|e| e.kind().to_string())?;
            Ok((codec.to_string(), duration, sha, len))
        })
        .await;
        match checked {
            Ok(Ok((codec, duration, sha256, bytes))) if bytes == total => Ok(Some((name, Finished { bytes, sha256, codec, duration }))),
            Ok(Ok(_)) => {
                files::remove(&part);
                Err(JobError::Retry("The download came out the wrong size.".into()))
            }
            Ok(Err(e)) => {
                tracing::warn!(video = vid, error = %e, "a download didn't decode");
                files::remove(&part);
                Err(JobError::Retry("The downloaded file didn't play.".into()))
            }
            Err(_) => Err(JobError::Retry("Encore couldn't check the download.".into())),
        }
    }

    /// Resolve and fetch over the network. Returns the file name and size.
    async fn fetch_stream(self: &Arc<Self>, row: &DownloadRow, attempt: u64, cancel: &Arc<AtomicBool>) -> std::result::Result<Option<(String, u64)>, JobError> {
        let vid = row.video_id.as_str();
        let Some(mut s) = self.resolve(vid, cancel).await? else { return Ok(None) };
        if row.loudness.is_none() {
            if let Some(l) = &self.d.loudness {
                if self.before_request(cancel).await {
                    if let Ok(v) = l.loudness(&row.track.id).await {
                        let _ = self.d.store.download_set_loudness(vid, v);
                    }
                }
            }
        }
        let stop = {
            let (me, c) = (self.clone(), cancel.clone());
            move || c.load(Ordering::SeqCst) || me.should_stop()
        };
        let progress = {
            let me = self.clone();
            move |b: u64| me.progress(attempt, b)
        };
        let mut recorded = row.itag.zip(row.total);
        let mut refreshed = false;
        let mut retotaled = false;
        loop {
            let total = match s.content_length {
                Some(t) if t > 0 => t,
                _ => return Err(JobError::Retry("YouTube didn't say how long this song is.".into())),
            };
            let ext = files::ext_for(s.itag, &s.mime, &s.codec);
            let name = format!("{vid}-{}.{ext}", s.itag);
            let part = self.layout.part(&name);
            for p in self.layout.parts_of(vid) {
                if p != part {
                    files::remove(&p);
                }
            }
            let src = Source { url: s.url.expose().clone(), headers: s.headers.clone(), total, ranges: s.range_safe };
            // Continue a partial file only for the same format and length,
            // and only if its tail is what the server has there now (D23).
            let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
            let from = if have > 0 && recorded == Some((s.itag, total)) && fetch::tail_matches(self.d.fetcher.as_ref(), &src, &part, have).await { have } else { 0 };
            if from > 0 {
                tracing::info!(video = vid, from, total, "continuing a download");
            }
            let _ = self.d.store.download_format(vid, attempt, &Format { itag: s.itag, ext: ext.into(), codec: s.codec.clone(), kbps: s.bitrate_kbps, total });
            recorded = Some((s.itag, total));
            self.set_total(attempt, total);
            if files::free_space(&self.layout.audio).is_some_and(|free| free < total - from + HEADROOM) {
                return Err(JobError::DiskFull);
            }
            match fetch::transfer(self.d.fetcher.as_ref(), &src, &part, from, &stop, &progress).await {
                Ok(Outcome::Done) => return Ok(Some((name, total))),
                Ok(Outcome::Stopped) => return Ok(None),
                Err(TransferError::Changed(t)) if from == 0 && !retotaled => {
                    // The resolver's length was an estimate: the server's wins.
                    retotaled = true;
                    s.content_length = Some(t);
                }
                Err(TransferError::Refused | TransferError::Changed(_)) if !refreshed => {
                    // The URL went stale: resolve once more and carry on.
                    refreshed = true;
                    self.d.resolver.forget(vid);
                    match self.resolve(vid, cancel).await? {
                        Some(fresh) => s = fresh,
                        None => return Ok(None),
                    }
                }
                Err(TransferError::Refused) => return Err(JobError::Retry("YouTube's media server refused this song.".into())),
                Err(TransferError::Changed(_)) => return Err(JobError::Retry("The song changed on YouTube while it downloaded.".into())),
                Err(TransferError::Network(e)) => {
                    tracing::info!(video = vid, error = %e, "download interrupted");
                    self.network_error();
                    return Err(JobError::Retry("The connection dropped while downloading.".into()));
                }
                Err(TransferError::Server(m)) => return Err(JobError::Retry(m)),
                Err(TransferError::Disk(e)) if e.kind() == std::io::ErrorKind::StorageFull => return Err(JobError::DiskFull),
                Err(TransferError::Disk(e)) => return Err(JobError::Retry(format!("Encore couldn't write the download ({}).", e.kind()))),
            }
        }
    }

    /// Put a checked file in place, if the attempt is still wanted.
    /// Returns the finished row.
    fn install(&self, vid: &str, attempt: u64, name: &str, fin: Finished) -> Option<DownloadRow> {
        let part = self.layout.part(name);
        let file = self.layout.file(name);
        let (p2, f2) = (part.clone(), file.clone());
        let r = self.d.store.download_commit(vid, attempt, &fin, move || {
            std::fs::rename(&p2, &f2)?;
            files::private(&f2);
            Ok(())
        });
        match r {
            Ok(Commit::Installed) => {
                let done = self.d.store.download_row(vid).ok().flatten()?;
                if let Some(sc) = Sidecar::of(&done) {
                    if let Ok(b) = serde_json::to_vec(&sc) {
                        let _ = files::write_atomic(&self.layout.sidecar(name), &b);
                    }
                }
                let mut m = self.mem.lock();
                if let Some(f) = self.local_file(&done) {
                    m.ready.insert(vid.to_string(), f);
                }
                m.breaker.level = 0;
                drop(m);
                tracing::info!(video = vid, bytes = fin.bytes, "downloaded");
                Some(done)
            }
            Ok(Commit::Stale) => {
                files::remove(&part);
                None
            }
            Err(e) => {
                tracing::warn!(video = vid, error = %e.message, "couldn't keep a download");
                files::remove(&part);
                let _ = self.d.store.download_requeue(vid, attempt, Some(now() + 60), true, Some("Encore couldn't keep the download."));
                None
            }
        }
    }

    /// Keep the song's cover with it (the 544 px one the app shows).
    async fn keep_cover(&self, r: &DownloadRow) {
        let (Some(key), Some(art)) = (&r.art_key, &self.d.art) else { return };
        if r.art_pinned || !self.mem.lock().online {
            return;
        }
        let path = self.layout.cover(key);
        let have = path.is_file()
            || match art.cover(key).await {
                Some(b) => files::write_atomic(&path, &b).is_ok(),
                None => false,
            };
        if have {
            let hue = encore_core::art::lookup(key).and_then(|e| e.hue);
            let _ = self.d.store.download_set_art_pinned(key, true, hue);
        } else {
            self.mem.lock().art_tried.insert(key.clone());
        }
    }

    /// Keep the song's lyrics with it. "No lyrics" is remembered for a
    /// week; a provider that couldn't be reached isn't remembered at all.
    async fn keep_lyrics(&self, r: &DownloadRow) {
        let Some(l) = &self.d.lyrics else { return };
        if !self.mem.lock().online || matches!(self.d.store.pinned_lyrics(&r.video_id), Ok(Some(Some(_)))) {
            return;
        }
        let never = AtomicBool::new(false);
        if !self.before_request(&never).await {
            return;
        }
        match l.fetch_for_download(&r.track).await {
            Ok(found) => {
                let _ = self.d.store.pin_lyrics(&r.video_id, found.as_ref(), Some(now() + LYRICS_RETRY));
            }
            Err(e) => {
                tracing::info!(video = %r.video_id, error = %e, "lyrics not kept for now");
                self.mem.lock().lyrics_tried.insert(r.video_id.clone());
            }
        }
    }

    /// A ready song's file is missing or damaged: it no longer counts.
    fn damaged(&self, video_id: &str) {
        self.mem.lock().ready.remove(video_id);
        tracing::warn!(video = video_id, "a downloaded file is missing or the wrong size");
        let _ = self.d.store.download_mark_damaged(video_id, "The downloaded file is missing or damaged. Download it again.");
        self.emit_row(video_id);
    }

    /// For the CLI: wait until nothing is left to do, or downloads wait on
    /// something only time or the user can change (returned).
    pub async fn settle(&self) -> Option<DownloadWait> {
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let (busy, wait) = {
                let m = self.mem.lock();
                (m.active.is_some() || !m.syncing.is_empty(), wait_of(&m).map(|w| w.0))
            };
            if busy {
                continue;
            }
            if wait.is_some() {
                return wait;
            }
            let next = self.d.store.download_next(now()).ok().flatten();
            let retry = self.d.store.download_next_retry().ok().flatten();
            if next.is_none() && retry.is_none() {
                return None;
            }
        }
    }
}

impl LocalAudio for DownloadManager {
    fn lookup(&self, video_id: &str) -> Option<LocalFile> {
        let f = self.mem.lock().ready.get(video_id).cloned()?;
        match std::fs::metadata(&f.path) {
            Ok(m) if m.len() == f.bytes => Some(f),
            _ => {
                self.damaged(video_id);
                None
            }
        }
    }

    fn has(&self, video_id: &str) -> bool {
        self.mem.lock().ready.contains_key(video_id)
    }

    fn collection(&self, account: &str, id: &ItemId) -> Option<Collection> {
        let id = normalize(id);
        let owner = match self.d.store.kept_row(account, id.as_str()) {
            Ok(Some(_)) => account.to_string(),
            _ => self.d.store.kept_rows().ok()?.into_iter().find(|k| k.set_id == id.as_str())?.account,
        };
        let (card, rows) = self.d.store.kept_snapshot(&owner, id.as_str()).ok()??;
        let n = rows.len();
        Some(Collection {
            id: card.id.clone(),
            kind: card.kind,
            title: card.title,
            subtitle: card.subtitle,
            artists: card.artists,
            year: card.year,
            art: card.art,
            description: None,
            stats: Some(if n == 1 { "1 song".to_string() } else { format!("{n} songs") }),
            track_count: Some(n as u32),
            entries: rows.into_iter().map(|(key, t)| Entry { entry_id: key, video_id: t.id.clone(), track: t, set_video_id: None }).collect(),
            cursor: None,
            playlist_id: card.play_id,
            saved: None,
            editable: false,
            privacy: None,
            related: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use encore_core::redact::Secret;
    use encore_core::service::{More, PlayFailure};
    use fetch::tests::FakeServer;

    struct FakeResolver {
        total: u64,
        kind: Option<PlayFailureKind>,
    }

    #[async_trait]
    impl StreamResolver for FakeResolver {
        async fn resolve(&self, video_id: &str, _auth: Option<&ResolveAuth>) -> std::result::Result<ResolvedStream, PlayFailure> {
            if let Some(k) = self.kind {
                return Err(PlayFailure::new(k, "no"));
            }
            Ok(ResolvedStream {
                video_id: video_id.into(),
                url: Secret::new("https://example.invalid/a".into()),
                headers: vec![],
                itag: 140,
                codec: "aac".into(),
                mime: "audio/mp4".into(),
                bitrate_kbps: Some(128),
                content_length: Some(self.total),
                duration: None,
                loudness_db: None,
                expires_at: None,
                range_safe: true,
                authenticated: false,
            })
        }
    }

    const VID: &str = "dQw4w9WgXcQ";

    fn setup(kind: Option<PlayFailureKind>) -> (Arc<DownloadManager>, PathBuf, Vec<u8>) {
        let dir = std::env::temp_dir().join(format!("encore-dl-{}-{}", std::process::id(), next_attempt()));
        let data = include_bytes!("../../encore-audio/fixtures/tone-aac.m4a").to_vec();
        let server = Arc::new(FakeServer::default());
        *server.data.lock() = data.clone();
        let deps = Deps {
            store: Arc::new(Store::open_memory().unwrap()),
            service: Arc::new(sync::tests::Pages { end: More::Ended }),
            resolver: Arc::new(FakeResolver { total: data.len() as u64, kind }),
            auth: Arc::new(|| None::<ResolveAuth>),
            gate: Arc::new(PlaybackGate::default()),
            fetcher: server,
            cache: None,
            lyrics: None,
            art: None,
            loudness: None,
            spacing: (Duration::ZERO, Duration::ZERO),
        };
        (DownloadManager::open(dir.clone(), deps).unwrap(), dir, data)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn downloads_a_song_and_finds_it_again() {
        let (m, dir, data) = setup(None);
        m.start();
        m.add(vec![sync::tests::track(VID)]).unwrap();
        assert_eq!(m.settle().await, None);
        let f = m.lookup(VID).expect("downloaded");
        assert_eq!(std::fs::read(&f.path).unwrap(), data);
        assert_eq!(f.codec, "aac");
        assert!(dir.join(format!("audio/{VID}-140.m4a.json")).is_file(), "with its sidecar");
        assert_eq!(m.summary().songs, 1);

        let r = m.remove(&[ItemId::ytm(VID)]).unwrap();
        assert_eq!(r.removed, 1);
        assert!(m.lookup(VID).is_none());
        assert!(!f.path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_bot_check_pauses_everything() {
        let (m, dir, _) = setup(Some(PlayFailureKind::Challenged));
        m.start();
        m.add(vec![sync::tests::track(VID)]).unwrap();
        assert_eq!(m.settle().await, Some(DownloadWait::BotCheck));
        let row = m.d.store.download_row(VID).unwrap().unwrap();
        assert_eq!(row.state, RowState::Queued, "the song waits, it didn't fail");
        assert_eq!(row.attempts, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_unavailable_song_fails_at_once() {
        let (m, dir, _) = setup(Some(PlayFailureKind::Unavailable));
        m.start();
        m.add(vec![sync::tests::track(VID)]).unwrap();
        assert_eq!(m.settle().await, None);
        assert_eq!(m.d.store.download_row(VID).unwrap().unwrap().state, RowState::Failed);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
