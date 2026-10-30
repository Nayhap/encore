//! The playback controller: owns the queue (D6), drives the audio engine
//! (D5), resolves streams (D3/D4), preloads the next track 20 s before the end,
//! extends the queue from radio when autoplay is on, and publishes revisioned
//! state, queue splices and 2 Hz position anchors (D13).

pub mod history;
pub mod types;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use encore_audio::{AudioCache, Engine, EngineEvent, HttpSpec, SourceSpec};
use encore_core::error::{AppError, Result};
use encore_core::model::*;
use encore_core::queue::{Origin, Queue, QueueEntry, QueueSource, Repeat, Step};
use encore_core::service::{MusicService, PlayFailure, PlayFailureKind, PlayReporter, ResolveAuth, ResolvedStream, StreamResolver};
use parking_lot::Mutex;
use tokio::sync::broadcast;

pub use types::*;

pub const PRELOAD_LEAD: f64 = 20.0;
const MAX_AUTO_SKIPS: u32 = 3;
const RADIO_LOW_WATER: usize = 3;
const SHUFFLE_ENUM_LIMIT: usize = 10_000;
/// How long a newly chosen song must stay chosen before its stream is
/// resolved (see `play_current`).
const SKIP_SETTLE: Duration = Duration::from_millis(400);

pub type AuthProvider = Arc<dyn Fn() -> Option<ResolveAuth> + Send + Sync>;

struct Loaded {
    load_id: u64,
    #[allow(dead_code)]
    uid: String,
    stream: ResolvedStream,
    from_cache: bool,
}

struct State {
    queue: Queue,
    phase: Phase,
    track_gen: u64,
    seek_gen: u64,
    revision: u64,
    volume: f64,
    muted: bool,
    error: Option<PlayFailure>,
    loaded: Option<Loaded>,
    /// Engine deck id → queue entry uid.
    decks: HashMap<u64, String>,
    preload: Option<(String, u64)>,
    preloading: bool,
    /// Position to resume at when nothing is loaded (restore, or a seek
    /// during loading).
    pending_pos: f64,
    skips: u32,
    radio_cursor: Option<String>,
    radio_loading: bool,
    radio_empty: u32,
    queue_loading: bool,
    last_queue: Vec<QueueEntry>,
    last_queue_rev: u64,
    last_duration: Option<f64>,
}

pub struct Player {
    engine: Arc<Engine>,
    resolver: Arc<dyn StreamResolver>,
    service: Arc<dyn MusicService>,
    auth: AuthProvider,
    st: Mutex<State>,
    tx: broadcast::Sender<PlayerEvent>,
    next_load: AtomicU64,
    rt: tokio::runtime::Handle,
    on_persist: Mutex<Option<Box<dyn Fn(&Queue, f64) + Send + Sync>>>,
    preloaded_streams: Mutex<HashMap<u64, ResolvedStream>>,
    cache: Option<AudioCache>,
    /// Report plays to the service's history (a user setting, D18).
    reporting: Arc<AtomicBool>,
}

fn s(n: u64) -> String {
    n.to_string()
}

