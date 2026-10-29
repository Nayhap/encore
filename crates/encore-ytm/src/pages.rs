//! Response → model. Pure functions over recorded JSON, tested on fixtures.
//! Continuation tokens are returned raw; the service wraps them in cursors.

use encore_auth::ChannelSelection;
use encore_core::model::*;
use serde_json::Value;

use crate::parse::*;

#[derive(Debug, Clone, PartialEq)]
pub enum Cont {
    /// Old style `nextContinuationData`: sent as `?ctoken=&continuation=&type=next`.
    Query(String),
    /// New style `continuationCommand.token`: sent as body `continuation`.
    Body(String),
}

pub struct Parsed<T> {
    pub value: T,
    pub cont: Option<Cont>,
    pub warnings: u32,
}

fn cont_of(v: &Value) -> Option<Cont> {
    next_continuation(v).map(Cont::Query).or_else(|| v.get("contents").and_then(item_continuation).map(Cont::Body))
}

// ---------- browse pages (home, explore, moods, charts) ----------

pub fn browse_page(resp: &Value) -> Result<Parsed<Page>, String> {
    let sl = section_list(resp).ok_or("no section list in browse response")?;
    let mut w = 0;
    let shelves = shelves_of(sl.get("contents").unwrap_or(&Value::Null), &mut w);
    let title = resp.get("header").and_then(|h| find(h, "title")).and_then(text_opt);
    Ok(Parsed { value: Page { title, shelves, chips: chips(sl), cursor: None }, cont: cont_of(sl), warnings: w })
}

/// `continuationContents.sectionListContinuation` (home "load more").
pub fn shelves_continuation(resp: &Value) -> Result<Parsed<Vec<Shelf>>, String> {
    let mut w = 0;
    if let Some(c) = resp.get("continuationContents").and_then(|c| c.get("sectionListContinuation")) {
        return Ok(Parsed { value: shelves_of(c.get("contents").unwrap_or(&Value::Null), &mut w), cont: cont_of(c), warnings: w });
    }
    if let Some(items) = appended_items(resp) {
        let shelves = shelves_of(items, &mut w);
        return Ok(Parsed { value: shelves, cont: item_continuation(items).map(Cont::Body), warnings: w });
    }
    Err("unexpected continuation response".into())
}

fn appended_items(resp: &Value) -> Option<&Value> {
    resp.get("onResponseReceivedActions")?.as_array()?.iter().find_map(|a| a.get("appendContinuationItemsAction").and_then(|x| x.get("continuationItems")))
}

// ---------- search ----------

fn bucket(r: &mut SearchResults, it: ShelfItem) {
    match it {
        // Podcasts are out of scope for M1.
        ShelfItem::Track(t) if t.kind == TrackKind::Episode => {}
        ShelfItem::Track(t) if t.kind == TrackKind::Video => r.videos.push(t),
        ShelfItem::Track(t) => r.songs.push(t),
        ShelfItem::Collection(c) if matches!(c.kind, CollectionKind::Album | CollectionKind::Single | CollectionKind::Ep) => r.albums.push(c),
        ShelfItem::Collection(c) => r.playlists.push(c),
        ShelfItem::Artist(a) => r.artists.push(a),
        ShelfItem::Link(_) => {}
    }
}

