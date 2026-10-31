//! YouTube Music via InnerTube: the `MusicService` implementation.

pub mod account;
pub mod client;
pub mod history;
pub mod pages;
pub mod parse;
pub mod playlists;
pub mod scrub;
pub mod tracking;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use encore_auth::Session;
use encore_core::error::{AppError, ErrorCode, Result};
use encore_core::model::*;
use encore_core::service::{Capabilities, LoudnessSource, More, MusicService, PlayReport, PlayReporter, RequestContext};
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
    History,
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

/// History removal tokens behind opaque row ids: in memory only, bound to
/// the account epoch, expiring with the cursors (D19).
#[derive(Default)]
struct RowTokens {
    map: Mutex<HashMap<String, (String, u64, Instant)>>,
}

impl RowTokens {
    fn put(&self, token: String, epoch: u64) -> String {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let mut m = self.map.lock();
        m.retain(|_, r| r.2.elapsed() < CURSOR_TTL);
        m.insert(id.clone(), (token, epoch, Instant::now()));
        id
    }
    fn get(&self, id: &str, epoch: u64) -> Option<String> {
        self.map.lock().get(id).filter(|r| r.1 == epoch && r.2.elapsed() < CURSOR_TTL).map(|r| r.0.clone())
    }
    fn forget(&self, id: &str) {
        self.map.lock().remove(id);
    }
}

const PAIR_TTL: Duration = Duration::from_secs(30 * 60);
const NO_PAIR_TTL: Duration = Duration::from_secs(10 * 60);

