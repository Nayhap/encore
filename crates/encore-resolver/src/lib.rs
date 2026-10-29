//! Stream resolution, M1 (D3): a resident yt-dlp helper in a managed venv.
//!
//! * The venv lives under the app data dir and is bootstrapped on first use
//!   with pinned `yt-dlp` and `yt-dlp-ejs`.
//! * Requests are JSON lines over stdin/stdout; cookies travel in memory per
//!   request and are never written to disk or passed on the command line.
//! * The JS runtime is the first of deno, node or bun found on this machine.
//! * Every resolved URL is probed (head + tail range) before it is handed to
//!   the engine (D4); a failing tail switches to sequential mode.
//! * `StreamResolver` keeps a native resolver as a later, swappable
//!   implementation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

pub const YT_DLP_VERSION: &str = "2026.10.26";
pub const YT_DLP_EJS_VERSION: &str = "0.8.0";
const HELPER_SRC: &str = include_str!("../helper/encore_resolver.py");
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(60);
const POOL: usize = 2;

#[derive(Clone, Debug)]
pub struct ResolverConfig {
    /// `<app data>/resolver`
    pub root: PathBuf,
    /// yt-dlp's player-JS cache (no secrets).
    pub cache_dir: PathBuf,
}

impl ResolverConfig {
    pub fn default_paths() -> Self {
        let data = dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("app.encore.desktop");
        let cache = dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("app.encore.desktop");
        ResolverConfig { root: data.join("resolver"), cache_dir: cache.join("yt-dlp") }
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ResolverStatus {
    pub ready: bool,
    pub python: Option<String>,
    pub js_runtime: Option<String>,
    pub yt_dlp: String,
    pub message: Option<String>,
}

struct Helper {
    child: Child,
    stdin: ChildStdin,
    stdout: tokio::io::Lines<BufReader<ChildStdout>>,
    next_id: u64,
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
    boot: tokio::sync::Mutex<Option<Result<Env, String>>>,
    pool: Vec<tokio::sync::Mutex<Option<Helper>>>,
    http: reqwest::Client,
    cache: Mutex<HashMap<String, Cached>>,
    inflight: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    status: Mutex<ResolverStatus>,
}

#[derive(Clone, Debug)]
struct Env {
    venv_python: PathBuf,
    helper: PathBuf,
    js: Option<(String, PathBuf)>,
}

fn search_paths() -> Vec<PathBuf> {
    // GUI apps get a minimal PATH, so look in the usual places as well.
    let mut v: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    for p in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
        v.push(PathBuf::from(p));
    }
    if let Some(h) = dirs::home_dir() {
        v.push(h.join(".deno/bin"));
        v.push(h.join(".bun/bin"));
        v.push(h.join(".local/bin"));
        v.push(h.join(".volta/bin"));
    }
    v
}

fn find_exe(name: &str) -> Option<PathBuf> {
    search_paths().into_iter().map(|d| d.join(name)).find(|p| p.is_file())
}

/// The first available JS runtime (deno, node, bun).
pub fn find_js_runtime() -> Option<(String, PathBuf)> {
    for n in ["deno", "node", "bun"] {
        if let Some(p) = find_exe(n) {
            return Some((n.to_string(), p));
        }
    }
    None
}

async fn python_ok(p: &Path) -> Option<String> {
    let out = Command::new(p).args(["-I", "-c", "import sys; print('%d.%d' % sys.version_info[:2])"]).output().await.ok()?;
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let (maj, min) = v.split_once('.')?;
    let ok = maj.parse::<u32>().ok()? == 3 && min.parse::<u32>().ok()? >= 10;
    ok.then_some(v)
}

async fn find_python() -> Option<(PathBuf, String)> {
    let mut cands = Vec::new();
    for n in ["python3.14", "python3.13", "python3.12", "python3.11", "python3.10", "python3"] {
        for d in search_paths() {
            let p = d.join(n);
            if p.is_file() && !cands.contains(&p) {
                cands.push(p);
            }
        }
    }
    for p in cands {
        // /usr/bin/python3 is a shim that may prompt to install the
        // Command Line Tools; only use it if they are present.
        if p == Path::new("/usr/bin/python3") && !Path::new("/Library/Developer/CommandLineTools/usr/bin/python3").exists() && !Path::new("/Applications/Xcode.app").exists() {
            continue;
        }
        if let Some(v) = python_ok(&p).await {
            return Some((p, v));
        }
    }
    None
}

fn marker() -> String {
    format!("yt-dlp=={YT_DLP_VERSION} yt-dlp-ejs=={YT_DLP_EJS_VERSION} helper={:x}", fxhash(HELPER_SRC))
}

fn fxhash(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
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

fn fail(kind: PlayFailureKind, m: impl Into<String>) -> PlayFailure {
    PlayFailure::new(kind, m)
}

impl YtDlpResolver {
    pub fn new(cfg: ResolverConfig) -> Arc<Self> {
        Arc::new(Self {
            cfg,
            boot: tokio::sync::Mutex::new(None),
            pool: (0..POOL).map(|_| tokio::sync::Mutex::new(None)).collect(),
            http: reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(20)).build().expect("http"),
            cache: Mutex::new(HashMap::new()),
            inflight: Mutex::new(HashMap::new()),
            status: Mutex::new(ResolverStatus { ready: false, python: None, js_runtime: None, yt_dlp: YT_DLP_VERSION.into(), message: None }),
        })
    }

    pub fn status(&self) -> ResolverStatus {
        self.status.lock().clone()
    }

    /// Create the venv and install the pinned packages if needed.
    pub async fn ensure_ready(&self) -> Result<(), String> {
        let mut b = self.boot.lock().await;
        if let Some(Ok(_)) = &*b {
            return Ok(());
        }
        let r = self.bootstrap().await;
        {
            let mut st = self.status.lock();
            match &r {
                Ok(env) => {
                    st.ready = true;
                    st.js_runtime = env.js.as_ref().map(|j| j.0.clone());
                    st.message = None;
                }
                Err(e) => {
                    st.ready = false;
                    st.message = Some(e.clone());
                }
            }
        }
        let out = r.as_ref().map(|_| ()).map_err(|e| e.clone());
        *b = Some(r);
        out
    }

    async fn env(&self) -> Result<Env, String> {
        self.ensure_ready().await?;
        match &*self.boot.lock().await {
            Some(Ok(e)) => Ok(e.clone()),
            Some(Err(e)) => Err(e.clone()),
            None => Err("resolver not initialised".into()),
        }
    }

    async fn bootstrap(&self) -> Result<Env, String> {
        let root = &self.cfg.root;
        std::fs::create_dir_all(root).map_err(|e| format!("cannot create {}: {}", root.display(), e.kind()))?;
        let venv = root.join("venv");
        let py = venv.join("bin/python3");
        let mark = venv.join(".encore-ready");
        let want = marker();
        let helper = root.join("encore_resolver.py");
        let ready = py.is_file() && std::fs::read_to_string(&mark).ok().as_deref() == Some(want.as_str());
        if !ready {
            let (sys_py, ver) = find_python().await.ok_or("Python 3.10 or newer was not found. Install Python 3 (for example with Homebrew) to enable playback.")?;
            self.status.lock().python = Some(ver);
            if !py.is_file() {
                let out = Command::new(&sys_py).args(["-m", "venv", "--clear"]).arg(&venv).output().await.map_err(|e| format!("could not start Python: {}", e.kind()))?;
                if !out.status.success() {
                    return Err(format!("creating the resolver venv failed: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("")));
                }
            }
            let out = Command::new(&py)
                .args(["-m", "pip", "install", "--disable-pip-version-check", "--no-input", "-q", "--upgrade"])
                .arg(format!("yt-dlp=={YT_DLP_VERSION}"))
                .arg(format!("yt-dlp-ejs=={YT_DLP_EJS_VERSION}"))
                .env("PIP_NO_INPUT", "1")
                .output()
                .await
                .map_err(|e| format!("could not run pip: {}", e.kind()))?;
            if !out.status.success() {
                let err = String::from_utf8_lossy(&out.stderr);
                return Err(format!("installing yt-dlp failed: {}", err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("pip error")));
            }
            std::fs::write(&helper, HELPER_SRC).map_err(|e| format!("cannot write helper: {}", e.kind()))?;
            std::fs::write(&mark, &want).map_err(|e| format!("cannot write marker: {}", e.kind()))?;
        } else if std::fs::read_to_string(&helper).ok().as_deref() != Some(HELPER_SRC) {
            std::fs::write(&helper, HELPER_SRC).map_err(|e| format!("cannot write helper: {}", e.kind()))?;
        }
        if let Some(v) = python_ok(&py).await {
            self.status.lock().python = Some(v);
        }
        let js = find_js_runtime();
        if js.is_none() {
            return Err("No JavaScript runtime (deno, node or bun) was found; YouTube's signature challenge needs one.".into());
        }
        Ok(Env { venv_python: py, helper, js })
    }

    async fn spawn(&self, env: &Env) -> Result<Helper, PlayFailure> {
        let mut cmd = Command::new(&env.venv_python);
        cmd.arg("-I").arg(&env.helper).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
        // Sanitised environment: no inherited proxies/credentials.
        cmd.env_clear();
        for k in ["HOME", "PATH", "LANG", "TMPDIR"] {
            if let Some(v) = std::env::var_os(k) {
                cmd.env(k, v);
            }
        }
        if let Some((_, p)) = &env.js {
            if let Some(dir) = p.parent() {
                let path = std::env::var_os("PATH").unwrap_or_default();
                let mut ps: Vec<PathBuf> = vec![dir.to_path_buf()];
                ps.extend(std::env::split_paths(&path));
                if let Ok(j) = std::env::join_paths(ps) {
                    cmd.env("PATH", j);
                }
            }
        }
        let mut child = cmd.spawn().map_err(|e| fail(PlayFailureKind::Unknown, format!("could not start the resolver: {}", e.kind())))?;
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut h = Helper { child, stdin, stdout, next_id: 1 };
        let hello = tokio::time::timeout(Duration::from_secs(20), h.stdout.next_line())
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
        let env = self.env().await.map_err(|e| fail(PlayFailureKind::Unknown, e))?;
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
        if guard.is_none() {
            *guard = Some(self.spawn(&env).await?);
        }
        let h = guard.as_mut().unwrap();
        let id = h.next_id;
        h.next_id += 1;
        let mut req = req;
        req["id"] = json!(id);
        req["cache_dir"] = json!(self.cfg.cache_dir.to_string_lossy());
        if let Some((n, p)) = &env.js {
            req["js_runtime"] = json!({ "name": n, "path": p.to_string_lossy() });
        }
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
                Err(e) if e.kind == PlayFailureKind::Unavailable || e.kind == PlayFailureKind::SessionExpired => return Err(e),
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
