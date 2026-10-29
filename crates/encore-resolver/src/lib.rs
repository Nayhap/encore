//! Stream resolution (D3): a resident yt-dlp helper on Encore's own Python
//! and Deno (D22).
//!
//! * The components are downloaded, verified and kept by [`runtimes`] under
//!   the app data dir, after the user approves; nothing is taken from the
//!   system. Until they are there, resolving fails with
//!   `PlayFailureKind::SetupNeeded` and the app offers the setup.
//! * Requests are JSON lines over stdin/stdout; cookies travel in memory per
//!   request and are never written to disk or passed on the command line.
//! * Every resolved URL is probed (head + tail range) before it is handed to
//!   the engine (D4); a failing tail switches to sequential mode.
//! * `StreamResolver` keeps a native resolver as a later, swappable
//!   implementation.

pub mod runtimes;

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use encore_core::redact::Secret;
use encore_core::service::{PlayFailure, PlayFailureKind, ResolveAuth, ResolvedStream, StreamResolver};
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};

pub use runtimes::manifest::{YT_DLP_EJS_VERSION, YT_DLP_VERSION};
pub use runtimes::{RuntimeStatus, Runtimes, RuntimesConfig, SetupError, SetupErrorKind, SetupPhase, SetupProgress};

const HELPER_SRC: &str = include_str!("../helper/encore_resolver.py");
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(60);
const POOL: usize = 2;

/// What resolving says until the components are installed.
pub const SETUP_NEEDED: &str = "Encore needs to finish setting up playback.";

#[derive(Clone, Debug)]
pub struct ResolverConfig {
    /// `<app data>`; the components go in `runtimes/` under it.
    pub data_dir: PathBuf,
    /// `<cache>`; yt-dlp's player-JS cache (no secrets) and Deno's cache.
    pub cache_dir: PathBuf,
}

impl ResolverConfig {
    pub fn default_paths() -> Self {
        let data = dirs::data_local_dir().unwrap_or_else(std::env::temp_dir).join("app.encore.desktop");
        let cache = dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("app.encore.desktop");
        ResolverConfig { data_dir: data, cache_dir: cache }
    }

    pub fn runtimes(&self) -> RuntimesConfig {
        RuntimesConfig::in_data_dir(&self.data_dir, &self.cache_dir)
    }
}

struct Helper {
    child: Child,
    stdin: ChildStdin,
    stdout: tokio::io::Lines<BufReader<ChildStdout>>,
    next_id: u64,
    /// The component set it was started from.
    generation: u64,
}

impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

#[derive(Clone)]
struct Cached {
    stream: ResolvedStream,
    until: Instant,
}

