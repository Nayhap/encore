//! YouTube Music via InnerTube: the `MusicService` implementation.

pub mod account;
pub mod client;
pub mod pages;
pub mod parse;
pub mod scrub;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use encore_auth::Session;
use encore_core::error::{AppError, ErrorCode, Result};
use encore_core::model::*;
use encore_core::service::{Capabilities, MusicService, RequestContext};
use parking_lot::Mutex;
use serde_json::{Value, json};

pub use client::{Client, InnerTube};
use pages::{Cont, Parsed};

const CURSOR_TTL: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Debug)]
enum CursorKind {
    Shelves,
    Search { query: String },
    Collection { offset: usize },
    Library {
        #[allow(dead_code)]
        section: LibrarySection,
    },
    Radio { playlist_id: Option<String> },
}

#[derive(Clone)]
struct CursorRec {
    kind: CursorKind,
    cont: Cont,
    epoch: u64,
    at: Instant,
}

/// In-memory continuation cursors with a TTL, bound to the account epoch (D10).
#[derive(Default)]
struct Cursors {
    map: Mutex<HashMap<String, CursorRec>>,
}

impl Cursors {
    fn put(&self, kind: CursorKind, cont: Option<Cont>, epoch: u64) -> Option<String> {
        let cont = cont?;
        let id = uuid::Uuid::new_v4().simple().to_string();
        let mut m = self.map.lock();
        m.retain(|_, r| r.at.elapsed() < CURSOR_TTL);
        m.insert(id.clone(), CursorRec { kind, cont, epoch, at: Instant::now() });
        Some(id)
    }
    fn take(&self, id: &str, epoch: u64) -> Result<CursorRec> {
        let m = self.map.lock();
        let r = m.get(id).cloned().ok_or_else(|| AppError::new(ErrorCode::Stale, "This list expired. Reload the page."))?;
        if r.epoch != epoch || r.at.elapsed() > CURSOR_TTL {
            return Err(AppError::new(ErrorCode::Stale, "This list belongs to another session. Reload the page."));
        }
        Ok(r)
    }
}

pub struct YtMusic {
    pub it: Arc<InnerTube>,
    cursors: Cursors,
}

fn parse_err(e: String) -> AppError {
    AppError::parse(format!("Could not read YouTube Music's answer ({e})."))
}

fn search_params(f: SearchFilter) -> &'static str {
    match f {
        SearchFilter::Songs => "EgWKAQIIAWoMEA4QChADEAQQCRAF",
        SearchFilter::Videos => "EgWKAQIQAWoMEA4QChADEAQQCRAF",
        SearchFilter::Albums => "EgWKAQIYAWoMEA4QChADEAQQCRAF",
        SearchFilter::Artists => "EgWKAQIgAWoMEA4QChADEAQQCRAF",
        SearchFilter::CommunityPlaylists => "EgeKAQQoAEABagwQDhAKEAMQBBAJEAU=",
        SearchFilter::FeaturedPlaylists => "EgeKAQQoADgBagwQDhAKEAMQBBAJEAU=",
    }
}

fn library_browse(s: LibrarySection) -> &'static str {
    match s {
        LibrarySection::Playlists => "FEmusic_liked_playlists",
        LibrarySection::Albums => "FEmusic_liked_albums",
        LibrarySection::Artists => "FEmusic_library_corpus_track_artists",
        LibrarySection::Subscriptions => "FEmusic_library_corpus_artists",
        LibrarySection::Songs => "FEmusic_liked_videos",
        LibrarySection::Liked => "VLLM",
        LibrarySection::History => "FEmusic_history",
    }
}

fn status_ok(v: &Value) -> MutationOutcome {
    match v.get("status").and_then(Value::as_str) {
        Some("STATUS_SUCCEEDED") | None => MutationOutcome::Done,
        Some(_) => MutationOutcome::Rejected,
    }
}

fn write_result(r: Result<Value>) -> Result<MutationOutcome> {
    match r {
        Ok(v) => Ok(status_ok(&v)),
        // A timeout or interrupted response leaves the server state unknown.
        Err(e) if e.code == ErrorCode::Network => Ok(MutationOutcome::OutcomeUnknown),
        Err(e) => Err(e),
    }
}

impl YtMusic {
    pub fn new(it: Arc<InnerTube>) -> Self {
        Self { it, cursors: Cursors::default() }
    }