pub fn search(resp: &Value, query: &str) -> Result<Parsed<SearchResults>, String> {
    let sl = section_list(resp).ok_or("no search results")?;
    let mut r = SearchResults { query: query.to_string(), ..Default::default() };
    let mut cont = None;
    let mut w = 0;
    for sec in sl.get("contents").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]) {
        if let Some(card) = sec.get("musicCardShelfRenderer") {
            r.top = card_top(card);
            // Songs listed under an artist's top card omit the artist.
            let owner = match &r.top {
                Some(ShelfItem::Artist(a)) => Some(Credit { name: a.name.clone(), id: Some(a.id.clone()) }),
                _ => None,
            };
            for mut it in items_of(card.get("contents").unwrap_or(&Value::Null)) {
                if let (ShelfItem::Track(t), Some(o)) = (&mut it, &owner) {
                    if t.artists.is_empty() {
                        t.artists.push(o.clone());
                    }
                }
                bucket(&mut r, it);
            }
            continue;
        }
        if let Some(ms) = sec.get("musicShelfRenderer") {
            if cont.is_none() {
                cont = next_continuation(ms).map(Cont::Query);
            }
        }
        if let Some(is) = sec.get("itemSectionRenderer") {
            if let Some(dym) = find(is, "didYouMeanRenderer").or_else(|| find(is, "showingResultsForRenderer")) {
                r.did_you_mean = dym.get("correctedQuery").and_then(text_opt);
            }
        }
        if let Some(s) = shelf(sec, &mut w) {
            for it in s.items {
                bucket(&mut r, it);
            }
        }
    }
    Ok(Parsed { value: r, cont, warnings: w })
}

pub fn search_continuation(resp: &Value, query: &str) -> Result<Parsed<SearchResults>, String> {
    let c = resp.get("continuationContents").and_then(|c| c.get("musicShelfContinuation")).ok_or("unexpected search continuation")?;
    let mut r = SearchResults { query: query.to_string(), ..Default::default() };
    for it in items_of(c.get("contents").unwrap_or(&Value::Null)) {
        bucket(&mut r, it);
    }
    Ok(Parsed { value: r, cont: next_continuation(c).map(Cont::Query), warnings: 0 })
}

pub fn suggestions(resp: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let mut found = Vec::new();
    find_all(resp, "searchSuggestionRenderer", &mut found);
    for s in found {
        if let Some(q) = find(s, "query").and_then(Value::as_str) {
            out.push(q.to_string());
        } else if let Some(t) = s.get("suggestion").and_then(text_opt) {
            out.push(t);
        }
    }
    out.dedup();
    out
}

// ---------- albums and playlists ----------

fn header_of(resp: &Value) -> Option<&Value> {
    let c = resp.get("contents")?;
    find(c, "musicResponsiveHeaderRenderer").or_else(|| resp.get("header").and_then(|h| find(h, "musicDetailHeaderRenderer"))).or_else(|| resp.get("header").and_then(|h| h.get("musicResponsiveHeaderRenderer")))
}

fn entry_from(track: Track, set_video_id: Option<String>, idx: usize) -> Entry {
    let entry_id = set_video_id.clone().unwrap_or_else(|| format!("{}#{idx}", track.id.remote()));
    Entry { entry_id, track, set_video_id }
}

fn entries_of(list: &Value, offset: usize) -> Vec<Entry> {
    let mut out = Vec::new();
    for (i, w) in list.as_array().map(Vec::as_slice).unwrap_or(&[]).iter().enumerate() {
        let Some(r) = w.get("musicResponsiveListItemRenderer") else { continue };
        if let Some(ShelfItem::Track(t)) = list_item(r) {
            let svid = r.get("playlistItemData").and_then(|p| p.get("playlistSetVideoId")).and_then(Value::as_str).map(str::to_string);
            out.push(entry_from(t, svid, offset + i));
        }
    }
    out
}