impl Player {
    /// Must be called from inside a tokio runtime.
    pub fn new(service: Arc<dyn MusicService>, resolver: Arc<dyn StreamResolver>, auth: AuthProvider, cache: Option<AudioCache>) -> Result<Arc<Self>> {
        let cache_for_player = cache.clone();
        let t0 = Instant::now();
        let engine = Arc::new(Engine::new(cache).map_err(|e| AppError::internal(format!("Audio output is unavailable: {e}")))?);
        tracing::info!(ms = t0.elapsed().as_millis() as u64, device = %engine.status().device, "audio engine ready");
        let events = engine.take_events().expect("engine events");
        let (tx, _) = broadcast::channel(512);
        let p = Arc::new(Player {
            engine,
            resolver,
            service,
            auth,
            st: Mutex::new(State {
                queue: Queue::new(rand_seed()),
                phase: Phase::Idle,
                track_gen: 0,
                seek_gen: 0,
                revision: 0,
                volume: 0.8,
                muted: false,
                error: None,
                loaded: None,
                decks: HashMap::new(),
                preload: None,
                preloading: false,
                pending_pos: 0.0,
                skips: 0,
                radio_cursor: None,
                radio_loading: false,
                radio_empty: 0,
                queue_loading: false,
                last_queue: vec![],
                last_queue_rev: 0,
                last_duration: None,
            }),
            tx,
            next_load: AtomicU64::new(1),
            rt: tokio::runtime::Handle::current(),
            on_persist: Mutex::new(None),
            preloaded_streams: Mutex::new(HashMap::new()),
            cache: cache_for_player,
            reporting: Arc::new(AtomicBool::new(false)),
        });
        p.engine.set_volume(0.8);
        // Engine events arrive on a std channel; forward them into the runtime.
        let weak = Arc::downgrade(&p);
        std::thread::Builder::new()
            .name("encore-player-events".into())
            .spawn(move || {
                while let Ok(ev) = events.recv() {
                    let Some(p) = weak.upgrade() else { break };
                    let p2 = p.clone();
                    p.rt.spawn(async move { p2.on_engine(ev).await });
                }
            })
            .expect("spawn");
        // Ticker: position anchors, preload and radio top-up.
        let weak = Arc::downgrade(&p);
        p.rt.spawn(async move {
            let mut iv = tokio::time::interval(Duration::from_millis(500));
            let mut last_persist = Instant::now();
            loop {
                iv.tick().await;
                let Some(p) = weak.upgrade() else { break };
                p.tick().await;
                if last_persist.elapsed() > Duration::from_secs(5) {
                    last_persist = Instant::now();
                    p.persist();
                }
            }
        });
        Ok(p)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<PlayerEvent> {
        self.tx.subscribe()
    }

    /// Report plays through `reporter` from now on (while the setting is on).
    pub fn report_history(&self, reporter: Arc<dyn PlayReporter>, on_registered: Arc<dyn Fn(&ItemId) + Send + Sync>) {
        let _guard = self.rt.enter();
        history::HistoryReporter::spawn(self.subscribe(), reporter, self.reporting.clone(), on_registered);
    }

    pub fn set_reporting(&self, on: bool) {
        self.reporting.store(on, Ordering::Relaxed);
    }

    pub fn reporting(&self) -> bool {
        self.reporting.load(Ordering::Relaxed)
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn on_persist(&self, f: impl Fn(&Queue, f64) + Send + Sync + 'static) {
        *self.on_persist.lock() = Some(Box::new(f));
    }

    fn persist(&self) {
        let (q, pos) = {
            let st = self.st.lock();
            (st.queue.clone(), self.position_locked(&st))
        };
        if let Some(f) = self.on_persist.lock().as_ref() {
            f(&q, pos);
        }
    }

    /// Restore a saved queue, paused at `position` (never auto-plays).
    pub fn restore(&self, queue: Queue, position: f64) {
        {
            let mut st = self.st.lock();
            st.queue = queue;
            st.pending_pos = position.max(0.0);
            st.last_duration = st.queue.current().and_then(|c| c.track.duration);
            st.phase = if st.queue.current().is_some() { Phase::Paused } else { Phase::Idle };
            st.track_gen += 1;
        }
        self.publish(true);
        let cur = self.st.lock().queue.current().cloned();
        let _ = self.tx.send(PlayerEvent::TrackChanged(cur));
    }

    // ---------- snapshots ----------

    fn position_locked(&self, st: &State) -> f64 {
        match &st.loaded {
            Some(l) => {
                let es = self.engine.status();
                if es.id == l.load_id { es.position } else { st.pending_pos }
            }
            None => st.pending_pos,
        }
    }

    fn snapshot_locked(&self, st: &State) -> PlayerSnapshot {
        let es = self.engine.status();
        let cur = st.queue.current().cloned();
        let loaded_now = st.loaded.as_ref().is_some_and(|l| l.load_id == es.id);
        let duration = if loaded_now { es.duration.or(st.last_duration) } else { st.last_duration }.or_else(|| cur.as_ref().and_then(|c| c.track.duration));
        PlayerSnapshot {
            revision: s(st.revision),
            track_gen: s(st.track_gen),
            seek_gen: s(st.seek_gen),
            phase: st.phase,
            playing: matches!(st.phase, Phase::Playing | Phase::Buffering | Phase::Loading) && !es.paused,
            current: cur,
            index: st.queue.index().map(|i| i as u32),
            position: self.position_locked(st),
            duration,
            shuffle: st.queue.shuffle,
            repeat: st.queue.repeat,
            autoplay: st.queue.autoplay,
            volume: st.volume,
            muted: st.muted,
            source: st.queue.source.clone(),
            stream: st.loaded.as_ref().map(|l| StreamInfo {
                itag: l.stream.itag,
                codec: l.stream.codec.clone(),
                kbps: l.stream.bitrate_kbps,
                from_cache: l.from_cache,
                seekable: l.stream.range_safe,
                authenticated: l.stream.authenticated,
            }),
            error: st.error.clone(),
            skipped_in_a_row: st.skips,
            no_output: es.device == encore_audio::engine::NO_OUTPUT,
            queue_loading: st.queue_loading,
        }
    }

    pub fn snapshot(&self) -> PlayerSnapshot {
        let st = self.st.lock();
        self.snapshot_locked(&st)
    }

    pub fn queue_snapshot(&self) -> QueueSnapshot {
        let st = self.st.lock();
        QueueSnapshot { revision: s(st.queue.revision()), entries: st.queue.entries().to_vec(), index: st.queue.index().map(|i| i as u32) }
    }

    pub fn anchor(&self) -> PositionAnchor {
        let st = self.st.lock();
        let snap = self.snapshot_locked(&st);
        PositionAnchor { track_gen: snap.track_gen, seek_gen: snap.seek_gen, position: snap.position, duration: snap.duration, playing: snap.playing && st.phase == Phase::Playing }
    }

    /// Emit state (and a queue splice if the queue changed).
    fn publish(&self, force_queue: bool) {
        let (snap, splice) = {
            let mut st = self.st.lock();
            st.revision += 1;
            let snap = self.snapshot_locked(&st);
            let qrev = st.queue.revision();
            let splice = if force_queue || qrev != st.last_queue_rev {
                let new = st.queue.entries().to_vec();
                let (start, delete, insert) = diff(&st.last_queue, &new);
                let sp = QueueSplice { base: s(st.last_queue_rev), revision: s(qrev), start: start as u32, delete: delete as u32, insert, index: st.queue.index().map(|i| i as u32) };
                st.last_queue = new;
                st.last_queue_rev = qrev;
                Some(sp)
            } else {
                None
            };
            (snap, splice)
        };
        if let Some(sp) = splice {
            let _ = self.tx.send(PlayerEvent::Queue(sp));
        }
        let _ = self.tx.send(PlayerEvent::State(snap));
    }

    fn send_anchor(&self) {
        let _ = self.tx.send(PlayerEvent::Position(self.anchor()));
    }

    // ---------- loading ----------

    fn http_spec(self: &Arc<Self>, s: &ResolvedStream) -> SourceSpec {
        let vid = s.video_id.clone();
        let me = Arc::downgrade(self);
        let refresher: encore_audio::Refresher = Arc::new(move || {
            // Called from the fetch thread when the URL expired (403/410).
            let p = me.upgrade().ok_or("player gone")?;
            p.forget_url(&vid);
            let auth = (p.auth)();
            let r = p.rt.block_on(p.resolver.resolve(&vid, auth.as_ref()));
            r.map(|s| s.url.expose().clone()).map_err(|e| e.message)
        });
        SourceSpec::Http(HttpSpec {
            url: s.url.expose().clone(),
            headers: s.headers.clone(),
            content_length: s.content_length,
            cache_key: format!("{}-{}", s.video_id, s.itag),
            range_safe: s.range_safe,
            refresher: Some(refresher),
        })
    }

    fn forget_url(&self, video_id: &str) {
        self.resolver.forget(video_id);
    }

    /// Start playing the current queue entry from `start` seconds.
    fn play_current(self: &Arc<Self>, start: f64) {
        let (uid, vid, my_gen) = {
            let mut st = self.st.lock();
            let Some(cur) = st.queue.current().cloned() else {
                st.phase = Phase::Idle;
                drop(st);
                self.publish(false);
                return;
            };
            st.track_gen += 1;
            st.seek_gen += 1;
            st.phase = Phase::Loading;
            st.error = None;
            st.pending_pos = start;
            st.loaded = None;
            st.last_duration = cur.track.duration;
            // A preloaded deck for this entry can be promoted directly.
            (cur.uid.clone(), cur.track.id.remote().to_string(), st.track_gen)
        };
        self.publish(false);
        let cur = self.st.lock().queue.current().cloned();
        let _ = self.tx.send(PlayerEvent::TrackChanged(cur));
        let me = self.clone();
        self.rt.spawn(async move {
            // Promote a preloaded deck when it is for this entry and we start at 0.
            let promoted = {
                let mut st = me.st.lock();
                match st.preload.take() {
                    Some((puid, load_id)) if puid == uid && start == 0.0 => Some(load_id),
                    Some(other) => {
                        st.preload = Some(other);
                        None
                    }
                    None => None,
                }
            };
            let auth = (me.auth)();
            let t0 = Instant::now();
            // A fully cached file plays at once, without resolving a URL.
            let cached = me.cache.as_ref().and_then(|c| c.find_video(&vid));
            // Pressing Next several times in a row used to resolve every song
            // passed over, one after another (seconds each, two at a time),
            // so the song the user stopped on started last. Wait a moment
            // and give up if another song was chosen meanwhile.
            if cached.is_none() && promoted.is_none() {
                tokio::time::sleep(SKIP_SETTLE).await;
                if me.st.lock().track_gen != my_gen {
                    return;
                }
            }
            let res = match cached {
                Some((path, itag)) => Ok(ResolvedStream {
                    video_id: vid.clone(),
                    url: encore_core::redact::Secret::new(String::new()),
                    headers: vec![],
                    itag,
                    codec: if matches!(itag, 249 | 250 | 251 | 774) { "opus".into() } else { "aac".into() },
                    mime: String::new(),
                    bitrate_kbps: None,
                    content_length: std::fs::metadata(&path).ok().map(|m| m.len()),
                    duration: None,
                    loudness_db: None,
                    expires_at: None,
                    range_safe: true,
                    authenticated: false,
                }),
                None => me.resolver.resolve(&vid, auth.as_ref()).await,
            };
            match &res {
                Ok(s) => tracing::info!(video = %vid, ms = t0.elapsed().as_millis() as u64, itag = s.itag, codec = %s.codec, range_safe = s.range_safe, authenticated = s.authenticated, "resolved"),
                Err(f) => tracing::warn!(video = %vid, ms = t0.elapsed().as_millis() as u64, kind = ?f.kind, message = %f.message, "resolve failed"),
            }
            let failed = {
                let mut st = me.st.lock();
                if st.track_gen != my_gen {
                    return; // superseded
                }
                match res {
                    Ok(stream) => {
                        let load_id = promoted.unwrap_or_else(|| me.next_load.fetch_add(1, Ordering::Relaxed));
                        let gain = stream.loudness_db.map(|l| -(l.max(0.0))).unwrap_or(0.0);
                        let spec = match &cached_path_of(&me, &stream) {
                            Some(p) => SourceSpec::File(p.clone()),
                            None => me.http_spec(&stream),
                        };
                        st.decks.insert(load_id, uid.clone());
                        st.last_duration = stream.duration.or(st.last_duration);
                        st.loaded = Some(Loaded { load_id, uid: uid.clone(), stream, from_cache: false });
                        let start = st.pending_pos;
                        drop(st);
                        if promoted.is_none() {
                            me.engine.clear_preload();
                        }
                        me.engine.load(load_id, spec, start, gain, true);
                        None
                    }
                    Err(f) => Some(f),
                }
            };
            match failed {
                None => me.publish(false),
                Some(f) => me.fail_current(f).await,
            }
        });
    }

    async fn fail_current(self: &Arc<Self>, f: PlayFailure) {
        let (title, skips, autoskip) = {
            let mut st = self.st.lock();
            st.error = Some(f.clone());
            st.phase = Phase::Error;
            st.loaded = None;
            let title = st.queue.current().map(|c| c.track.title.clone()).unwrap_or_default();
            // Only skip what is about this song. With no connection, or
            // YouTube asking for a bot check, the next songs fail the same
            // way: skipping took a minute or more and lost the user's place.
            let autoskip = matches!(f.kind, PlayFailureKind::Unavailable | PlayFailureKind::Unknown) && st.skips < MAX_AUTO_SKIPS;
            if autoskip {
                st.skips += 1;
            }
            (title, st.skips, autoskip)
        };
        if f.kind == PlayFailureKind::SessionExpired {
            let _ = self.tx.send(PlayerEvent::SessionExpired);
        }
        self.publish(false);
        if matches!(f.kind, PlayFailureKind::Setup | PlayFailureKind::Network | PlayFailureKind::Challenged) {
            let _ = self.tx.send(PlayerEvent::Notice(f.message.clone()));
        } else if autoskip {
            let _ = self.tx.send(PlayerEvent::Notice(format!("Skipped “{title}”: {}", f.message)));
            tokio::time::sleep(Duration::from_millis(600)).await;
            let step = self.st.lock().queue.next();
            self.apply_step(step, true).await;
        } else if skips >= MAX_AUTO_SKIPS {
            let _ = self.tx.send(PlayerEvent::Notice(format!("Stopped after {MAX_AUTO_SKIPS} songs in a row could not play: {}", f.message)));
        }
    }

    async fn apply_step(self: &Arc<Self>, step: Step, natural: bool) {
        match step {
            Step::Play(_) => self.play_current(0.0),
            Step::Restart => {
                let loaded = self.st.lock().loaded.as_ref().map(|l| l.load_id);
                let alive = loaded.is_some_and(|id| self.engine.status().id == id) && !natural;
                if alive {
                    self.seek(0.0);
                    self.resume();
                } else {
                    self.play_current(0.0);
                }
            }
            Step::NeedRadio => {
                let added = self.fetch_radio().await;
                let step = {
                    let mut st = self.st.lock();
                    if added > 0 { st.queue.next() } else { Step::End }
                };
                if let Step::Play(_) = step {
                    self.play_current(0.0);
                } else {
                    self.end_of_queue();
                }
            }
            Step::End => self.end_of_queue(),
        }
    }

    fn end_of_queue(&self) {
        {
            let mut st = self.st.lock();
            st.phase = Phase::Ended;
            st.pending_pos = st.last_duration.unwrap_or(0.0);
        }
        self.engine.pause();
        self.publish(false);
    }

    /// Pull more tracks from the radio / up-next endpoint (autoplay).
    async fn fetch_radio(self: &Arc<Self>) -> usize {
        let (seed, cursor) = {
            let mut st = self.st.lock();
            if st.radio_loading || st.radio_empty >= 3 {
                return 0;
            }
            st.radio_loading = true;
            let seed = match st.queue.source.as_ref().and_then(|s| s.radio_playlist.clone()) {
                Some(pl) => RadioSeed::Playlist { playlist_id: pl },
                None => match st.queue.entries().last() {
                    Some(e) => RadioSeed::Track { id: e.track.id.clone() },
                    None => {
                        st.radio_loading = false;
                        return 0;
                    }
                },
            };
            (seed, st.radio_cursor.clone())
        };
        let ctx = self.service.context();
        let res = self.service.up_next(&ctx, &seed, cursor.as_deref()).await;
        let mut st = self.st.lock();
        st.radio_loading = false;
        match res {
            Ok(u) => {
                st.radio_cursor = u.cursor;
                if let Some(src) = st.queue.source.as_mut() {
                    if src.radio_playlist.is_none() {
                        src.radio_playlist = u.playlist_id;
                    }
                } else {
                    st.queue.source = Some(QueueSource { label: "Radio".into(), collection_id: None, radio_playlist: u.playlist_id });
                }
                let n = st.queue.extend_radio(u.tracks);
                if n == 0 {
                    st.radio_empty += 1;
                } else {
                    st.radio_empty = 0;
                }
                let playing = st.phase == Phase::Playing;
                drop(st);
                self.publish(false);
                if playing && n > 0 {
                    self.warm_upcoming();
                }
                n
            }
            Err(_) => {
                st.radio_empty += 1;
                0
            }
        }
    }

    async fn tick(self: &Arc<Self>) {
        let (phase, pos, dur, upcoming, need_radio) = {
            let st = self.st.lock();
            let snap = self.snapshot_locked(&st);
            let up = st.queue.upcoming().cloned();
            let need_radio = st.queue.autoplay && st.queue.repeat == Repeat::Off && st.queue.remaining() < RADIO_LOW_WATER && !st.radio_loading && st.radio_empty < 3 && st.queue.current().is_some() && !st.queue_loading;
            (st.phase, snap.position, snap.duration, up, need_radio)
        };
        if matches!(phase, Phase::Playing | Phase::Buffering) {
            self.send_anchor();
        }
        if need_radio && matches!(phase, Phase::Playing) {
            let me = self.clone();
            tokio::spawn(async move {
                me.fetch_radio().await;
            });
        }
        // Preload the next entry 20 s before the end (D5).
        if phase == Phase::Playing {
            if let (Some(d), Some(up)) = (dur, upcoming) {
                let should = d - pos <= PRELOAD_LEAD;
                let have = { self.st.lock().preload.as_ref().map(|p| p.0.clone()) };
                if should && have.as_deref() != Some(up.uid.as_str()) {
                    let busy = { std::mem::replace(&mut self.st.lock().preloading, true) };
                    if !busy {
                        let me = self.clone();
                        tokio::spawn(async move { me.preload(up).await });
                    }
                }
            }
        }
    }

    async fn preload(self: &Arc<Self>, entry: QueueEntry) {
        let auth = (self.auth)();
        let vid = entry.track.id.remote().to_string();
        let res = match self.cache.as_ref().and_then(|c| c.find_video(&vid)) {
            Some((_, itag)) => Ok(ResolvedStream {
                video_id: vid.clone(),
                url: encore_core::redact::Secret::new(String::new()),
                headers: vec![],
                itag,
                codec: if matches!(itag, 249 | 250 | 251 | 774) { "opus".into() } else { "aac".into() },
                mime: String::new(),
                bitrate_kbps: None,
                content_length: None,
                duration: None,
                loudness_db: None,
                expires_at: None,
                range_safe: true,
                authenticated: false,
            }),
            None => self.resolver.resolve(&vid, auth.as_ref()).await,
        };
        let mut st = self.st.lock();
        st.preloading = false;
        // Still the right entry to come next?
        if st.queue.upcoming().map(|u| u.uid.as_str()) != Some(entry.uid.as_str()) {
            return;
        }
        if let Ok(stream) = res {
            let load_id = self.next_load.fetch_add(1, Ordering::Relaxed);
            st.decks.insert(load_id, entry.uid.clone());
            st.preload = Some((entry.uid.clone(), load_id));
            let gain = stream.loudness_db.map(|l| -(l.max(0.0))).unwrap_or(0.0);
            let spec = match cached_path_of(self, &stream) {
                Some(p) => SourceSpec::File(p),
                None => self.http_spec(&stream),
            };
            drop(st);
            self.engine.preload(load_id, spec, gain);
            // Keep the stream info for when the deck takes over.
            self.preloaded_streams.lock().insert(load_id, stream);
        }
    }

    /// Resolve the next entry's URL early (the resolver caches it), so an
    /// explicit Next does not wait for extraction and the server's
    /// "available at" delay.
    fn warm_upcoming(self: &Arc<Self>) {
        let Some(up) = self.st.lock().queue.upcoming().cloned() else { return };
        if self.cache.as_ref().is_some_and(|c| c.find_video(up.track.id.remote()).is_some()) {
            return;
        }
        let me = self.clone();
        tokio::spawn(async move {
            let auth = (me.auth)();
            let _ = me.resolver.resolve(up.track.id.remote(), auth.as_ref()).await;
        });
    }

    /// Drop a preload that no longer matches what comes next.
    fn check_preload(&self) {
        let mut st = self.st.lock();
        let up = st.queue.upcoming().map(|u| u.uid.clone());
        if let Some((puid, _)) = &st.preload {
            if Some(puid) != up.as_ref() {
                st.preload = None;
                drop(st);
                self.engine.clear_preload();
            }
        }
    }

    async fn on_engine(self: &Arc<Self>, ev: EngineEvent) {
        tracing::info!(event = ?ev, "engine");
        match ev {
            EngineEvent::Ready { id, duration, from_cache, .. } => {
                let mut st = self.st.lock();
                if let Some(l) = st.loaded.as_mut() {
                    if l.load_id == id {
                        l.from_cache = from_cache;
                        if duration.is_some() {
                            st.last_duration = duration;
                            // Remember it on the queue entry (restored queues show it).
                            if let Some(cur) = st.queue.current().filter(|c| c.track.duration.is_none()).map(|c| c.track.id.0.clone()) {
                                st.queue.update_track(&cur, |t| t.duration = duration);
                            }
                        }
                    }
                }
                drop(st);
                self.publish(false);
            }
            EngineEvent::Started { id } => {
                let mut st = self.st.lock();
                if st.loaded.as_ref().is_some_and(|l| l.load_id == id) {
                    st.phase = Phase::Playing;
                    st.skips = 0;
                    drop(st);
                    self.publish(false);
                    self.send_anchor();
                    self.warm_upcoming();
                }
            }
            EngineEvent::Buffering { id, buffering } => {
                let mut st = self.st.lock();
                if st.loaded.as_ref().is_some_and(|l| l.load_id == id) && matches!(st.phase, Phase::Playing | Phase::Buffering) {
                    st.phase = if buffering { Phase::Buffering } else { Phase::Playing };
                    drop(st);
                    self.publish(false);
                    self.send_anchor();
                }
            }
            EngineEvent::Advanced { from, to } => {
                // The preloaded deck took over at the natural end.
                let ok = {
                    let mut st = self.st.lock();
                    if st.loaded.as_ref().map(|l| l.load_id) != Some(from) {
                        false
                    } else {
                        let next_uid = st.decks.get(&to).cloned();
                        let step = st.queue.on_ended();
                        let now_uid = st.queue.current().map(|c| c.uid.clone());
                        st.preload = None;
                        if matches!(step, Step::Play(_)) && now_uid == next_uid {
                            st.track_gen += 1;
                            st.seek_gen += 1;
                            st.phase = Phase::Playing;
                            st.pending_pos = 0.0;
                            st.error = None;
                            let stream = self.preloaded_streams.lock().remove(&to);
                            st.last_duration = stream.as_ref().and_then(|s| s.duration).or(st.queue.current().and_then(|c| c.track.duration));
                            st.loaded = stream.map(|stream| Loaded { load_id: to, uid: now_uid.clone().unwrap_or_default(), stream, from_cache: false });
                            true
                        } else {
                            false
                        }
                    }
                };
                if ok {
                    self.publish(false);
                    self.send_anchor();
                    let cur = self.st.lock().queue.current().cloned();
                    let _ = self.tx.send(PlayerEvent::TrackChanged(cur));
                } else {
                    self.play_current(0.0);
                }
            }
            EngineEvent::Ended { id } => {
                let step = {
                    let mut st = self.st.lock();
                    if st.loaded.as_ref().map(|l| l.load_id) != Some(id) {
                        return;
                    }
                    st.queue.on_ended()
                };
                self.apply_step(step, true).await;
            }
            EngineEvent::Failed { id, error } => {
                let current = self.st.lock().loaded.as_ref().is_some_and(|l| l.load_id == id);
                if current {
                    let pos = self.position_locked(&self.st.lock());
                    let f = PlayFailure::classify(&error, false);
                    // A network failure mid-track: re-resolve once and resume.
                    let retried = {
                        let mut st = self.st.lock();
                        let r = st.error.is_none() && pos > 1.0;
                        if r {
                            st.error = Some(f.clone());
                        }
                        r
                    };
                    if retried {
                        self.play_current(pos);
                    } else {
                        self.fail_current(f).await;
                    }
                }
            }
            EngineEvent::DeviceChanged { .. } => self.publish(false),
            EngineEvent::NoOutput { error } => {
                tracing::warn!(%error, "no audio output");
                self.publish(false);
                let _ = self.tx.send(PlayerEvent::Notice("No audio output is available. Connect speakers or headphones, or check your sound settings; Encore starts playing as soon as one works.".into()));
            }
        }
    }

    // ---------- commands (the UI contract) ----------

    pub fn play_tracks(self: &Arc<Self>, tracks: Vec<Track>, start: usize, source: Option<QueueSource>, shuffle: bool) -> Result<()> {
        {
            let mut st = self.st.lock();
            st.queue.replace(tracks, start, source, shuffle, Origin::Collection)?;
            st.radio_cursor = None;
            st.radio_empty = 0;
            st.skips = 0;
            st.preload = None;
        }
        self.engine.clear_preload();
        self.play_current(0.0);
        self.publish(false);
        Ok(())
    }

    /// Play a collection: first page now, the rest loaded in order in the
    /// background. With shuffle, the whole collection (bounded) is enumerated
    /// first so shuffle covers every song (D6).
    pub async fn play_collection(self: &Arc<Self>, id: &ItemId, start: usize, shuffle: bool) -> Result<()> {
        let ctx = self.service.context();
        let c = self.service.collection(&ctx, id).await?;
        let mut entries = c.entries.clone();
        let mut cursor = c.cursor.clone();
        let src = QueueSource { label: c.title.clone(), collection_id: Some(id.as_str().to_string()), radio_playlist: c.playlist_id.clone().map(|p| if p.starts_with("RD") { p } else { format!("RDAMPL{p}") }) };
        if shuffle {
            let t0 = Instant::now();
            while let Some(cid) = cursor.take() {
                if entries.len() >= SHUFFLE_ENUM_LIMIT || t0.elapsed() > Duration::from_secs(60) {
                    return Err(AppError::invalid("This playlist is too long to shuffle completely."));
                }
                let (more, next) = self.service.collection_more(&ctx, &cid).await?;
                entries.extend(more);
                cursor = next;
            }
        }
        let tracks: Vec<Track> = entries.into_iter().map(|e| e.track).filter(|t| t.playable).collect();
        if tracks.is_empty() {
            return Err(AppError::invalid("Nothing in this collection can be played."));
        }
        let start = if shuffle { (rand_seed() as usize) % tracks.len() } else { start.min(tracks.len() - 1) };
        self.play_tracks(tracks, start, Some(src), shuffle)?;
        if let Some(cid) = cursor {
            let me = self.clone();
            let rev = self.st.lock().queue.source.clone();
            self.st.lock().queue_loading = true;
            tokio::spawn(async move {
                let mut cur = Some(cid);
                let mut pages = 0;
                while let Some(c) = cur.take() {
                    pages += 1;
                    let Ok((more, next)) = me.service.collection_more(&ctx, &c).await else { break };
                    let mut st = me.st.lock();
                    if st.queue.source != rev {
                        break; // queue replaced meanwhile
                    }
                    st.queue.append(more.into_iter().map(|e| e.track).filter(|t| t.playable).collect(), Origin::Collection);
                    drop(st);
                    me.check_preload();
                    me.publish(false);
                    cur = next;
                    if pages > 200 {
                        break;
                    }
                }
                me.st.lock().queue_loading = false;
                me.publish(false);
            });
        }
        Ok(())
    }

    /// Play one track; without context, start its radio.
    pub fn play_track(self: &Arc<Self>, track: Track) -> Result<()> {
        let src = QueueSource { label: format!("{} radio", track.title), collection_id: None, radio_playlist: Some(format!("RDAMVM{}", track.id.remote())) };
        self.play_tracks(vec![track], 0, Some(src), false)
    }

    pub fn toggle(self: &Arc<Self>) {
        let (phase, loaded_alive) = {
            let st = self.st.lock();
            let alive = st.loaded.as_ref().is_some_and(|l| self.engine.status().id == l.load_id);
            (st.phase, alive)
        };
        match phase {
            Phase::Playing | Phase::Buffering | Phase::Loading => self.pause(),
            Phase::Paused if loaded_alive => self.resume(),
            Phase::Ended => {
                let restart = self.st.lock().queue.current().is_some();
                if restart {
                    self.play_current(0.0);
                }
            }
            _ => {
                let pos = self.st.lock().pending_pos;
                if self.st.lock().queue.current().is_some() {
                    self.play_current(pos);
                }
            }
        }
    }

    pub fn pause(&self) {
        self.engine.pause();
        {
            let mut st = self.st.lock();
            if matches!(st.phase, Phase::Playing | Phase::Buffering | Phase::Loading) {
                st.phase = Phase::Paused;
            }
            st.pending_pos = self.position_locked(&st);
        }
        self.publish(false);
        self.send_anchor();
    }

    pub fn resume(&self) {
        self.engine.resume();
        {
            let mut st = self.st.lock();
            if st.phase == Phase::Paused {
                st.phase = Phase::Playing;
            }
        }
        self.publish(false);
        self.send_anchor();
    }

    pub async fn next(self: &Arc<Self>) {
        let step = {
            let mut st = self.st.lock();
            st.skips = 0;
            st.queue.next()
        };
        self.apply_step(step, false).await;
    }

    pub async fn prev(self: &Arc<Self>) {
        let pos = self.snapshot().position;
        let step = self.st.lock().queue.prev(pos);
        self.apply_step(step, false).await;
    }

    pub fn seek(&self, secs: f64) {
        if !secs.is_finite() {
            return;
        }
        let alive = {
            let mut st = self.st.lock();
            st.seek_gen += 1;
            let dur = st.last_duration;
            let secs = match dur {
                Some(d) => secs.clamp(0.0, (d - 0.25).max(0.0)),
                None => secs.max(0.0),
            };
            st.pending_pos = secs;
            let alive = st.loaded.as_ref().is_some_and(|l| self.engine.status().id == l.load_id);
            if st.phase == Phase::Ended {
                st.phase = Phase::Paused;
            }
            alive.then_some(secs)
        };
        if let Some(s) = alive {
            self.engine.seek(s);
        }
        self.publish(false);
        self.send_anchor();
    }

    pub fn jump(self: &Arc<Self>, uid: &str) -> Result<()> {
        let step = self.st.lock().queue.jump(uid)?;
        if let Step::Play(_) = step {
            self.play_current(0.0);
        }
        Ok(())
    }

    pub fn play_next(self: &Arc<Self>, tracks: Vec<Track>) {
        let start = self.st.lock().queue.play_next(tracks);
        self.after_edit(start);
    }

    pub fn add_to_queue(self: &Arc<Self>, tracks: Vec<Track>) {
        let start = self.st.lock().queue.append(tracks, Origin::Manual);
        self.after_edit(start);
    }

    fn after_edit(self: &Arc<Self>, start: Option<usize>) {
        if start.is_some() {
            self.play_current(0.0);
        }
        self.check_preload();
        self.publish(false);
        if self.st.lock().phase == Phase::Playing {
            self.warm_upcoming();
        }
    }

    pub fn remove(self: &Arc<Self>, uid: &str) -> Result<()> {
        self.st.lock().queue.remove(uid)?;
        self.after_edit(None);
        Ok(())
    }

    pub fn move_entry(self: &Arc<Self>, uid: &str, before: Option<&str>) -> Result<()> {
        self.st.lock().queue.move_entry(uid, before)?;
        self.after_edit(None);
        Ok(())
    }

    pub fn clear_up_next(self: &Arc<Self>) {
        {
            let mut st = self.st.lock();
            st.queue.clear_up_next();
            st.radio_cursor = None;
            st.radio_empty = 0;
        }
        self.after_edit(None);
    }

    pub fn set_shuffle(self: &Arc<Self>, on: bool) {
        self.st.lock().queue.set_shuffle(on);
        self.after_edit(None);
    }

    pub fn cycle_repeat(self: &Arc<Self>) -> Repeat {
        let r = self.st.lock().queue.cycle_repeat();
        self.after_edit(None);
        r
    }

    pub fn set_repeat(self: &Arc<Self>, r: Repeat) {
        self.st.lock().queue.set_repeat(r);
        self.after_edit(None);
    }

    pub fn set_autoplay(self: &Arc<Self>, on: bool) {
        {
            let mut st = self.st.lock();
            st.queue.set_autoplay(on);
            st.radio_empty = 0;
        }
        self.publish(false);
    }

    pub fn set_volume(&self, v: f64) {
        if !v.is_finite() {
            return;
        }
        let v = v.clamp(0.0, 1.0);
        let muted = {
            let mut st = self.st.lock();
            st.volume = v;
            if v > 0.0 {
                st.muted = false;
            }
            st.muted
        };
        self.engine.set_volume(if muted { 0.0 } else { v as f32 });
        self.publish(false);
    }

    pub fn set_muted(&self, m: bool) {
        let v = {
            let mut st = self.st.lock();
            st.muted = m;
            st.volume
        };
        self.engine.set_volume(if m { 0.0 } else { v as f32 });
        self.publish(false);
    }

    /// Switch a queue entry to the other version of its recording (song ↔
    /// music video, D4: only ever on request). The entry keeps its uid, its
    /// place and the queue's source. For the entry that is playing, the
    /// position carries over through the service's segment map when it has
    /// one, otherwise the new version starts from the beginning; a paused
    /// entry stays paused.
    pub fn switch_version(self: &Arc<Self>, uid: &str, pair: &Counterpart) -> Result<()> {
        let (current, playing, pos) = {
            let st = self.st.lock();
            let i = st.queue.position_of(uid).ok_or_else(|| AppError::new(encore_core::error::ErrorCode::Stale, "That song is no longer in the queue."))?;
            let e = &st.queue.entries()[i];
            if e.track.id != pair.of {
                return Err(AppError::new(encore_core::error::ErrorCode::Stale, "That version no longer matches what is in the queue."));
            }
            let current = st.queue.index() == Some(i);
            (current, matches!(st.phase, Phase::Playing | Phase::Buffering | Phase::Loading), self.position_locked(&st))
        };
        let mut track = pair.track.clone();
        track.counterpart = Some(pair.of.clone());
        track.liked = None;
        let start = pair.map_position(pos).unwrap_or(0.0);
        self.st.lock().queue.replace_entry_track(uid, track)?;
        if !current {
            self.after_edit(None);
            return Ok(());
        }
        if playing {
            self.engine.clear_preload();
            self.st.lock().preload = None;
            self.play_current(start);
        } else {
            {
                let mut st = self.st.lock();
                st.track_gen += 1;
                st.seek_gen += 1;
                st.loaded = None;
                st.error = None;
                st.pending_pos = start;
                st.last_duration = st.queue.current().and_then(|c| c.track.duration);
                if st.queue.current().is_some() {
                    st.phase = Phase::Paused;
                }
            }
            self.engine.stop();
            self.publish(false);
            let cur = self.st.lock().queue.current().cloned();
            let _ = self.tx.send(PlayerEvent::TrackChanged(cur));
        }
        self.check_preload();
        Ok(())
    }

    /// Play `track` right away, keeping the rest of the queue: it goes in
    /// after the current entry and becomes current.
    pub fn play_now(self: &Arc<Self>, track: Track) -> Result<()> {
        let uid = {
            let mut st = self.st.lock();
            let had = st.queue.current().is_some();
            st.queue.play_next(vec![track]);
            if !had {
                drop(st);
                self.after_edit(Some(0));
                return Ok(());
            }
            st.queue.index().and_then(|i| st.queue.get(i + 1)).map(|u| u.uid.clone())
        };
        match uid {
            Some(u) => self.jump(&u),
            None => Ok(()),
        }
    }

    pub fn update_track(&self, id: &str, f: impl Fn(&mut Track)) {
        self.st.lock().queue.update_track(id, f);
        self.publish(false);
    }

    /// Stop everything (account switch / sign-out).
    pub fn stop(&self) {
        {
            let mut st = self.st.lock();
            st.track_gen += 1;
            st.loaded = None;
            st.preload = None;
            st.phase = if st.queue.current().is_some() { Phase::Paused } else { Phase::Idle };
        }
        self.engine.stop();
        self.publish(false);
    }

    pub fn queue_clone(&self) -> Queue {
        self.st.lock().queue.clone()
    }
}

/// The cached file for a synthetic (cache-hit) stream.
fn cached_path_of(p: &Player, s: &ResolvedStream) -> Option<std::path::PathBuf> {
    if !s.url.expose().is_empty() {
        return None;
    }
    p.cache.as_ref().and_then(|c| c.find_video(&s.video_id)).map(|x| x.0)
}

fn rand_seed() -> u64 {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    (t.as_nanos() as u64) ^ ((std::process::id() as u64) << 32)
}