pub struct YtDlpResolver {
    cfg: ResolverConfig,
    runtimes: Arc<Runtimes>,
    pool: Vec<tokio::sync::Mutex<Option<Helper>>>,
    http: reqwest::Client,
    cache: Mutex<HashMap<String, Cached>>,
    inflight: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

#[derive(Clone, Debug)]
struct Env {
    python: PathBuf,
    site: PathBuf,
    helper: PathBuf,
    deno: PathBuf,
    generation: u64,
}

fn fail(kind: PlayFailureKind, m: impl Into<String>) -> PlayFailure {
    PlayFailure::new(kind, m)
}

#[derive(Deserialize)]
struct HelperFormat {
    url: String,
    #[serde(default)]
    headers: HashMap<String, String>,
    itag: u32,
    ext: Option<String>,
    acodec: Option<String>,
    abr: Option<f64>,
    filesize: Option<f64>,
    duration: Option<f64>,
    expires: Option<i64>,
    #[serde(default)]
    available_at: Option<f64>,
    authenticated: bool,
}

fn fxhash(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

impl YtDlpResolver {
    pub fn new(cfg: ResolverConfig, runtimes: Arc<Runtimes>) -> Arc<Self> {
        Arc::new(Self {
            cfg,
            runtimes,
            pool: (0..POOL).map(|_| tokio::sync::Mutex::new(None)).collect(),
            http: reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(20)).build().expect("http"),
            cache: Mutex::new(HashMap::new()),
            inflight: Mutex::new(HashMap::new()),
        })
    }

    pub fn runtimes(&self) -> &Arc<Runtimes> {
        &self.runtimes
    }

    /// Stop the helper processes (before the components are removed, or
    /// after they changed).
    pub async fn stop_helpers(&self) {
        for m in &self.pool {
            *m.lock().await = None;
        }
    }

    /// The installed set, with the helper script written next to it.
    fn env(&self) -> Result<Env, PlayFailure> {
        let generation = self.runtimes.generation();
        let set = self.runtimes.active().ok_or_else(|| fail(PlayFailureKind::SetupNeeded, SETUP_NEEDED))?;
        let helper = self.runtimes.root().join("encore_resolver.py");
        if std::fs::read_to_string(&helper).ok().as_deref() != Some(HELPER_SRC) {
            runtimes::write_atomic(&helper, HELPER_SRC.as_bytes()).map_err(|e| fail(PlayFailureKind::Unknown, format!("could not write the resolver helper: {}", e.kind())))?;
        }
        Ok(Env { python: set.python, site: set.site, helper, deno: set.deno, generation })
    }

    async fn spawn(&self, env: &Env) -> Result<Helper, PlayFailure> {
        let mut cmd = runtimes::helper_command(&env.python, self.runtimes.deno_dir());
        cmd.arg("-I").arg(&env.helper).arg("--site").arg(&env.site).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| fail(PlayFailureKind::Unknown, format!("could not start the resolver: {}", e.kind())))?;
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut h = Helper { child, stdin, stdout, next_id: 1, generation: env.generation };
        let hello = tokio::time::timeout(Duration::from_secs(30), h.stdout.next_line())
            .await
            .map_err(|_| fail(PlayFailureKind::Unknown, "the resolver did not start in time"))?
            .map_err(|_| fail(PlayFailureKind::Unknown, "the resolver closed its output"))?
            .ok_or_else(|| fail(PlayFailureKind::Unknown, "the resolver exited at start"))?;
        let v: Value = serde_json::from_str(&hello).map_err(|_| fail(PlayFailureKind::Unknown, "the resolver sent an unreadable greeting"))?;
        if v.get("ok").and_then(Value::as_bool) != Some(true) {
            return Err(fail(PlayFailureKind::Unknown, v.get("error").and_then(Value::as_str).unwrap_or("resolver failed to start").to_string()));
        }
        Ok(h)
    }