    fn check(&self, ctx: &RequestContext) -> Result<()> {
        if ctx.epoch != self.it.epoch() { Err(AppError::stale()) } else { Ok(()) }
    }

    fn require_auth(&self) -> Result<()> {
        if self.it.signed_in() { Ok(()) } else { Err(AppError::auth_required()) }
    }

    async fn browse(&self, browse_id: &str, params: Option<&str>) -> Result<Value> {
        let mut body = json!({ "browseId": browse_id });
        if let Some(p) = params {
            body["params"] = json!(p);
        }
        self.it.post(Client::WebRemix, "browse", body, &[]).await
    }

    async fn continue_browse(&self, endpoint: &str, cont: &Cont, extra: Value) -> Result<Value> {
        match cont {
            Cont::Query(t) => self.it.post(Client::WebRemix, endpoint, extra, &[("ctoken", t), ("continuation", t), ("type", "next")]).await,
            Cont::Body(t) => {
                let mut b = extra;
                b["continuation"] = json!(t);
                self.it.post(Client::WebRemix, endpoint, b, &[]).await
            }
        }
    }

    fn page_with_cursor(&self, p: Parsed<Page>, epoch: u64) -> Page {
        let mut page = p.value;
        page.cursor = self.cursors.put(CursorKind::Shelves, p.cont, epoch);
        page
    }

    /// `account/accounts_list` for the signed-in cookies: the channel picker.
    pub async fn accounts(&self) -> Result<Vec<pages::AccountEntry>> {
        if !self.it.has_cookies() {
            return Err(AppError::auth_required());
        }
        let v = self.it.post(Client::WebRemix, "account/accounts_list", json!({}), &[]).await?;
        let a = pages::accounts(&v);
        if a.is_empty() {
            return Err(AppError::new(ErrorCode::SessionExpired, "YouTube Music did not list any account for these cookies. The session is probably signed out.").with_action("reimport"));
        }
        Ok(a)
    }

    /// Validate the selected channel by loading a private surface (Liked music).
    pub async fn validate(&self) -> Result<usize> {
        let v = self.browse("VLLM", None).await?;
        if parse::is_signed_out(&v) {
            return Err(AppError::session_expired());
        }
        let p = pages::collection(&v, &ItemId::ytm("VLLM")).map_err(parse_err)?;
        Ok(p.value.entries.len())
    }

    pub fn session(&self) -> Option<Session> {
        self.it.session()
    }

    /// Raw call for fixture recording (anonymous only).
    pub async fn raw(&self, endpoint: &str, body: Value) -> Result<Value> {
        self.it.post(Client::WebRemix, endpoint, body, &[]).await
    }
    pub async fn raw_android(&self, endpoint: &str, body: Value) -> Result<Value> {
        self.it.post(Client::AndroidMusic, endpoint, body, &[]).await
    }
}

#[async_trait]
impl MusicService for YtMusic {
    fn service(&self) -> ServiceId {
        ServiceId::YoutubeMusic
    }

    fn capabilities(&self) -> Capabilities {
        let s = self.it.signed_in();
        Capabilities { signed_in: s, library: s, mutations: s, radio: true, lyrics: true }
    }

    fn context(&self) -> RequestContext {
        let account = self.it.session().map(|s| s.account_key()).unwrap_or_else(|| "anonymous".into());
        RequestContext { account, epoch: self.it.epoch() }
    }

    async fn home(&self, ctx: &RequestContext) -> Result<Page> {
        self.check(ctx)?;
        let v = self.browse("FEmusic_home", None).await?;
        let p = pages::browse_page(&v).map_err(parse_err)?;
        Ok(self.page_with_cursor(p, ctx.epoch))
    }

    async fn explore(&self, ctx: &RequestContext) -> Result<Page> {
        self.check(ctx)?;
        let v = self.browse("FEmusic_explore", None).await?;
        let p = pages::browse_page(&v).map_err(parse_err)?;
        Ok(self.page_with_cursor(p, ctx.epoch))
    }

    async fn browse_page(&self, ctx: &RequestContext, browse_id: &str, params: Option<&str>) -> Result<Page> {
        self.check(ctx)?;
        if !(browse_id.starts_with("FE") || browse_id.starts_with("MPAD") || browse_id.starts_with("MPTR") || browse_id.starts_with("UC") || browse_id.starts_with("VL")) {
            return Err(AppError::invalid("That page is not supported."));
        }
        let v = self.browse(browse_id, params).await?;
        let p = pages::browse_page(&v).map_err(parse_err)?;
        Ok(self.page_with_cursor(p, ctx.epoch))
    }

