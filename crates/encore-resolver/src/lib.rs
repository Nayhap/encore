//! Stream resolution (D3, D37): Encore's own YouTube extractor.
//!
//! * The InnerTube clients, requests and fallbacks are yt-dlp's, ported
//!   module for module ([`base`] is `_base.py`, [`video`] is `_video.py`)
//!   at the commit in `upstream.toml`, so an upstream fix maps onto a file.
//! * The `n` and signature challenges are solved by yt-dlp-ejs, vendored
//!   verbatim, on QuickJS-ng compiled into Encore ([`jsc`]). Nothing is
//!   downloaded or installed, and no other program runs.
//! * One client at a time, the next only when this one gave no
//!   D5-preferred AAC; a resolve runs as one frozen [`base::Identity`]
//!   (D2), and its URL is cached under it.
//! * Every resolved URL is probed (head + tail range) before it is handed
//!   to the engine (D4); a failing tail switches to sequential mode.
//! * Failures are kinds, never guessed from text (D4): a bot check is
//!   `Challenged`, a restriction `Unavailable`, a 401 with the session
//!   `SessionExpired`, a player Encore can't read `Unknown` with a note
//!   that Encore needs an update.

pub mod base;
pub mod cleanup;
pub mod http;
pub mod jsc;
pub mod video;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use encore_core::redact::Secret;
use encore_core::service::{PlayFailure, PlayFailureKind, ResolveAuth, ResolvedStream, StreamResolver};
use parking_lot::Mutex;
use serde::Serialize;

use base::Identity;
use jsc::quickjs::{Limits, QuickJsSolver};
use jsc::{ChallengeRequest, ChallengeType, SolveError, Solver};
use video::formats::{self, Candidate, Skipped};
use video::playability::{self, Challenge, Playability, Restriction};
use video::player::{self, CallError};
use video::player_js::{Player, PlayerJs, PlayerJsError};
use video::webpage;

/// yt-dlp's version and commit the extractor was ported from.
pub const YT_DLP_VERSION: &str = "2026.10.26";
pub const YT_DLP_COMMIT: &str = "b77c930";
pub use jsc::ejs::EJS_VERSION;
pub use jsc::quickjs::ENGINE;

/// A preroll wait Encore accepts without looking at other clients.
const SHORT_WAIT: u64 = 15;
/// The longest one resolve may take before the preroll (a cold solve is
/// up to 30 s on a slow machine).
const RESOLVE_DEADLINE: Duration = Duration::from_secs(90);

/// What a song that needs a newer Encore says (D4: `Unknown`).
pub const NEEDS_UPDATE: &str = "YouTube changed how its songs play, and Encore needs an update to follow. Check for updates in Settings.";
/// The same, as Settings → Playback shows it (Updates is further down).
pub const NEEDS_UPDATE_NOTICE: &str = "YouTube changed how its songs play, and this version of Encore can't follow yet. Check for a newer one under Updates, below.";
/// A solve that didn't finish (out of time or memory): worth another try.
pub const SOLVER_DIDNT_FINISH: &str = "Encore couldn't finish preparing this song. Try again.";

#[derive(Clone, Debug)]
pub struct ResolverConfig {
    /// `<app data>`: only for the one-time cleanup of D22's components.
    pub data_dir: PathBuf,
    /// `<cache>`: the player scripts (`player/`, no secrets).
    pub cache_dir: PathBuf,
}

impl ResolverConfig {
    pub fn default_paths() -> Self {
        let data = dirs::data_local_dir().unwrap_or_else(std::env::temp_dir).join("app.encore.desktop");
        let cache = dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("app.encore.desktop");
        ResolverConfig { data_dir: data, cache_dir: cache }
    }
}

/// Knobs for `encore-cli resolve`.
#[derive(Clone, Debug, Default)]
pub struct ResolveOptions {
    /// Ask only this client (a key of `INNERTUBE_CLIENTS`).
    pub client: Option<String>,
}

/// What a resolve did, for `encore-cli resolve --diagnostics` and the log.
/// Never a URL, a cookie or a challenge value.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostics {
    pub signed_in: bool,
    pub attempts: Vec<Attempt>,
    pub player: Option<String>,
    pub solves: Vec<SolveTime>,
    pub chosen: Option<String>,
    pub waited_secs: u64,
    pub probes: u32,
    pub cached: bool,
    /// Signed out: the session in use (cookie names and whether a visitor
    /// came with it, never values).
    pub session: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attempt {
    pub client: String,
    pub outcome: String,
    pub status: Option<String>,
    /// YouTube's reason and subreason, shortened.
    pub reason: Option<String>,
    pub itag: Option<u32>,
    pub formats: usize,
    pub skipped: Skipped,
    pub ms: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SolveTime {
    pub ms: u64,
    pub preprocessed: bool,
    pub n: bool,
    pub sig: bool,
}

/// The built-in engine, for Settings → Playback (D37).
#[derive(Clone, Debug, Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    /// "yt-dlp-ejs 0.8.0"
    pub solver: String,
    /// "quickjs-ng 0.16.2"
    pub engine: String,
    /// "yt-dlp 2026.10.26 (b77c930)"
    pub extractor: String,
    /// The player script in use (`9fcf08e8/main`), once one was fetched.
    pub player: Option<String>,
    /// Set while the last resolve failed for a reason only an update fixes.
    pub notice: Option<String>,
}

/// Why one client gave nothing, before it becomes a D4 kind.
#[derive(Clone, Debug, PartialEq)]
enum Cause {
    Network(String),
    Challenged(Challenge),
    /// YouTube answered for another video ("your IP is likely being blocked").
    OtherVideo,
    Unauthorized,
    Restricted(Restriction),
    Drm,
    AgeGated,
    /// `sign_in`: the reason asked to sign in, in so many words.
    LoginRequired { sign_in: bool },
    ClientRefused,
    /// Formats only over SABR.
    Sabr,
    /// Only formats that need a GVS PO token (Encore has no source).
    PoTokenRequired,
    NoAudio,
    /// The solver couldn't read the player or gave a wrong answer: only an
    /// update fixes that.
    Solver(String),
    /// The solver ran out of time or memory, or its thread failed: another
    /// try may work.
    SolverTransient(String),
    PlayerJs(String),
    Refused(u16),
    Unreadable,
    Closed,
}

