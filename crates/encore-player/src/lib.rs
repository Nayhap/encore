//! The playback controller: owns the queue (D6), drives the audio engine
//! (D5), resolves streams (D3/D4), preloads the next track 20 s before the end,
//! extends the queue from radio when autoplay is on, and publishes revisioned
//! state, queue splices and 2 Hz position anchors (D13).
//!
//! Downloaded songs play from disk before anything is resolved. Offline,
//! every step (next, previous, natural end, preload) passes over songs that
//! aren't downloaded, using one rule (`avail`), and radio waits (D23).
//!
//! An A–B loop and a sleep timer (D26) live here too: the engine keeps the
//! loop to the frame and fades out for the timer; the player decides when.

pub mod history;
pub mod plays;
pub mod types;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use encore_audio::{AudioCache, Engine, EngineEvent, HttpSpec, SourceSpec};
use encore_core::error::{AppError, ErrorCode, Result};
use encore_core::model::*;
use encore_core::queue::{Origin, Queue, QueueEntry, QueueSource, Repeat, Step};
use encore_core::redact::Secret;
use encore_core::local::Owner;
use encore_core::local::playback::{ShuffleCycle, ShuffleCycles};
use encore_core::local::plays::PlaySink;
use encore_core::paging::{self, Limits, Pace, Stop};
use encore_core::service::{Admission, LocalAudio, MusicService, PlayFailure, PlayFailureKind, PlayReporter, PlaybackGate, ResolveAuth, ResolvedStream, StreamResolver};
use parking_lot::Mutex;
use tokio::sync::broadcast;

pub use types::*;

pub const PRELOAD_LEAD: f64 = 20.0;
const MAX_AUTO_SKIPS: u32 = 3;
const RADIO_LOW_WATER: usize = 3;
/// How long a newly chosen song must stay chosen before its stream is
/// resolved (see `play_current`).
const SKIP_SETTLE: Duration = Duration::from_millis(400);
const NOTHING_OFFLINE: &str = "You're offline, and nothing else in the queue is downloaded.";
/// The shortest A–B loop (D26).
pub const MIN_LOOP: f64 = 1.0;
/// The sleep timer fades out over its last seconds (D26).
pub const SLEEP_FADE: f64 = 10.0;
const SLEPT: &str = "Paused by the sleep timer.";

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
    /// The preloaded deck plays a file, not a stream.
    preload_local: bool,
    preloading: bool,
    /// Position to resume at when nothing is loaded (restore, or a seek
    /// during loading).
    pending_pos: f64,
    skips: u32,
    radio_cursor: Option<String>,
    radio_loading: bool,
    radio_empty: u32,
    /// The track generation an offline skip already moved on from, so a
    /// failed load and the connection dropping don't both skip.
    skipped_gen: Option<u64>,
    queue_loading: bool,
    last_queue: Vec<QueueEntry>,
    last_queue_rev: u64,
    last_duration: Option<f64>,
    /// Bumped whenever radio results would no longer be wanted (Autoplay
    /// turned off, the queue replaced): a response for an older value is
    /// dropped (D33).
    radio_gen: u64,
    /// The shuffle cycle of the collection playing, while shuffled (D33).
    cycle: Option<CycleRun>,
    /// The A–B loop on the song playing (D26).
    ab_loop: Option<AbLoop>,
    /// The sleep timer (D26).
    sleep: Option<Sleep>,
}

struct AbLoop {
    range: LoopRange,
    /// The track generation it was set in: a loop ends with its song.
    track_gen: u64,
    /// Repeat-one, set aside while the loop runs and put back after.
    repeat: Option<Repeat>,
}

enum Sleep {
    /// Pause at this time, fading out over the last `SLEEP_FADE` seconds.
    /// `fading`: the track generation the engine's fade was started in (a
    /// new song loaded plays at full volume, so the fade starts again).
    At { deadline: Instant, fading: Option<u64> },
    EndOfSong,
    EndOfQueue,
}