pub fn collection(resp: &Value, id: &ItemId) -> Result<Parsed<Collection>, String> {
    let bid = id.remote().to_string();
    let h = header_of(resp).ok_or("no collection header")?;
    let title = h.get("title").and_then(text_opt).ok_or("collection without title")?;
    let sub_runs = h.get("subtitle").map(runs).unwrap_or(&[]);
    let first_label = sub_runs.first().and_then(|r| r.get("text")).and_then(Value::as_str);
    let kind = collection_kind(&bid, None, first_label);
    let mut artists: Vec<Credit> = Vec::new();
    for key in ["straplineTextOne", "facepile"] {
        if let Some(v) = h.get(key) {
            for r in runs(v) {
                if let Some(n) = r.get("navigationEndpoint").map(nav) {
                    if let (Some(b), Some(t)) = (n.browse_id, r.get("text").and_then(Value::as_str)) {
                        artists.push(Credit { name: t.to_string(), id: Some(ItemId::ytm(&b)) });
                    }
                }
            }
            if artists.is_empty() {
                // facepile: avatarStackViewModel { text.content, rendererContext…browseEndpoint }
                let name = text_opt(v).or_else(|| find(v, "content").and_then(Value::as_str).map(str::to_string));
                if let Some(t) = name.filter(|t| !t.trim().is_empty()) {
                    let id = find(v, "browseEndpoint").and_then(|b| b.get("browseId")).and_then(Value::as_str).map(ItemId::ytm);
                    artists.push(Credit { name: t, id });
                }
            }
        }
        if !artists.is_empty() {
            break;
        }
    }
    let year = sub_runs.iter().filter_map(|r| r.get("text").and_then(Value::as_str)).find_map(|t| (t.len() == 4 && t.chars().all(|c| c.is_ascii_digit())).then(|| t.parse().ok()).flatten());
    let subtitle = h.get("subtitle").and_then(text).unwrap_or_default();
    let stats = h.get("secondSubtitle").and_then(text_opt);
    let track_count = stats.as_deref().and_then(|s| s.split(SEP).find(|p| p.ends_with(" songs") || p.ends_with(" song") || p.ends_with(" tracks") || p.ends_with(" track") || p.ends_with(" episodes"))).and_then(|p| p.split(' ').next()).and_then(|n| n.replace(',', "").parse().ok());
    let description = h.get("description").and_then(|d| find(d, "description")).and_then(text_opt);
    let art = art_of(h);
    let editable = resp.get("contents").and_then(|c| find(c, "musicEditablePlaylistDetailHeaderRenderer")).is_some();
    let saved = h.get("buttons").and_then(Value::as_array).and_then(|bs| bs.iter().find_map(|b| b.get("toggleButtonRenderer"))).and_then(|t| t.get("isToggled")).and_then(Value::as_bool);
    let mut playlist_id = h
        .get("buttons")
        .and_then(Value::as_array)
        .and_then(|bs| bs.iter().find_map(|b| b.get("musicPlayButtonRenderer")))
        .and_then(|p| find(p, "playlistId"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let sl = section_list(resp).ok_or("no track list")?;
    let contents = sl.get("contents").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
    let mut entries = Vec::new();
    let mut cont = None;
    let mut related = Vec::new();
    let mut w = 0;
    for sec in contents {
        if let Some(ps) = sec.get("musicPlaylistShelfRenderer") {
            let list = ps.get("contents").unwrap_or(&Value::Null);
            entries.extend(entries_of(list, entries.len()));
            cont = item_continuation(list).map(Cont::Body).or_else(|| next_continuation(ps).map(Cont::Query));
            if playlist_id.is_none() {
                playlist_id = ps.get("playlistId").and_then(Value::as_str).map(str::to_string);
            }
        } else if let Some(ms) = sec.get("musicShelfRenderer").filter(|_| entries.is_empty()) {
            let list = ms.get("contents").unwrap_or(&Value::Null);
            entries.extend(entries_of(list, 0));
            cont = item_continuation(list).map(Cont::Body).or_else(|| next_continuation(ms).map(Cont::Query));
        } else if let Some(s) = shelf(sec, &mut w) {
            if !s.items.is_empty() {
                related.push(s);
            }
        }
    }
    let album_like = matches!(kind, CollectionKind::Album | CollectionKind::Single | CollectionKind::Ep);
    if kind == CollectionKind::Liked {
        for e in entries.iter_mut() {
            e.track.liked = Some(true);
        }
    }
    if playlist_id.is_none() {
        playlist_id = contents.iter().find_map(|s| find(s, "watchEndpoint")).and_then(|w| w.get("playlistId")).and_then(Value::as_str).map(str::to_string);
    }
    if playlist_id.is_none() && !album_like {
        playlist_id = Some(bid.strip_prefix("VL").unwrap_or(&bid).to_string());
    }
    if album_like {
        // Album rows omit artist, album and artwork: stamp them from the header.
        for e in entries.iter_mut() {
            if e.track.artists.is_empty() {
                e.track.artists = artists.clone();
            }
            if e.track.album.is_none() {
                e.track.album = Some(Credit { name: title.clone(), id: Some(id.clone()) });
            }
            if e.track.art.is_none() {
                e.track.art = art.clone();
            }
        }
    }
    Ok(Parsed {
        value: Collection {
            id: id.clone(),
            kind,
            title,
            subtitle,
            artists,
            year,
            art,
            description,
            stats,
            track_count,
            entries,
            cursor: None,
            playlist_id,
            saved,
            editable,
            related,
        },
        cont,
        warnings: w,
    })
}

pub fn collection_continuation(resp: &Value, offset: usize) -> Result<Parsed<Vec<Entry>>, String> {
    if let Some(items) = appended_items(resp) {
        return Ok(Parsed { value: entries_of(items, offset), cont: item_continuation(items).map(Cont::Body), warnings: 0 });
    }
    if let Some(c) = resp.get("continuationContents").and_then(|c| c.get("musicPlaylistShelfContinuation").or_else(|| c.get("musicShelfContinuation"))) {
        return Ok(Parsed { value: entries_of(c.get("contents").unwrap_or(&Value::Null), offset), cont: next_continuation(c).map(Cont::Query), warnings: 0 });
    }
    Err("unexpected playlist continuation".into())
}

// ---------- artist ----------

pub fn artist(resp: &Value, id: &ItemId) -> Result<Parsed<Artist>, String> {
    let h = resp
        .get("header")
        .and_then(|h| h.get("musicImmersiveHeaderRenderer").or_else(|| h.get("musicVisualHeaderRenderer")).or_else(|| h.get("musicHeaderRenderer")))
        .ok_or("no artist header")?;
    let name = h.get("title").and_then(text_opt).ok_or("artist without name")?;
    let sb = h.get("subscriptionButton").and_then(|s| s.get("subscribeButtonRenderer"));
    let subscribers = sb.and_then(|s| s.get("longSubscriberCountText").or_else(|| s.get("subscriberCountText"))).and_then(text_opt).or_else(|| h.get("monthlyListenerCount").and_then(text_opt));
    let subscribed = sb.and_then(|s| s.get("subscribed")).and_then(Value::as_bool);
    let channel_id = sb.and_then(|s| s.get("channelId")).and_then(Value::as_str).map(str::to_string);
    let radio_playlist = h.get("startRadioButton").and_then(|b| find(b, "playlistId")).and_then(Value::as_str).map(str::to_string);
    let shuffle_playlist = h.get("playButton").and_then(|b| find(b, "playlistId")).and_then(Value::as_str).map(str::to_string);
    let mut description = h.get("description").and_then(text_opt);
    let art = art_of(h);
    let sl = section_list(resp).ok_or("no artist sections")?;
    let mut top_songs = Vec::new();
    let mut top_more = None;
    let mut shelves = Vec::new();
    let mut w = 0;
    for sec in sl.get("contents").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]) {
        if let Some(d) = sec.get("musicDescriptionShelfRenderer") {
            if description.is_none() {
                description = d.get("description").and_then(text_opt);
            }
            continue;
        }
        if let Some(s) = shelf(sec, &mut w) {
            if top_songs.is_empty() && s.layout == ShelfLayout::List && s.items.iter().all(|i| matches!(i, ShelfItem::Track(_))) {
                top_songs = s.items.into_iter().filter_map(|i| if let ShelfItem::Track(t) = i { Some(t) } else { None }).collect();
                top_more = s.more;
            } else if !s.items.is_empty() {
                shelves.push(s);
            }
        }
    }
    Ok(Parsed {
        value: Artist { id: id.clone(), name, subscribers, description, art, top_songs, top_songs_more: top_more, shelves, subscribed, channel_id, radio_playlist, shuffle_playlist },
        cont: None,
        warnings: w,
    })
}