impl Cause {
    fn label(&self) -> String {
        match self {
            Cause::Network(m) => format!("network: {m}"),
            Cause::Challenged(c) => format!("challenged: {c:?}"),
            Cause::OtherVideo => "answered for another video".into(),
            Cause::Unauthorized => "401 with the session".into(),
            Cause::Restricted(r) => format!("restricted: {r:?}"),
            Cause::Drm => "DRM only".into(),
            Cause::AgeGated => "age-gated".into(),
            Cause::LoginRequired { sign_in } => if *sign_in { "sign-in required".into() } else { "login required".into() },
            Cause::ClientRefused => "unplayable on this client".into(),
            Cause::Sabr => "SABR only".into(),
            Cause::PoTokenRequired => "skipped: needs a PO token".into(),
            Cause::NoAudio => "no audio formats".into(),
            Cause::Solver(m) => format!("solver: {m}"),
            Cause::SolverTransient(m) => format!("solver didn't finish: {m}"),
            Cause::PlayerJs(m) => format!("player JS: {m}"),
            Cause::Refused(s) => format!("refused ({s})"),
            Cause::Unreadable => "unreadable answer".into(),
            Cause::Closed => "closing".into(),
        }
    }

    /// Stop asking other clients: asking again is how an account gets
    /// flagged (D23), the connection is gone, or Encore is closing. A
    /// restriction is not final: one client's refusal can be another's
    /// stream (yt-dlp asks them all).
    fn final_now(&self) -> bool {
        matches!(self, Cause::Network(_) | Cause::Challenged(_) | Cause::Unauthorized | Cause::Closed)
    }

    /// Evidence that wins over a stream another client gave: a bot check
    /// or a refused session must reach the caller (D23's breaker, sign-in).
    fn overrides_streams(&self) -> bool {
        matches!(self, Cause::Challenged(_) | Cause::Unauthorized)
    }
}

fn fail(kind: PlayFailureKind, m: impl Into<String>) -> PlayFailure {
    PlayFailure::new(kind, m)
}

/// The D4 kind and sentence for what the clients said, most telling first:
/// a bot check or a refused session; then a route that offered audio
/// Encore couldn't open (only an update fixes that, so it must not hide
/// behind another client's refusal); then the content's own restrictions;
/// then the connection and the rest.
fn failure(causes: &[Cause], signed_in: bool) -> PlayFailure {
    use PlayFailureKind as K;
    let has = |f: &dyn Fn(&Cause) -> bool| causes.iter().any(f);
    if let Some(Cause::Challenged(c)) = causes.iter().find(|c| matches!(c, Cause::Challenged(_))) {
        return fail(
            K::Challenged,
            match (c, signed_in) {
                (Challenge::RateLimit, _) => "YouTube is limiting how many songs Encore can start. Wait a few minutes and try again.",
                (_, true) => "YouTube is asking Encore to prove it isn't a bot. Wait a few minutes and try again.",
                (_, false) => "YouTube is asking Encore to prove it isn't a bot. Wait a few minutes and try again, or sign in from Settings.",
            },
        );
    }
    if has(&|c| *c == Cause::Unauthorized) {
        return fail(K::SessionExpired, "Your YouTube Music session expired. Sign in again from Settings.");
    }
    if has(&|c| matches!(c, Cause::Solver(_) | Cause::PlayerJs(_) | Cause::Sabr | Cause::Unreadable)) {
        return fail(K::Unknown, NEEDS_UPDATE);
    }
    if has(&|c| matches!(c, Cause::SolverTransient(_))) {
        return fail(K::Unknown, SOLVER_DIDNT_FINISH);
    }
    if let Some(Cause::Restricted(r)) = causes.iter().find(|c| matches!(c, Cause::Restricted(_))) {
        return fail(
            K::Unavailable,
            match r {
                Restriction::Geo => "This song isn't available in your country.",
                Restriction::Private => "This song is private on YouTube.",
                Restriction::Removed => "This song is no longer available on YouTube.",
                Restriction::Premium | Restriction::Members => "This song needs a YouTube Music Premium or channel membership.",
                Restriction::Live => "This is a live stream or a premiere, which Encore can't play.",
            },
        );
    }
    if has(&|c| *c == Cause::AgeGated) {
        return fail(K::Unavailable, if signed_in { "YouTube won't play this age-restricted song for this account. It may want the account to verify its age." } else { "This song is age-restricted. Sign in from Settings to play it." });
    }
    if has(&|c| *c == Cause::Drm) {
        return fail(K::Unavailable, "This song is protected (DRM) and Encore can't play it.");
    }
    if has(&|c| matches!(c, Cause::Network(_))) {
        return fail(K::Network, "Couldn't reach YouTube. Check your internet connection.");
    }
    // Only an explicit "sign in" while signed in can mean the session went;
    // anything else is about the song (D4).
    if has(&|c| matches!(c, Cause::LoginRequired { sign_in: true })) && signed_in {
        return fail(K::SessionExpired, "Your YouTube Music session expired. Sign in again from Settings.");
    }
    if has(&|c| matches!(c, Cause::LoginRequired { .. })) {
        return fail(K::Unavailable, if signed_in { "YouTube won't play this song for this account." } else { "YouTube wants you signed in to play this song. Sign in from Settings." });
    }
    if has(&|c| *c == Cause::OtherVideo) && causes.iter().all(|c| matches!(c, Cause::OtherVideo | Cause::PoTokenRequired)) {
        return fail(K::Challenged, "YouTube is blocking this connection. Wait a few minutes and try again.");
    }
    if has(&|c| *c == Cause::Closed) {
        return fail(K::Unknown, "Encore is closing.");
    }
    if has(&|c| *c == Cause::PoTokenRequired) && causes.iter().all(|c| *c == Cause::PoTokenRequired) {
        return fail(K::Unknown, NEEDS_UPDATE);
    }
    if has(&|c| *c == Cause::NoAudio || *c == Cause::ClientRefused) {
        return fail(K::Unavailable, "YouTube offered no audio for this song.");
    }
    fail(K::Unknown, "YouTube refused to play this song. Try again later.")
}

