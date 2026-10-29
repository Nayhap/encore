//! Media sources for the decoder.
//!
//! `HttpSource` is a seekable `Read + Seek` view over a remote file. A fetch
//! thread downloads 1 MiB ranges into a sparse `.part` file in the cache
//! directory, starting at whatever position the decoder is reading and then
//! filling the rest. Reads block until the bytes they need are present. When
//! every chunk is present the file is installed in the [`AudioCache`] and later
//! plays from disk.
//!
//! `range_safe = false` switches to one sequential GET (D4): the decoder can
//! only seek within what has already arrived.
//!
//! The URL is a signed googlevideo URL: it is never logged or put in errors.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use self::pos_io::PositionalIo;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex};

use crate::cache::{AudioCache, sanitize_key};

pub const CHUNK: u64 = 1024 * 1024;

pub type Refresher = Arc<dyn Fn() -> Result<String, String> + Send + Sync>;

#[derive(Clone)]
pub struct HttpSpec {
    /// Signed stream URL. Secret: never log.
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub content_length: Option<u64>,
    pub cache_key: String,
    pub range_safe: bool,
    /// Called once on a 403/410 to get a fresh URL for the same format.
    pub refresher: Option<Refresher>,
}

impl std::fmt::Debug for HttpSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpSpec")
            .field("url", &"<redacted>")
            .field("content_length", &self.content_length)
            .field("cache_key", &self.cache_key)
            .field("range_safe", &self.range_safe)
            .finish()
    }
}

#[derive(Clone, Debug)]
pub enum SourceSpec {
    File(PathBuf),
    Http(HttpSpec),
}

impl SourceSpec {
    pub fn cache_key(&self) -> Option<&str> {
        match self {
            SourceSpec::Http(h) => Some(&h.cache_key),
            SourceSpec::File(_) => None,
        }
    }
}

/// Download statistics, readable while playing.
#[derive(Default)]
pub struct NetStats {
    pub bytes: AtomicU64,
    pub requests: AtomicU64,
    pub refreshes: AtomicU64,
}

struct State {
    len: u64,
    have: Vec<bool>,
    want: u64,
    error: Option<String>,
    complete: bool,
    installed: Option<PathBuf>,
}

struct Shared {
    st: Mutex<State>,
    cv: Condvar,
    cancel: AtomicBool,
    stats: NetStats,
}

pub struct HttpSource {
    shared: Arc<Shared>,
    file: File,
    pos: u64,
    len: u64,
    key: String,
    cache: Option<AudioCache>,
}

impl Drop for HttpSource {
    fn drop(&mut self) {
        // Keep downloading in the background so the cache fills, unless
        // nothing will ever use the file (no cache). The fetch thread exits
        // when the file is complete or on error.
        let mostly_done = {
            let st = self.shared.st.lock();
            let have = st.have.iter().filter(|h| **h).count();
            have * 2 >= st.have.len()
        };
        if self.cache.is_none() || !mostly_done {
            self.shared.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(c) = &self.cache {
            c.mark_closed(&self.key);
        }
        self.shared.cv.notify_all();
    }
}

fn client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .expect("http client")
}

/// Classify an HTTP failure without leaking the URL.
fn describe(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "network timeout".into()
    } else if e.is_connect() {
        "could not connect to the media server".into()
    } else if let Some(s) = e.status() {
        format!("media server answered {}", s.as_u16())
    } else {
        "network error".into()
    }
}

