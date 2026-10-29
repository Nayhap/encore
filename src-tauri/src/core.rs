//! Backend state shared by commands, events and OS integration.

use std::path::PathBuf;
use std::sync::Arc;

use encore_audio::AudioCache;
use encore_core::error::{AppError, ErrorCode, Result};
use encore_core::queue::Queue;
use encore_core::service::{MusicService, ResolveAuth};
use encore_core::store::Store;
use encore_lyrics::LyricsService;
use encore_player::Player;
use encore_resolver::{ResolverConfig, YtDlpResolver};
use encore_ytm::account::Accounts;
use encore_ytm::{InnerTube, YtMusic};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Settings scope and keys in the store (D11).
pub const SETTINGS: &str = "settings";
pub const REPORT_HISTORY: &str = "report_history";

pub struct Paths {
    pub data: PathBuf,
    pub cache: PathBuf,
    pub logs: PathBuf,
}

impl Paths {
    pub fn new() -> Self {
        // Local (not roaming) data on Windows: the resolver venv lives here.
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
    #[allow(dead_code)]
    pub paths: Paths,
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
        let resolver = YtDlpResolver::new(ResolverConfig { root: paths.data.join("resolver"), cache_dir: paths.cache.join("yt-dlp") });
        let audio_cache = AudioCache::new(paths.cache.join("audio"), encore_audio::cache::DEFAULT_LIMIT).map_err(|e| AppError::internal(format!("audio cache: {}", e.kind())))?;
        let player = Player::new(ytm.clone(), resolver.clone(), auth_provider(it.clone()), Some(audio_cache.clone()))?;
        // On unless turned off (it only ever reports for a signed-in account).
        player.set_reporting(store.get::<bool>(SETTINGS, REPORT_HISTORY).ok().flatten().unwrap_or(true));
        let lyrics = Arc::new(LyricsService::new(ytm.clone(), Some(store.clone())));
        let art = Arc::new(crate::art::ArtCache::new(paths.cache.join("art"), store.clone()));
        let core = Arc::new(Core { ytm, accounts, player, lyrics, resolver, store, audio_cache, art, paths });
        Ok(core)
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
                        let _ = c.store.put("art", &a.key, &(entry.base, entry.hue), Some(180 * 24 * 3600));
                        done.insert(a.key.clone());
                    }
                }
            }
        });
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
                Ok(v)
            }
            Err(e) if e.code == ErrorCode::Network => match self.store.get_any::<T>(&scope, key) {
                Ok(Some((mut v, _, _))) => {
                    strip(&mut v);
                    Ok(v)
                }
                _ => Err(e),
            },
            Err(e) => Err(e),
        }
    }
}

#[derive(Serialize)]
struct SavedQueueRef<'a> {
    queue: &'a Queue,
    position: f64,
}
