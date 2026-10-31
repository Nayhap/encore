//! Fetching a resolved stream into a `.part` file (D23): 1 MiB ranges in
//! order, every answer checked. A `206` must carry exactly the range asked
//! for (start, end and total) and exactly that many bytes; a `200` is the
//! whole file and replaces whatever was there; `416` means the file
//! changed, so it starts over once. The signed URL is never logged.

use std::io::SeekFrom;
use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

pub const CHUNK: u64 = 1024 * 1024;
/// The tail of a partial file compared before continuing it.
pub const OVERLAP: u64 = 4096;
/// Longer than any song: refuse instead of filling the disk.
pub const MAX_TOTAL: u64 = 1024 * 1024 * 1024;

#[derive(Debug)]
pub struct Fetched {
    pub status: u16,
    pub content_range: Option<String>,
    pub body: Vec<u8>,
}

#[derive(Debug)]
pub enum FetchError {
    /// No answer: connection, DNS, timeout.
    Transport(String),
    /// The body was longer than allowed.
    TooLong,
}

/// One GET. With `range`, asks for those bytes (inclusive). Reads at most
/// `limit` bytes of body.
#[async_trait]
pub trait Fetcher: Send + Sync {
    async fn get(&self, url: &str, headers: &[(String, String)], range: Option<(u64, u64)>, limit: u64) -> Result<Fetched, FetchError>;
}

pub struct HttpFetcher {
    http: reqwest::Client,
}

impl Default for HttpFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpFetcher {
    pub fn new() -> Self {
        HttpFetcher { http: reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(60)).build().expect("http client") }
    }
}

fn describe(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "network timeout".into()
    } else if e.is_connect() {
        "could not connect to the media server".into()
    } else {
        "network error".into()
    }
}

#[async_trait]
impl Fetcher for HttpFetcher {
    async fn get(&self, url: &str, headers: &[(String, String)], range: Option<(u64, u64)>, limit: u64) -> Result<Fetched, FetchError> {
        // The bytes as stored: no compression in between.
        let mut rb = self.http.get(url).header(reqwest::header::ACCEPT_ENCODING, "identity");
        for (k, v) in headers {
            rb = rb.header(k.as_str(), v.as_str());
        }
        if let Some((a, b)) = range {
            rb = rb.header(reqwest::header::RANGE, format!("bytes={a}-{b}"));
        }
        let mut r = rb.send().await.map_err(|e| FetchError::Transport(describe(&e)))?;
        let status = r.status().as_u16();
        let content_range = r.headers().get(reqwest::header::CONTENT_RANGE).and_then(|v| v.to_str().ok()).map(str::to_string);
        let mut body = Vec::new();
        if status == 200 || status == 206 {
            while let Some(c) = r.chunk().await.map_err(|e| FetchError::Transport(describe(&e)))? {
                if (body.len() + c.len()) as u64 > limit {
                    return Err(FetchError::TooLong);
                }
                body.extend_from_slice(&c);
            }
        }
        Ok(Fetched { status, content_range, body })
    }
}

/// `bytes <a>-<b>/<total>`.
pub fn parse_content_range(v: &str) -> Option<(u64, u64, u64)> {
    let rest = v.trim().strip_prefix("bytes")?.trim();
    let (span, total) = rest.split_once('/')?;
    let (a, b) = span.split_once('-')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?, total.trim().parse().ok()?))
}

/// The stream to fetch.
#[derive(Clone)]
pub struct Source {
    /// Signed URL. Secret: never log.
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub total: u64,
    /// false: deep ranges failed the probe, fetch it in one request (D4).
    pub ranges: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// Paused, cancelled or offline: the partial file stays.
    Stopped,
}

#[derive(Debug)]
pub enum TransferError {
    /// 403/410: the URL went stale. Resolve again and continue.
    Refused,
    /// The server says the song has another length now.
    Changed(u64),
    Network(String),
    /// The server misbehaved; a sentence for people.
    Server(String),
    Disk(std::io::Error),
}

const TRIES: u32 = 4;