pub struct YtMusic {
    pub it: Arc<InnerTube>,
    cursors: Cursors,
    rows: RowTokens,
    /// Song/video pairs by (account, track id); absences expire sooner.
    pairs: Mutex<HashMap<(String, String), (Option<Counterpart>, Instant)>>,
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

fn norm(s: &str) -> String {
    s.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A title without the usual version labels: "Song (Official Video)" is "song".
fn norm_title(s: &str) -> String {
    let mut t = norm(s);
    for tag in ["official music video", "official video", "official audio", "official visualizer", "lyric video", "lyrics", "audio", "video", "hd", "4k", "remastered"] {
        for (o, c) in [("(", ")"), ("[", "]")] {
            t = t.replace(&format!("{o}{tag}{c}"), "");
        }
    }
    norm(t.trim_end_matches(|c: char| c == '-' || c.is_whitespace()))
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
        Self { it, cursors: Cursors::default(), rows: RowTokens::default(), pairs: Mutex::new(HashMap::new()) }
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

    fn history_page(&self, raw: history::RawHistory, epoch: u64) -> HistoryPage {
        let groups = raw
            .groups
            .into_iter()
            .map(|g| HistoryGroup {
                title: g.title,
                rows: g
                    .rows
                    .into_iter()
                    .map(|(track, token)| {
                        let removable = token.is_some();
                        let row_id = match token {
                            Some(t) => self.rows.put(t, epoch),
                            None => uuid::Uuid::new_v4().simple().to_string(),
                        };
                        HistoryRow { row_id, track, removable }
                    })
                    .collect(),
            })
            .collect();
        HistoryPage { groups, notice: raw.notice, paused: raw.paused, cursor: self.cursors.put(CursorKind::History, raw.cont, epoch) }
    }

    async fn edit(&self, playlist_id: &str, actions: Vec<Value>) -> Result<MutationOutcome> {
        if actions.is_empty() {
            return Ok(MutationOutcome::Done);
        }
        match self.it.post(Client::WebRemix, "browse/edit_playlist", playlists::edit_body(playlist_id, actions), &[]).await {
            Ok(v) => Ok(playlists::edit_outcome(&v)),
            Err(e) if e.code == ErrorCode::Network => Ok(MutationOutcome::OutcomeUnknown),
            Err(e) => Err(e),
        }
    }

    /// Last resort for a pair: a search for the other kind, accepted only
    /// on the same title, the same first artist and a length within 5 s.
    async fn counterpart_by_search(&self, ctx: &RequestContext, t: &Track) -> Result<Option<Counterpart>> {
        let (filter, want) = match t.kind {
            TrackKind::Song => (SearchFilter::Videos, TrackKind::Video),
            TrackKind::Video => (SearchFilter::Songs, TrackKind::Song),
            _ => return Ok(None),
        };
        let (Some(artist), Some(dur)) = (t.artists.first(), t.duration) else { return Ok(None) };
        let r = self.search(ctx, &format!("{} {}", t.title, artist.name), Some(filter)).await?;
        let pool = if want == TrackKind::Video { r.videos } else { r.songs };
        let title = norm_title(&t.title);
        let found = pool.into_iter().find(|c| {
            c.id != t.id
                && norm_title(&c.title) == title
                && c.artists.first().is_some_and(|a| norm(&a.name) == norm(&artist.name))
                && c.duration.is_some_and(|d| (d - dur).abs() <= 5.0)
        });
        Ok(found.map(|mut track| {
            track.kind = want;
            track.counterpart = Some(t.id.clone());
            Counterpart { of: t.id.clone(), track, segments: vec![] }
        }))
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
        let (entries, more) = self.collection_more_checked(ctx, cursor).await?;
        Ok((entries, match more {
            More::Next(c) => Some(c),
            More::Ended | More::Stalled => None,
        }))
    }

    async fn collection_more_checked(&self, ctx: &RequestContext, cursor: &str) -> Result<(Vec<Entry>, More)> {
        self.check(ctx)?;
        let c = self.cursors.take(cursor, ctx.epoch)?;
        let CursorKind::Collection { offset } = c.kind else { return Err(AppError::invalid("Not a playlist cursor.")) };
        let v = self.continue_browse("browse", &c.cont, json!({})).await?;
        let p = pages::collection_continuation(&v, offset).map_err(parse_err)?;
        let n = offset + p.value.len();
        // Stop if the server repeats the same token or sends an empty page
        // with another continuation (no progress): the list may go on.
        let same = matches!((&p.cont, &c.cont), (Some(a), b) if a == b);
        let more = if same || (p.value.is_empty() && p.cont.is_some()) {
            More::Stalled
        } else {
            match self.cursors.put(CursorKind::Collection { offset: n }, p.cont, ctx.epoch) {
                Some(next) => More::Next(next),
                None => More::Ended,
            }
        };
        Ok((p.value, more))
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

    async fn playlist_add(&self, ctx: &RequestContext, playlist_id: &str, tracks: &[ItemId], allow_duplicates: bool) -> Result<MutationOutcome> {
        self.check(ctx)?;
        self.require_auth()?;
        let ids: Vec<&str> = tracks.iter().map(|t| t.remote()).collect();
        self.edit(playlist_id, playlists::add_actions(&ids, allow_duplicates)).await
    }

    async fn playlist_remove(&self, ctx: &RequestContext, playlist_id: &str, entries: &[(ItemId, String)]) -> Result<MutationOutcome> {
        self.check(ctx)?;
        self.require_auth()?;
        let pl = playlist_id.strip_prefix("VL").unwrap_or(playlist_id);
        let actions: Vec<Value> = entries.iter().map(|(t, s)| json!({ "action": "ACTION_REMOVE_VIDEO", "removedVideoId": t.remote(), "setVideoId": s })).collect();
        write_result(self.it.post(Client::WebRemix, "browse/edit_playlist", json!({ "playlistId": pl, "actions": actions }), &[]).await)
    }

    async fn playlist_create(&self, ctx: &RequestContext, title: &str, description: &str, privacy: Privacy, tracks: &[ItemId]) -> Result<PlaylistCreated> {
        self.check(ctx)?;
        self.require_auth()?;
        let title = playlists::check_title(title).map_err(AppError::invalid)?;
        let description = playlists::check_description(description).map_err(AppError::invalid)?;
        let ids: Vec<&str> = tracks.iter().map(|t| t.remote()).collect();
        match self.it.post(Client::WebRemix, "playlist/create", playlists::create_body(&title, &description, privacy, &ids), &[]).await {
            Ok(v) => Ok(match playlists::created(&v) {
                Some(pl) => PlaylistCreated { outcome: MutationOutcome::Done, id: Some(ItemId::ytm(&format!("VL{pl}"))), playlist_id: Some(pl) },
                None => PlaylistCreated { outcome: MutationOutcome::Rejected, id: None, playlist_id: None },
            }),
            // Maybe created, maybe not: never create again on our own.
            Err(e) if e.code == ErrorCode::Network => Ok(PlaylistCreated { outcome: MutationOutcome::OutcomeUnknown, id: None, playlist_id: None }),
            Err(e) => Err(e),
        }
    }

    async fn playlist_edit(&self, ctx: &RequestContext, playlist_id: &str, edit: &PlaylistEdit) -> Result<MutationOutcome> {
        self.check(ctx)?;
        self.require_auth()?;
        let mut e = edit.clone();
        if let Some(t) = &e.title {
            e.title = Some(playlists::check_title(t).map_err(AppError::invalid)?);
        }
        if let Some(d) = &e.description {
            e.description = Some(playlists::check_description(d).map_err(AppError::invalid)?);
        }
        self.edit(playlist_id, playlists::edit_actions(&e)).await
    }

    async fn playlist_delete(&self, ctx: &RequestContext, playlist_id: &str) -> Result<MutationOutcome> {
        self.check(ctx)?;
        self.require_auth()?;
        match self.it.post(Client::WebRemix, "playlist/delete", json!({ "playlistId": playlists::bare(playlist_id) }), &[]).await {
            Ok(v) => Ok(playlists::delete_outcome(&v)),
            Err(e) if e.code == ErrorCode::Network => Ok(MutationOutcome::OutcomeUnknown),
            Err(e) => Err(e),
        }
    }

    async fn playlist_move(&self, ctx: &RequestContext, playlist_id: &str, set_video_id: &str, before: Option<&str>) -> Result<MutationOutcome> {
        self.check(ctx)?;
        self.require_auth()?;
        if before == Some(set_video_id) {
            return Ok(MutationOutcome::Done);
        }
        self.edit(playlist_id, vec![playlists::move_action(set_video_id, before)]).await
    }

    async fn history(&self, ctx: &RequestContext) -> Result<HistoryPage> {
        self.check(ctx)?;
        self.require_auth()?;
        let v = self.browse("FEmusic_history", None).await?;
        if parse::is_signed_out(&v) {
            return Err(AppError::session_expired());
        }
        let raw = history::history(&v).map_err(parse_err)?;
        self.check(ctx)?;
        Ok(self.history_page(raw, ctx.epoch))
    }

    async fn history_more(&self, ctx: &RequestContext, cursor: &str) -> Result<HistoryPage> {
        self.check(ctx)?;
        self.require_auth()?;
        let c = self.cursors.take(cursor, ctx.epoch)?;
        let CursorKind::History = c.kind else { return Err(AppError::invalid("Not a history cursor.")) };
        let v = self.continue_browse("browse", &c.cont, json!({})).await?;
        let mut raw = history::history_continuation(&v).map_err(parse_err)?;
        // Stop on a repeated token or an empty page.
        if raw.groups.is_empty() || raw.cont.as_ref() == Some(&c.cont) {
            raw.cont = None;
        }
        Ok(self.history_page(raw, ctx.epoch))
    }

    async fn history_remove(&self, ctx: &RequestContext, row_ids: &[String]) -> Result<MutationOutcome> {
        self.check(ctx)?;
        self.require_auth()?;
        let tokens: Vec<String> = row_ids.iter().filter_map(|r| self.rows.get(r, ctx.epoch)).collect();
        if tokens.is_empty() || tokens.len() != row_ids.len() {
            return Err(AppError::new(ErrorCode::Stale, "This history list is out of date. Reload it and try again."));
        }
        let out = match self.it.post(Client::WebRemix, "feedback", json!({ "feedbackTokens": tokens }), &[]).await {
            Ok(v) => history::feedback_outcome(&v),
            Err(e) if e.code == ErrorCode::Network => MutationOutcome::OutcomeUnknown,
            Err(e) => return Err(e),
        };
        if out == MutationOutcome::Done {
            row_ids.iter().for_each(|r| self.rows.forget(r));
        }
        Ok(out)
    }

    async fn counterpart(&self, ctx: &RequestContext, track: &Track) -> Result<Option<Counterpart>> {
        self.check(ctx)?;
        let key = (ctx.account.clone(), track.id.0.clone());
        if let Some((c, at)) = self.pairs.lock().get(&key).cloned() {
            if at.elapsed() < if c.is_some() { PAIR_TTL } else { NO_PAIR_TTL } {
                return Ok(c);
            }
        }
        let id = &track.id;
        // 1. The service's own pairing in `next`, with the timeline map.
        let mut found = None;
        for body in [json!({ "videoId": id.remote(), "isAudioOnly": true }), json!({ "videoId": id.remote(), "playlistId": format!("RDAMVM{}", id.remote()), "isAudioOnly": true, "params": "wAEB" })] {
            let v = self.it.post(Client::WebRemix, "next", body, &[]).await?;
            if let Some(c) = pages::counterpart(&v, id) {
                found = Some(c);
                break;
            }
        }
        // A pairing already known for this track (album rows, D4) wins over a
        // different one.
        if let Some(known) = &track.counterpart {
            if found.as_ref().is_none_or(|c| &c.track.id != known) {
                found = match self.track(ctx, known).await {
                    Ok(mut t) => {
                        if t.kind == TrackKind::Unknown {
                            t.kind = if track.kind == TrackKind::Song { TrackKind::Video } else { TrackKind::Song };
                        }
                        t.counterpart = Some(id.clone());
                        Some(Counterpart { of: id.clone(), track: t, segments: vec![] })
                    }
                    Err(e) if e.code == ErrorCode::NotFound => None,
                    Err(e) => return Err(e),
                };
            }
        }
        // 2. A strictly matching search result.
        if found.is_none() {
            found = self.counterpart_by_search(ctx, track).await?;
        }
        self.check(ctx)?;
        self.pairs.lock().insert(key, (found.clone(), Instant::now()));
        Ok(found)
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

/// `playerConfig.audioConfig.loudnessDb` of a `/player` answer.
fn loudness_of(v: &Value) -> Option<f32> {
    v.pointer("/playerConfig/audioConfig/loudnessDb").and_then(Value::as_f64).filter(|l| l.is_finite()).map(|l| l as f32)
}

/// For downloads (D23): the loudness YouTube measured, so a downloaded song
/// is normalised like the app (D5). Asked as whoever is signed in; an
/// answer without it is `None`, never an error.
#[async_trait]
impl LoudnessSource for YtMusic {
    async fn loudness(&self, id: &ItemId) -> Result<Option<f32>> {
        let body = json!({ "videoId": id.remote(), "contentCheckOk": true, "racyCheckOk": true });
        let v = self.it.post(Client::WebRemix, "player", body, &[]).await?;
        Ok(loudness_of(&v))
    }
}

#[async_trait]
impl PlayReporter for YtMusic {
    async fn open_report(&self, id: &ItemId) -> Result<Option<Arc<dyn PlayReport>>> {
        self.it.bootstrap().await;
        // Pinned now: the `/player` call and every ping of this play go out
        // as this identity.
        let ident = self.it.identity();
        if !ident.signed_in() {
            return Ok(None);
        }
        let body = json!({
            "videoId": id.remote(),
            "playbackContext": { "contentPlaybackContext": { "referer": format!("{}/watch?v={}", client::ORIGIN, id.remote()) } },
            "contentCheckOk": true,
            "racyCheckOk": true,
        });
        let v = self.it.post_as(&ident, Client::WebRemix, "player", body, &[]).await?;
        let Some(tracking) = tracking::PlaybackTracking::from_player(&v) else {
            tracing::info!(video = %id, "play not reportable (no tracking for this account)");
            return Ok(None);
        };
        Ok(Some(Arc::new(tracking::YtReport { it: self.it.clone(), id: ident, tracking, cpn: tracking::cpn() })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_loudness_from_the_player_answer() {
        let v = json!({ "playerConfig": { "audioConfig": { "loudnessDb": -3.25, "perceptualLoudnessDb": -17.25 } } });
        assert_eq!(loudness_of(&v), Some(-3.25));
        assert_eq!(loudness_of(&json!({ "playabilityStatus": { "status": "OK" } })), None);
    }

    #[test]
    fn titles_compare_without_version_labels() {
        assert_eq!(norm_title("Harder, Better, Faster, Stronger (Official Video)"), norm_title("Harder, Better, Faster, Stronger"));
        assert_eq!(norm_title("One More Time [Official Music Video]"), "one more time");
        assert_ne!(norm_title("One More Time (Live)"), norm_title("One More Time"));
    }
}