/// The search through clients: the best stream so far and every refusal.
#[derive(Default)]
struct Search {
    causes: Vec<Cause>,
    best: Option<Ready>,
}

impl Search {
    /// A client gave a stream. Returns whether to stop asking.
    fn found(&mut self, r: Ready) -> bool {
        let done = r.acceptable && r.wait <= SHORT_WAIT;
        let key = |r: &Ready| (r.wait > SHORT_WAIT, r.rank);
        if self.best.as_ref().is_none_or(|b| key(&r) < key(b)) {
            self.best = Some(r);
        }
        done
    }

    /// A client gave nothing. Returns whether to stop asking.
    fn refused(&mut self, c: Cause) -> bool {
        let stop = c.final_now();
        self.causes.push(c);
        stop
    }

    /// The stream to play, unless a bot check or a refused session came
    /// up: then that is the answer, whatever another client gave (D23).
    fn finish(self, signed_in: bool) -> Result<Ready, PlayFailure> {
        match self.best {
            Some(b) if !self.causes.iter().any(Cause::overrides_streams) => Ok(b),
            _ => Err(failure(&self.causes, signed_in)),
        }
    }
}

/// A URL ready to probe, with the client it came from.
struct Ready {
    stream: ResolvedStream,
    rank: (bool, i32, usize, i64),
    /// D5's AAC in the right track: no need to ask further.
    acceptable: bool,
    /// Seconds from the answer until the URL works (the preroll).
    wait: u64,
    client: String,
}

/// Why a probe failed: refused (403/410) or anything else.
enum Probe {
    Refused(u16),
    Failed(PlayFailure),
}

pub(crate) fn fxhash(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

#[derive(Clone)]
struct Cached {
    stream: ResolvedStream,
    until: Instant,
}

/// Redirects only to YouTube's own https hosts (the media and the API).
fn redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|a| {
        let ok = a.url().scheme() == "https" && a.url().host_str().is_some_and(|h| h.ends_with(".googlevideo.com") || h == "www.youtube.com" || h == "music.youtube.com");
        if ok && a.previous().len() < 5 { a.follow() } else { a.stop() }
    })
}

pub type VisitorSource = Arc<dyn Fn() -> Option<String> + Send + Sync>;

pub struct NativeResolver {
    http: reqwest::Client,
    player_js: PlayerJs,
    solver: Arc<QuickJsSolver>,
    solved: jsc::cache::SolvedCache,
    cache: Mutex<HashMap<String, Cached>>,
    inflight: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// The signed-out session (watch page cookies and visitor, memory only).
    anon: tokio::sync::Mutex<Option<webpage::AnonSession>>,
    /// Where a signed-out resolve gets its visitor when the watch page gave
    /// none: encore-ytm's (D37).
    visitor_source: Mutex<Option<VisitorSource>>,
    notice: Mutex<Option<String>>,
    /// Counts the signed-out sessions (part of the cache key).
    anon_generation: std::sync::atomic::AtomicU64,
    /// Encore is quitting or installing an update (D24).
    closed: AtomicBool,
    /// Wakes a resolve waiting out a preroll when Encore quits.
    closing: tokio::sync::Notify,
}

