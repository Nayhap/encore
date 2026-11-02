//! Lyrics chain (D12): YouTube Music timed lyrics, then LRCLIB `/api/get`
//! (exact) and `/api/search` (scored on title + artist + version, duration
//! within ±2 s), then YouTube Music plain lyrics, then LRCLIB plain.
//! Positive results are cached for 30 days, misses for 1 day. Lyrics kept
//! with a download (D23) come first and are all there is offline.

pub mod lrc;
pub mod matching;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use encore_core::model::*;
use encore_core::service::MusicService;
use encore_core::store::Store;
use serde::Deserialize;

use matching::{Candidate, Query};

const POSITIVE_TTL: i64 = 30 * 24 * 3600;
const NEGATIVE_TTL: i64 = 24 * 3600;
const SCOPE: &str = "lyrics";
const UA: &str = concat!("Encore/", env!("CARGO_PKG_VERSION"), " (YouTube Music desktop client; https://github.com/encore-app)");

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LrclibRecord {
    pub id: Option<i64>,
    pub track_name: Option<String>,
    pub artist_name: Option<String>,
    pub album_name: Option<String>,
    pub duration: Option<f64>,
    #[serde(default)]
    pub instrumental: bool,
    pub plain_lyrics: Option<String>,
    pub synced_lyrics: Option<String>,
}

#[derive(serde::Serialize, Deserialize)]
#[serde(untagged)]
enum CacheEntry {
    Hit(Lyrics),
    Miss { miss: bool },
}

pub struct LyricsService {
    service: Arc<dyn MusicService>,
    store: Option<Arc<Store>>,
    http: reqwest::Client,
    lrclib_base: String,
    /// Offline, nothing is fetched: only kept and cached lyrics are shown.
    online: AtomicBool,
}

impl LyricsService {
    pub fn new(service: Arc<dyn MusicService>, store: Option<Arc<Store>>) -> Self {
        Self::with_base(service, store, "https://lrclib.net")
    }

    pub fn with_base(service: Arc<dyn MusicService>, store: Option<Arc<Store>>, base: &str) -> Self {
        let http = reqwest::Client::builder().user_agent(UA).connect_timeout(Duration::from_secs(5)).timeout(Duration::from_secs(8)).build().expect("http");
        Self { service, store, http, lrclib_base: base.trim_end_matches('/').to_string(), online: AtomicBool::new(true) }
    }

    pub fn set_online(&self, on: bool) {
        self.online.store(on, Ordering::Relaxed);
    }

    /// Lyrics kept with a download: `Some(None)` when the providers had none.
    fn pinned(&self, id: &ItemId) -> Option<Option<Lyrics>> {
        self.store.as_ref()?.pinned_lyrics(id.remote()).ok().flatten()
    }

    fn cache_get(&self, id: &ItemId) -> Option<Option<Lyrics>> {
        let s = self.store.as_ref()?;
        match s.get::<CacheEntry>(SCOPE, id.as_str()).ok()?? {
            CacheEntry::Hit(l) => Some(Some(l)),
            CacheEntry::Miss { .. } => Some(None),
        }
    }

    fn cache_put(&self, id: &ItemId, l: &Option<Lyrics>) {
        if let Some(s) = &self.store {
            let _ = match l {
                Some(l) => s.put(SCOPE, id.as_str(), &CacheEntry::Hit(l.clone()), Some(POSITIVE_TTL)),
                None => s.put(SCOPE, id.as_str(), &CacheEntry::Miss { miss: true }, Some(NEGATIVE_TTL)),
            };
        }
    }

    pub fn forget(&self, id: &ItemId) {
        if let Some(s) = &self.store {
            let _ = s.delete(SCOPE, id.as_str());
        }
    }

    /// Best lyrics for a track, from cache or the provider chain.
    pub async fn get(&self, track: &Track) -> Option<Lyrics> {
        let online = self.online.load(Ordering::Relaxed);
        match self.pinned(&track.id) {
            Some(Some(l)) => return Some(l),
            Some(None) if !online => return None,
            _ => {}
        }
        if let Some(c) = self.cache_get(&track.id) {
            return c;
        }
        if !online {
            return None;
        }
        let r = self.fetch(track).await;
        match &r {
            Ok(l) => self.cache_put(&track.id, l),
            // Transport failures are not negative-cached.
            Err(_) => {}
        }
        r.ok().flatten()
    }