/// A shuffle cycle in use: whose it is, the songs that make up the
/// collection, and the cycle as stored.
struct CycleRun {
    owner: Owner,
    members: HashSet<String>,
    c: ShuffleCycle,
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
    /// Downloaded songs (D23).
    local: Mutex<Option<Arc<dyn LocalAudio>>>,
    online: Arc<AtomicBool>,
    /// Counts connection changes: even online, odd offline (for history).
    link: Arc<AtomicU64>,
    /// Held while a play or preload resolves; downloads wait for it.
    gate: Arc<PlaybackGate>,
    /// A private session runs (D36): no history, no Last.fm, no presence.
    private: Arc<AtomicBool>,
    /// When the private session ends by itself; `None` while one runs:
    /// until Encore quits.
    private_until: Mutex<Option<Instant>>,
    /// What may join the queue (D36, D27); lane A asks it.
    admission: Mutex<Option<Arc<dyn Admission>>>,
    /// Where shuffle cycles are kept (D33).
    cycles: Mutex<Option<Arc<dyn ShuffleCycles>>>,
    /// Ends the play being reported before Encore quits (D24).
    settle: Mutex<Option<history::Settle>>,
    /// The local play log's observer, which ends its open play before
    /// Encore quits (D30, D24).
    observer: Mutex<Option<Arc<plays::PlayObserver>>>,
    /// Encore is quitting or installing an update: nothing starts (D24).
    closed: AtomicBool,
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
                preload_local: false,
                preloading: false,
                pending_pos: 0.0,
                skips: 0,
                radio_cursor: None,
                radio_loading: false,
                radio_empty: 0,
                skipped_gen: None,
                queue_loading: false,
                last_queue: vec![],
                last_queue_rev: 0,
                last_duration: None,
                radio_gen: 0,
                cycle: None,
                ab_loop: None,
                sleep: None,
            }),
            tx,
            next_load: AtomicU64::new(1),
            rt: tokio::runtime::Handle::current(),
            on_persist: Mutex::new(None),
            preloaded_streams: Mutex::new(HashMap::new()),
            cache: cache_for_player,
            reporting: Arc::new(AtomicBool::new(false)),
            local: Mutex::new(None),
            online: Arc::new(AtomicBool::new(true)),
            link: Arc::new(AtomicU64::new(0)),
            gate: Arc::new(PlaybackGate::default()),
            private: Arc::new(AtomicBool::new(false)),
            private_until: Mutex::new(None),
            admission: Mutex::new(None),
            cycles: Mutex::new(None),
            settle: Mutex::new(None),
            observer: Mutex::new(None),
            closed: AtomicBool::new(false),
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

    /// Report plays through `reporter` from now on (while the setting is on
    /// and Encore is online).
    pub fn report_history(&self, reporter: Arc<dyn PlayReporter>, on_registered: Arc<dyn Fn(&ItemId) + Send + Sync>) {
        let _guard = self.rt.enter();
        let s = history::HistoryReporter::spawn(self.subscribe(), reporter, self.reporting.clone(), self.link.clone(), self.private.clone(), on_registered);
        *self.settle.lock() = Some(s);
    }

    /// Record every play in the local log through `sink` (D30), whatever
    /// the history setting and the connection. Returns the observer, whose
    /// events (play started, counted, ended) others follow.
    pub fn observe_plays(self: &Arc<Self>, sink: Arc<dyn PlaySink>) -> Arc<plays::PlayObserver> {
        let _guard = self.rt.enter();
        let svc = self.service.clone();
        let o = plays::PlayObserver::spawn(self.subscribe(), sink, Arc::new(move || svc.context().account), self.online.clone(), self.private.clone());
        *self.observer.lock() = Some(o.clone());
        o
    }

    /// Start or end a private session (D36) that lasts until Encore quits.
    /// The flag is shared with the history reporter and the play observer;
    /// the snapshot shows it.
    pub fn set_private(&self, on: bool) {
        if on { self.start_private_session(None) } else { self.end_private_session() }
    }

    pub fn private(&self) -> bool {
        self.private.load(Ordering::SeqCst)
    }

    /// Start a private session (D36): for `length` (lane D offers an hour),
    /// or until Encore quits with `None`. Starting again while one runs
    /// sets its new end. The player's tick ends it on time and says so.
    pub fn start_private_session(&self, length: Option<Duration>) {
        *self.private_until.lock() = length.map(|l| Instant::now() + l);
        self.private.store(true, Ordering::SeqCst);
        self.publish(false);
    }

    /// End the private session now, if one runs.
    pub fn end_private_session(&self) {
        *self.private_until.lock() = None;
        if self.private.swap(false, Ordering::SeqCst) {
            self.publish(false);
        }
    }

    /// The private session, as the snapshot shows it.
    pub fn private_session(&self) -> Option<PrivateSession> {
        self.private().then(|| PrivateSession { remaining: self.private_until.lock().map(|u| u.saturating_duration_since(Instant::now()).as_secs_f64()) })
    }

    /// Called by the tick: a timed private session that ran out ends.
    fn expire_private(&self) {
        let due = self.private() && self.private_until.lock().is_some_and(|u| Instant::now() >= u);
        if due {
            self.end_private_session();
            let _ = self.tx.send(PlayerEvent::Notice("Your private session ended.".into()));
        }
    }

    /// What may join the queue from now on (D36, D27).
    pub fn set_admission(&self, a: Arc<dyn Admission>) {
        *self.admission.lock() = Some(a);
    }

    /// Keep shuffle cycles in `c` (D33).
    pub fn set_shuffle_cycles(&self, c: Arc<dyn ShuffleCycles>) {
        *self.cycles.lock() = Some(c);
    }

    /// The tracks of a list that may join the queue: all that the admission
    /// accepts (D36), plus the one at `keep`, which the user picked by name.
    /// Returns the list and where `keep` is in it.
    fn admit_list(&self, tracks: Vec<Track>, keep: Option<usize>) -> (Vec<Track>, usize) {
        let Some(a) = self.admission.lock().clone() else { return (tracks, keep.unwrap_or(0)) };
        let mut at = 0;
        let mut out = Vec::with_capacity(tracks.len());
        for (i, t) in tracks.into_iter().enumerate() {
            if Some(i) == keep {
                at = out.len();
                out.push(t);
            } else if a.admits(&t) {
                out.push(t);
            }
        }
        (out, at)
    }

    /// Whether `t` may join the queue; everything does until an
    /// `Admission` is set.
    pub fn admits(&self, t: &Track) -> bool {
        self.admission.lock().as_ref().is_none_or(|a| a.admits(t))
    }

    /// Before Encore quits or installs an update (D24): the play being
    /// reported ends, and its last ping goes out within `bound`. Returns
    /// false when it didn't make it in time.
    pub async fn settle_history(&self, bound: Duration) -> bool {
        let s = self.settle.lock().clone();
        match s {
            Some(s) => s.settle(bound).await,
            None => true,
        }
    }

    /// Before Encore quits or installs an update (D24): the play the local
    /// log has open ends with what was heard so far (D30), within `bound`.
    pub async fn settle_plays(&self, bound: Duration) -> bool {
        let o = self.observer.lock().clone();
        match o {
            Some(o) => o.settle(bound).await,
            None => true,
        }
    }

    /// Play downloaded songs from disk (D23).
    pub fn set_local_audio(&self, local: Arc<dyn LocalAudio>) {
        *self.local.lock() = Some(local);
    }

    /// Offline, only downloaded songs play and radio waits (D23).
    pub fn set_online(self: &Arc<Self>, on: bool) {
        let was = self.online.swap(on, Ordering::SeqCst);
        if was == on {
            return;
        }
        self.link.fetch_add(1, Ordering::SeqCst);
        let ok = self.avail();
        let (stalled, track_gen) = {
            let mut st = self.st.lock();
            if on {
                // Radio gets another chance once the connection is back.
                st.radio_empty = 0;
                if st.error.as_ref().is_some_and(|e| e.message == NOTHING_OFFLINE) {
                    st.error = None;
                    st.phase = Phase::Paused;
                }
            } else if st.preload.is_some() && !st.preload_local {
                // A deck streaming over the network won't finish: the
                // file (if the song is downloaded) is preloaded instead.
                st.preload = None;
                drop(st);
                self.engine.clear_preload();
                st = self.st.lock();
            }
            // The song stopped for lack of a connection and isn't
            // downloaded: go on to the next one that is.
            let cur_missing = st.queue.current().is_some_and(|c| !ok(&c.track));
            (!on && st.phase == Phase::Error && cur_missing, st.track_gen)
        };
        self.check_preload();
        self.publish(false);
        if stalled {
            let me = self.clone();
            self.rt.spawn(async move { me.skip_unavailable(track_gen).await });
        }
    }

    pub fn online(&self) -> bool {
        self.online.load(Ordering::SeqCst)
    }

    pub fn gate(&self) -> Arc<PlaybackGate> {
        self.gate.clone()
    }

    /// The one rule for what can play now: anything online; offline, only
    /// downloaded songs.
    fn avail(&self) -> impl Fn(&Track) -> bool {
        let online = self.online();
        let local = self.local.lock().clone();
        move |t: &Track| online || local.as_ref().is_some_and(|l| l.has(t.id.remote()))
    }

    /// Say how many songs a step passed over for not being downloaded.
    fn note_passed(&self, n: usize) {
        if n > 0 {
            let text = if n == 1 { "Skipped a song that isn't downloaded.".to_string() } else { format!("Skipped {n} songs that aren't downloaded.") };
            let _ = self.tx.send(PlayerEvent::Notice(text));
        }
    }

    /// A file to play `vid` from without resolving: a download first, then
    /// the playback cache.
    fn local_source(&self, vid: &str) -> Option<(ResolvedStream, PathBuf)> {
        let local = self.local.lock().clone();
        if let Some(f) = local.and_then(|l| l.lookup(vid)) {
            let s = ResolvedStream {
                video_id: vid.into(),
                url: Secret::new(String::new()),
                headers: vec![],
                itag: f.itag,
                codec: f.codec.clone(),
                mime: String::new(),
                bitrate_kbps: f.kbps,
                content_length: Some(f.bytes),
                duration: f.duration,
                loudness_db: f.loudness_db,
                expires_at: None,
                range_safe: true,
                authenticated: false,
                available_at: None,
            };
            return Some((s, f.path));
        }
        let (path, itag) = self.cache.as_ref()?.find_video(vid)?;
        let s = ResolvedStream {
            video_id: vid.into(),
            url: Secret::new(String::new()),
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
            available_at: None,
        };
        Some((s, path))
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

    /// Before Encore quits or installs an update (D24): pause, drop what
    /// is loaded or preloading (a resolve that finishes later finds an old
    /// generation and starts nothing), refuse to start anything, and save
    /// the queue and position as they are now, not on the 5 s tick.
    pub fn shut_down(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.pause();
        {
            let mut st = self.st.lock();
            // The loop goes with the deck, and repeat-one comes back before
            // the queue is saved; a sleep timer doesn't outlive Encore (D26).
            if let Some(r) = st.ab_loop.take().and_then(|l| l.repeat) {
                st.queue.set_repeat(r);
            }
            st.sleep = None;
            st.track_gen += 1;
            st.loaded = None;
            st.preload = None;
            st.phase = if st.queue.current().is_some() { Phase::Paused } else { Phase::Idle };
        }
        self.engine.set_loop(None);
        self.engine.cancel_fade();
        self.engine.hold_next(false);
        self.engine.stop();
        self.publish(false);
        self.persist();
    }

    /// The exit was called off: playback may start again (still paused,
    /// where it was).
    pub fn reopen(&self) {
        self.closed.store(false, Ordering::SeqCst);
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
            st.radio_gen += 1;
            st.cycle = None;
        }
        self.publish(true);
        let cur = self.st.lock().queue.current().cloned();
        let _ = self.tx.send(PlayerEvent::TrackChanged(cur));
    }

    /// Replace the queue with a saved one (lane C's saved queues, D33),
    /// paused at `position` in its current song: the order, uids, current
    /// entry, shuffle and repeat come from `queue`, Autoplay stays as the
    /// user has it now. Whatever was playing stops; nothing plays until the
    /// user presses play.
    pub fn restore_paused(&self, mut queue: Queue, position: f64) -> Result<()> {
        if queue.current().is_none() {
            return Err(AppError::invalid("This saved queue is empty."));
        }
        {
            let mut st = self.st.lock();
            queue.autoplay = st.queue.autoplay;
            st.queue = queue;
            st.track_gen += 1;
            st.seek_gen += 1;
            st.loaded = None;
            st.preload = None;
            st.error = None;
            st.skips = 0;
            st.radio_cursor = None;
            st.radio_empty = 0;
            st.radio_gen += 1;
            st.cycle = None;
            st.last_duration = st.queue.current().and_then(|c| c.track.duration);
            let end = st.last_duration.map(|d| (d - 0.25).max(0.0)).unwrap_or(f64::MAX);
            st.pending_pos = if position.is_finite() { position.clamp(0.0, end) } else { 0.0 };
            st.phase = Phase::Paused;
        }
        self.engine.stop();
        self.publish(true);
        let cur = self.st.lock().queue.current().cloned();
        let _ = self.tx.send(PlayerEvent::TrackChanged(cur));
        Ok(())
    }

    /// The queue and the position in its current song, for saving it.
    pub fn queue_and_position(&self) -> (Queue, f64) {
        let st = self.st.lock();
        (st.queue.clone(), self.position_locked(&st))
    }

    /// The block list (or another admission rule) changed (lane D, D36):
    /// songs it now refuses leave what's up next, and if the song playing is
    /// one of them, playback moves on as Next does.
    pub async fn admission_changed(self: &Arc<Self>) {
        let Some(a) = self.admission.lock().clone() else { return };
        let skip = {
            let mut st = self.st.lock();
            st.queue.retain_upcoming(|e| a.admits(&e.track));
            st.queue.current().is_some_and(|c| !a.admits(&c.track)) && matches!(st.phase, Phase::Playing | Phase::Buffering | Phase::Loading)
        };
        self.check_preload();
        self.publish(false);
        if skip {
            self.next().await;
        }
    }

    // ---------- the A–B loop (D26) ----------

    /// Loop the song playing between `a` and `b` seconds (in either order).
    /// The engine wraps at B to the frame; while the loop runs nothing is
    /// preloaded, repeat-one is set aside, and the song never ends. Playing
    /// from outside the loop jumps to A.
    pub fn set_loop(&self, a: f64, b: f64) -> Result<()> {
        if !a.is_finite() || !b.is_finite() {
            return Err(AppError::invalid("That isn't a time in this song."));
        }
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let (range, jump) = {
            let mut st = self.st.lock();
            let es = self.engine.status();
            let Some(l) = st.loaded.as_ref().filter(|l| l.load_id == es.id) else {
                return Err(AppError::invalid("Play the song to loop part of it."));
            };
            if !l.stream.range_safe {
                return Err(AppError::invalid("This song's stream can't jump back, so it can't loop."));
            }
            let Some(dur) = es.duration.or(st.last_duration) else {
                return Err(AppError::invalid("This song's length isn't known yet. Try again in a moment."));
            };
            let range = LoopRange { a: a.clamp(0.0, dur), b: b.clamp(0.0, dur) };
            if range.b - range.a < MIN_LOOP {
                return Err(AppError::invalid("A loop needs to be at least a second long."));
            }
            let tg = st.track_gen;
            let repeat = match &st.ab_loop {
                Some(l) if l.track_gen == tg => l.repeat,
                _ => (st.queue.repeat == Repeat::One).then_some(Repeat::One),
            };
            if st.queue.repeat == Repeat::One {
                st.queue.set_repeat(Repeat::Off);
            }
            st.ab_loop = Some(AbLoop { range, track_gen: tg, repeat });
            st.preload = None;
            let pos = self.position_locked(&st);
            (range, (pos < range.a - 0.05 || pos >= range.b).then_some(range.a))
        };
        self.engine.clear_preload();
        self.engine.set_loop(Some((range.a, range.b)));
        if let Some(t) = jump {
            self.seek(t);
        } else {
            self.publish(false);
            self.send_anchor();
        }
        Ok(())
    }

    /// End the loop: the song plays on from where it is, and repeat-one
    /// comes back if the loop set it aside.
    pub fn clear_loop(&self) {
        let had = {
            let mut st = self.st.lock();
            let tg = st.track_gen;
            match st.ab_loop.take() {
                Some(l) => {
                    if let Some(r) = l.repeat {
                        st.queue.set_repeat(r);
                    }
                    l.track_gen == tg
                }
                None => false,
            }
        };
        if had {
            self.engine.set_loop(None);
            self.publish(false);
            self.send_anchor();
        }
    }

    fn snapshot_loop(&self) -> Option<LoopRange> {
        let st = self.st.lock();
        st.ab_loop.as_ref().filter(|l| l.track_gen == st.track_gen).map(|l| l.range)
    }

    /// A loop ends with its song: once the track generation moved on, put
    /// repeat-one back and forget it (the new deck never had it).
    fn end_stale_loop(st: &mut State) {
        if st.ab_loop.as_ref().is_some_and(|l| l.track_gen != st.track_gen)
            && let Some(r) = st.ab_loop.take().and_then(|l| l.repeat)
        {
            st.queue.set_repeat(r);
        }
    }

    // ---------- the sleep timer (D26) ----------

    /// Start the sleep timer: after `minutes` (`SleepMode::Timer`), at the
    /// end of the song playing, or at the end of the queue (no radio is
    /// added meanwhile). Replaces a timer that runs.
    pub fn set_sleep(&self, mode: SleepMode, minutes: Option<f64>) -> Result<()> {
        let sleep = match mode {
            SleepMode::Timer => {
                let m = minutes.filter(|m| m.is_finite() && *m > 0.0 && *m <= 24.0 * 60.0).ok_or_else(|| AppError::invalid("Choose how long until Encore pauses."))?;
                Sleep::At { deadline: Instant::now() + Duration::from_secs_f64(m * 60.0), fading: None }
            }
            SleepMode::EndOfSong => {
                if self.snapshot_loop().is_some() {
                    return Err(AppError::invalid("Clear the loop to stop at the end of this song."));
                }
                Sleep::EndOfSong
            }
            SleepMode::EndOfQueue => Sleep::EndOfQueue,
        };
        self.st.lock().sleep = Some(sleep);
        self.engine.cancel_fade();
        self.sync_hold();
        self.check_preload();
        self.publish(false);
        Ok(())
    }

    pub fn cancel_sleep(&self) {
        let had = self.st.lock().sleep.take().is_some();
        if had {
            self.engine.cancel_fade();
            self.sync_hold();
            self.publish(false);
        }
    }

    /// The song playing is the last before the sleep timer stops playback.
    fn sleep_holds(st: &State) -> bool {
        match st.sleep {
            Some(Sleep::EndOfSong) => true,
            Some(Sleep::EndOfQueue) => st.queue.index().is_none_or(|i| i + 1 >= st.queue.len()),
            _ => false,
        }
    }

    /// Keep the engine from taking up the preloaded deck when this song is
    /// the sleep timer's last (the engine switches decks on its own, before
    /// the player could stop it), and drop a preload that won't be used.
    fn sync_hold(&self) {
        let hold = Self::sleep_holds(&self.st.lock());
        self.engine.hold_next(hold);
        if hold && self.st.lock().preload.take().is_some() {
            self.engine.clear_preload();
        }
    }

    /// Called by the tick: fade out over the timer's last seconds, pause
    /// when it runs out. Paused, a timer that runs out just ends.
    fn sleep_tick(&self) {
        let (fade, fire, ended) = {
            let mut st = self.st.lock();
            let playing = matches!(st.phase, Phase::Playing | Phase::Buffering | Phase::Loading);
            let tg = st.track_gen;
            match st.sleep.as_mut() {
                Some(Sleep::At { deadline, fading }) => {
                    let rem = deadline.saturating_duration_since(Instant::now()).as_secs_f64();
                    if !playing {
                        if rem <= 0.0 {
                            st.sleep = None;
                            (None, false, true)
                        } else {
                            (None, false, false)
                        }
                    } else if rem <= 0.0 {
                        (None, true, false)
                    } else if rem <= SLEEP_FADE && *fading != Some(tg) {
                        *fading = Some(tg);
                        (Some(rem), false, false)
                    } else {
                        (None, false, false)
                    }
                }
                _ => (None, false, false),
            }
        };
        if let Some(r) = fade {
            self.engine.fade_out(r);
        }
        if fire {
            self.sleep_fire();
        } else if ended {
            self.publish(false);
        }
        if matches!(self.st.lock().sleep, Some(Sleep::EndOfQueue)) {
            self.sync_hold();
        }
    }

    /// The timer ran out: pause (the engine has faded to silence; resuming
    /// brings the volume back).
    fn sleep_fire(&self) {
        if self.st.lock().sleep.take().is_none() {
            return;
        }
        self.pause();
        let _ = self.tx.send(PlayerEvent::Notice(SLEPT.into()));
    }

    /// The song the sleep timer waited for has ended: the next one waits,
    /// paused at its start (or the queue has ended).
    fn sleep_at_end(&self) {
        let ok = self.avail();
        let moved = {
            let mut st = self.st.lock();
            let to_end = matches!(st.sleep, Some(Sleep::EndOfQueue));
            st.sleep = None;
            let step = if to_end { Step::End } else { st.queue.on_ended_where(&ok) };
            match step {
                Step::Play(_) | Step::Restart => {
                    st.track_gen += 1;
                    st.seek_gen += 1;
                    st.loaded = None;
                    st.preload = None;
                    st.error = None;
                    st.pending_pos = 0.0;
                    st.last_duration = st.queue.current().and_then(|c| c.track.duration);
                    st.phase = Phase::Paused;
                    true
                }
                _ => {
                    st.phase = Phase::Ended;
                    st.loaded = None;
                    st.pending_pos = st.last_duration.unwrap_or(0.0);
                    false
                }
            }
        };
        self.engine.hold_next(false);
        self.engine.stop();
        self.publish(false);
        if moved {
            let cur = self.st.lock().queue.current().cloned();
            let _ = self.tx.send(PlayerEvent::TrackChanged(cur));
        }
        let _ = self.tx.send(PlayerEvent::Notice(SLEPT.into()));
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
            ab_loop: st.ab_loop.as_ref().filter(|l| l.track_gen == st.track_gen).map(|l| l.range),
            sleep: st.sleep.as_ref().map(|s| match s {
                Sleep::At { deadline, .. } => SleepTimer { mode: SleepMode::Timer, remaining: Some(deadline.saturating_duration_since(Instant::now()).as_secs_f64()) },
                Sleep::EndOfSong => SleepTimer { mode: SleepMode::EndOfSong, remaining: None },
                Sleep::EndOfQueue => SleepTimer { mode: SleepMode::EndOfQueue, remaining: None },
            }),
            private: self.private_session(),
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
            Self::end_stale_loop(&mut st);
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
        if self.closed.load(Ordering::SeqCst) {
            return;
        }
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
            let t0 = Instant::now();
            // A downloaded or fully cached file plays at once, without
            // resolving a URL (and without the player components, D22).
            let local = me.local_source(&vid);
            if local.is_none() && !me.online() {
                me.skip_unavailable(my_gen).await;
                return;
            }
            // Pressing Next several times in a row used to resolve every song
            // passed over, one after another (seconds each, two at a time),
            // so the song the user stopped on started last. Wait a moment
            // and give up if another song was chosen meanwhile.
            if local.is_none() && promoted.is_none() {
                tokio::time::sleep(SKIP_SETTLE).await;
                if me.st.lock().track_gen != my_gen {
                    return;
                }
            }
            let (res, path) = match local {
                Some((s, p)) => (Ok(s), Some(p)),
                None => {
                    let res = {
                        let _busy = me.gate.enter();
                        me.resolve_as_now(&vid).await
                    };
                    // A preroll (D37): said, and waited out without holding
                    // the gate, so downloads and paging go on meanwhile.
                    let res = match res {
                        Ok(s) if s.available_at.is_some() => {
                            if let Some(secs) = s.wait_secs().filter(|&w| w >= 3) {
                                let _ = me.tx.send(PlayerEvent::Notice(format!("This song starts in {secs} seconds: YouTube holds it back for the length of an ad.")));
                            }
                            let r = me.resolver.ready(s, (me.auth)().as_ref()).await;
                            if me.st.lock().track_gen != my_gen {
                                return;
                            }
                            r
                        }
                        r => r,
                    };
                    (res, None)
                }
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
                        let spec = match path {
                            Some(p) => SourceSpec::File(p),
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
        // The connection went while this song was loading and it isn't
        // downloaded: go on as `set_online(false)` does for a stopped song.
        if !self.online() {
            let ok = self.avail();
            let (missing, track_gen) = {
                let st = self.st.lock();
                (st.queue.current().is_some_and(|c| !ok(&c.track)), st.track_gen)
            };
            if missing {
                self.skip_unavailable(track_gen).await;
                return;
            }
        }
        if matches!(f.kind, PlayFailureKind::Network | PlayFailureKind::Challenged) {
            let _ = self.tx.send(PlayerEvent::Notice(f.message.clone()));
        } else if autoskip {
            let _ = self.tx.send(PlayerEvent::Notice(format!("Skipped “{title}”: {}", f.message)));
            tokio::time::sleep(Duration::from_millis(600)).await;
            let ok = self.avail();
            let step = self.st.lock().queue.next_where(&ok);
            self.apply_step(step, true).await;
        } else if skips >= MAX_AUTO_SKIPS {
            let _ = self.tx.send(PlayerEvent::Notice(format!("Stopped after {MAX_AUTO_SKIPS} songs in a row could not play: {}", f.message)));
        }
    }

    /// Offline, the current song isn't downloaded: go on to the next one
    /// that is. Not a failure of the song, so it doesn't count toward D4's
    /// three skips. With nothing left to play, stop with one message.
    async fn skip_unavailable(self: &Arc<Self>, my_gen: u64) {
        let (step, title, passed) = {
            let mut st = self.st.lock();
            if st.track_gen != my_gen || st.skipped_gen == Some(my_gen) {
                return;
            }
            st.skipped_gen = Some(my_gen);
            let title = st.queue.current().map(|c| c.track.title.clone()).unwrap_or_default();
            let ok = self.avail();
            let step = st.queue.next_where(&ok);
            (step, title, st.queue.passed())
        };
        if let Step::Play(_) = step {
            let text = if passed == 0 { format!("Skipped “{title}”: it isn't downloaded.") } else { format!("Skipped {} songs that aren't downloaded.", passed + 1) };
            let _ = self.tx.send(PlayerEvent::Notice(text));
            self.play_current(0.0);
            return;
        }
        {
            let mut st = self.st.lock();
            st.phase = Phase::Error;
            st.loaded = None;
            st.error = Some(PlayFailure::new(PlayFailureKind::Network, NOTHING_OFFLINE));
        }
        self.publish(false);
        let _ = self.tx.send(PlayerEvent::Notice(NOTHING_OFFLINE.into()));
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
                let ok = self.avail();
                let step = {
                    let mut st = self.st.lock();
                    if added > 0 { st.queue.next_where(&ok) } else { Step::End }
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
        // Offline, radio waits for the connection (D23).
        if !self.online() {
            return 0;
        }
        let (seed, cursor, radio_gen) = {
            let mut st = self.st.lock();
            if st.radio_loading || st.radio_empty >= 3 {
                return 0;
            }
            if !st.queue.autoplay {
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
            (seed, st.radio_cursor.clone(), st.radio_gen)
        };
        let ctx = self.service.context();
        let res = self.service.up_next(&ctx, &seed, cursor.as_deref()).await;
        let mut st = self.st.lock();
        st.radio_loading = false;
        // Autoplay went off (or the queue was replaced) while this was on
        // its way: nothing joins the queue (D33).
        if st.radio_gen != radio_gen || !st.queue.autoplay {
            return 0;
        }
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
                let tracks: Vec<Track> = u.tracks.into_iter().filter(|t| self.admits(t)).collect();
                let n = st.queue.extend_radio(tracks);
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
        if self.closed.load(Ordering::SeqCst) {
            return;
        }
        self.expire_private();
        self.sleep_tick();
        let (phase, pos, dur, upcoming, need_radio) = {
            let st = self.st.lock();
            let snap = self.snapshot_locked(&st);
            let ok = self.avail();
            // Nothing is preloaded while a loop runs or the sleep timer
            // stops after this song (D26).
            let up = if snap.ab_loop.is_some() || Self::sleep_holds(&st) { None } else { st.queue.upcoming_where(&ok).cloned() };
            let until_end = matches!(st.sleep, Some(Sleep::EndOfQueue));
            let need_radio = st.queue.autoplay && !until_end && st.queue.repeat == Repeat::Off && st.queue.remaining() < RADIO_LOW_WATER && !st.radio_loading && st.radio_empty < 3 && st.queue.current().is_some() && !st.queue_loading && self.online();
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

    /// Resolve as whoever is signed in now. A result that comes back after
    /// the account or channel changed belongs to the old identity (D2): it
    /// is dropped and the song resolved again as the new one.
    async fn resolve_as_now(&self, vid: &str) -> std::result::Result<ResolvedStream, PlayFailure> {
        let key = |a: &Option<ResolveAuth>| a.as_ref().map(|a| a.key.clone());
        let mut auth = (self.auth)();
        for _ in 0..2 {
            let r = self.resolver.resolve(vid, auth.as_ref()).await;
            let now = (self.auth)();
            if key(&now) == key(&auth) {
                return r;
            }
            tracing::info!(video = vid, "the account changed during a resolve; resolving again");
            auth = now;
        }
        self.resolver.resolve(vid, auth.as_ref()).await
    }

    async fn preload(self: &Arc<Self>, entry: QueueEntry) {
        let vid = entry.track.id.remote().to_string();
        let (res, path) = match self.local_source(&vid) {
            Some((s, p)) => (Ok(s), Some(p)),
            None if !self.online() => (Err(PlayFailure::new(PlayFailureKind::Network, "offline")), None),
            None => {
                let res = {
                    let _busy = self.gate.enter();
                    self.resolve_as_now(&vid).await
                };
                // The preroll is waited out here, without the gate (D37).
                let res = match res {
                    Ok(s) => self.resolver.ready(s, (self.auth)().as_ref()).await,
                    e => e,
                };
                (res, None)
            }
        };
        let ok = self.avail();
        let mut st = self.st.lock();
        st.preloading = false;
        // Still the right entry to come next?
        if st.queue.upcoming_where(&ok).map(|u| u.uid.as_str()) != Some(entry.uid.as_str()) {
            return;
        }
        // A loop started or the sleep timer stops after this song (D26).
        if st.ab_loop.as_ref().is_some_and(|l| l.track_gen == st.track_gen) || Self::sleep_holds(&st) {
            return;
        }
        // Resolved online, but the connection went meanwhile.
        if path.is_none() && !self.online() {
            return;
        }
        if let Ok(stream) = res {
            let load_id = self.next_load.fetch_add(1, Ordering::Relaxed);
            st.decks.insert(load_id, entry.uid.clone());
            st.preload = Some((entry.uid.clone(), load_id));
            st.preload_local = path.is_some();
            let gain = stream.loudness_db.map(|l| -(l.max(0.0))).unwrap_or(0.0);
            let spec = match path {
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
        if !self.online() {
            return;
        }
        let Some(up) = self.st.lock().queue.upcoming().cloned() else { return };
        if self.local_source(up.track.id.remote()).is_some() {
            return;
        }
        let me = self.clone();
        tokio::spawn(async move {
            let auth = (me.auth)();
            let _busy = me.gate.enter();
            let _ = me.resolver.resolve(up.track.id.remote(), auth.as_ref()).await;
        });
    }

    /// Drop a preload that no longer matches what comes next.
    fn check_preload(&self) {
        let ok = self.avail();
        let mut st = self.st.lock();
        let up = st.queue.upcoming_where(&ok).map(|u| u.uid.clone());
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
                    self.note_cycle();
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
                let avail = self.avail();
                let (ok, passed) = {
                    let mut st = self.st.lock();
                    if st.loaded.as_ref().map(|l| l.load_id) != Some(from) {
                        (false, 0)
                    } else {
                        let next_uid = st.decks.get(&to).cloned();
                        let step = st.queue.on_ended_where(&avail);
                        let passed = st.queue.passed();
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
                            (true, passed)
                        } else {
                            (false, passed)
                        }
                    }
                };
                self.note_passed(passed);
                if ok {
                    self.note_cycle();
                    self.publish(false);
                    self.send_anchor();
                    let cur = self.st.lock().queue.current().cloned();
                    let _ = self.tx.send(PlayerEvent::TrackChanged(cur));
                } else {
                    self.play_current(0.0);
                }
            }
            EngineEvent::Ended { id } => {
                let avail = self.avail();
                let (step, passed) = {
                    let mut st = self.st.lock();
                    if st.loaded.as_ref().map(|l| l.load_id) != Some(id) {
                        return;
                    }
                    if Self::sleep_holds(&st) {
                        drop(st);
                        self.sleep_at_end();
                        return;
                    }
                    (st.queue.on_ended_where(&avail), st.queue.passed())
                };
                self.note_passed(passed);
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
            EngineEvent::FadedOut => {
                let due = matches!(self.st.lock().sleep, Some(Sleep::At { deadline, .. }) if deadline.saturating_duration_since(Instant::now()) < Duration::from_secs(1));
                if due {
                    self.sleep_fire();
                }
            }
            EngineEvent::NoOutput { error } => {
                tracing::warn!(%error, "no audio output");
                self.publish(false);
                let _ = self.tx.send(PlayerEvent::Notice("No audio output is available. Connect speakers or headphones, or check your sound settings; Encore starts playing as soon as one works.".into()));
            }
        }
    }

    // ---------- commands (the UI contract) ----------

    /// Replace the queue with `tracks` and play from `start`. Songs the
    /// admission refuses (D36) are left out, except the one at `start`.
    pub fn play_tracks(self: &Arc<Self>, tracks: Vec<Track>, start: usize, source: Option<QueueSource>, shuffle: bool) -> Result<()> {
        let (tracks, start) = self.admit_list(tracks, Some(start));
        self.start_queue(None, |q| q.replace(tracks, start, source, shuffle, Origin::Collection).map(|_| ()))
    }

    /// Replace the queue through `fill` and play its current entry.
    fn start_queue(self: &Arc<Self>, cycle: Option<CycleRun>, fill: impl FnOnce(&mut Queue) -> Result<()>) -> Result<()> {
        {
            let mut st = self.st.lock();
            fill(&mut st.queue)?;
            st.radio_cursor = None;
            st.radio_empty = 0;
            st.radio_gen += 1;
            st.skips = 0;
            st.preload = None;
            st.cycle = cycle;
        }
        self.engine.clear_preload();
        self.play_current(0.0);
        self.publish(false);
        Ok(())
    }

    /// Shuffle a whole collection in its shuffle cycle (D33): `members` are
    /// the collection's songs that can play here, `tracks` those to queue
    /// now (offline, only the downloaded ones). The cycle is read, fitted
    /// to the collection as it is now and saved under `owner`.
    fn play_cycle(self: &Arc<Self>, owner: Owner, collection_id: &str, members: HashSet<String>, tracks: Vec<Track>, source: QueueSource) -> Result<()> {
        let store = self.cycles.lock().clone();
        let mut c = store.as_ref().and_then(|s| s.load(&owner, collection_id)).unwrap_or_else(|| ShuffleCycle::new(collection_id));
        let before = c.clone();
        let played = c.fit(&members);
        if let Some(s) = store.as_ref().filter(|_| c != before) {
            s.save(&owner, &c);
        }
        let run = CycleRun { owner, members, c };
        self.start_queue(Some(run), |q| q.replace_cycle(tracks, Some(source), &played).map(|_| ()))
    }

    /// The song playing counts toward its collection's shuffle cycle (D33):
    /// called once it is heard. A finished cycle starts the next one.
    fn note_cycle(&self) {
        let save = {
            let mut st = self.st.lock();
            let Some(vid) = st.queue.current().filter(|c| c.origin == Origin::Collection).map(|c| c.track.id.remote().to_string()) else { return };
            let shuffle = st.queue.shuffle;
            let Some(run) = st.cycle.as_mut().filter(|r| shuffle && r.members.contains(&vid)) else { return };
            if !run.c.mark(&vid) {
                return;
            }
            if run.c.played.len() >= run.members.len() {
                run.c.cycle += 1;
                run.c.played.clear();
            }
            (run.owner.clone(), run.c.clone())
        };
        if let Some(s) = self.cycles.lock().clone() {
            s.save(&save.0, &save.1);
        }
    }

    /// The cycle for shuffling the queue as it is (shuffle turned on while
    /// a collection plays): the one in use, or the stored one.
    fn cycle_for_queue(&self) -> Option<CycleRun> {
        let owner = Owner::of(&self.service.context());
        let (cid, members) = {
            let mut st = self.st.lock();
            let cid = st.queue.source.as_ref()?.collection_id.clone()?;
            if st.queue_loading {
                return None;
            }
            if let Some(run) = st.cycle.take().filter(|r| r.c.collection_id == cid && r.owner == owner) {
                return Some(run);
            }
            let members: HashSet<String> = st.queue.entries().iter().filter(|e| e.origin == Origin::Collection).map(|e| e.track.id.remote().to_string()).collect();
            (cid, members)
        };
        let store = self.cycles.lock().clone()?;
        let mut c = store.load(&owner, &cid).unwrap_or_else(|| ShuffleCycle::new(&cid));
        c.fit(&members);
        Some(CycleRun { owner, members, c })
    }

    /// Play a collection: first page now, the rest read in order through
    /// the paced background lane (`encore_core::paging`) and appended. With
    /// shuffle, the whole collection (bounded) is read first, at the
    /// interactive pace, so shuffle covers every song (D6); a list that
    /// didn't really end isn't shuffled as if it had. `params` comes with
    /// links that carry them (an artist's "All songs").
    pub async fn play_collection(self: &Arc<Self>, id: &ItemId, params: Option<&str>, start: usize, shuffle: bool) -> Result<()> {
        if !self.online() {
            return self.play_kept(id, start, shuffle);
        }
        let ctx = self.service.context();
        let owner = Owner::of(&ctx);
        let c = match self.service.collection_at(&ctx, id, params).await {
            Ok(c) => c,
            // The connection went: a kept copy still plays.
            Err(e) if e.code == ErrorCode::Network => return self.play_kept(id, start, shuffle).map_err(|_| e),
            Err(e) => return Err(e),
        };
        let mut entries = c.entries.clone();
        let mut cursor = c.cursor.clone();
        let src = QueueSource { label: c.title.clone(), collection_id: Some(id.as_str().to_string()), radio_playlist: c.playlist_id.clone().map(|p| if p.starts_with("RD") { p } else { format!("RDAMPL{p}") }) };
        // Same account, still online: otherwise the answers are dropped.
        let same = {
            let (svc, ctx, online) = (self.service.clone(), ctx.clone(), self.online.clone());
            move || online.load(Ordering::SeqCst) && svc.context() == ctx
        };
        if shuffle {
            if let Some(cid) = cursor.take() {
                let mut rest = Vec::new();
                match paging::read_rest(&*self.service, &ctx, cid, &self.gate, Pace::Interactive, Limits::SHUFFLE, &same, &mut |page| rest.extend(page)).await {
                    Ok(()) => entries.extend(rest),
                    Err(Stop::Limit) => return Err(AppError::invalid("This playlist is too long to shuffle completely.")),
                    Err(Stop::Stalled) => return Err(AppError::network("YouTube Music stopped listing this playlist partway, so it can't be shuffled completely. Try again in a moment.")),
                    Err(Stop::Failed(e)) => return Err(e),
                    Err(Stop::Cancelled) => return Err(AppError::stale()),
                }
            }
        }
        let tracks: Vec<Track> = entries.into_iter().map(|e| e.track).filter(|t| t.playable).collect();
        if tracks.is_empty() {
            return Err(AppError::invalid("Nothing in this collection can be played."));
        }
        if shuffle {
            // Blocked songs (D36) are neither queued nor needed to finish a cycle.
            let (tracks, _) = self.admit_list(tracks, None);
            if tracks.is_empty() {
                return Err(AppError::invalid("Nothing in this collection can be played."));
            }
            let members = tracks.iter().map(|t| t.id.remote().to_string()).collect();
            self.play_cycle(owner, id.as_str(), members, tracks, src)?;
        } else {
            let start = start.min(tracks.len() - 1);
            self.play_tracks(tracks, start, Some(src), false)?;
        }
        if let Some(cid) = cursor {
            let me = self.clone();
            let rev = self.st.lock().queue.source.clone();
            self.st.lock().queue_loading = true;
            let id = id.clone();
            tokio::spawn(async move {
                let (me2, rev2) = (me.clone(), rev.clone());
                // The queue replaced meanwhile: the rest isn't wanted.
                let still = move || same() && me2.st.lock().queue.source == rev2;
                let r = paging::read_rest(&*me.service, &ctx, cid, &me.gate, Pace::Background, Limits::QUEUE, &still, &mut |page| {
                    let mut st = me.st.lock();
                    if st.queue.source != rev {
                        return;
                    }
                    let page: Vec<Track> = page.into_iter().map(|e| e.track).filter(|t| t.playable && me.admits(t)).collect();
                    st.queue.append(page, Origin::Collection);
                    drop(st);
                    me.check_preload();
                    me.publish(false);
                })
                .await;
                if let Err(stop) = r {
                    tracing::info!(collection = %id, ?stop, "the rest of a collection wasn't read to its end");
                }
                me.st.lock().queue_loading = false;
                me.publish(false);
            });
        }
        Ok(())
    }

    /// Play the saved order of a kept collection, as much of it as is
    /// downloaded (D23). `start` is a row of the full list.
    fn play_kept(self: &Arc<Self>, id: &ItemId, start: usize, shuffle: bool) -> Result<()> {
        let local = self.local.lock().clone().ok_or_else(|| AppError::network("You're offline. Connect to the internet to play this."))?;
        let ctx = self.service.context();
        let owner = Owner::of(&ctx);
        let account = ctx.account;
        let c = local.collection(&account, id).ok_or_else(|| AppError::network("You're offline, and this isn't downloaded. Connect to the internet to play it."))?;
        let here: Vec<bool> = c.entries.iter().map(|e| local.has(e.track.id.remote())).collect();
        let tracks: Vec<Track> = c.entries.iter().zip(&here).filter(|(_, h)| **h).map(|(e, _)| e.track.clone()).collect();
        if tracks.is_empty() {
            return Err(AppError::network("None of these songs are downloaded yet. Connect to the internet to play them."));
        }
        let src = QueueSource { label: c.title.clone(), collection_id: Some(id.as_str().to_string()), radio_playlist: None };
        if shuffle {
            // The cycle is the whole collection's: songs that aren't
            // downloaded stay unplayed in it until they play online.
            let members = c.entries.iter().map(|e| &e.track).filter(|t| t.playable && self.admits(t)).map(|t| t.id.remote().to_string()).collect();
            let (tracks, _) = self.admit_list(tracks, None);
            if tracks.is_empty() {
                return Err(AppError::network("None of these songs are downloaded yet. Connect to the internet to play them."));
            }
            return self.play_cycle(owner, id.as_str(), members, tracks, src);
        }
        let start = if here.get(start).copied().unwrap_or(false) {
            here[..start].iter().filter(|h| **h).count()
        } else if start == 0 {
            0
        } else {
            return Err(AppError::network("This song isn't downloaded. Connect to the internet to play it."));
        };
        self.play_tracks(tracks, start, Some(src), false)
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

    /// Try the current song again if it stopped on an error (after the
    /// session was refreshed, D38); otherwise nothing.
    pub fn retry_failed(self: &Arc<Self>) {
        let pos = {
            let st = self.st.lock();
            if st.phase != Phase::Error || st.queue.current().is_none() {
                return;
            }
            st.pending_pos
        };
        self.play_current(pos);
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
        if self.closed.load(Ordering::SeqCst) {
            return;
        }
        self.engine.resume();
        {
            let mut st = self.st.lock();
            if st.phase == Phase::Paused {
                st.phase = Phase::Playing;
            }
            // Resuming plays at full volume: a sleep fade starts again.
            if let Some(Sleep::At { fading, .. }) = st.sleep.as_mut() {
                *fading = None;
            }
        }
        self.publish(false);
        self.send_anchor();
    }

    pub async fn next(self: &Arc<Self>) {
        let ok = self.avail();
        let (step, passed) = {
            let mut st = self.st.lock();
            st.skips = 0;
            (st.queue.next_where(&ok), st.queue.passed())
        };
        self.note_passed(passed);
        self.apply_step(step, false).await;
    }

    pub async fn prev(self: &Arc<Self>) {
        let pos = self.snapshot().position;
        let ok = self.avail();
        let (step, passed) = {
            let mut st = self.st.lock();
            (st.queue.prev_where(pos, &ok), st.queue.passed())
        };
        self.note_passed(passed);
        self.apply_step(step, false).await;
    }

    /// Seeking outside an A–B loop ends it (D26); inside, it stays.
    pub fn seek(&self, secs: f64) {
        if !secs.is_finite() {
            return;
        }
        let outside = self.snapshot_loop().is_some_and(|l| secs < l.a - 0.05 || secs > l.b + 0.05);
        if outside {
            self.clear_loop();
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

    /// Several songs at once (an album, a selection) pass the admission
    /// (D36); one song asked for by name always goes in.
    fn admit_many(&self, tracks: Vec<Track>) -> Vec<Track> {
        if tracks.len() < 2 { tracks } else { self.admit_list(tracks, None).0 }
    }

    pub fn play_next(self: &Arc<Self>, tracks: Vec<Track>) {
        let tracks = self.admit_many(tracks);
        let start = self.st.lock().queue.play_next(tracks);
        self.after_edit(start);
    }

    pub fn add_to_queue(self: &Arc<Self>, tracks: Vec<Track>) {
        let tracks = self.admit_many(tracks);
        let start = self.st.lock().queue.append(tracks, Origin::Manual);
        self.after_edit(start);
    }

    fn after_edit(self: &Arc<Self>, start: Option<usize>) {
        if start.is_some() {
            self.play_current(0.0);
        }
        self.sync_hold();
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
            st.radio_gen += 1;
        }
        self.after_edit(None);
    }

    /// Turning shuffle on while a collection plays shuffles in its cycle
    /// (D33): the songs not yet played in it come first.
    pub fn set_shuffle(self: &Arc<Self>, on: bool) {
        let run = if on { self.cycle_for_queue() } else { None };
        {
            let mut st = self.st.lock();
            match run {
                Some(run) => {
                    let played: HashSet<String> = run.c.played.iter().cloned().collect();
                    st.queue.set_shuffle_fresh(true, Some(&|t: &Track| !played.contains(t.id.remote())));
                    st.cycle = Some(run);
                }
                None => st.queue.set_shuffle(on),
            }
        }
        self.after_edit(None);
    }

    pub fn cycle_repeat(self: &Arc<Self>) -> Repeat {
        let r = {
            let mut st = self.st.lock();
            if let Some(l) = st.ab_loop.as_mut() {
                l.repeat = None; // the user's newer choice stands after the loop
            }
            st.queue.cycle_repeat()
        };
        self.after_edit(None);
        r
    }

    pub fn set_repeat(self: &Arc<Self>, r: Repeat) {
        {
            let mut st = self.st.lock();
            if let Some(l) = st.ab_loop.as_mut() {
                l.repeat = None;
            }
            st.queue.set_repeat(r);
        }
        self.after_edit(None);
    }

    /// Autoplay off means nothing joins the queue that the user didn't
    /// choose (D33): radio songs still to come leave it, and radio on its
    /// way is dropped when it arrives.
    pub fn set_autoplay(self: &Arc<Self>, on: bool) {
        {
            let mut st = self.st.lock();
            st.queue.set_autoplay(on);
            st.radio_empty = 0;
            st.radio_gen += 1;
            if !on {
                st.queue.retain_upcoming(|e| e.origin != Origin::Radio);
            }
        }
        self.check_preload();
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

/// A seed from the system's random source (v4 UUIDs come from it).
fn rand_seed() -> u64 {
    uuid::Uuid::new_v4().as_u64_pair().0
}