/// Fetch `[from, total)` of `src` into `part`, which already holds `from`
/// good bytes. `stop` is checked between ranges; `progress` gets the bytes
/// in the file so far.
pub async fn transfer(f: &dyn Fetcher, src: &Source, part: &Path, from: u64, stop: &(dyn Fn() -> bool + Send + Sync), progress: &(dyn Fn(u64) + Send + Sync)) -> Result<Outcome, TransferError> {
    if src.total == 0 || src.total > MAX_TOTAL {
        return Err(TransferError::Server("YouTube gave this song an impossible size.".into()));
    }
    let mut file = tokio::fs::OpenOptions::new().create(true).truncate(false).write(true).open(part).await.map_err(TransferError::Disk)?;
    file.set_len(from).await.map_err(TransferError::Disk)?;
    let mut off = from;
    let mut failures = 0u32;
    let mut restarted = false;
    progress(off);
    while off < src.total {
        if stop() {
            file.flush().await.map_err(TransferError::Disk)?;
            return Ok(Outcome::Stopped);
        }
        let a = off;
        let b = if src.ranges { (off + CHUNK).min(src.total) - 1 } else { src.total - 1 };
        let range = if src.ranges || a > 0 { Some((a, b)) } else { None };
        let got = match f.get(&src.url, &src.headers, range, src.total).await {
            Ok(g) => g,
            Err(FetchError::TooLong) => return Err(TransferError::Server("YouTube's media server sent more than the song.".into())),
            Err(FetchError::Transport(e)) => {
                failures += 1;
                if failures >= TRIES {
                    return Err(TransferError::Network(e));
                }
                tokio::time::sleep(Duration::from_millis(500 * failures as u64)).await;
                continue;
            }
        };
        match got.status {
            206 => {
                let cr = got.content_range.as_deref().and_then(parse_content_range);
                match cr {
                    Some((_, _, total)) if total != src.total => return Err(TransferError::Changed(total)),
                    Some((x, y, _)) if x == a && y == b && got.body.len() as u64 == b - a + 1 => {}
                    _ => {
                        failures += 1;
                        if failures >= TRIES {
                            return Err(TransferError::Server("YouTube's media server sent the wrong part of the song.".into()));
                        }
                        continue;
                    }
                }
                file.seek(SeekFrom::Start(a)).await.map_err(TransferError::Disk)?;
                file.write_all(&got.body).await.map_err(TransferError::Disk)?;
                off = b + 1;
                failures = 0;
                progress(off);
            }
            // The whole file: it replaces the partial one.
            200 => {
                if got.body.len() as u64 != src.total {
                    failures += 1;
                    if failures >= TRIES {
                        return Err(TransferError::Server("YouTube's media server sent an incomplete song.".into()));
                    }
                    continue;
                }
                file.set_len(0).await.map_err(TransferError::Disk)?;
                file.seek(SeekFrom::Start(0)).await.map_err(TransferError::Disk)?;
                file.write_all(&got.body).await.map_err(TransferError::Disk)?;
                off = src.total;
                progress(off);
            }
            // The range doesn't fit what the server has: start over, once.
            416 if !restarted && a > 0 => {
                restarted = true;
                file.set_len(0).await.map_err(TransferError::Disk)?;
                off = 0;
                progress(off);
            }
            403 | 410 => return Err(TransferError::Refused),
            st => {
                failures += 1;
                if failures >= TRIES {
                    return Err(TransferError::Server(format!("YouTube's media server answered {st}.")));
                }
                let wait = if st == 429 { 5000 } else { 1000 };
                tokio::time::sleep(Duration::from_millis(wait * failures as u64)).await;
            }
        }
    }
    file.flush().await.map_err(TransferError::Disk)?;
    file.sync_all().await.map_err(TransferError::Disk)?;
    Ok(Outcome::Done)
}