impl NativeResolver {
    pub fn new(cfg: ResolverConfig) -> Arc<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .gzip(true)
            .brotli(true)
            .redirect(redirect_policy())
            .build()
            .expect("http");
        Arc::new(Self {
            player_js: PlayerJs::new(http.clone(), cfg.cache_dir.clone()),
            http,
            solver: QuickJsSolver::new(Limits::default()),
            solved: Default::default(),
            cache: Mutex::new(HashMap::new()),
            inflight: Mutex::new(HashMap::new()),
            anon: tokio::sync::Mutex::new(None),
            visitor_source: Mutex::new(None),
            notice: Mutex::new(None),
            anon_generation: std::sync::atomic::AtomicU64::new(0),
            closed: AtomicBool::new(false),
            closing: tokio::sync::Notify::new(),
        })
    }

    /// Signed-out resolves take their visitor from here when the watch page
    /// gave none.
    pub fn set_visitor_source(&self, f: VisitorSource) {
        *self.visitor_source.lock() = Some(f);
    }

    pub fn solver(&self) -> &Arc<QuickJsSolver> {
        &self.solver
    }

    /// Before Encore quits or installs an update (D24): no resolve starts
    /// and a running solve is interrupted.
    pub async fn shut_down(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.closing.notify_waiters();
        self.solver.close();
    }

    fn closing_error() -> PlayFailure {
        fail(PlayFailureKind::Unknown, "Encore is closing.")
    }

    /// Sleep until `at`, unless Encore quits first.
    async fn wait_until(&self, at: Instant) -> Result<(), PlayFailure> {
        let woken = self.closing.notified();
        tokio::pin!(woken);
        woken.as_mut().enable();
        if self.closed.load(Ordering::SeqCst) {
            return Err(Self::closing_error());
        }
        tokio::select! {
            _ = tokio::time::sleep_until(at.into()) => Ok(()),
            _ = woken => Err(Self::closing_error()),
        }
    }

    /// Quitting didn't happen after all.
    pub fn reopen(&self) {
        self.closed.store(false, Ordering::SeqCst);
        self.solver.reopen();
    }

    pub fn clear_cache(&self) {
        self.cache.lock().clear();
    }

    /// Settings → Reset player cache: the URLs, the player scripts (memory
    /// and disk), the preprocessed players and the solved challenges.
    pub fn reset_player_cache(&self) {
        self.clear_cache();
        self.player_js.forget(true);
        self.solver.clear();
        self.solved.clear();
        *self.notice.lock() = None;
    }

    pub fn engine_status(&self) -> EngineStatus {
        EngineStatus {
            solver: format!("yt-dlp-ejs {EJS_VERSION}"),
            engine: ENGINE.into(),
            extractor: format!("yt-dlp {YT_DLP_VERSION} ({YT_DLP_COMMIT})"),
            player: self.player_js.current_id(),
            notice: self.notice.lock().clone(),
        }
    }

    /// Resolve with options, returning what happened along the way.
    pub async fn resolve_with(&self, video_id: &str, auth: Option<&ResolveAuth>, opts: &ResolveOptions) -> (Result<ResolvedStream, PlayFailure>, Diagnostics) {
        let mut diag = Diagnostics::default();
        let r = self.resolve_inner(video_id, auth, opts, &mut diag).await;
        (r, diag)
    }

    fn cache_key(video_id: &str, id: &Identity, opts: &ResolveOptions) -> String {
        format!("{video_id}|{}|{}", id.key, opts.client.as_deref().unwrap_or(""))
    }

    async fn resolve_inner(&self, video_id: &str, auth: Option<&ResolveAuth>, opts: &ResolveOptions, diag: &mut Diagnostics) -> Result<ResolvedStream, PlayFailure> {
        if !(video_id.len() == 11 && video_id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')) {
            return Err(fail(PlayFailureKind::Unavailable, "This isn't a YouTube song Encore can play."));
        }
        if self.closed.load(Ordering::SeqCst) {
            return Err(Self::closing_error());
        }
        // The identity is frozen here, for every request of this resolve.
        let mut id = Identity::from_auth(auth, None);
        diag.signed_in = id.is_authenticated();
        if !id.is_authenticated() {
            match self.anonymous(&mut id, video_id).await {
                Ok(d) => diag.session = Some(d),
                Err(f) => return Err(f),
            }
        }
        let key = Self::cache_key(video_id, &id, opts);
        let lock = self.inflight.lock().entry(key.clone()).or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))).clone();
        let _g = lock.lock().await;
        if let Some(c) = self.cache.lock().get(&key)
            && c.until > Instant::now()
        {
            diag.cached = true;
            return Ok(c.stream.clone());
        }
        let r = match tokio::time::timeout(RESOLVE_DEADLINE, self.resolve_fresh(video_id, &mut id, opts, diag)).await {
            Ok(r) => r,
            Err(_) => Err(fail(PlayFailureKind::Network, "Getting the song from YouTube took too long. Check your internet connection and try again.")),
        };
        if let Ok(s) = &r {
            self.cache_stream(&key, s);
            *self.notice.lock() = None;
        }
        self.inflight.lock().remove(&key);
        r
    }

    fn cache_stream(&self, key: &str, s: &ResolvedStream) {
        let now = encore_core::store::now();
        let ttl = s.expires_at.map(|e| (e - now - 300).clamp(0, 3600)).unwrap_or(1800) as u64;
        self.cache.lock().insert(key.to_string(), Cached { stream: s.clone(), until: Instant::now() + Duration::from_secs(ttl) });
    }

    /// Extract, then probe; a URL that must wait for its preroll comes
    /// back unprobed, with `available_at`, for `ready` (no lock is held
    /// while it waits).
    async fn resolve_fresh(&self, video_id: &str, id: &mut Identity, opts: &ResolveOptions, diag: &mut Diagnostics) -> Result<ResolvedStream, PlayFailure> {
        let mut last = None;
        for _ in 0..2 {
            let ready = self.extract(video_id, id, opts, diag).await?;
            let mut s = ready.stream;
            if s.wait_secs().is_some() {
                tracing::info!(video = video_id, secs = ready.wait, client = %ready.client, "the URL answers after a preroll");
                diag.waited_secs += ready.wait;
                return Ok(s);
            }
            if self.closed.load(Ordering::SeqCst) {
                return Err(Self::closing_error());
            }
            diag.probes += 1;
            match self.probe_checked(&s).await {
                Ok((len, range_ok)) => {
                    if len > 0 {
                        s.content_length = Some(len);
                    }
                    s.range_safe = range_ok;
                    s.available_at = None;
                    return Ok(s);
                }
                // Resolve once more for a fresh URL; refused twice, it is a
                // failure of this song, not a bot check (D4).
                Err(Probe::Refused(st)) => {
                    tracing::info!(video = video_id, status = st, client = %ready.client, "the media server refused a fresh URL");
                    self.forget(video_id);
                    last = Some(fail(PlayFailureKind::Unknown, "YouTube's media server refused this song. Try again later."));
                }
                Err(Probe::Failed(e)) => return Err(e),
            }
        }
        Err(last.unwrap_or_else(|| fail(PlayFailureKind::Unknown, "could not resolve the stream")))
    }

    /// Ask the clients in turn for a playable URL.
    async fn extract(&self, video_id: &str, id: &mut Identity, opts: &ResolveOptions, diag: &mut Diagnostics) -> Result<Ready, PlayFailure> {
        let mut plan = video::plan::Plan::new(id.is_authenticated(), opts.client.as_deref());
        let mut search = Search::default();
        while let Some(key) = plan.pop_next() {
            let t0 = Instant::now();
            let Some(c) = base::client(&key) else {
                search.refused(Cause::Refused(0));
                continue;
            };
            let mut attempt = Attempt { client: key.clone(), outcome: String::new(), status: None, reason: None, itag: None, formats: 0, skipped: Skipped::default(), ms: 0 };
            let r = self.ask(video_id, c, id, &mut plan, &mut attempt, diag).await;
            attempt.ms = t0.elapsed().as_millis() as u64;
            let stop = match r {
                Ok(ready) => {
                    attempt.outcome = if ready.wait > SHORT_WAIT { format!("itag {} after a {} s preroll", ready.stream.itag, ready.wait) } else { format!("itag {}", ready.stream.itag) };
                    search.found(ready)
                }
                Err(cause) => {
                    attempt.outcome = cause.label();
                    tracing::info!(video = video_id, client = %key, cause = %attempt.outcome, "a client gave no stream");
                    search.refused(cause)
                }
            };
            diag.attempts.push(attempt);
            if stop {
                break;
            }
        }
        let signed_in = id.is_authenticated();
        match search.finish(signed_in) {
            Ok(r) => {
                diag.chosen = Some(format!("{} itag {} {}", r.client, r.stream.itag, r.stream.codec));
                Ok(r)
            }
            Err(f) => {
                // A bot check signed out: the next resolve starts a new session.
                if f.kind == PlayFailureKind::Challenged && !signed_in {
                    *self.anon.lock().await = None;
                }
                if f.message == NEEDS_UPDATE {
                    *self.notice.lock() = Some(NEEDS_UPDATE_NOTICE.into());
                }
                Err(f)
            }
        }
    }

    /// Signed out: the session the watch page starts (`video::webpage`),
    /// kept for a while; encore-ytm's visitor if the page gave none. A rate
    /// limit on the page ends the resolve as a bot check (D23).
    async fn anonymous(&self, id: &mut Identity, video_id: &str) -> Result<String, PlayFailure> {
        let mut g = self.anon.lock().await;
        if !g.as_ref().is_some_and(|s| s.fresh()) {
            let generation = self.anon_generation.fetch_add(1, Ordering::SeqCst) + 1;
            *g = match webpage::load(&self.http, video_id, generation).await {
                Ok(s) => s,
                Err(_) => {
                    tracing::info!("the watch page answered with a rate limit");
                    return Err(failure(&[Cause::Challenged(Challenge::RateLimit)], false));
                }
            };
            tracing::info!(ok = g.is_some(), visitor = g.as_ref().is_some_and(|s| s.visitor.is_some()), "signed-out session from the watch page");
        }
        id.key = format!("anonymous:{}", g.as_ref().map(|s| s.generation).unwrap_or(0));
        id.anon_jar = Some(g.as_ref().map(|s| s.jar.clone()).unwrap_or_else(webpage::consent_jar));
        if let Some(s) = g.as_ref() {
            id.visitor_data = s.visitor.clone();
        }
        let from_page = id.visitor_data.is_some();
        if id.visitor_data.is_none() {
            id.visitor_data = self.visitor_source.lock().as_ref().and_then(|f| f());
        }
        let names: Vec<&str> = g.as_ref().map(|s| s.jar.records().iter().map(|c| c.name.as_str()).collect()).unwrap_or_default();
        Ok(format!("cookies [{}], visitor {}", names.join(", "), if from_page { "from the watch page" } else if id.visitor_data.is_some() { "from encore-ytm" } else { "none" }))
    }

    async fn player(&self, diag: &mut Diagnostics) -> Result<Player, Cause> {
        match self.player_js.get().await {
            Ok(p) => {
                diag.player = Some(p.script.id.clone());
                Ok(p)
            }
            Err(PlayerJsError::Network(m)) => Err(Cause::Network(m)),
            Err(PlayerJsError::Missing(m)) => Err(Cause::PlayerJs(m)),
            Err(PlayerJsError::RateLimited) => Err(Cause::Challenged(Challenge::RateLimit)),
        }
    }

    /// One client: its `/player` answer, its best audio format, solved.
    async fn ask(&self, video_id: &str, c: &base::InnertubeClient, id: &mut Identity, plan: &mut video::plan::Plan, attempt: &mut Attempt, diag: &mut Diagnostics) -> Result<Ready, Cause> {
        use base::clients::{StreamingProtocol, gvs_pot_required};
        // Encore has no PO token: a client that needs one isn't asked.
        if gvs_pot_required(c.gvs_policy(StreamingProtocol::Https), false, false) {
            return Err(Cause::PoTokenRequired);
        }
        if c.require_auth && !id.is_authenticated() {
            return Err(Cause::LoginRequired { sign_in: false });
        }
        let player = if c.require_js_player { Some(self.player(diag).await?) } else { None };
        // `_download_ytcfg`, for the clients that need their page's config.
        let ytcfg = base::ytcfg::download(&self.http, c, id, video_id).await.map_err(|_| Cause::Challenged(Challenge::RateLimit))?;
        if id.visitor_data.is_none() {
            id.visitor_data = ytcfg.as_ref().and_then(|y| y["VISITOR_DATA"].as_str()).map(str::to_string);
        }
        let pr = match player::call(&self.http, c, id, video_id, player.as_ref().and_then(|p| p.sts), ytcfg.as_ref()).await {
            Ok(v) => v,
            Err(CallError::Network(m)) => return Err(Cause::Network(m)),
            Err(CallError::Unauthorized) => return Err(Cause::Unauthorized),
            Err(CallError::RateLimited) => return Err(Cause::Challenged(Challenge::RateLimit)),
            Err(CallError::Refused(s)) => return Err(Cause::Refused(s)),
            Err(CallError::Unreadable) => return Err(Cause::Unreadable),
        };
        if self.closed.load(Ordering::SeqCst) {
            return Err(Cause::Closed);
        }
        attempt.status = Some(playability::status(&pr).to_string());
        let (reason, sub) = playability::reasons(&pr);
        let reason = [reason, sub.unwrap_or_default()].iter().filter(|r| !r.is_empty()).cloned().collect::<Vec<_>>().join(". ");
        if !reason.is_empty() {
            attempt.reason = Some(encore_core::redact::redact(&reason).chars().take(160).collect());
        }
        if playability::other_video(&pr, video_id).is_some() {
            return Err(Cause::OtherVideo);
        }
        let p = playability::classify(&pr);
        plan.after(&c.key, p == Playability::AgeGated, matches!(p, Playability::ClientRefused | Playability::Restricted(Restriction::Removed)));
        match p {
            Playability::Ok => {}
            Playability::AgeGated => return Err(Cause::AgeGated),
            Playability::Challenged(ch) => return Err(Cause::Challenged(ch)),
            // An ERROR from visionOS may be "made for kids": another client
            // may play it, so it is no reason to stop (`_video.py:3144`).
            // Embedded errors ("Error code: 152") are the embed's, too.
            Playability::Restricted(Restriction::Removed) if c.key == "visionos" || c.is_embedded() => return Err(Cause::ClientRefused),
            Playability::Restricted(r) => return Err(Cause::Restricted(r)),
            Playability::LoginRequired { sign_in } => return Err(Cause::LoginRequired { sign_in }),
            Playability::ClientRefused => return Err(Cause::ClientRefused),
        }
        let (cands, skipped) = formats::candidates(&pr, c, false);
        attempt.formats = cands.len();
        attempt.skipped = skipped;
        let Some(cand) = formats::pick(&cands) else {
            return Err(if skipped.sabr > 0 {
                Cause::Sabr
            } else if skipped.po_token > 0 {
                Cause::PoTokenRequired
            } else if skipped.drm > 0 || pr["streamingData"]["licenseInfos"].is_array() {
                Cause::Drm
            } else {
                Cause::NoAudio
            });
        };
        attempt.itag = Some(cand.itag);
        let (sig, n) = if cand.s.is_some() || cand.n.is_some() {
            let player = match player {
                Some(p) => p,
                None => self.player(diag).await?,
            };
            self.solve(cand, &player, diag).await?
        } else {
            (None, None)
        };
        let url = formats::build_url(cand, sig.as_deref(), n.as_deref()).ok_or_else(|| Cause::Solver("the stream URL isn't a YouTube media address".into()))?;
        let loudness = pr["playerConfig"]["audioConfig"]["loudnessDb"].as_f64().map(|l| l as f32);
        let mut headers = Vec::new();
        if let Some(ua) = c.user_agent() {
            headers.push(("User-Agent".to_string(), ua.to_string()));
        }
        // `_get_available_at_timestamp`: counted from this answer.
        let wait = formats::preroll_seconds(&pr);
        Ok(Ready {
            rank: cand.rank(),
            acceptable: cand.acceptable(),
            wait,
            client: c.key.clone(),
            stream: ResolvedStream {
                video_id: video_id.into(),
                expires_at: formats::expires(&url),
                url: Secret::new(url),
                headers,
                itag: cand.itag,
                codec: cand.codec.clone(),
                mime: cand.mime.clone(),
                bitrate_kbps: cand.bitrate,
                content_length: cand.content_length,
                duration: cand.duration,
                loudness_db: loudness,
                range_safe: true,
                authenticated: id.is_authenticated() && c.supports_cookies,
                available_at: (wait > 0).then(|| Instant::now() + Duration::from_secs(wait) + Duration::from_millis(250)),
            },
        })
    }

    /// The candidate's signature and `n`, from the cache or the solver.
    async fn solve(&self, cand: &Candidate, player: &Player, diag: &mut Diagnostics) -> Result<(Option<String>, Option<String>), Cause> {
        let key = jsc::cache::key(&player.script.id, &player.script.code);
        let mut n = cand.n.as_ref().and_then(|c| self.solved.n(&key, c));
        let slen = cand.s.as_ref().map(|s| s.chars().count());
        let mut perm = slen.and_then(|l| self.solved.sig(&key, l));
        let mut reqs = Vec::new();
        if let (Some(c), None) = (&cand.n, &n) {
            reqs.push(ChallengeRequest { kind: ChallengeType::N, challenges: vec![c.clone()] });
        }
        if let (Some(l), None) = (slen, &perm) {
            reqs.push(ChallengeRequest { kind: ChallengeType::Sig, challenges: vec![jsc::sig_spec_challenge(l)] });
        }
        if !reqs.is_empty() {
            let t = Instant::now();
            let out = self.solver.solve(&player.script, reqs.clone()).await.map_err(|e| match e {
                SolveError::Closed => Cause::Closed,
                SolveError::Engine(m) => Cause::SolverTransient(m),
                // ejs can't read this player: a newer one may be out, so the
                // next resolve asks the iframe API again.
                SolveError::Player(m) => {
                    self.player_js.forget(false);
                    Cause::Solver(m)
                }
            })?;
            diag.solves.push(SolveTime { ms: t.elapsed().as_millis() as u64, preprocessed: self.solver.last_timing().preprocessed_hit, n: n.is_none() && cand.n.is_some(), sig: perm.is_none() && slen.is_some() });
            for (req, res) in reqs.iter().zip(out) {
                let data = res.map_err(Cause::Solver)?;
                let challenge = &req.challenges[0];
                let answer = data.get(challenge).ok_or_else(|| Cause::Solver("no answer".into()))?;
                match req.kind {
                    ChallengeType::N => {
                        self.solved.put_n(&key, challenge, answer);
                        n = Some(answer.clone());
                    }
                    ChallengeType::Sig => {
                        let p = jsc::sig_permutation(answer);
                        let len = challenge.chars().count();
                        if p.iter().any(|&i| i >= len) {
                            return Err(Cause::Solver("the signature answer doesn't fit".into()));
                        }
                        self.solved.put_sig(&key, len, p.clone());
                        perm = Some(p);
                    }
                }
            }
        }
        let sig = match (&cand.s, &perm) {
            (Some(s), Some(p)) => Some(jsc::apply_permutation(p, s).ok_or_else(|| Cause::Solver("the signature answer doesn't fit".into()))?),
            _ => None,
        };
        Ok((sig, n))
    }

    /// Head + tail range probe (D4). Returns the verified content length and
    /// whether deep ranges work.
    pub async fn probe(&self, s: &ResolvedStream) -> Result<(u64, bool), PlayFailure> {
        self.probe_checked(s).await.map_err(|p| match p {
            Probe::Refused(st) => fail(PlayFailureKind::Unknown, format!("the media server refused the stream ({st})")),
            Probe::Failed(f) => f,
        })
    }

    async fn probe_checked(&self, s: &ResolvedStream) -> Result<(u64, bool), Probe> {
        let get = |range: String| {
            let mut rb = self.http.get(s.url.expose()).header("Range", range);
            for (k, v) in &s.headers {
                rb = rb.header(k.as_str(), v.as_str());
            }
            rb.send()
        };
        let mut r = get("bytes=0-1023".into()).await.map_err(|e| Probe::Failed(fail(PlayFailureKind::Network, if e.is_timeout() { "YouTube's media server didn't answer in time. Check your internet connection." } else { "Couldn't reach YouTube's media server. Check your internet connection." })))?;
        let st = r.status().as_u16();
        // Usually a URL that went stale or belongs to another client: the
        // caller resolves once more (D23), it isn't a bot check.
        if st == 403 || st == 410 {
            return Err(Probe::Refused(st));
        }
        if st != 206 && st != 200 {
            return Err(Probe::Failed(fail(PlayFailureKind::Network, format!("the media server answered {st}"))));
        }
        let total = r.headers().get("content-range").and_then(|v| v.to_str().ok()).and_then(|v| v.rsplit('/').next()).and_then(|t| t.parse::<u64>().ok()).or(s.content_length);
        // A server that ignores the range answers 200 with the whole file:
        // read only the start of it.
        let mut head: Vec<u8> = Vec::new();
        while head.len() < 1024 {
            match r.chunk().await {
                Ok(Some(c)) => head.extend_from_slice(&c),
                Ok(None) => break,
                Err(_) => return Err(Probe::Failed(fail(PlayFailureKind::Network, "the media download was interrupted"))),
            }
        }
        drop(r);
        // Check the bytes look like media (MP4 ftyp / WebM EBML).
        let looks = head.len() >= 8 && (&head[4..8] == b"ftyp" || head[..4] == [0x1a, 0x45, 0xdf, 0xa3]);
        if !looks {
            return Err(Probe::Failed(fail(PlayFailureKind::Unknown, "the media server sent something that is not audio")));
        }
        let Some(total) = total else { return Ok((0, false)) };
        if total < 4096 {
            return Ok((total, true));
        }
        let tail = get(format!("bytes={}-{}", total - 1024, total - 1)).await;
        let ok = matches!(&tail, Ok(r) if r.status().as_u16() == 206);
        Ok((total, ok))
    }
}