    async fn more_shelves(&self, ctx: &RequestContext, cursor: &str) -> Result<Page> {
        self.check(ctx)?;
        let c = self.cursors.take(cursor, ctx.epoch)?;
        let v = self.continue_browse("browse", &c.cont, json!({})).await?;
        let p = pages::shelves_continuation(&v).map_err(parse_err)?;
        Ok(Page { title: None, shelves: p.value, chips: vec![], cursor: self.cursors.put(CursorKind::Shelves, p.cont, ctx.epoch) })
    }

    async fn search(&self, ctx: &RequestContext, query: &str, filter: Option<SearchFilter>) -> Result<SearchResults> {
        self.check(ctx)?;
        let q = query.trim();
        if q.is_empty() {
            return Ok(SearchResults { query: String::new(), ..Default::default() });
        }
        let mut body = json!({ "query": q });
        if let Some(f) = filter {
            body["params"] = json!(search_params(f));
        }
        let v = self.it.post(Client::WebRemix, "search", body, &[]).await?;
        let p = pages::search(&v, q).map_err(parse_err)?;
        let mut r = p.value;
        if filter.is_some() {
            r.cursor = self.cursors.put(CursorKind::Search { query: q.into() }, p.cont, ctx.epoch);
        }
        Ok(r)
    }

    async fn search_more(&self, ctx: &RequestContext, cursor: &str) -> Result<SearchResults> {
        self.check(ctx)?;
        let c = self.cursors.take(cursor, ctx.epoch)?;
        let CursorKind::Search { query } = c.kind.clone() else { return Err(AppError::invalid("Not a search cursor.")) };
        let v = self.continue_browse("search", &c.cont, json!({})).await?;
        let p = pages::search_continuation(&v, &query).map_err(parse_err)?;
        let mut r = p.value;
        r.cursor = self.cursors.put(CursorKind::Search { query }, p.cont, ctx.epoch);
        Ok(r)
    }

    async fn suggestions(&self, ctx: &RequestContext, query: &str) -> Result<Vec<String>> {
        self.check(ctx)?;
        if query.trim().is_empty() {
            return Ok(vec![]);
        }
        let v = self.it.post(Client::WebRemix, "music/get_search_suggestions", json!({ "input": query }), &[]).await?;
        Ok(pages::suggestions(&v))
    }

    async fn collection(&self, ctx: &RequestContext, id: &ItemId) -> Result<Collection> {
        self.check(ctx)?;
        let mut bid = id.remote().to_string();
        if !(bid.starts_with("MPRE") || bid.starts_with("VL") || bid.starts_with("FE")) {
            bid = format!("VL{bid}");
        }
        if bid == "VLLM" {
            self.require_auth()?;
        }
        let v = self.browse(&bid, None).await?;
        let p = pages::collection(&v, &ItemId::ytm(&bid)).map_err(parse_err)?;
        let n = p.value.entries.len();
        let mut c = p.value;
        c.cursor = self.cursors.put(CursorKind::Collection { offset: n }, p.cont, ctx.epoch);
        Ok(c)
    }

    async fn collection_more(&self, ctx: &RequestContext, cursor: &str) -> Result<(Vec<Entry>, Option<String>)> {
        self.check(ctx)?;
        let c = self.cursors.take(cursor, ctx.epoch)?;
        let CursorKind::Collection { offset } = c.kind else { return Err(AppError::invalid("Not a playlist cursor.")) };
        let v = self.continue_browse("browse", &c.cont, json!({})).await?;
        let p = pages::collection_continuation(&v, offset).map_err(parse_err)?;
        let n = offset + p.value.len();
        // Stop if the server repeats the same token (no progress).
        let same = matches!((&p.cont, &c.cont), (Some(a), b) if a == b);
        let next = if same || p.value.is_empty() { None } else { self.cursors.put(CursorKind::Collection { offset: n }, p.cont, ctx.epoch) };
        Ok((p.value, next))
    }

