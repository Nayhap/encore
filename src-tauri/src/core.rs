//! Backend state shared by commands, events and OS integration.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use encore_audio::AudioCache;
use encore_core::error::{AppError, ErrorCode, Result};
use encore_core::queue::Queue;
use encore_core::service::MusicService;
use encore_core::store::{self, Recovery, Store};
use encore_downloads::DownloadManager;
use encore_lyrics::LyricsService;
use encore_player::Player;
use encore_player::plays::PlayObserver;
use encore_resolver::{NativeResolver, ResolverConfig};
use encore_ytm::account::Accounts;
use encore_ytm::{InnerTube, YtMusic};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Settings scope and keys in the store (D11).
pub const SETTINGS: &str = "settings";
pub const REPORT_HISTORY: &str = "report_history";
/// In-app updates (D24): install automatically (default on), the version
/// the user skipped, when GitHub last answered a check, and the version
/// whose "What's new" was last shown.
pub const AUTO_UPDATE: &str = "auto_update";
pub const UPDATE_SKIPPED: &str = "update_skipped";
pub const UPDATE_CHECKED_AT: &str = "update_checked_at";
pub const LAST_SEEN_VERSION: &str = "last_seen_version";

/// Why downloads are off, when the system gives Encore no data folder.
pub const DOWNLOADS_OFF: &str = "Downloads are off: Encore can't find a folder to keep them in on this computer.";

/// Why the backend couldn't start, for the UI (see `bootstrap`).
pub struct StartupFailure(pub String);

pub fn startup_failure_message(detail: &str) -> String {
    let logs = Paths::new().logs.join("encore.log");
    format!("{detail}. Check that the disk isn't full and that Encore can write to its data folder, then restart Encore. Details are in {}.", logs.display())
}

pub struct Paths {
    pub data: PathBuf,
    pub cache: PathBuf,
    pub logs: PathBuf,
}

impl Paths {
    pub fn new() -> Self {
        // Local (not roaming) data on Windows: downloads and the database live here.
        let data = dirs::data_local_dir().unwrap_or_else(std::env::temp_dir).join("app.encore.desktop");
        let cache = dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("app.encore.desktop");
        let logs = if cfg!(target_os = "macos") {
            dirs::home_dir().map(|h| h.join("Library/Logs/app.encore.desktop")).unwrap_or_else(|| cache.join("logs"))
        } else if cfg!(windows) {
            data.join("logs")
        } else {
            // ~/.local/state/app.encore.desktop
            dirs::state_dir().unwrap_or_else(|| cache.clone()).join("app.encore.desktop")
        };
        Paths { data, cache, logs }
    }

    /// `<app data>/downloads`. Downloads are kept for good, so never in a
    /// temporary folder: without a data folder there are no downloads (D23).
    pub fn downloads() -> Option<PathBuf> {
        dirs::data_local_dir().map(|d| d.join("app.encore.desktop").join("downloads"))
    }
}

pub struct Core {
    pub ytm: Arc<YtMusic>,
    pub accounts: Arc<Accounts>,
    pub player: Arc<Player>,
    pub lyrics: Arc<LyricsService>,
    pub resolver: Arc<NativeResolver>,
    pub store: Arc<Store>,
    pub audio_cache: AudioCache,
    pub art: Arc<crate::art::ArtCache>,
    /// `None` when downloads can't work here (`DOWNLOADS_OFF`).
    pub downloads: Option<Arc<DownloadManager>>,
    pub connectivity: Arc<crate::connectivity::Connectivity>,
    /// Every play, as the local log records it (D30): lanes D and E follow it.
    #[allow(dead_code)]
    pub plays: Arc<PlayObserver>,
    /// The feature areas' own state (`crate::features`).
    #[allow(dead_code)]
    pub features: crate::features::Features,
    #[allow(dead_code)]
    pub paths: Paths,
    /// A message from startup for the UI, and whether the UI is listening
    /// yet (events sent before it subscribes are lost).
    pub startup_notice: parking_lot::Mutex<(bool, Option<String>)>,
    /// `prepare_exit` ran (and wasn't undone).
    exit_prepared: tokio::sync::Mutex<bool>,
}

