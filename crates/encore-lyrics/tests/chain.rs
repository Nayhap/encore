//! Chain order with a fake service and an unreachable LRCLIB.

use std::sync::Arc;

use async_trait::async_trait;
use encore_core::error::{AppError, Result};
use encore_core::model::*;
use encore_core::service::{Capabilities, MusicService, RequestContext};
use encore_lyrics::LyricsService;

/// The lyrics it has, and how often it was asked (each answer takes a
/// moment, as a request would).
/// Its session's epoch can move on, as after a re-import.
struct Fake(Option<Lyrics>, Arc<std::sync::atomic::AtomicUsize>, std::sync::atomic::AtomicU64);

fn fake(l: Option<Lyrics>) -> Arc<Fake> {
    Arc::new(Fake(l, Default::default(), 1.into()))
}

fn track() -> Track {
    Track { id: ItemId::ytm("abc"), title: "Song".into(), artists: vec![Credit { name: "Artist".into(), id: None }], album: None, duration: Some(200.0), explicit: false, art: None, kind: TrackKind::Song, plays: None, liked: None, playable: true, counterpart: None }
}

#[async_trait]
impl MusicService for Fake {
    fn service(&self) -> ServiceId {
        ServiceId::YoutubeMusic
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }
    fn context(&self) -> RequestContext {
        RequestContext { account: "test".into(), epoch: self.2.load(std::sync::atomic::Ordering::SeqCst) }
    }
    async fn lyrics(&self, _ctx: &RequestContext, _id: &ItemId) -> Result<Option<Lyrics>> {
        self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        Ok(self.0.clone())
    }
    async fn home(&self, _ctx: &RequestContext) -> Result<Page> {
        Err(AppError::internal("unused"))
    }
    async fn explore(&self, _ctx: &RequestContext) -> Result<Page> {
        Err(AppError::internal("unused"))
    }
    async fn browse_page(&self, _ctx: &RequestContext, _b: &str, _p: Option<&str>) -> Result<Page> {
        Err(AppError::internal("unused"))
    }
    async fn more_shelves(&self, _ctx: &RequestContext, _c: &str) -> Result<Page> {
        Err(AppError::internal("unused"))
    }
    async fn search(&self, _ctx: &RequestContext, _q: &str, _f: Option<SearchFilter>) -> Result<SearchResults> {
        Err(AppError::internal("unused"))
    }
    async fn search_more(&self, _ctx: &RequestContext, _c: &str) -> Result<SearchResults> {
        Err(AppError::internal("unused"))
    }
    async fn suggestions(&self, _ctx: &RequestContext, _q: &str) -> Result<Vec<String>> {
        Err(AppError::internal("unused"))
    }
    async fn collection(&self, _ctx: &RequestContext, _id: &ItemId) -> Result<Collection> {
        Err(AppError::internal("unused"))
    }
    async fn collection_more(&self, _ctx: &RequestContext, _c: &str) -> Result<(Vec<Entry>, Option<String>)> {
        Err(AppError::internal("unused"))
    }
    async fn artist(&self, _ctx: &RequestContext, _id: &ItemId) -> Result<Artist> {
        Err(AppError::internal("unused"))
    }
    async fn library(&self, _ctx: &RequestContext, _s: LibrarySection) -> Result<LibraryPage> {
        Err(AppError::internal("unused"))
    }
    async fn library_more(&self, _ctx: &RequestContext, _s: LibrarySection, _c: &str) -> Result<LibraryPage> {
        Err(AppError::internal("unused"))
    }
    async fn track(&self, _ctx: &RequestContext, _id: &ItemId) -> Result<Track> {
        Err(AppError::internal("unused"))
    }
    async fn up_next(&self, _ctx: &RequestContext, _s: &RadioSeed, _c: Option<&str>) -> Result<UpNext> {
        Err(AppError::internal("unused"))
    }
    async fn set_like(&self, _ctx: &RequestContext, _id: &ItemId, _l: bool) -> Result<MutationOutcome> {
        Err(AppError::internal("unused"))
    }
    async fn set_saved(&self, _ctx: &RequestContext, _p: &str, _s: bool) -> Result<MutationOutcome> {
        Err(AppError::internal("unused"))
    }
    async fn set_subscribed(&self, _ctx: &RequestContext, _c: &str, _s: bool) -> Result<MutationOutcome> {
        Err(AppError::internal("unused"))
    }
    async fn playlist_add(&self, _ctx: &RequestContext, _p: &str, _t: &[ItemId], _d: bool) -> Result<MutationOutcome> {
        Err(AppError::internal("unused"))
    }
    async fn playlist_remove(&self, _ctx: &RequestContext, _p: &str, _e: &[(ItemId, String)]) -> Result<MutationOutcome> {
        Err(AppError::internal("unused"))
    }
}