    async fn call(&self, req: Value) -> Result<Value, PlayFailure> {
        let env = self.env()?;
        // Pick a free helper slot, else wait on the first.
        let mut slot = None;
        for m in &self.pool {
            if let Ok(g) = m.try_lock() {
                slot = Some(g);
                break;
            }
        }
        let mut guard = match slot {
            Some(g) => g,
            None => self.pool[0].lock().await,
        };
        // A helper from an older component set is replaced.
        if guard.as_ref().is_some_and(|h| h.generation != env.generation) {
            *guard = None;
        }
        if guard.is_none() {
            *guard = Some(self.spawn(&env).await?);
        }
        let h = guard.as_mut().unwrap();
        let id = h.next_id;
        h.next_id += 1;
        let mut req = req;
        req["id"] = json!(id);
        req["cache_dir"] = json!(self.cfg.cache_dir.join("yt-dlp").to_string_lossy());
        req["js_runtime"] = json!({ "name": "deno", "path": env.deno.to_string_lossy() });
        let line = serde_json::to_string(&req).unwrap() + "\n";
        let io = async {
            h.stdin.write_all(line.as_bytes()).await?;
            h.stdin.flush().await?;
            loop {
                let Some(l) = h.stdout.next_line().await? else { return Err(std::io::Error::other("resolver exited")) };
                if let Ok(v) = serde_json::from_str::<Value>(&l) {
                    if v.get("id").and_then(Value::as_u64) == Some(id) {
                        return Ok(v);
                    }
                }
            }
        };
        match tokio::time::timeout(RESOLVE_TIMEOUT, io).await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(_)) => {
                *guard = None; // restart next time
                Err(fail(PlayFailureKind::Unknown, "the resolver stopped unexpectedly"))
            }
            Err(_) => {
                *guard = None; // kill on drop
                Err(fail(PlayFailureKind::Network, "resolving the stream timed out"))
            }
        }
    }

    pub async fn health(&self) -> Result<String, PlayFailure> {
        let v = self.call(json!({ "op": "health" })).await?;
        Ok(v.get("health").and_then(|h| h.get("yt_dlp")).and_then(Value::as_str).unwrap_or("?").to_string())
    }

    async fn extract(&self, video_id: &str, auth: Option<&ResolveAuth>) -> Result<ResolvedStream, PlayFailure> {
        let cookies: Vec<Value> = auth
            .map(|a| a.cookies.iter().map(|c| json!({ "name": c.name, "value": c.value, "domain": c.domain, "path": c.path, "secure": c.secure, "http_only": c.http_only, "expires": c.expires })).collect())
            .unwrap_or_default();
        let v = self.call(json!({ "op": "resolve", "video_id": video_id, "cookies": cookies })).await?;
        if v.get("ok").and_then(Value::as_bool) != Some(true) {
            let msg = v.get("error").and_then(Value::as_str).unwrap_or("could not resolve the stream");
            let kind = match v.get("kind").and_then(Value::as_str) {
                Some("challenged") => PlayFailureKind::Challenged,
                Some("unavailable") => PlayFailureKind::Unavailable,
                Some("sessionExpired") => PlayFailureKind::SessionExpired,
                Some("network") => PlayFailureKind::Network,
                _ => PlayFailure::classify(msg, auth.is_some()).kind,
            };
            return Err(fail(kind, msg.to_string()));
        }
        let f: HelperFormat = serde_json::from_value(v.get("format").cloned().unwrap_or_default()).map_err(|_| fail(PlayFailureKind::Unknown, "the resolver returned an unreadable format"))?;
        // Honour the server's "not before" time for this URL (TV client URLs
        // answer 403 for the first few seconds).
        if let Some(at) = f.available_at {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64();
            let wait = (at - now + 0.25).clamp(0.0, 15.0);
            if wait > 0.0 {
                tokio::time::sleep(Duration::from_secs_f64(wait)).await;
            }
        }
        let codec = match f.acodec.as_deref() {
            Some(a) if a.starts_with("opus") => "opus",
            Some(a) if a.starts_with("mp4a") => "aac",
            Some(a) => a,
            None => "unknown",
        }
        .to_string();
        let mime = match f.ext.as_deref() {
            Some("webm") => "audio/webm",
            Some("m4a") | Some("mp4") => "audio/mp4",
            _ => "application/octet-stream",
        }
        .to_string();
        Ok(ResolvedStream {
            video_id: video_id.into(),
            url: Secret::new(f.url),
            headers: f.headers.into_iter().filter(|(k, _)| !k.eq_ignore_ascii_case("cookie")).collect(),
            itag: f.itag,
            codec,
            mime,
            bitrate_kbps: f.abr.map(|a| a as u32),
            content_length: f.filesize.map(|s| s as u64),
            duration: f.duration,
            loudness_db: None,
            expires_at: f.expires,
            range_safe: true,
            authenticated: f.authenticated,
        })
    }

    /// Head + tail range probe (D4). Returns the verified content length and
    /// whether deep ranges work.
    pub async fn probe(&self, s: &ResolvedStream) -> Result<(u64, bool), PlayFailure> {
        let get = |range: String| {
            let mut rb = self.http.get(s.url.expose()).header("Range", range);
            for (k, v) in &s.headers {
                rb = rb.header(k.as_str(), v.as_str());
            }
            rb.send()
        };
        let r = get("bytes=0-1023".into()).await.map_err(|e| fail(PlayFailureKind::Network, if e.is_timeout() { "media server timed out" } else { "could not reach the media server" }))?;
        let st = r.status().as_u16();
        if st == 403 || st == 410 {
            return Err(fail(PlayFailureKind::Challenged, format!("the media server refused the stream ({st})")));
        }
        if st != 206 && st != 200 {
            return Err(fail(PlayFailureKind::Network, format!("the media server answered {st}")));
        }
        let total = r.headers().get("content-range").and_then(|v| v.to_str().ok()).and_then(|v| v.rsplit('/').next()).and_then(|t| t.parse::<u64>().ok()).or(s.content_length);
        let head = r.bytes().await.map_err(|_| fail(PlayFailureKind::Network, "the media download was interrupted"))?;
        // Check the bytes look like media (MP4 ftyp / WebM EBML).
        let looks = head.len() >= 8 && (&head[4..8] == b"ftyp" || head[..4] == [0x1a, 0x45, 0xdf, 0xa3]);
        if !looks {
            return Err(fail(PlayFailureKind::Unknown, "the media server sent something that is not audio"));
        }
        let Some(total) = total else { return Ok((0, false)) };
        if total < 4096 {
            return Ok((total, true));
        }
        let tail = get(format!("bytes={}-{}", total - 1024, total - 1)).await;
        let ok = matches!(&tail, Ok(r) if r.status().as_u16() == 206);
        Ok((total, ok))
    }

    fn cache_key(video_id: &str, auth: Option<&ResolveAuth>) -> String {
        let who = auth.and_then(|a| a.cookies.iter().find(|c| c.name == "SAPISID").map(|c| fxhash(&c.value))).map(|h| format!("{h:x}")).unwrap_or_else(|| "anon".into());
        format!("{video_id}|{who}")
    }


    pub fn clear_cache(&self) {
        self.cache.lock().clear();
    }
}