/// The queue as last saved in full (D6, D11). `stamp` is newer than 0.6.2
/// (which reads past it); `queue-at` checkpoints carry it.
#[derive(Serialize, Deserialize)]
struct SavedQueue {
    queue: Queue,
    position: f64,
    #[serde(default)]
    stamp: Option<String>,
}

/// Where that queue had got to since: the entry playing and the position
/// in it, for the whole write named `stamp`.
#[derive(Serialize, Deserialize)]
struct QueueAt {
    stamp: String,
    at: encore_core::queue::Checkpoint,
    position: f64,
}

/// The queue saved under `scope`, moved on to its last checkpoint when
/// that belongs to it, with the stamp of its whole write.
fn saved_queue(store: &Store, scope: &str) -> Option<(Queue, f64, Option<String>)> {
    let saved = store.get::<SavedQueue>(scope, "queue").ok()??;
    let (mut queue, mut position) = (saved.queue, saved.position);
    if let (Some(stamp), Ok(Some(at))) = (&saved.stamp, store.get::<QueueAt>(scope, "queue-at")) {
        if at.stamp == *stamp && queue.resume(&at.at) {
            position = at.position;
        }
    }
    Some((queue, position, saved.stamp))
}

pub fn auth_provider(it: Arc<InnerTube>) -> encore_player::AuthProvider {
    // One snapshot per resolve, keyed by account, channel and epoch (D2).
    Arc::new(move || it.resolve_auth())
}

impl Core {
    /// Build everything. Must run inside the tokio runtime.
    pub fn new() -> Result<Arc<Self>> {
        let paths = Paths::new();
        std::fs::create_dir_all(&paths.data).map_err(|e| AppError::internal(format!("cannot create data dir: {}", e.kind())))?;
        let store = Arc::new(Store::open(&paths.data.join("encore.sqlite"))?);
        // Artwork keys survive restarts (queue restore shows covers).
        for (k, v) in store.list("art").unwrap_or_default() {
            if let Ok(e) = serde_json::from_str::<(String, Option<(u16, u16)>)>(&v) {
                encore_core::art::restore(k, e.0, e.1);
            }
        }
        let it = InnerTube::new("en", "US");
        let ytm = Arc::new(YtMusic::new(it.clone()));
        let accounts = Accounts::new(ytm.clone(), encore_auth::store::session_store());
        let rcfg = ResolverConfig { data_dir: paths.data.clone(), cache_dir: paths.cache.clone() };
        let resolver = NativeResolver::new(rcfg);
        // Signed out, the stream `/player` presents encore-ytm's visitor (D37).
        let visitor = it.clone();
        resolver.set_visitor_source(Arc::new(move || visitor.visitor_data()));
        let audio_cache = AudioCache::new(paths.cache.join("audio"), encore_audio::cache::DEFAULT_LIMIT).map_err(|e| AppError::internal(format!("audio cache: {}", e.kind())))?;
        let player = Player::new(ytm.clone(), resolver.clone(), auth_provider(it.clone()), Some(audio_cache.clone()))?;
        // On unless turned off (it only ever reports for a signed-in account).
        player.set_reporting(store.get::<bool>(SETTINGS, REPORT_HISTORY).ok().flatten().unwrap_or(true));
        let lyrics = Arc::new(LyricsService::new(ytm.clone(), Some(store.clone())));
        let downloads_dir = Paths::downloads();
        let art = Arc::new(crate::art::ArtCache::new(paths.cache.join("art"), store.clone(), downloads_dir.as_ref().map(|d| d.join("art"))));
        let connectivity = crate::connectivity::Connectivity::new();
        let downloads = match downloads_dir {
            None => {
                tracing::warn!("no data folder: downloads are off");
                None
            }
            Some(dir) => {
                let deps = encore_downloads::Deps {
                    store: store.clone(),
                    service: ytm.clone(),
                    resolver: resolver.clone(),
                    auth: auth_provider(it.clone()),
                    gate: player.gate(),
                    fetcher: Arc::new(encore_downloads::HttpFetcher::new()),
                    cache: Some(audio_cache.clone()),
                    lyrics: Some(lyrics.clone()),
                    art: Some(art.clone()),
                    loudness: Some(ytm.clone()),
                    spacing: (Duration::from_secs(2), Duration::from_secs(4)),
                };
                match DownloadManager::open(dir, deps) {
                    Ok(d) => {
                        player.set_local_audio(d.clone());
                        let c = connectivity.clone();
                        d.on_network_error(move || c.transport_failed());
                        Some(d)
                    }
                    Err(e) => {
                        tracing::warn!(error = %e.message, "downloads are off");
                        None
                    }
                }
            }
        };
        let plays = player.observe_plays(store.clone());
        let core = Arc::new(Core {
            ytm,
            accounts,
            player,
            lyrics,
            resolver,
            store,
            audio_cache,
            art,
            downloads,
            connectivity,
            plays,
            features: Default::default(),
            paths,
            startup_notice: parking_lot::Mutex::new((false, None)),
            exit_prepared: tokio::sync::Mutex::new(false),
        });
        core.keep_backups();
        Ok(core)
    }