// ---------- library ----------

pub fn library(resp: &Value, section: LibrarySection) -> Result<Parsed<Vec<ShelfItem>>, String> {
    if section == LibrarySection::Liked {
        let c = collection(resp, &ItemId::ytm("VLLM"))?;
        return Ok(Parsed { value: c.value.entries.into_iter().map(|e| ShelfItem::Track(e.track)).collect(), cont: c.cont, warnings: c.warnings });
    }
    let sl = section_list(resp).ok_or("no library content")?;
    let mut items = Vec::new();
    let mut cont = None;
    let mut w = 0;
    for sec in sl.get("contents").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]) {
        // grids and shelves may sit inside an itemSectionRenderer
        let inner = sec.get("itemSectionRenderer").and_then(|i| i.get("contents")).and_then(|c| c.get(0)).unwrap_or(sec);
        for key in ["gridRenderer", "musicShelfRenderer"] {
            if let Some(r) = inner.get(key) {
                if cont.is_none() {
                    cont = next_continuation(r).map(Cont::Query).or_else(|| r.get("items").or_else(|| r.get("contents")).and_then(item_continuation).map(Cont::Body));
                }
            }
        }
        if let Some(s) = shelf(inner, &mut w) {
            items.extend(s.items);
        }
    }
    // "New playlist" / "Liked music" pseudo-items are links or the LM playlist.
    if section == LibrarySection::Playlists {
        items.retain(|i| !matches!(i, ShelfItem::Link(_)));
    }
    Ok(Parsed { value: items, cont, warnings: w })
}