impl HttpSource {
    pub fn open(spec: HttpSpec, cache: Option<AudioCache>) -> io::Result<Self> {
        let key = sanitize_key(&spec.cache_key).unwrap_or_else(|| "stream".into());
        let len = match spec.content_length {
            Some(l) if l > 0 => l,
            _ => probe_len(&spec)?,
        };
        let nchunks = len.div_ceil(CHUNK) as usize;
        let part = match &cache {
            Some(c) => c.part_path(&format!("{key}-{}", rand_suffix())),
            None => std::env::temp_dir().join(format!("encore-{key}-{}.part", rand_suffix())),
        };
        let wf = OpenOptions::new().create(true).truncate(true).read(true).write(true).open(&part)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o600));
        }
        wf.set_len(len)?;
        let rf = File::open(&part)?;
        let shared = Arc::new(Shared {
            st: Mutex::new(State { len, have: vec![false; nchunks], want: 0, error: None, complete: false, installed: None }),
            cv: Condvar::new(),
            cancel: AtomicBool::new(false),
            stats: NetStats::default(),
        });
        if let Some(c) = &cache {
            c.mark_open(&key);
        }
        let sh = shared.clone();
        let c2 = cache.clone();
        let k2 = key.clone();
        std::thread::Builder::new()
            .name("encore-fetch".into())
            .spawn(move || {
                let r = if spec.range_safe { fetch_ranges(&sh, &wf, spec) } else { fetch_sequential(&sh, &wf, spec) };
                let done = sh.st.lock().complete;
                if let Err(e) = r {
                    sh.st.lock().error = Some(e);
                }
                drop(wf);
                if done {
                    if let Some(c) = &c2 {
                        match c.install(&k2, &part) {
                            Ok(p) => sh.st.lock().installed = Some(p),
                            Err(_) => {
                                let _ = std::fs::remove_file(&part);
                            }
                        }
                    }
                } else {
                    // Readers keep their own handle, so unlinking is safe.
                    let _ = std::fs::remove_file(&part);
                }
                if c2.is_none() && done {
                    let _ = std::fs::remove_file(&part);
                }
                sh.cv.notify_all();
            })?;
        Ok(Self { shared, file: rf, pos: 0, len, key, cache })
    }

    pub fn stats(&self) -> (u64, u64, u64) {
        let s = &self.shared.stats;
        (s.bytes.load(Ordering::Relaxed), s.requests.load(Ordering::Relaxed), s.refreshes.load(Ordering::Relaxed))
    }

    /// Wait until `[pos, pos+n)` (clamped to one chunk) is available.
    fn wait_for(&self, pos: u64) -> io::Result<u64> {
        let ci = (pos / CHUNK) as usize;
        let mut st = self.shared.st.lock();
        if st.want != pos {
            st.want = pos;
            self.shared.cv.notify_all();
        }
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            if st.complete || st.have.get(ci).copied().unwrap_or(true) {
                let end = ((ci as u64 + 1) * CHUNK).min(st.len);
                return Ok(end);
            }
            if let Some(e) = &st.error {
                return Err(io::Error::other(e.clone()));
            }
            if Instant::now() > deadline {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "media download stalled"));
            }
            self.shared.cv.wait_for(&mut st, Duration::from_millis(250));
        }
    }
}

fn rand_suffix() -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    format!("{:x}{:x}", t.as_nanos() & 0xffff_ffff, std::process::id())
}

impl Read for HttpSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.len || buf.is_empty() {
            return Ok(0);
        }
        let end = self.wait_for(self.pos)?;
        let n = ((end - self.pos) as usize).min(buf.len());
        let got = self.file.read_at(&mut buf[..n], self.pos)?;
        self.pos += got as u64;
        Ok(got)
    }
}

impl Seek for HttpSource {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let p = match to {
            SeekFrom::Start(p) => p as i128,
            SeekFrom::End(d) => self.len as i128 + d as i128,
            SeekFrom::Current(d) => self.pos as i128 + d as i128,
        };
        if p < 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "seek before start"));
        }
        self.pos = p as u64;
        Ok(self.pos)
    }
}