    async fn artist(&self, ctx: &RequestContext, id: &ItemId) -> Result<Artist> {
        self.check(ctx)?;
        let v = self.browse(id.remote(), None).await?;
        Ok(pages::artist(&v, id).map_err(parse_err)?.value)
    }

    async fn library(&self, ctx: &RequestContext, section: LibrarySection) -> Result<LibraryPage> {
        self.check(ctx)?;
        self.require_auth()?;
        let v = self.browse(library_browse(section), None).await?;
        if parse::is_signed_out(&v) {
            return Err(AppError::session_expired());
        }
        let p = pages::library(&v, section).map_err(parse_err)?;
        let kind = if section == LibrarySection::Liked { CursorKind::Collection { offset: p.value.len() } } else { CursorKind::Library { section } };
        Ok(LibraryPage { section, items: p.value, cursor: self.cursors.put(kind, p.cont, ctx.epoch) })
    }

    async fn library_more(&self, ctx: &RequestContext, section: LibrarySection, cursor: &str) -> Result<LibraryPage> {
        self.check(ctx)?;
        self.require_auth()?;
        let c = self.cursors.take(cursor, ctx.epoch)?;
        let v = self.continue_browse("browse", &c.cont, json!({})).await?;
        match c.kind {
            CursorKind::Collection { offset } => {
                let p = pages::collection_continuation(&v, offset).map_err(parse_err)?;
                let n = offset + p.value.len();
                let items: Vec<ShelfItem> = p.value.into_iter().map(|e| ShelfItem::Track(e.track)).collect();
                let cursor = if items.is_empty() { None } else { self.cursors.put(CursorKind::Collection { offset: n }, p.cont, ctx.epoch) };
                Ok(LibraryPage { section, items, cursor })
            }
            _ => {
                let p = pages::library_continuation(&v).map_err(parse_err)?;
                let cursor = if p.value.is_empty() { None } else { self.cursors.put(CursorKind::Library { section }, p.cont, ctx.epoch) };
                Ok(LibraryPage { section, items: p.value, cursor })
            }
        }
    }

    async fn track(&self, ctx: &RequestContext, id: &ItemId) -> Result<Track> {
        self.check(ctx)?;
        let v = self.it.post(Client::WebRemix, "next", json!({ "videoId": id.remote(), "isAudioOnly": true }), &[]).await?;
        if let Ok(n) = pages::next(&v) {
            if let Some(t) = n.tracks.into_iter().find(|t| &t.id == id) {
                return Ok(t);
            }
        }
        // Signed-in answers sometimes omit the panel without a playlist; ask
        // for the track's radio, whose first entry is the track itself.
        let v = self
            .it
            .post(Client::WebRemix, "next", json!({ "videoId": id.remote(), "playlistId": format!("RDAMVM{}", id.remote()), "isAudioOnly": true, "params": "wAEB" }), &[])
            .await?;
        let n = pages::next(&v).map_err(parse_err)?;
        n.tracks.into_iter().find(|t| &t.id == id).ok_or_else(|| AppError::not_found("YouTube Music could not find that song."))
    }

    async fn up_next(&self, ctx: &RequestContext, seed: &RadioSeed, cursor: Option<&str>) -> Result<UpNext> {
        self.check(ctx)?;
        let base = |pl: Option<&str>| {
            let mut b = json!({ "isAudioOnly": true, "enablePersistentPlaylistPanel": true, "tunerSettingValue": "AUTOMIX_SETTING_NORMAL" });
            if let Some(p) = pl {
                b["playlistId"] = json!(p);
            }
            b
        };
        let v = if let Some(cid) = cursor {
            let c = self.cursors.take(cid, ctx.epoch)?;
            let CursorKind::Radio { playlist_id } = c.kind.clone() else { return Err(AppError::invalid("Not a radio cursor.")) };
            let (Cont::Query(t) | Cont::Body(t)) = &c.cont;
            let mut b = base(playlist_id.as_deref());
            b["continuation"] = json!(t);
            self.it.post(Client::WebRemix, "next", b, &[]).await?
        } else {
            let mut b = match seed {
                RadioSeed::Track { id } => {
                    let mut b = base(Some(&format!("RDAMVM{}", id.remote())));
                    b["videoId"] = json!(id.remote());
                    b["params"] = json!("wAEB");
                    b
                }
                RadioSeed::Playlist { playlist_id } => {
                    let pl = playlist_id.strip_prefix("VL").unwrap_or(playlist_id);
                    base(Some(pl))
                }
            };
            b["watchEndpointMusicSupportedConfigs"] = json!({ "watchEndpointMusicConfig": { "musicVideoType": "MUSIC_VIDEO_TYPE_ATV" } });
            self.it.post(Client::WebRemix, "next", b, &[]).await?
        };
        let n = pages::next(&v).map_err(parse_err)?;
        let playlist_id = n.playlist_id.clone().or_else(|| match seed {
            RadioSeed::Playlist { playlist_id } => Some(playlist_id.clone()),
            RadioSeed::Track { id } => Some(format!("RDAMVM{}", id.remote())),
        });
        let cursor = self.cursors.put(CursorKind::Radio { playlist_id: playlist_id.clone() }, n.cont.map(Cont::Body), ctx.epoch);
        Ok(UpNext { tracks: n.tracks, playlist_id, cursor })
    }