    /// The daily backup of the database (D11): checked soon after launch,
    /// then every hour, on a thread of its own.
    fn keep_backups(&self) {
        let store = self.store.clone();
        let _ = std::thread::Builder::new().name("encore-backups".into()).spawn(move || {
            std::thread::sleep(Duration::from_secs(20));
            loop {
                match store.backup_if_due() {
                    Ok(Some(p)) => tracing::info!(backup = %p.display(), "database backed up"),
                    Ok(None) => {}
                    Err(e) => tracing::warn!(error = %e.message, "couldn't back up the database"),
                }
                std::thread::sleep(Duration::from_secs(3600));
            }
        });
    }

    /// What to tell the user when this start replaced a damaged database;
    /// `found` is how many downloaded songs were found again from their files.
    pub fn recovery_notice(&self, found: usize) -> Option<String> {
        let songs = if found == 1 { "1 downloaded song".to_string() } else { format!("{found} downloaded songs") };
        match self.store.recovery() {
            // The downloads index was lost on its own: as 0.4 said it (D23).
            None => (found > 0).then(|| format!("Encore's database was damaged and replaced. It found your {songs} again; download playlists and albums again to keep them up to date.")),
            Some(Recovery::Restored { made }) => {
                let mut t = format!("Encore's database was damaged, so it was restored from a backup made {}. Anything you saved or changed since then is gone.", age(store::now() - made));
                if found > 0 {
                    t.push_str(&format!(" Encore also found {songs} newer than the backup."));
                }
                Some(t)
            }
            Some(Recovery::Fresh) if found > 0 => Some(format!("Encore's database was damaged and no backup of it worked, so it started a new one. It found your {songs} again; download playlists and albums again to keep them up to date. Your settings, bookmarks and listening stats are gone.")),
            Some(Recovery::Fresh) => Some("Encore's database was damaged and no backup of it worked, so it started a new one. Your settings, bookmarks and listening stats are gone; your downloads and your YouTube Music library are not.".into()),
        }
    }

    /// Show `text` once the UI is up (now, if it already is). Returns the
    /// text when it should be sent as an event right away.
    pub fn startup_notice(&self, text: String) -> Option<String> {
        let mut n = self.startup_notice.lock();
        if n.0 { Some(text) } else {
            n.1 = Some(text);
            None
        }
    }

    /// The UI is listening: hand over any notice saved for it.
    pub fn take_startup_notice(&self) -> Option<String> {
        let mut n = self.startup_notice.lock();
        n.0 = true;
        n.1.take()
    }

    pub fn service(&self) -> Arc<dyn MusicService> {
        self.ytm.clone()
    }

    pub fn account_scope(&self) -> String {
        format!("account:{}", self.ytm.context().account)
    }

    /// Restore the queue for the current account, paused, and save it
    /// there from now on.
    pub fn restore_queue(&self) {
        let scope = self.account_scope();
        match saved_queue(&self.store, &scope) {
            Some((queue, position, stamp)) => self.player.restore_saved(queue, position, scope, stamp),
            None => self.player.save_to(Some(scope)),
        }
    }

    /// After an account change: that account's saved queue, or an empty one
    /// (another account's queue never stays visible). The queue that was
    /// showing was saved under its own account, never this one.
    pub fn switch_queue(&self) {
        let scope = self.account_scope();
        match saved_queue(&self.store, &scope) {
            Some((queue, position, stamp)) => self.player.restore_saved(queue, position, scope, stamp),
            None => self.player.restore_saved(Queue::default(), 0.0, scope, None),
        }
    }