#[async_trait]
impl StreamResolver for NativeResolver {
    async fn resolve(&self, video_id: &str, auth: Option<&ResolveAuth>) -> Result<ResolvedStream, PlayFailure> {
        let (r, d) = self.resolve_with(video_id, auth, &ResolveOptions::default()).await;
        if !d.cached {
            let clients: Vec<String> = d.attempts.iter().map(|a| format!("{}: {}", a.client, a.outcome)).collect();
            tracing::info!(video = video_id, signed_in = d.signed_in, ?clients, solves = ?d.solves.iter().map(|s| s.ms).collect::<Vec<_>>(), waited = d.waited_secs, "resolve");
        }
        r
    }

    fn forget(&self, video_id: &str) {
        self.cache.lock().retain(|k, _| !k.starts_with(&format!("{video_id}|")));
    }

    /// After the preroll: probe (D4), and resolve once more if the media
    /// server refuses the URL.
    async fn ready(&self, s: ResolvedStream, auth: Option<&ResolveAuth>) -> Result<ResolvedStream, PlayFailure> {
        let mut s = s;
        for round in 0..2 {
            let Some(at) = s.available_at else { return Ok(s) };
            self.wait_until(at).await?;
            match self.probe_checked(&s).await {
                Ok((len, range_ok)) => {
                    if len > 0 {
                        s.content_length = Some(len);
                    }
                    s.range_safe = range_ok;
                    s.available_at = None;
                    // The cached copy is ready too now.
                    let url = s.url.expose().clone();
                    for c in self.cache.lock().values_mut() {
                        if c.stream.url.expose() == &url {
                            c.stream = s.clone();
                        }
                    }
                    return Ok(s);
                }
                Err(Probe::Refused(st)) if round == 0 => {
                    tracing::info!(video = %s.video_id, status = st, "the media server refused the URL after its preroll");
                    self.forget(&s.video_id);
                    s = self.resolve(&s.video_id, auth).await?;
                }
                Err(Probe::Refused(_)) => break,
                Err(Probe::Failed(e)) => return Err(e),
            }
        }
        Err(fail(PlayFailureKind::Unknown, "YouTube's media server refused this song. Try again later."))
    }
}