impl symphonia::core::io::MediaSource for HttpSource {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

fn apply_headers(mut rb: reqwest::blocking::RequestBuilder, spec: &HttpSpec) -> reqwest::blocking::RequestBuilder {
    for (k, v) in &spec.headers {
        rb = rb.header(k.as_str(), v.as_str());
    }
    rb
}

fn probe_len(spec: &HttpSpec) -> io::Result<u64> {
    let c = client();
    let r = apply_headers(c.get(&spec.url), spec)
        .header("Range", "bytes=0-0")
        .send()
        .map_err(|e| io::Error::other(describe(&e)))?;
    if let Some(cr) = r.headers().get("content-range").and_then(|v| v.to_str().ok()) {
        if let Some(total) = cr.rsplit('/').next().and_then(|t| t.parse::<u64>().ok()) {
            return Ok(total);
        }
    }
    if r.status().is_success() {
        if let Some(l) = r.content_length() {
            return Ok(l);
        }
    }
    Err(io::Error::other(format!("media server answered {}", r.status().as_u16())))
}

fn next_missing(st: &State) -> Option<usize> {
    let start = (st.want / CHUNK) as usize;
    (start..st.have.len()).chain(0..start.min(st.have.len())).find(|&i| !st.have[i])
}

fn fetch_ranges(sh: &Shared, wf: &File, mut spec: HttpSpec) -> Result<(), String> {
    let c = client();
    let mut refreshed = false;
    let mut failures = 0u32;
    let mut buf = Vec::with_capacity(CHUNK as usize);
    loop {
        if sh.cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let (ci, len) = {
            let mut st = sh.st.lock();
            match next_missing(&st) {
                Some(i) => (i, st.len),
                None => {
                    st.complete = true;
                    sh.cv.notify_all();
                    return Ok(());
                }
            }
        };
        let a = ci as u64 * CHUNK;
        let b = ((ci as u64 + 1) * CHUNK).min(len) - 1;
        sh.stats.requests.fetch_add(1, Ordering::Relaxed);
        let res = apply_headers(c.get(&spec.url), &spec).header("Range", format!("bytes={a}-{b}")).send();
        let resp = match res {
            Ok(r) => r,
            Err(e) => {
                failures += 1;
                if failures > 4 {
                    return Err(describe(&e));
                }
                std::thread::sleep(Duration::from_millis(500 * failures as u64));
                continue;
            }
        };
        let status = resp.status().as_u16();
        if status == 403 || status == 410 {
            if !refreshed {
                if let Some(rf) = &spec.refresher {
                    refreshed = true;
                    sh.stats.refreshes.fetch_add(1, Ordering::Relaxed);
                    match rf() {
                        Ok(u) => {
                            spec.url = u;
                            continue;
                        }
                        Err(e) => return Err(format!("stream URL expired and refresh failed: {e}")),
                    }
                }
            }
            return Err(format!("media server refused the request ({status})"));
        }
        if status != 206 {
            // A 200 means the server ignored Range: fall back to sequential.
            if status == 200 && a == 0 {
                drop(resp);
                return fetch_sequential(sh, wf, spec);
            }
            failures += 1;
            if failures > 4 {
                return Err(format!("media server answered {status}"));
            }
            std::thread::sleep(Duration::from_millis(500 * failures as u64));
            continue;
        }
        buf.clear();
        let mut r = resp;
        if let Err(e) = r.read_to_end(&mut buf) {
            failures += 1;
            if failures > 4 {
                return Err(format!("download interrupted: {}", e.kind()));
            }
            continue;
        }
        if buf.len() as u64 != b - a + 1 {
            failures += 1;
            if failures > 4 {
                return Err("media server sent a short range".into());
            }
            continue;
        }
        failures = 0;
        wf.write_all_at(&buf, a).map_err(|e| format!("cache write failed: {}", e.kind()))?;
        sh.stats.bytes.fetch_add(buf.len() as u64, Ordering::Relaxed);
        let mut st = sh.st.lock();
        st.have[ci] = true;
        sh.cv.notify_all();
    }
}

fn fetch_sequential(sh: &Shared, wf: &File, spec: HttpSpec) -> Result<(), String> {
    let c = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| "http client".to_string())?;
    sh.stats.requests.fetch_add(1, Ordering::Relaxed);
    let mut r = apply_headers(c.get(&spec.url), &spec).send().map_err(|e| describe(&e))?;
    if !r.status().is_success() {
        return Err(format!("media server answered {}", r.status().as_u16()));
    }
    let len = sh.st.lock().len;
    let mut off = 0u64;
    let mut buf = vec![0u8; 64 * 1024];
    while off < len {
        if sh.cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let n = r.read(&mut buf).map_err(|e| format!("download interrupted: {}", e.kind()))?;
        if n == 0 {
            break;
        }
        wf.write_all_at(&buf[..n], off).map_err(|e| format!("cache write failed: {}", e.kind()))?;
        sh.stats.bytes.fetch_add(n as u64, Ordering::Relaxed);
        let before = off;
        off += n as u64;
        let mut st = sh.st.lock();
        for ci in (before / CHUNK)..=(off.min(len) / CHUNK) {
            let end = ((ci + 1) * CHUNK).min(len);
            if end <= off && (ci as usize) < st.have.len() {
                st.have[ci as usize] = true;
            }
        }
        sh.cv.notify_all();
    }
    let mut st = sh.st.lock();
    if off >= len {
        st.complete = true;
        st.have.iter_mut().for_each(|h| *h = true);
        sh.cv.notify_all();
        Ok(())
    } else {
        Err("download ended early".into())
    }
}

/// Positional reads and writes on one shared file (Unix `pread`/`pwrite`;
/// Windows `ReadFile`/`WriteFile` with an offset). Nothing here uses the
/// file cursor, so Windows moving it is harmless.
mod pos_io {
    use std::fs::File;
    use std::io;

    pub trait PositionalIo {
        fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<usize>;
        fn write_all_at(&self, buf: &[u8], offset: u64) -> io::Result<()>;
    }

    #[cfg(unix)]
    impl PositionalIo for File {
        fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<usize> {
            std::os::unix::fs::FileExt::read_at(self, buf, offset)
        }
        fn write_all_at(&self, buf: &[u8], offset: u64) -> io::Result<()> {
            std::os::unix::fs::FileExt::write_all_at(self, buf, offset)
        }
    }

    #[cfg(windows)]
    impl PositionalIo for File {
        fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<usize> {
            match std::os::windows::fs::FileExt::seek_read(self, buf, offset) {
                // Reading at or past the end reports EOF as an error on Windows.
                Err(e) if e.raw_os_error() == Some(38) => Ok(0), // ERROR_HANDLE_EOF
                r => r,
            }
        }
        fn write_all_at(&self, mut buf: &[u8], mut offset: u64) -> io::Result<()> {
            while !buf.is_empty() {
                match std::os::windows::fs::FileExt::seek_write(self, buf, offset) {
                    Ok(0) => return Err(io::Error::new(io::ErrorKind::WriteZero, "failed to write whole buffer")),
                    Ok(n) => {
                        buf = &buf[n..];
                        offset += n as u64;
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => return Err(e),
                }
            }
            Ok(())
        }
    }
}