    pub fn install_persistence(self: &Arc<Self>) {
        let w = Arc::downgrade(self);
        let saved_art = parking_lot::Mutex::new(std::collections::HashSet::<String>::new());
        self.player.on_persist(move |save| {
            let Some(c) = w.upgrade() else { return false };
            let (scope, stamp, q, position) = match save {
                // Only where the queue is: a small row beside the whole one.
                encore_player::SaveQueue::Checkpoint { scope, stamp, at, position } => return c.store.put(scope, "queue-at", &QueueAtRef { stamp, at, position }, None).is_ok(),
                encore_player::SaveQueue::Full { scope, stamp, queue, position } => (scope, stamp, queue, position),
            };
            if c.store.put(scope, "queue", &SavedQueueRef { queue: q, position, stamp }, None).is_err() {
                return false;
            }
            // Restored queues need their artwork keys after a restart.
            let mut done = saved_art.lock();
            for e in q.entries() {
                let Some(a) = &e.track.art else { continue };
                if done.contains(&a.key) {
                    continue;
                }
                if let Some(entry) = encore_core::art::lookup(&a.key) {
                    // A downloaded song's cover never expires (D23).
                    let ttl = if c.art.is_kept(&a.key) { None } else { Some(180 * 24 * 3600) };
                    let _ = c.store.put("art", &a.key, &(entry.base, entry.hue), ttl);
                    done.insert(a.key.clone());
                }
            }
            true
        });
    }

    /// Before Encore quits or installs an update (D24). Idempotent, and a
    /// second caller waits for the first. Every step is bounded, so quitting
    /// never hangs on the network:
    /// 1. playback stops (paused, nothing loading starts later, no command
    ///    starts it) and the queue and position are saved now;
    /// 2. the play being reported gets its last history ping, and the local
    ///    play log closes its open play with the time heard (≤ 2 s, D30);
    /// 3. downloads stop after the current range and let go of their
    ///    folder, without touching the user's pause (≤ 5 s);
    /// 4. the resolver stops: a running challenge solve is interrupted and no
    ///    resolve starts again (D37).
    ///
    /// The Windows installer path ends in `process::exit`, so all of this
    /// has to be done before `Update::install` is called.
    pub async fn prepare_exit(&self) {
        let mut done = self.exit_prepared.lock().await;
        if *done {
            return;
        }
        let t0 = std::time::Instant::now();
        self.player.shut_down();
        let (history, plays) = tokio::join!(self.player.settle_history(Duration::from_secs(2)), self.player.settle_plays(Duration::from_secs(2)));
        let downloads = match &self.downloads {
            Some(d) => tokio::time::timeout(Duration::from_secs(5), d.quiesce()).await.is_ok(),
            None => true,
        };
        let resolver = tokio::time::timeout(Duration::from_secs(3), self.resolver.shut_down()).await.is_ok();
        tracing::info!(history, plays, downloads, resolver, ms = t0.elapsed().as_millis() as u64, "ready to exit");
        *done = true;
    }

    /// The exit didn't happen after all (an update failed to install):
    /// downloads and the resolver carry on. Playback stays paused.
    pub async fn abort_exit(&self) {
        let mut done = self.exit_prepared.lock().await;
        if !std::mem::take(&mut *done) {
            return;
        }
        self.resolver.reopen();
        self.player.reopen();
        if let Some(d) = &self.downloads {
            d.resume_after_quiesce();
        }
    }

    /// A request that got no answer may mean the connection is gone: the
    /// connection check takes a look (D23). Every command that asks
    /// YouTube passes its result through here or through `cached`.
    pub fn net<T>(&self, r: Result<T>) -> Result<T> {
        if r.as_ref().is_err_and(|e| e.code == ErrorCode::Network) {
            self.connectivity.transport_failed();
        }
        r
    }

    /// Refuse what needs YouTube while offline, with a reason (D23).
    pub fn require_online(&self, what: &str) -> Result<()> {
        if self.connectivity.online() { Ok(()) } else { Err(AppError::network(format!("You're offline. Connect to the internet to {what}."))) }
    }