pub fn library_continuation(resp: &Value) -> Result<Parsed<Vec<ShelfItem>>, String> {
    if let Some(items) = appended_items(resp) {
        return Ok(Parsed { value: items_of(items), cont: item_continuation(items).map(Cont::Body), warnings: 0 });
    }
    let cc = resp.get("continuationContents").ok_or("unexpected library continuation")?;
    for (k, list) in [("gridContinuation", "items"), ("musicShelfContinuation", "contents"), ("musicPlaylistShelfContinuation", "contents")] {
        if let Some(c) = cc.get(k) {
            return Ok(Parsed { value: items_of(c.get(list).unwrap_or(&Value::Null)), cont: next_continuation(c).map(Cont::Query), warnings: 0 });
        }
    }
    Err("unexpected library continuation".into())
}

// ---------- up next / radio ----------

pub struct NextInfo {
    pub tracks: Vec<Track>,
    pub playlist_id: Option<String>,
    pub cont: Option<String>,
    pub lyrics_browse_id: Option<String>,
    pub related_browse_id: Option<String>,
}

pub fn next(resp: &Value) -> Result<NextInfo, String> {
    let panel = if let Some(c) = resp.get("continuationContents").and_then(|c| c.get("playlistPanelContinuation")) {
        c
    } else {
        find(resp.get("contents").ok_or("no next contents")?, "playlistPanelRenderer").ok_or("no queue in next response")?
    };
    let tracks = panel.get("contents").and_then(Value::as_array).map(|a| a.iter().filter_map(item).filter_map(|i| if let ShelfItem::Track(t) = i { Some(t) } else { None }).collect()).unwrap_or_default();
    let playlist_id = panel.get("playlistId").and_then(Value::as_str).map(str::to_string);
    let cont = next_continuation(panel);
    let mut lyrics_browse_id = None;
    let mut related_browse_id = None;
    if let Some(tabs) = resp.get("contents").and_then(|c| find(c, "tabs")).and_then(Value::as_array) {
        for t in tabs {
            let tr = t.get("tabRenderer").unwrap_or(&Value::Null);
            if tr.get("unselectable").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            if let Some(b) = tr.get("endpoint").and_then(|e| find(e, "browseId")).and_then(Value::as_str) {
                if b.starts_with("MPLY") {
                    lyrics_browse_id = Some(b.to_string());
                } else if b.starts_with("MPTR") {
                    related_browse_id = Some(b.to_string());
                }
            }
        }
    }
    Ok(NextInfo { tracks, playlist_id, cont, lyrics_browse_id, related_browse_id })
}

