//! Media sources for the decoder.
//!
//! `HttpSource` is a seekable `Read + Seek` view over a remote file. A fetch
//! thread downloads 1 MiB ranges into a sparse `.part` file in the cache
//! directory, starting at whatever position the decoder is reading and then
//! filling the rest. Each range is streamed to the file as it arrives and its
//! received bytes can be read at once, so the decoder starts on the first
//! few KiB rather than the whole MiB; a seek to a chunk not yet there leaves
//! the rest of the current one for later. A range counts only when its
//! `Content-Range` names it and its body ends exactly there. Reads block until the bytes they
//! need are present. When every chunk is whole the file is installed in the
//! [`AudioCache`] and later plays from disk.
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
    /// Whole chunks.
    have: Vec<bool>,
    /// Bytes received from the start of each chunk that isn't whole yet:
    /// readable before the rest of the chunk arrives.
    got: Vec<u32>,
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

/// One client for every source, so a song's ranges and the next song's
/// reuse the media server's connections, and one runtime thread serves
/// them all. It sends no header of its own (no cookie ever goes to the
/// media server, D3): each request carries its spec's headers. A read
/// waits at most 30 s.
fn client() -> &'static reqwest::blocking::Client {
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| reqwest::blocking::Client::builder().connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(30)).build().expect("http client"))
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
            st: Mutex::new(State { len, have: vec![false; nchunks], got: vec![0; nchunks], want: 0, error: None, complete: false, installed: None }),
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

    /// Wait until the byte at `pos` is available; returns the end of what is
    /// readable from there (within one chunk).
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
            let ready = ci as u64 * CHUNK + st.got[ci] as u64;
            if pos < ready {
                return Ok(ready);
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

/// Records `upto` bytes received from the start of chunk `ci`: readable at
/// once. The chunk counts as whole only when its response checked out to
/// the end (`whole`), so a body that turns out wrong is never installed.
fn received(st: &mut State, ci: usize, upto: u64, whole: bool) {
    let size = ((ci as u64 + 1) * CHUNK).min(st.len) - ci as u64 * CHUNK;
    st.got[ci] = st.got[ci].max(upto.min(size) as u32);
    if whole && upto >= size {
        st.have[ci] = true;
    }
}

/// A 206 names the bytes asked for in its `Content-Range` (and the file's
/// length, when it says), and its `Content-Length`, when given, is theirs:
/// no byte of it is used otherwise.
fn range_matches(headers: &reqwest::header::HeaderMap, a: u64, b: u64, len: u64) -> bool {
    let Some(cr) = headers.get("content-range") else { return false };
    let Some(spec) = cr.to_str().ok().and_then(|v| v.strip_prefix("bytes ")) else { return false };
    let Some((range, total)) = spec.split_once('/') else { return false };
    let Some((x, y)) = range.split_once('-') else { return false };
    let total_ok = total == "*" || total.parse::<u64>().is_ok_and(|t| t == len);
    let length_ok = headers.get("content-length").and_then(|v| v.to_str().ok()).is_none_or(|l| l.trim().parse::<u64>().is_ok_and(|l| l == b - a + 1));
    x.trim().parse::<u64>().is_ok_and(|x| x == a) && y.trim().parse::<u64>().is_ok_and(|y| y == b) && total_ok && length_ok
}

/// What a range response ended with.
enum Streamed {
    /// Every byte asked for, and the body ended there.
    Whole,
    /// The decoder wants a chunk elsewhere: the rest of this one later.
    Left,
    /// Cancelled.
    Cancelled,
    /// The body ended early or broke off; `progress` when it brought at
    /// least `PROGRESS` new bytes first.
    Short { kind: Option<io::ErrorKind>, progress: bool },
    /// More bytes than asked for: nothing of this response is kept.
    Long,
}

fn fetch_ranges(sh: &Shared, wf: &File, mut spec: HttpSpec) -> Result<(), String> {
    let c = client();
    let mut refreshed = false;
    let mut failures = 0u32;
    loop {
        if sh.cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let (ci, len, from) = {
            let mut st = sh.st.lock();
            match next_missing(&st) {
                Some(i) => (i, st.len, st.got[i] as u64),
                None => {
                    st.complete = true;
                    sh.cv.notify_all();
                    return Ok(());
                }
            }
        };
        // A chunk partly received goes on from where it stopped.
        let a = ci as u64 * CHUNK + from;
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
        if status != 206 || !range_matches(resp.headers(), a, b, len) {
            // A 200, or a 206 that doesn't say which bytes it holds, from
            // the start: the server doesn't do ranges, so one plain GET.
            if a == 0 && (status == 200 || (status == 206 && resp.headers().get("content-range").is_none())) {
                drop(resp);
                return fetch_sequential(sh, wf, spec);
            }
            failures += 1;
            if failures > 4 {
                return Err(if status == 206 { "media server sent another range".into() } else { format!("media server answered {status}") });
            }
            std::thread::sleep(Duration::from_millis(500 * failures as u64));
            continue;
        }
        match stream_range(sh, wf, resp, ci, a, b)? {
            Streamed::Whole => failures = 0,
            Streamed::Left => {}
            Streamed::Cancelled => return Ok(()),
            // What did arrive is kept; the next request asks for the rest.
            // A response that got further isn't counted against the song.
            Streamed::Short { progress: true, .. } => failures = 0,
            Streamed::Short { kind, progress: false } => {
                failures += 1;
                if failures > 4 {
                    return Err(match kind {
                        Some(k) => format!("download interrupted: {k}"),
                        None => "media server sent a short range".into(),
                    });
                }
            }
            Streamed::Long => {
                failures += 1;
                if failures > 4 {
                    return Err("media server sent more than it was asked for".into());
                }
            }
        }
    }
}

/// How long the fetch waits on a body before looking at what the decoder
/// wants again.
const BODY_POLL: Duration = Duration::from_millis(100);
/// A response cut short after this much isn't held against the song: a
/// server that sends a few bytes and hangs up still fails it.
const PROGRESS: u64 = 64 * 1024;

/// Streams one validated range response into the file, publishing each
/// piece as it lands. The body is read on a thread of its own, so a seek
/// elsewhere leaves a read that is waiting on the network (and its
/// connection) behind at once. The chunk is whole only when the body ended
/// exactly at `b`; a longer one takes back what it wrote.
fn stream_range(sh: &Shared, wf: &File, r: reqwest::blocking::Response, ci: usize, a: u64, b: u64) -> Result<Streamed, String> {
    let base = ci as u64 * CHUNK;
    let asked = b - a + 1;
    let (tx, rx) = std::sync::mpsc::sync_channel::<io::Result<Vec<u8>>>(4);
    let mut r = r;
    std::thread::Builder::new()
        .name("encore-body".into())
        .spawn(move || {
            // Up to one byte past what was asked: enough to see a long body.
            let mut total = 0u64;
            loop {
                let mut buf = vec![0u8; 64 * 1024];
                match r.read(&mut buf) {
                    Ok(0) => {
                        let _ = tx.send(Ok(Vec::new()));
                        return;
                    }
                    Ok(n) => {
                        buf.truncate(n);
                        total += n as u64;
                        if tx.send(Ok(buf)).is_err() || total > asked {
                            return;
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => {
                        let _ = tx.send(Err(e));
                        return;
                    }
                }
            }
        })
        .map_err(|e| format!("could not start a download: {}", e.kind()))?;
    let mut off = a;
    loop {
        if sh.cancel.load(Ordering::Relaxed) {
            return Ok(Streamed::Cancelled);
        }
        match rx.recv_timeout(BODY_POLL) {
            Ok(Ok(piece)) if piece.is_empty() => {
                // The end of the body.
                if off == b + 1 {
                    let mut st = sh.st.lock();
                    received(&mut st, ci, off - base, true);
                    sh.cv.notify_all();
                    return Ok(Streamed::Whole);
                }
                return Ok(Streamed::Short { kind: None, progress: off - a >= PROGRESS });
            }
            Ok(Ok(piece)) => {
                if off + piece.len() as u64 > b + 1 {
                    // Not the bytes asked for after all: take back what this
                    // response wrote; the chunk is fetched again.
                    let mut st = sh.st.lock();
                    st.got[ci] = (a - base) as u32;
                    return Ok(Streamed::Long);
                }
                wf.write_all_at(&piece, off).map_err(|e| format!("cache write failed: {}", e.kind()))?;
                sh.stats.bytes.fetch_add(piece.len() as u64, Ordering::Relaxed);
                off += piece.len() as u64;
                let mut st = sh.st.lock();
                received(&mut st, ci, off - base, false);
                sh.cv.notify_all();
            }
            Ok(Err(e)) => return Ok(Streamed::Short { kind: Some(e.kind()), progress: off - a >= PROGRESS }),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(Streamed::Short { kind: None, progress: off - a >= PROGRESS }),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
        // The decoder went to a chunk that isn't here: fetch that first.
        let st = sh.st.lock();
        let wanted = (st.want / CHUNK) as usize;
        if off <= b && wanted != ci && st.have.get(wanted).is_some_and(|h| !h) {
            return Ok(Streamed::Left);
        }
    }
}

fn fetch_sequential(sh: &Shared, wf: &File, spec: HttpSpec) -> Result<(), String> {
    let c = client();
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
            if (ci as usize) < st.have.len() {
                received(&mut st, ci as usize, off.min(len).saturating_sub(ci * CHUNK), true);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::atomic::AtomicUsize;

    const LEN: usize = 3 * CHUNK as usize + 12_345;

    fn data() -> Vec<u8> {
        (0..LEN).map(|i| (i.wrapping_mul(31) ^ (i >> 9)) as u8).collect()
    }

    /// How the test server answers the n-th range request (from 0).
    #[derive(Clone, Copy)]
    enum Answer {
        Normal,
        /// 64 KiB, then the rest after a pause.
        Trickle(u64),
        /// The first `n` bytes, then the connection closes.
        Cut(usize),
        /// A 206 for other bytes than asked, naming them.
        Other,
        /// The whole file labelled 206, saying nothing of a range.
        Unlabelled,
        /// The right range, then 100 bytes more, ended by closing.
        Long,
    }

    struct Server {
        url: String,
        ranges: Arc<Mutex<Vec<(u64, u64)>>>,
    }

    /// A one-file HTTP/1.1 server that honours `Range` (closing after each
    /// answer), answering request n as `plan(n)` says.
    fn serve(plan: impl Fn(usize) -> Answer + Send + Sync + 'static) -> Server {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/audio", l.local_addr().unwrap());
        let ranges = Arc::new(Mutex::new(Vec::new()));
        let (r2, plan) = (ranges.clone(), Arc::new(plan));
        let count = Arc::new(AtomicUsize::new(0));
        std::thread::spawn(move || {
            let body = Arc::new(data());
            for s in l.incoming().flatten() {
                let (r2, plan, count, body) = (r2.clone(), plan.clone(), count.clone(), body.clone());
                std::thread::spawn(move || {
                    let mut rd = BufReader::new(s.try_clone().unwrap());
                    let mut range = None;
                    loop {
                        let mut line = String::new();
                        if rd.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                            break;
                        }
                        if let Some(v) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                            let (a, b) = v.trim().split_once('-').unwrap();
                            range = Some((a.parse::<u64>().unwrap(), b.parse::<u64>().unwrap()));
                        }
                    }
                    let mut s = s;
                    let Some((a, b)) = range else {
                        // A plain GET: the whole file.
                        let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {LEN}\r\nConnection: close\r\n\r\n").as_bytes());
                        let _ = s.write_all(&body);
                        return;
                    };
                    r2.lock().push((a, b));
                    let n = count.fetch_add(1, Ordering::SeqCst);
                    let (from, to) = match plan(n) {
                        Answer::Other => (a + 7, b + 7),
                        _ => (a, b),
                    };
                    let part = &body[from as usize..=(to as usize).min(LEN - 1)];
                    let head = match plan(n) {
                        Answer::Unlabelled => format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {LEN}\r\nConnection: close\r\n\r\n"),
                        Answer::Long => format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {from}-{to}/{LEN}\r\nConnection: close\r\n\r\n"),
                        _ => format!("HTTP/1.1 206 Partial Content\r\nContent-Type: audio/mp4\r\nContent-Length: {}\r\nContent-Range: bytes {from}-{to}/{LEN}\r\nConnection: close\r\n\r\n", part.len()),
                    };
                    let _ = s.write_all(head.as_bytes());
                    match plan(n) {
                        Answer::Unlabelled => drop(s.write_all(&body)),
                        Answer::Long => {
                            let _ = s.write_all(part);
                            let _ = s.write_all(&[0xee; 100]);
                        }
                        Answer::Normal | Answer::Other => drop(s.write_all(part)),
                        Answer::Trickle(ms) => {
                            let k = part.len().min(64 * 1024);
                            let _ = s.write_all(&part[..k]);
                            let _ = s.flush();
                            std::thread::sleep(Duration::from_millis(ms));
                            let _ = s.write_all(&part[k..]);
                        }
                        Answer::Cut(k) => drop(s.write_all(&part[..k.min(part.len())])),
                    }
                });
            }
        });
        Server { url, ranges }
    }

    fn open(url: &str) -> HttpSource {
        let spec = HttpSpec { url: url.into(), headers: vec![], content_length: Some(LEN as u64), cache_key: "test-140".into(), range_safe: true, refresher: None };
        HttpSource::open(spec, None).unwrap()
    }

    #[test]
    fn the_first_bytes_play_before_the_whole_range_arrives() {
        let srv = serve(|n| if n == 0 { Answer::Trickle(1500) } else { Answer::Normal });
        let mut src = open(&srv.url);
        let t = Instant::now();
        let mut head = [0u8; 1024];
        src.read_exact(&mut head).unwrap();
        assert!(t.elapsed() < Duration::from_millis(1000), "waited {:?}", t.elapsed());
        assert_eq!(&head[..], &data()[..1024]);
        // Still one 1 MiB request for the chunk: steady requests stay large.
        assert_eq!(srv.ranges.lock()[0], (0, CHUNK - 1));
        src.seek(SeekFrom::Start(0)).unwrap();
        let mut all = Vec::new();
        src.read_to_end(&mut all).unwrap();
        assert!(all == data(), "the whole file reads back");
    }

    #[test]
    fn a_cut_range_goes_on_from_where_it_stopped() {
        let srv = serve(|n| if n == 0 { Answer::Cut(300_000) } else { Answer::Normal });
        let mut src = open(&srv.url);
        let mut all = Vec::new();
        src.read_to_end(&mut all).unwrap();
        assert!(all == data());
        let ranges = srv.ranges.lock().clone();
        assert_eq!(ranges[0], (0, CHUNK - 1));
        assert_eq!(ranges[1], (300_000, CHUNK - 1), "only the rest is asked for again");
    }

    #[test]
    fn a_range_for_other_bytes_is_never_written() {
        let srv = serve(|n| if n == 0 { Answer::Other } else { Answer::Normal });
        let mut src = open(&srv.url);
        let mut all = Vec::new();
        src.read_to_end(&mut all).unwrap();
        assert!(all == data());
    }

    #[test]
    fn a_seek_elsewhere_does_not_wait_for_the_current_range() {
        let srv = serve(|n| if n == 0 { Answer::Trickle(3000) } else { Answer::Normal });
        let mut src = open(&srv.url);
        let mut b = [0u8; 16];
        src.read_exact(&mut b).unwrap();
        let far = 2 * CHUNK + 500_000;
        let t = Instant::now();
        src.seek(SeekFrom::Start(far)).unwrap();
        src.read_exact(&mut b).unwrap();
        assert!(t.elapsed() < Duration::from_millis(1500), "waited {:?}", t.elapsed());
        assert_eq!(&b[..], &data()[far as usize..far as usize + 16]);
        src.seek(SeekFrom::Start(0)).unwrap();
        let mut all = Vec::new();
        src.read_to_end(&mut all).unwrap();
        assert!(all == data());
        // The first chunk was finished from what had arrived, not again.
        let ranges = srv.ranges.lock().clone();
        assert!(ranges.iter().skip(1).any(|&(a, b)| a > 0 && a < CHUNK && b == CHUNK - 1), "{ranges:?}");
    }

    #[test]
    fn a_partly_received_file_is_never_installed() {
        let dir = std::env::temp_dir().join(format!("encore-src-test-{}", rand_suffix()));
        let cache = AudioCache::new(&dir, 1 << 30).unwrap();
        // Chunk 0 always breaks off: the source fails, and nothing is cached.
        let srv = serve(|_| Answer::Cut(1000));
        let spec = HttpSpec { url: srv.url.clone(), headers: vec![], content_length: Some(LEN as u64), cache_key: "cut-140".into(), range_safe: true, refresher: None };
        let mut src = HttpSource::open(spec, Some(cache.clone())).unwrap();
        let mut all = Vec::new();
        assert!(src.read_to_end(&mut all).is_err());
        drop(src);
        std::thread::sleep(Duration::from_millis(300));
        assert!(cache.lookup("cut-140").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn content_ranges_are_checked() {
        let mut h = reqwest::header::HeaderMap::new();
        assert!(!range_matches(&h, 0, 9, 100), "a 206 must say which bytes it holds");
        h.insert("content-range", "bytes 5-9/100".parse().unwrap());
        assert!(range_matches(&h, 5, 9, 100));
        assert!(!range_matches(&h, 0, 9, 100));
        assert!(!range_matches(&h, 5, 9, 200), "another file's length");
        h.insert("content-range", "bytes 5-9/*".parse().unwrap());
        assert!(range_matches(&h, 5, 9, 100));
        h.insert("content-length", "5".parse().unwrap());
        assert!(range_matches(&h, 5, 9, 100));
        h.insert("content-length", "100".parse().unwrap());
        assert!(!range_matches(&h, 5, 9, 100), "a body longer than the range");
    }

    /// Reads the whole source and checks it, then that the file the cache
    /// kept (if any) is the file.
    fn read_through_cache(srv: &Server, key: &str) -> (io::Result<Vec<u8>>, Option<Vec<u8>>) {
        let dir = std::env::temp_dir().join(format!("encore-src-{key}-{}", rand_suffix()));
        let cache = AudioCache::new(&dir, 1 << 30).unwrap();
        let spec = HttpSpec { url: srv.url.clone(), headers: vec![], content_length: Some(LEN as u64), cache_key: key.into(), range_safe: true, refresher: None };
        let mut src = HttpSource::open(spec, Some(cache.clone())).unwrap();
        let mut all = Vec::new();
        let r = src.read_to_end(&mut all).map(|_| all);
        drop(src);
        std::thread::sleep(Duration::from_millis(400));
        let kept = cache.lookup(key).map(|p| std::fs::read(p).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
        (r, kept)
    }

    #[test]
    fn a_206_without_a_range_is_read_as_one_plain_download() {
        let srv = serve(|_| Answer::Unlabelled);
        let (r, kept) = read_through_cache(&srv, "unlabelled-140");
        assert!(r.unwrap() == data());
        assert!(kept.is_some_and(|k| k == data()), "the cached file is the file");
    }

    #[test]
    fn a_body_longer_than_its_range_is_never_kept() {
        let srv = serve(|n| if n == 0 { Answer::Long } else { Answer::Normal });
        let (r, kept) = read_through_cache(&srv, "long-140");
        assert!(r.unwrap() == data());
        assert!(kept.is_some_and(|k| k == data()));
        let ranges = srv.ranges.lock().clone();
        // Asked for again, whole; the next chunk may be fetched first.
        assert!(ranges[1..].contains(&(0, CHUNK - 1)), "the chunk is asked for again, whole: {ranges:?}");
        // Always too long: the song fails and nothing is cached.
        let srv = serve(|_| Answer::Long);
        let (r, kept) = read_through_cache(&srv, "longer-140");
        assert!(r.is_err() || kept.is_none());
        assert!(kept.is_none());
    }

    #[test]
    fn a_seek_leaves_a_read_already_waiting_on_the_network() {
        let srv = serve(|n| if n == 0 { Answer::Trickle(3000) } else { Answer::Normal });
        let mut src = open(&srv.url);
        let mut b = [0u8; 16];
        src.read_exact(&mut b).unwrap();
        // The fetch is now blocked on the rest of the first range.
        std::thread::sleep(Duration::from_millis(400));
        let far = CHUNK + 1000;
        let t = Instant::now();
        src.seek(SeekFrom::Start(far)).unwrap();
        src.read_exact(&mut b).unwrap();
        assert!(t.elapsed() < Duration::from_millis(1000), "waited {:?}", t.elapsed());
        assert_eq!(&b[..], &data()[far as usize..far as usize + 16]);
    }
}