    /// Serve from the network; on a network failure fall back to the last
    /// cached copy for this account (cursors stripped: they do not survive).
    pub async fn cached<T, F, Fut>(&self, key: &str, fetch: F, strip: impl Fn(&mut T)) -> Result<T>
    where
        T: Serialize + DeserializeOwned,
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let scope = self.account_scope();
        match fetch().await {
            Ok(v) => {
                let _ = self.store.put(&scope, key, &v, Some(7 * 24 * 3600));
                // An answer while marked offline: the connection is back.
                if !self.connectivity.online() {
                    self.connectivity.check();
                }
                Ok(v)
            }
            Err(e) if e.code == ErrorCode::Network => {
                self.connectivity.transport_failed();
                match self.store.get_any::<T>(&scope, key) {
                    Ok(Some((mut v, _, _))) => {
                        strip(&mut v);
                        Ok(v)
                    }
                    _ => Err(e),
                }
            }
            Err(e) => Err(e),
        }
    }
}

/// "6 hours ago", "yesterday", "3 days ago".
fn age(secs: i64) -> String {
    let h = secs.max(0) / 3600;
    match h {
        0 => "less than an hour ago".into(),
        1 => "an hour ago".into(),
        2..=23 => format!("{h} hours ago"),
        24..=47 => "yesterday".into(),
        _ => format!("{} days ago", h / 24),
    }
}

#[derive(Serialize)]
struct SavedQueueRef<'a> {
    queue: &'a Queue,
    position: f64,
    stamp: &'a str,
}

#[derive(Serialize)]
struct QueueAtRef<'a> {
    stamp: &'a str,
    at: &'a encore_core::queue::Checkpoint,
    position: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use encore_core::model::{ItemId, Track, TrackKind};
    use encore_core::queue::Origin;

    fn track(n: &str) -> Track {
        Track { id: ItemId::ytm(n), title: n.into(), artists: vec![], album: None, duration: Some(200.0), art: None, kind: TrackKind::Song, explicit: false, playable: true, liked: None, counterpart: None, plays: None }
    }

    fn queue() -> Queue {
        let mut q = Queue::new(3);
        q.replace(vec![track("a"), track("b"), track("c")], 0, None, false, Origin::Collection).unwrap();
        q
    }

    #[test]
    fn a_queue_saved_by_0_6_restores_as_it_was() {
        let store = Store::open_memory().unwrap();
        let q = queue();
        // 0.6.2's row: the queue and position, no stamp.
        store.put_raw("account:a", "queue", &serde_json::json!({ "queue": q, "position": 61.5 }).to_string(), None).unwrap();
        let (back, position, stamp) = saved_queue(&store, "account:a").unwrap();
        assert_eq!(position, 61.5);
        assert_eq!(stamp, None);
        assert_eq!(back.index(), Some(0));
        assert!(saved_queue(&store, "account:b").is_none());
    }

    #[test]
    fn a_checkpoint_moves_the_saved_queue_on() {
        let store = Store::open_memory().unwrap();
        let q = queue();
        store.put("account:a", "queue", &SavedQueueRef { queue: &q, position: 10.0, stamp: "one" }, None).unwrap();
        let mut later = q.clone();
        later.next();
        let at = later.checkpoint().unwrap();
        store.put("account:a", "queue-at", &QueueAtRef { stamp: "one", at: &at, position: 33.0 }, None).unwrap();
        let (back, position, stamp) = saved_queue(&store, "account:a").unwrap();
        assert_eq!((back.index(), position, stamp.as_deref()), (Some(1), 33.0, Some("one")));
        // A checkpoint of an older whole write is left out.
        store.put("account:a", "queue", &SavedQueueRef { queue: &q, position: 5.0, stamp: "two" }, None).unwrap();
        let (back, position, _) = saved_queue(&store, "account:a").unwrap();
        assert_eq!((back.index(), position), (Some(0), 5.0));
        // And 0.6.2 reads the new row, past its stamp.
        let old: serde_json::Value = store.get("account:a", "queue").unwrap().unwrap();
        assert!(serde_json::from_value::<OldSavedQueue>(old).is_ok());
    }

    /// 0.6.2's row, as it reads it.
    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct OldSavedQueue {
        queue: Queue,
        position: f64,
    }
}