// ---------- lyrics ----------

pub fn timed_lyrics(resp: &Value, track: &ItemId) -> Option<Lyrics> {
    let data = find(resp, "timedLyricsData")?.as_array()?;
    let mut lines = Vec::new();
    for l in data {
        let text = l.get("lyricLine").and_then(Value::as_str).unwrap_or("").trim().to_string();
        let cue = l.get("cueRange")?;
        let start = cue.get("startTimeMilliseconds").and_then(|v| v.as_str().and_then(|s| s.parse::<f64>().ok()).or_else(|| v.as_f64()))? / 1000.0;
        let end = cue.get("endTimeMilliseconds").and_then(|v| v.as_str().and_then(|s| s.parse::<f64>().ok()).or_else(|| v.as_f64())).map(|e| e / 1000.0);
        // "♪" marks instrumental intros; leave them out of the sung lines.
        if text.is_empty() || text == "♪" {
            continue;
        }
        lines.push(LyricLine { t: start, end, text, words: None });
    }
    if lines.len() < 2 {
        return None;
    }
    lines.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
    let attribution = find(resp, "sourceMessage").and_then(text_opt).or_else(|| find(resp, "footer").and_then(text_opt));
    Some(Lyrics { track_id: track.clone(), sync: LyricsSync::Line, lines, plain: None, source: "YouTube Music".into(), attribution })
}

pub fn plain_lyrics(resp: &Value, track: &ItemId) -> Option<Lyrics> {
    let d = find(resp.get("contents")?, "musicDescriptionShelfRenderer")?;
    let text = d.get("description").and_then(text_opt)?;
    let attribution = d.get("footer").and_then(text_opt);
    Some(Lyrics { track_id: track.clone(), sync: LyricsSync::None, lines: vec![], plain: Some(text), source: "YouTube Music".into(), attribution })
}

// ---------- accounts ----------

pub struct AccountEntry {
    pub channel: Channel,
    pub selection: ChannelSelection,
    pub selected: bool,
}

pub fn accounts(resp: &Value) -> Vec<AccountEntry> {
    let mut items = Vec::new();
    find_all(resp, "accountItem", &mut items);
    let mut out = Vec::new();
    for it in items {
        let Some(name) = it.get("accountName").and_then(text_opt) else { continue };
        let tokens = find(it, "supportedTokens").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
        let page_id = tokens.iter().find_map(|t| t.get("pageIdToken").and_then(|p| p.get("pageId"))).and_then(Value::as_str).map(str::to_string);
        let datasync = tokens.iter().find_map(|t| t.get("datasyncIdToken").and_then(|d| d.get("datasyncIdToken"))).and_then(Value::as_str);
        let user_session_id = datasync.and_then(|d| d.split_once("||")).map(|(_, u)| u.to_string()).filter(|u| page_id.is_some() && !u.is_empty());
        let auth_user = tokens
            .iter()
            .find_map(|t| t.get("accountSigninToken").and_then(|s| s.get("signinUrl")).and_then(Value::as_str))
            .and_then(|u| u.split(['?', '&']).find_map(|kv| kv.strip_prefix("authuser=")))
            .and_then(|n| n.parse().ok())
            .unwrap_or(0u32);
        let handle = it.get("channelHandle").and_then(text_opt).or_else(|| it.get("accountByline").and_then(text_opt).filter(|b| b.starts_with('@')));
        let selected = it.get("isSelected").and_then(Value::as_bool).unwrap_or(false);
        let photo = it.get("accountPhoto").and_then(thumbnail_url).and_then(|u| encore_core::art::register(&u));
        let key = format!("{auth_user}:{}", page_id.as_deref().unwrap_or("primary"));
        out.push(AccountEntry {
            channel: Channel { key, name: name.clone(), handle: handle.clone(), page_id: page_id.clone(), auth_user, is_brand: page_id.is_some(), photo },
            selection: ChannelSelection { auth_user, page_id, user_session_id, name, handle },
            selected,
        });
    }
    out
}
