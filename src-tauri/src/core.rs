//! Backend state shared by commands, events and OS integration.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use encore_audio::AudioCache;
use encore_core::error::{AppError, ErrorCode, Result};
use encore_core::queue::Queue;
use encore_core::service::{MusicService, ResolveAuth};
use encore_core::store::Store;
use encore_downloads::DownloadManager;
use encore_lyrics::LyricsService;
use encore_player::Player;
use encore_resolver::{ResolverConfig, Runtimes, YtDlpResolver};
use encore_ytm::account::Accounts;
use encore_ytm::{InnerTube, YtMusic};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Settings scope and keys in the store (D11).
pub const SETTINGS: &str = "settings";
pub const REPORT_HISTORY: &str = "report_history";

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
        // Local (not roaming) data on Windows: the player components live here.
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
    pub resolver: Arc<YtDlpResolver>,
    pub store: Arc<Store>,
    pub audio_cache: AudioCache,
    pub art: Arc<crate::art::ArtCache>,
    /// `None` when downloads can't work here (`DOWNLOADS_OFF`).
    pub downloads: Option<Arc<DownloadManager>>,
    pub connectivity: Arc<crate::connectivity::Connectivity>,
    #[allow(dead_code)]
    pub paths: Paths,
    /// A message from startup for the UI, and whether the UI is listening
    /// yet (events sent before it subscribes are lost).
    pub startup_notice: parking_lot::Mutex<(bool, Option<String>)>,
}

#[derive(Serialize, Deserialize)]
struct SavedQueue {
    queue: Queue,
    position: f64,
}

pub fn auth_provider(it: Arc<InnerTube>) -> encore_player::AuthProvider {
    Arc::new(move || {
        let s = it.session()?;
        let ch = s.channel.as_ref()?;
        Some(ResolveAuth { cookies: s.jar.records().to_vec(), auth_user: ch.auth_user, page_id: ch.page_id.clone() })
    })
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
        let runtimes = Runtimes::new(rcfg.runtimes());
        let resolver = YtDlpResolver::new(rcfg, runtimes);
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
        let core = Arc::new(Core { ytm, accounts, player, lyrics, resolver, store, audio_cache, art, downloads, connectivity, paths, startup_notice: parking_lot::Mutex::new((false, None)) });
        Ok(core)
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

    /// Restore the queue for the current account, paused.
    pub fn restore_queue(&self) {
        if let Ok(Some(saved)) = self.store.get::<SavedQueue>(&self.account_scope(), "queue") {
            self.player.restore(saved.queue, saved.position);
        }
    }

    /// After an account change: that account's saved queue, or an empty one
    /// (another account's queue never stays visible).
    pub fn switch_queue(&self) {
        match self.store.get::<SavedQueue>(&self.account_scope(), "queue") {
            Ok(Some(saved)) => self.player.restore(saved.queue, saved.position),
            _ => self.player.restore(Queue::default(), 0.0),
        }
    }

    pub fn install_persistence(self: &Arc<Self>) {
        let w = Arc::downgrade(self);
        let saved_art = parking_lot::Mutex::new(std::collections::HashSet::<String>::new());
        self.player.on_persist(move |q, pos| {
            if let Some(c) = w.upgrade() {
                let _ = c.store.put(&c.account_scope(), "queue", &SavedQueueRef { queue: q, position: pos }, None);
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
            }
        });
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

#[derive(Serialize)]
struct SavedQueueRef<'a> {
    queue: &'a Queue,
    position: f64,
}