    async fn set_like(&self, ctx: &RequestContext, id: &ItemId, liked: bool) -> Result<MutationOutcome> {
        self.check(ctx)?;
        self.require_auth()?;
        let ep = if liked { "like/like" } else { "like/removelike" };
        write_result(self.it.post(Client::WebRemix, ep, json!({ "target": { "videoId": id.remote() } }), &[]).await)
    }

    async fn set_saved(&self, ctx: &RequestContext, playlist_id: &str, saved: bool) -> Result<MutationOutcome> {
        self.check(ctx)?;
        self.require_auth()?;
        let ep = if saved { "like/like" } else { "like/removelike" };
        let pl = playlist_id.strip_prefix("VL").unwrap_or(playlist_id);
        write_result(self.it.post(Client::WebRemix, ep, json!({ "target": { "playlistId": pl } }), &[]).await)
    }

    async fn set_subscribed(&self, ctx: &RequestContext, channel_id: &str, subscribed: bool) -> Result<MutationOutcome> {
        self.check(ctx)?;
        self.require_auth()?;
        let ep = if subscribed { "subscription/subscribe" } else { "subscription/unsubscribe" };
        write_result(self.it.post(Client::WebRemix, ep, json!({ "channelIds": [channel_id] }), &[]).await)
    }

    async fn playlist_add(&self, ctx: &RequestContext, playlist_id: &str, tracks: &[ItemId]) -> Result<MutationOutcome> {
        self.check(ctx)?;
        self.require_auth()?;
        let pl = playlist_id.strip_prefix("VL").unwrap_or(playlist_id);
        let actions: Vec<Value> = tracks.iter().map(|t| json!({ "action": "ACTION_ADD_VIDEO", "addedVideoId": t.remote() })).collect();
        write_result(self.it.post(Client::WebRemix, "browse/edit_playlist", json!({ "playlistId": pl, "actions": actions }), &[]).await)
    }

    async fn playlist_remove(&self, ctx: &RequestContext, playlist_id: &str, entries: &[(ItemId, String)]) -> Result<MutationOutcome> {
        self.check(ctx)?;
        self.require_auth()?;
        let pl = playlist_id.strip_prefix("VL").unwrap_or(playlist_id);
        let actions: Vec<Value> = entries.iter().map(|(t, s)| json!({ "action": "ACTION_REMOVE_VIDEO", "removedVideoId": t.remote(), "setVideoId": s })).collect();
        write_result(self.it.post(Client::WebRemix, "browse/edit_playlist", json!({ "playlistId": pl, "actions": actions }), &[]).await)
    }

    async fn lyrics(&self, ctx: &RequestContext, id: &ItemId) -> Result<Option<Lyrics>> {
        self.check(ctx)?;
        let v = self.it.post(Client::WebRemix, "next", json!({ "videoId": id.remote(), "isAudioOnly": true }), &[]).await?;
        let n = pages::next(&v).map_err(parse_err)?;
        let Some(bid) = n.lyrics_browse_id else { return Ok(None) };
        // Timed lines come from the Android Music client (anonymous is fine).
        if let Ok(a) = self.it.post(Client::AndroidMusic, "browse", json!({ "browseId": bid }), &[]).await {
            if let Some(l) = pages::timed_lyrics(&a, id) {
                return Ok(Some(l));
            }
        }
        let w = self.browse(&bid, None).await?;
        Ok(pages::plain_lyrics(&w, id))
    }
}