fn line(t: f64, s: &str) -> LyricLine {
    LyricLine { t, end: None, text: s.into(), words: None }
}

#[tokio::test]
async fn youtube_timed_wins_and_is_cached() {
    let timed = Lyrics { track_id: ItemId::ytm("abc"), sync: LyricsSync::Line, lines: vec![line(1.0, "a"), line(2.0, "b")], plain: None, source: "YouTube Music".into(), attribution: None };
    let store = Arc::new(encore_core::store::Store::open_memory().unwrap());
    let s = LyricsService::with_base(fake(Some(timed.clone())), Some(store.clone()), "http://127.0.0.1:9");
    assert_eq!(s.get(&track()).await, Some(timed.clone()));
    // Second call is served from the cache even with a service that has nothing.
    let s2 = LyricsService::with_base(fake(None), Some(store), "http://127.0.0.1:9");
    assert_eq!(s2.get(&track()).await, Some(timed));
}

#[tokio::test]
async fn plain_fallback_when_lrclib_is_down() {
    let plain = Lyrics { track_id: ItemId::ytm("abc"), sync: LyricsSync::None, lines: vec![], plain: Some("words".into()), source: "YouTube Music".into(), attribution: None };
    let store = Arc::new(encore_core::store::Store::open_memory().unwrap());
    let s = LyricsService::with_base(fake(Some(plain.clone())), Some(store.clone()), "http://127.0.0.1:9");
    assert_eq!(s.get(&track()).await, Some(plain));
}

#[tokio::test]
async fn misses_are_not_cached_when_providers_fail() {
    let store = Arc::new(encore_core::store::Store::open_memory().unwrap());
    let s = LyricsService::with_base(fake(None), Some(store.clone()), "http://127.0.0.1:9");
    assert_eq!(s.get(&track()).await, None);
    assert_eq!(store.count("lyrics"), 0, "a transport failure must not be negative-cached");
}

#[tokio::test]
async fn lookups_of_one_song_at_once_share_one_walk() {
    let timed = Lyrics { track_id: ItemId::ytm("abc"), sync: LyricsSync::Line, lines: vec![line(1.0, "a"), line(2.0, "b")], plain: None, source: "YouTube Music".into(), attribution: None };
    let f = fake(Some(timed.clone()));
    let s = Arc::new(LyricsService::with_base(f.clone(), None, "http://127.0.0.1:9"));
    let t = track();
    let (a, b) = tokio::join!(s.get(&t), s.fetch_for_download(&t));
    assert_eq!((a, b), (Some(timed.clone()), Ok(Some(timed.clone()))));
    assert_eq!(f.1.load(std::sync::atomic::Ordering::SeqCst), 1, "asked once");
    // One that gives up doesn't stop the other.
    let s2 = s.clone();
    let quitter = tokio::spawn(async move { s2.get(&track()).await });
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    quitter.abort();
    assert_eq!(s.get(&track()).await, Some(timed));
}

#[tokio::test]
async fn another_session_of_the_same_account_asks_again() {
    let timed = Lyrics { track_id: ItemId::ytm("abc"), sync: LyricsSync::Line, lines: vec![line(1.0, "a"), line(2.0, "b")], plain: None, source: "YouTube Music".into(), attribution: None };
    let f = fake(Some(timed.clone()));
    let s = Arc::new(LyricsService::with_base(f.clone(), None, "http://127.0.0.1:9"));
    let s2 = s.clone();
    let first = tokio::spawn(async move { s2.get(&track()).await });
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    f.2.store(2, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(s.get(&track()).await, Some(timed.clone()));
    assert_eq!(first.await.unwrap(), Some(timed));
    assert_eq!(f.1.load(std::sync::atomic::Ordering::SeqCst), 2, "the new session's lookup doesn't join the old one");
}