#[async_trait]
impl StreamResolver for YtDlpResolver {
    async fn resolve(&self, video_id: &str, auth: Option<&ResolveAuth>) -> Result<ResolvedStream, PlayFailure> {
        let key = Self::cache_key(video_id, auth);
        let lock = self.inflight.lock().entry(key.clone()).or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))).clone();
        let _g = lock.lock().await;
        if let Some(c) = self.cache.lock().get(&key) {
            if c.until > Instant::now() {
                return Ok(c.stream.clone());
            }
        }
        let mut last = None;
        for attempt in 0..2 {
            let mut s = match self.extract(video_id, auth).await {
                Ok(s) => s,
                Err(e) if matches!(e.kind, PlayFailureKind::Unavailable | PlayFailureKind::SessionExpired | PlayFailureKind::SetupNeeded) => return Err(e),
                Err(e) => {
                    last = Some(e);
                    continue;
                }
            };
            match self.probe(&s).await {
                Ok((len, range_ok)) => {
                    if len > 0 {
                        s.content_length = Some(len);
                    }
                    s.range_safe = range_ok;
                    let now = encore_core::store::now();
                    let ttl = s.expires_at.map(|e| (e - now - 300).clamp(0, 3600)).unwrap_or(1800) as u64;
                    self.cache.lock().insert(key.clone(), Cached { stream: s.clone(), until: Instant::now() + Duration::from_secs(ttl) });
                    return Ok(s);
                }
                Err(e) => {
                    let _ = attempt;
                    last = Some(e);
                }
            }
        }
        self.inflight.lock().remove(&key);
        Err(last.unwrap_or_else(|| fail(PlayFailureKind::Unknown, "could not resolve the stream")))
    }

    fn forget(&self, video_id: &str) {
        self.cache.lock().retain(|k, _| !k.starts_with(&format!("{video_id}|")));
    }
}