/// Run the embedded solver on the synthetic player (no network): the
/// self-test's hard check that the engine and the bundles work here.
pub async fn solver_self_check() -> Result<u64, String> {
    if !jsc::ejs::bundles_verified() {
        return Err("the solver bundles don't match their pinned hashes".into());
    }
    let s = QuickJsSolver::new(Limits::default());
    let t = Instant::now();
    let script = jsc::PlayerScript { id: "synthetic/main".into(), code: Arc::from(jsc::SYNTHETIC_PLAYER) };
    let reqs = vec![
        ChallengeRequest { kind: ChallengeType::N, challenges: vec!["abcdef".into()] },
        ChallengeRequest { kind: ChallengeType::Sig, challenges: vec![jsc::sig_spec_challenge(6)] },
    ];
    let out = s.solve(&script, reqs).await.map_err(|e| e.to_string())?;
    let n = out[0].as_ref().map_err(|e| e.clone())?;
    let sig = out[1].as_ref().map_err(|e| e.clone())?;
    let perm = jsc::sig_permutation(&sig[&jsc::sig_spec_challenge(6)]);
    if n["abcdef"] != jsc::SYNTHETIC_N_ABCDEF || jsc::apply_permutation(&perm, "abcdef").as_deref() != Some(jsc::SYNTHETIC_SIG_ABCDEF) {
        return Err("the solver gave wrong answers".into());
    }
    s.close();
    Ok(t.elapsed().as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn causes_become_d4_kinds() {
        use PlayFailureKind as K;
        let k = |c: &[Cause], signed: bool| failure(c, signed).kind;
        assert_eq!(k(&[Cause::Challenged(Challenge::BotCheck)], false), K::Challenged);
        assert_eq!(k(&[Cause::ClientRefused, Cause::Challenged(Challenge::Captcha)], true), K::Challenged);
        assert_eq!(k(&[Cause::Unauthorized], true), K::SessionExpired);
        assert_eq!(k(&[Cause::Restricted(Restriction::Geo)], false), K::Unavailable);
        assert_eq!(k(&[Cause::AgeGated, Cause::PoTokenRequired], true), K::Unavailable);
        assert_eq!(k(&[Cause::LoginRequired { sign_in: true }], true), K::SessionExpired);
        assert_eq!(k(&[Cause::LoginRequired { sign_in: false }], true), K::Unavailable, "a LOGIN_REQUIRED without 'sign in' is about the song");
        assert_eq!(k(&[Cause::LoginRequired { sign_in: true }], false), K::Unavailable);
        // Signed in, a private video: embedded refuses, the TV clients say private.
        let private = failure(&[Cause::ClientRefused, Cause::Restricted(Restriction::Private), Cause::Restricted(Restriction::Private)], true);
        assert_eq!((private.kind, private.message.as_str()), (K::Unavailable, "This song is private on YouTube."));
        // An extractor failure isn't hidden behind another client's refusal.
        assert_eq!(failure(&[Cause::AgeGated, Cause::Solver("x".into()), Cause::ClientRefused], false).message, NEEDS_UPDATE);
        assert_eq!(failure(&[Cause::LoginRequired { sign_in: true }, Cause::Sabr], true).message, NEEDS_UPDATE);
        // A solve out of time is worth another try, not an update.
        let t = failure(&[Cause::SolverTransient("out of time".into())], false);
        assert_eq!((t.kind, t.message.as_str()), (K::Unknown, SOLVER_DIDNT_FINISH));
        assert_eq!(k(&[Cause::PoTokenRequired, Cause::NoAudio], false), K::Unavailable);
        assert_eq!(failure(&[Cause::PoTokenRequired], true).message, NEEDS_UPDATE);
        assert_eq!(k(&[Cause::OtherVideo, Cause::OtherVideo], false), K::Challenged);
        assert_eq!(k(&[Cause::Sabr, Cause::ClientRefused], false), K::Unknown);
        assert_eq!(failure(&[Cause::Solver("x".into())], false).message, NEEDS_UPDATE);
        assert_eq!(k(&[Cause::Network("timed out".into())], false), K::Network);
        assert_eq!(k(&[Cause::PlayerJs("answered 404".into())], false), K::Unknown);
        assert_eq!(k(&[Cause::Refused(403)], false), K::Unknown);
        assert_eq!(k(&[Cause::NoAudio], false), K::Unavailable);
        // A bare 403 is never a bot check (D4).
        assert_ne!(k(&[Cause::Refused(403), Cause::Refused(403)], true), K::Challenged);
    }

    fn ready(itag: u32, rank_lang: i32, acceptable: bool) -> Ready {
        Ready {
            stream: ResolvedStream { video_id: "x".into(), url: Secret::new(String::new()), headers: vec![], itag, codec: "aac".into(), mime: "audio/mp4".into(), bitrate_kbps: None, content_length: None, duration: None, loudness_db: None, expires_at: None, range_safe: true, authenticated: false, available_at: None },
            rank: (false, -rank_lang, formats::PREFERENCE.iter().position(|&i| i == itag).unwrap_or(9), 0),
            acceptable,
            wait: 0,
            client: "c".into(),
        }
    }

    #[test]
    fn a_bot_check_after_a_fallback_wins() {
        let mut s = Search::default();
        assert!(!s.found(ready(251, -1, false)), "Opus doesn't end the search");
        assert!(s.refused(Cause::Challenged(Challenge::BotCheck)));
        assert_eq!(s.finish(false).err().map(|f| f.kind), Some(PlayFailureKind::Challenged));
        let mut s = Search::default();
        s.found(ready(251, -1, false));
        s.refused(Cause::Unauthorized);
        assert_eq!(s.finish(true).err().map(|f| f.kind), Some(PlayFailureKind::SessionExpired));
        // An ordinary refusal after a fallback keeps the fallback.
        let mut s = Search::default();
        s.found(ready(251, -1, false));
        assert!(!s.refused(Cause::Restricted(Restriction::Removed)), "a restriction doesn't end the search");
        s.refused(Cause::ClientRefused);
        assert_eq!(s.finish(false).ok().map(|r| r.stream.itag), Some(251));
    }

    #[test]
    fn the_original_track_beats_a_dubbed_aac_across_clients() {
        let mut s = Search::default();
        s.found(ready(251, 10, false));
        assert!(!s.found(ready(140, -1, false)), "a dubbed AAC doesn't end the search");
        assert_eq!(s.finish(false).ok().map(|r| r.stream.itag), Some(251));
    }

    #[test]
    fn what_ends_the_search() {
        assert!(Cause::Challenged(Challenge::RateLimit).final_now());
        assert!(!Cause::Restricted(Restriction::Private).final_now());
        assert!(!Cause::AgeGated.final_now());
        assert!(!Cause::Sabr.final_now());
        assert!(!Cause::Solver("x".into()).final_now());
        assert!(!Cause::Refused(403).final_now());
    }

    #[tokio::test]
    async fn the_solver_checks_itself_offline() {
        assert!(solver_self_check().await.is_ok());
    }

    #[test]
    fn identities_scope_the_cache() {
        let anon = Identity::anonymous(None);
        let opts = ResolveOptions::default();
        assert_eq!(NativeResolver::cache_key("4D7u5KF7SP8", &anon, &opts), "4D7u5KF7SP8|anonymous|");
        let mut next = Identity::anonymous(None);
        next.key = "anonymous:2".into();
        assert_ne!(NativeResolver::cache_key("4D7u5KF7SP8", &next, &opts), NativeResolver::cache_key("4D7u5KF7SP8", &anon, &opts), "a new signed-out session never gets the old one's URLs");
        let c = |n: &str| encore_core::service::CookieRecord { name: n.into(), value: "v".into(), domain: ".youtube.com".into(), path: "/".into(), secure: true, http_only: false, expires: None };
        let a = ResolveAuth { cookies: vec![c("SAPISID"), c("LOGIN_INFO")], key: "acct:2:page".into(), ..Default::default() };
        let id = Identity::from_auth(Some(&a), Some("anon visitor".into()));
        assert_eq!(NativeResolver::cache_key("4D7u5KF7SP8", &id, &opts), "4D7u5KF7SP8|acct:2:page|");
        assert_eq!(id.visitor_data, None, "a signed-in resolve never takes the anonymous visitor");
        // Cookies without a login are a signed-out resolve.
        let b = ResolveAuth { cookies: vec![c("PREF")], key: "acct:3".into(), ..Default::default() };
        assert_eq!(Identity::from_auth(Some(&b), None).key, "anonymous");
    }
}