/// Before continuing a partial file: its last 4 KiB must be what the server
/// has at that place now. Anything else and the file starts over.
pub async fn tail_matches(f: &dyn Fetcher, src: &Source, part: &Path, len: u64) -> bool {
    if len < OVERLAP || len > src.total || !src.ranges {
        return false;
    }
    let a = len - OVERLAP;
    let mut mine = vec![0u8; OVERLAP as usize];
    let read = async {
        let mut file = tokio::fs::File::open(part).await?;
        file.seek(SeekFrom::Start(a)).await?;
        file.read_exact(&mut mine).await?;
        Ok::<(), std::io::Error>(())
    };
    if read.await.is_err() {
        return false;
    }
    match f.get(&src.url, &src.headers, Some((a, len - 1)), OVERLAP).await {
        Ok(g) => g.status == 206 && g.content_range.as_deref().and_then(parse_content_range) == Some((a, len - 1, src.total)) && g.body == mine,
        Err(_) => false,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use parking_lot::Mutex;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Serves `data`, with knobs for the server misbehaving.
    #[derive(Default)]
    pub struct FakeServer {
        pub data: Mutex<Vec<u8>>,
        /// Answer ranges with the wrong start.
        pub wrong_range: AtomicBool,
        /// Ignore Range and send the whole file with 200.
        pub ignore_range: AtomicBool,
        pub refuse: AtomicBool,
        pub requests: Mutex<Vec<Option<(u64, u64)>>>,
    }

    #[async_trait]
    impl Fetcher for FakeServer {
        async fn get(&self, _url: &str, _h: &[(String, String)], range: Option<(u64, u64)>, limit: u64) -> Result<Fetched, FetchError> {
            self.requests.lock().push(range);
            let data = self.data.lock().clone();
            if self.refuse.load(Ordering::SeqCst) {
                return Ok(Fetched { status: 403, content_range: None, body: vec![] });
            }
            let total = data.len() as u64;
            match range {
                Some((a, b)) if !self.ignore_range.load(Ordering::SeqCst) => {
                    let b = b.min(total - 1);
                    let shift = if self.wrong_range.load(Ordering::SeqCst) && a > 0 { 1 } else { 0 };
                    let (x, y) = (a - shift, b - shift);
                    let body = data[x as usize..=y as usize].to_vec();
                    if body.len() as u64 > limit {
                        return Err(FetchError::TooLong);
                    }
                    Ok(Fetched { status: 206, content_range: Some(format!("bytes {x}-{y}/{total}")), body })
                }
                _ => Ok(Fetched { status: 200, content_range: None, body: data }),
            }
        }
    }

    pub fn song(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 7 % 251) as u8).collect()
    }

    fn src(total: usize) -> Source {
        Source { url: "https://example.invalid/x".into(), headers: vec![], total: total as u64, ranges: true }
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("encore-dl-{}-{name}.part", std::process::id()))
    }

    #[tokio::test]
    async fn fetches_in_ranges_and_checks_them() {
        let data = song(CHUNK as usize * 2 + 1000);
        let s = FakeServer::default();
        *s.data.lock() = data.clone();
        let p = tmp("ranges");
        let r = transfer(&s, &src(data.len()), &p, 0, &|| false, &|_| {}).await.unwrap();
        assert_eq!(r, Outcome::Done);
        assert_eq!(std::fs::read(&p).unwrap(), data);
        assert_eq!(s.requests.lock().len(), 3);
        let _ = std::fs::remove_file(&p);
    }

    #[tokio::test]
    async fn a_wrong_content_range_is_never_written() {
        let data = song(CHUNK as usize * 2);
        let s = FakeServer::default();
        *s.data.lock() = data.clone();
        s.wrong_range.store(true, Ordering::SeqCst);
        let p = tmp("wrong");
        let r = transfer(&s, &src(data.len()), &p, 0, &|| false, &|_| {}).await;
        assert!(matches!(r, Err(TransferError::Server(_))), "{r:?}");
        assert_eq!(std::fs::metadata(&p).unwrap().len(), CHUNK, "only the first, correct range");
        let _ = std::fs::remove_file(&p);
    }

    #[tokio::test]
    async fn a_200_replaces_the_partial_file() {
        let data = song(CHUNK as usize + 10);
        let s = FakeServer::default();
        *s.data.lock() = data.clone();
        s.ignore_range.store(true, Ordering::SeqCst);
        let p = tmp("whole");
        std::fs::write(&p, vec![0xffu8; 5000]).unwrap();
        let r = transfer(&s, &src(data.len()), &p, 5000, &|| false, &|_| {}).await.unwrap();
        assert_eq!(r, Outcome::Done);
        assert_eq!(std::fs::read(&p).unwrap(), data);
        let _ = std::fs::remove_file(&p);
    }

    #[tokio::test]
    async fn resumes_only_when_the_tail_matches() {
        let data = song(CHUNK as usize + 50_000);
        let s = FakeServer::default();
        *s.data.lock() = data.clone();
        let p = tmp("resume");
        std::fs::write(&p, &data[..20_000]).unwrap();
        assert!(tail_matches(&s, &src(data.len()), &p, 20_000).await);
        // Same length, other bytes: the server now has another file.
        let mut other = data.clone();
        other[19_000] ^= 0xff;
        *s.data.lock() = other;
        assert!(!tail_matches(&s, &src(data.len()), &p, 20_000).await);
        let _ = std::fs::remove_file(&p);
    }

    #[tokio::test]
    async fn stops_between_ranges_and_keeps_the_part() {
        let data = song(CHUNK as usize * 3);
        let s = Arc::new(FakeServer::default());
        *s.data.lock() = data.clone();
        let p = tmp("stop");
        let n = Arc::new(Mutex::new(0u64));
        let n2 = n.clone();
        let r = transfer(s.as_ref(), &src(data.len()), &p, 0, &move || *n2.lock() >= CHUNK, &|b| *n.lock() = b).await.unwrap();
        assert_eq!(r, Outcome::Stopped);
        assert_eq!(std::fs::metadata(&p).unwrap().len(), CHUNK);
        let _ = std::fs::remove_file(&p);
    }

    #[tokio::test]
    async fn a_stale_url_is_reported() {
        let s = FakeServer::default();
        *s.data.lock() = song(1000);
        s.refuse.store(true, Ordering::SeqCst);
        let p = tmp("refused");
        assert!(matches!(transfer(&s, &src(1000), &p, 0, &|| false, &|_| {}).await, Err(TransferError::Refused)));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn parses_content_range() {
        assert_eq!(parse_content_range("bytes 0-1023/4096"), Some((0, 1023, 4096)));
        assert_eq!(parse_content_range("bytes */4096"), None);
    }
}