    /// The provider chain without any cache, for keeping lyrics with a
    /// download: `Ok(None)` means no provider has lyrics for this song,
    /// `Err` that one couldn't be asked (not worth remembering).
    pub async fn fetch_for_download(&self, track: &Track) -> Result<Option<Lyrics>, String> {
        self.fetch(track).await
    }

    async fn fetch(&self, track: &Track) -> Result<Option<Lyrics>, String> {
        let ctx = self.service.context();
        let yt = self.service.lyrics(&ctx, &track.id).await;
        let mut net_error = yt.is_err();
        let yt = yt.ok().flatten();
        if let Some(l) = &yt {
            if l.sync != LyricsSync::None {
                return Ok(Some(l.clone()));
            }
        }
        let q = Query::from_track(track);
        let lr = self.lrclib(&q, track).await;
        let (synced, plain) = match lr {
            Ok(x) => x,
            Err(_) => {
                net_error = true;
                (None, None)
            }
        };
        if let Some(l) = synced {
            return Ok(Some(l));
        }
        if let Some(l) = yt {
            return Ok(Some(l));
        }
        if let Some(l) = plain {
            return Ok(Some(l));
        }
        if net_error { Err("lyrics providers unreachable".into()) } else { Ok(None) }
    }

    /// Returns (synced, plain) LRCLIB results that pass the matching rules.
    async fn lrclib(&self, q: &Query, track: &Track) -> Result<(Option<Lyrics>, Option<Lyrics>), String> {
        let mut candidates: Vec<LrclibRecord> = Vec::new();
        // Exact /api/get first (needs album and duration for best results).
        let mut params: Vec<(&str, String)> = vec![("track_name", q.title_raw.clone()), ("artist_name", q.artist_raw.clone())];
        if let Some(a) = &q.album_raw {
            params.push(("album_name", a.clone()));
        }
        if let Some(d) = q.duration {
            params.push(("duration", format!("{}", d.round() as i64)));
        }
        let r = self.http.get(format!("{}/api/get", self.lrclib_base)).query(&params).send().await.map_err(|e| e.to_string())?;
        if r.status().is_success() {
            if let Ok(rec) = r.json::<LrclibRecord>().await {
                candidates.push(rec);
            }
        } else if r.status().as_u16() >= 500 {
            return Err(format!("lrclib {}", r.status()));
        }
        let usable = |c: &LrclibRecord| c.synced_lyrics.as_deref().is_some_and(|s| lrc::parse(s).len() >= 4);
        if !candidates.iter().any(|c| usable(c) && q.accepts(&Candidate::from(c))) {
            let r = self
                .http
                .get(format!("{}/api/search", self.lrclib_base))
                .query(&[("track_name", q.title_clean.as_str()), ("artist_name", q.artist_raw.as_str())])
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if r.status().is_success() {
                if let Ok(list) = r.json::<Vec<LrclibRecord>>().await {
                    candidates.extend(list.into_iter().take(20));
                }
            }
        }
        let scored: Vec<(f64, &LrclibRecord)> = candidates.iter().filter_map(|c| q.score(&Candidate::from(c)).map(|s| (s, c))).collect();
        let best_synced = matching::pick(scored.iter().filter(|(_, c)| usable(c)).copied().collect());
        let best_plain = matching::pick(scored.iter().filter(|(_, c)| c.plain_lyrics.as_deref().is_some_and(|s| s.lines().count() >= 4)).copied().collect());
        let synced = best_synced.and_then(|c| {
            let lines = lrc::parse(c.synced_lyrics.as_deref()?);
            // Reject timings that run past the song (wrong recording).
            if lines.len() < 4 {
                return None;
            }
            if let (Some(d), Some(last)) = (q.duration, lines.last()) {
                if last.t > d + 5.0 {
                    return None;
                }
            }
            let sync = if lines.iter().any(|l| l.words.is_some()) { LyricsSync::Word } else { LyricsSync::Line };
            Some(Lyrics { track_id: track.id.clone(), sync, lines, plain: None, source: "LRCLIB".into(), attribution: Some("Lyrics from LRCLIB".into()) })
        });
        let plain = best_plain.and_then(|c| {
            Some(Lyrics { track_id: track.id.clone(), sync: LyricsSync::None, lines: vec![], plain: Some(c.plain_lyrics.clone()?), source: "LRCLIB".into(), attribution: Some("Lyrics from LRCLIB".into()) })
        });
        Ok((synced, plain))
    }
}
