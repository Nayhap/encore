//! Renderer parsers. Dispatch is by renderer key, never by position, and
//! unknown renderers are skipped (counted in `warnings`) so a new shelf type
//! degrades to "missing", not "broken".

use encore_core::art;
use encore_core::model::*;
use serde_json::Value;

pub const SEP: &str = " • ";

// ---------- generic helpers ----------

pub fn text(v: &Value) -> Option<String> {
    if let Some(s) = v.get("simpleText").and_then(Value::as_str) {
        return Some(s.to_string());
    }
    let runs = v.get("runs")?.as_array()?;
    let s: String = runs.iter().filter_map(|r| r.get("text").and_then(Value::as_str)).collect();
    Some(s)
}

pub fn text_opt(v: &Value) -> Option<String> {
    text(v).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

pub fn runs(v: &Value) -> &[Value] {
    v.get("runs").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

/// Depth-first search for the first value under `key`.
pub fn find<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(m) => {
            if let Some(x) = m.get(key) {
                return Some(x);
            }
            m.values().find_map(|x| find(x, key))
        }
        Value::Array(a) => a.iter().find_map(|x| find(x, key)),
        _ => None,
    }
}

/// Collect every value under `key` anywhere in the tree (not descending into matches).
pub fn find_all<'a>(v: &'a Value, key: &str, out: &mut Vec<&'a Value>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                if k == key {
                    out.push(x);
                } else {
                    find_all(x, key, out);
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|x| find_all(x, key, out)),
        _ => {}
    }
}

pub fn thumbnail_url(v: &Value) -> Option<String> {
    let thumbs = find(v, "thumbnails")?.as_array()?;
    thumbs
        .iter()
        .filter_map(|t| Some((t.get("width").and_then(Value::as_u64).unwrap_or(0), t.get("url")?.as_str()?)))
        .max_by_key(|(w, _)| *w)
        .map(|(_, u)| u.to_string())
}

pub fn art_of(v: &Value) -> Option<ArtRef> {
    // Prefer the thumbnail of the renderer itself, not of nested items.
    let src = v.get("thumbnail").or_else(|| v.get("thumbnailRenderer")).unwrap_or(v);
    thumbnail_url(src).and_then(|u| art::register(&u))
}

pub fn parse_duration(s: &str) -> Option<f64> {
    let parts: Vec<&str> = s.trim().split(':').collect();
    if parts.len() < 2 || parts.len() > 3 || parts.iter().any(|p| p.is_empty() || !p.chars().all(|c| c.is_ascii_digit())) {
        return None;
    }
    let mut secs = 0f64;
    for p in parts {
        secs = secs * 60.0 + p.parse::<f64>().ok()?;
    }
    Some(secs)
}

fn is_year(s: &str) -> Option<u16> {
    let s = s.trim();
    (s.len() == 4 && s.chars().all(|c| c.is_ascii_digit())).then(|| s.parse().ok()).flatten()
}

pub struct Nav {
    pub video_id: Option<String>,
    pub playlist_id: Option<String>,
    pub video_type: Option<String>,
    pub browse_id: Option<String>,
    pub params: Option<String>,
    pub page_type: Option<String>,
}

pub fn nav(endpoint: &Value) -> Nav {
    let w = endpoint.get("watchEndpoint");
    let wp = endpoint.get("watchPlaylistEndpoint");
    let b = endpoint.get("browseEndpoint");
    Nav {
        video_id: w.and_then(|w| w.get("videoId")).and_then(Value::as_str).map(str::to_string),
        playlist_id: w.and_then(|w| w.get("playlistId")).or_else(|| wp.and_then(|w| w.get("playlistId"))).and_then(Value::as_str).map(str::to_string),
        video_type: w.and_then(|w| find(w, "musicVideoType")).and_then(Value::as_str).map(str::to_string),
        browse_id: b.and_then(|b| b.get("browseId")).and_then(Value::as_str).map(str::to_string),
        params: b.and_then(|b| b.get("params")).and_then(Value::as_str).map(str::to_string),
        page_type: b.and_then(|b| find(b, "pageType")).and_then(Value::as_str).map(str::to_string),
    }
}

fn kind_of_video(t: Option<&str>) -> TrackKind {
    match t {
        Some("MUSIC_VIDEO_TYPE_ATV") => TrackKind::Song,
        Some("MUSIC_VIDEO_TYPE_OMV") | Some("MUSIC_VIDEO_TYPE_UGC") | Some("MUSIC_VIDEO_TYPE_OFFICIAL_SOURCE_MUSIC") => TrackKind::Video,
        Some("MUSIC_VIDEO_TYPE_PODCAST_EPISODE") => TrackKind::Episode,
        _ => TrackKind::Unknown,
    }
}

pub fn explicit(v: &Value) -> bool {
    let mut out = Vec::new();
    find_all(v, "iconType", &mut out);
    out.iter().any(|x| x.as_str() == Some("MUSIC_EXPLICIT_BADGE"))
}

fn liked(v: &Value) -> Option<bool> {
    find(v, "likeStatus").and_then(Value::as_str).map(|s| s == "LIKE")
}

pub fn collection_kind(browse_id: &str, page_type: Option<&str>, first_label: Option<&str>) -> CollectionKind {
    let label = first_label.unwrap_or("").to_ascii_lowercase();
    if browse_id == "VLLM" || browse_id == "LM" {
        return CollectionKind::Liked;
    }
    if browse_id.starts_with("MPRE") || page_type == Some("MUSIC_PAGE_TYPE_ALBUM") || page_type == Some("MUSIC_PAGE_TYPE_AUDIOBOOK") {
        return match label.as_str() {
            "single" => CollectionKind::Single,
            "ep" => CollectionKind::Ep,
            _ => CollectionKind::Album,
        };
    }
    let bare = browse_id.strip_prefix("VL").unwrap_or(browse_id);
    if bare.starts_with("RD") {
        return CollectionKind::Mix;
    }
    CollectionKind::Playlist
}

/// Split subtitle runs into " • " segments.
fn segments(rs: &[Value]) -> Vec<Vec<&Value>> {
    let mut out: Vec<Vec<&Value>> = vec![vec![]];
    for r in rs {
        let t = r.get("text").and_then(Value::as_str).unwrap_or("");
        if t.trim() == "•" {
            out.push(vec![]);
        } else {
            out.last_mut().unwrap().push(r);
        }
    }
    out.into_iter().filter(|s| !s.is_empty()).collect()
}

fn seg_text(seg: &[&Value]) -> String {
    seg.iter().filter_map(|r| r.get("text").and_then(Value::as_str)).collect::<String>().trim().to_string()
}

const TYPE_LABELS: &[&str] = &["Song", "Video", "Album", "Single", "EP", "Playlist", "Artist", "Episode", "Podcast", "Profile", "Station", "Audiobook"];

#[derive(Default)]
struct Meta {
    artists: Vec<Credit>,
    album: Option<Credit>,
    duration: Option<f64>,
    plays: Option<String>,
    year: Option<u16>,
    label: Option<String>,
    rest: Vec<String>,
}

fn credits_from(seg: &[&Value]) -> Vec<Credit> {
    let linked: Vec<Credit> = seg
        .iter()
        .filter_map(|r| {
            let n = nav(r.get("navigationEndpoint")?);
            let pt = n.page_type.as_deref()?;
            if pt == "MUSIC_PAGE_TYPE_ARTIST" || pt == "MUSIC_PAGE_TYPE_USER_CHANNEL" || pt == "MUSIC_PAGE_TYPE_LIBRARY_ARTIST" {
                Some(Credit { name: r.get("text")?.as_str()?.trim().to_string(), id: n.browse_id.map(|b| ItemId::ytm(&b)) })
            } else {
                None
            }
        })
        .collect();
    if !linked.is_empty() {
        return linked;
    }
    // Plain text artist list: "A, B & C".
    let t = seg_text(seg);
    t.split(", ").flat_map(|p| p.split(" & ")).map(|p| p.trim()).filter(|p| !p.is_empty()).map(|p| Credit { name: p.to_string(), id: None }).collect()
}

fn meta_from_runs(rs: &[Value]) -> Meta {
    let mut m = Meta::default();
    for (i, seg) in segments(rs).iter().enumerate() {
        let t = seg_text(seg);
        let page = seg.iter().find_map(|r| r.get("navigationEndpoint").map(nav)).and_then(|n| n.page_type.clone().map(|p| (p, n)));
        match page {
            Some((p, n)) if p == "MUSIC_PAGE_TYPE_ALBUM" => {
                m.album = Some(Credit { name: t, id: n.browse_id.map(|b| ItemId::ytm(&b)) });
            }
            Some((p, _)) if p.contains("ARTIST") || p == "MUSIC_PAGE_TYPE_USER_CHANNEL" => {
                m.artists.extend(credits_from(seg));
            }
            _ => {
                if i == 0 && TYPE_LABELS.contains(&t.as_str()) {
                    m.label = Some(t);
                } else if let Some(d) = parse_duration(&t) {
                    m.duration = Some(d);
                } else if let Some(y) = is_year(&t) {
                    m.year = Some(y);
                } else if t.ends_with(" plays") || t.ends_with(" views") || t.ends_with(" play") {
                    m.plays = Some(t);
                } else if !t.is_empty() {
                    m.rest.push(t);
                }
            }
        }
    }
    m
}

fn flex_columns(r: &Value) -> Vec<&Value> {
    r.get("flexColumns")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|c| c.get("musicResponsiveListItemFlexColumnRenderer").and_then(|x| x.get("text"))).collect())
        .unwrap_or_default()
}

fn greyed(r: &Value) -> bool {
    r.get("musicItemRendererDisplayPolicy").and_then(Value::as_str) == Some("MUSIC_ITEM_RENDERER_DISPLAY_POLICY_GREY_OUT")
}

// ---------- item parsers ----------

/// `musicResponsiveListItemRenderer` → track, collection or artist.
pub fn list_item(r: &Value) -> Option<ShelfItem> {
    let cols = flex_columns(r);
    let title_v = cols.first().copied();
    let title = title_v.and_then(text_opt);
    let title_nav = title_v.and_then(|t| runs(t).first()).and_then(|r| r.get("navigationEndpoint")).map(nav);
    let video_id = r
        .get("playlistItemData")
        .and_then(|p| p.get("videoId"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| title_nav.as_ref().and_then(|n| n.video_id.clone()))
        .or_else(|| find(r.get("overlay").unwrap_or(&Value::Null), "watchEndpoint").and_then(|w| w.get("videoId")).and_then(Value::as_str).map(str::to_string));
    let mut sub_runs: Vec<Value> = Vec::new();
    for (i, c) in cols.iter().enumerate().skip(1) {
        let rs = runs(c);
        if rs.is_empty() {
            continue;
        }
        if i > 1 && !sub_runs.is_empty() {
            sub_runs.push(serde_json::json!({"text": SEP}));
        }
        sub_runs.extend(rs.iter().cloned());
    }
    let meta = meta_from_runs(&sub_runs);
    if let Some(vid) = video_id {
        let title = title?;
        let vtype = title_nav.as_ref().and_then(|n| n.video_type.clone()).or_else(|| find(r, "musicVideoType").and_then(Value::as_str).map(str::to_string));
        let mut kind = kind_of_video(vtype.as_deref());
        if kind == TrackKind::Unknown {
            kind = match meta.label.as_deref() {
                Some("Song") => TrackKind::Song,
                Some("Video") => TrackKind::Video,
                Some("Episode") => TrackKind::Episode,
                _ => TrackKind::Unknown,
            };
        }
        let fixed = r
            .get("fixedColumns")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(|c| find(c, "text"))
            .and_then(text_opt)
            .and_then(|t| parse_duration(&t));
        let mut artists = meta.artists;
        if artists.is_empty() && meta.label.is_none() {
            // Some rows give the artist as the first plain segment.
            if let Some(first) = meta.rest.first() {
                artists = first.split(", ").map(|n| Credit { name: n.to_string(), id: None }).collect();
            }
        }
        return Some(ShelfItem::Track(Track {
            id: ItemId::ytm(&vid),
            title,
            artists,
            album: meta.album,
            duration: fixed.or(meta.duration),
            explicit: explicit(r.get("badges").unwrap_or(&Value::Null)) || explicit(r.get("subtitleBadges").unwrap_or(&Value::Null)),
            art: art_of(r),
            kind,
            plays: meta.plays,
            liked: liked(r.get("menu").unwrap_or(&Value::Null)),
            playable: !greyed(r),
        }));
    }
    let n = r.get("navigationEndpoint").map(nav)?;
    // Library artist pages (MPLAUC…) map to the artist channel.
    let bid = n.browse_id.clone().map(|b| b.strip_prefix("MPLA").map(str::to_string).unwrap_or(b))?;
    let title = title?;
    let subtitle = text(&serde_json::json!({ "runs": sub_runs })).unwrap_or_default();
    match n.page_type.as_deref() {
        Some("MUSIC_PAGE_TYPE_ARTIST") | Some("MUSIC_PAGE_TYPE_USER_CHANNEL") | Some("MUSIC_PAGE_TYPE_LIBRARY_ARTIST") => {
            Some(ShelfItem::Artist(ArtistCard { id: ItemId::ytm(&bid), name: title, subtitle, art: art_of(r) }))
        }
        Some("MUSIC_PAGE_TYPE_ALBUM") | Some("MUSIC_PAGE_TYPE_PLAYLIST") | Some("MUSIC_PAGE_TYPE_AUDIOBOOK") => {
            let kind = collection_kind(&bid, n.page_type.as_deref(), meta.label.as_deref());
            Some(ShelfItem::Collection(CollectionCard {
                id: ItemId::ytm(&bid),
                kind,
                title,
                subtitle,
                artists: meta.artists,
                year: meta.year,
                art: art_of(r),
                play_id: find(r.get("overlay").unwrap_or(&Value::Null), "playlistId").and_then(Value::as_str).map(str::to_string),
            }))
        }
        _ => None,
    }
}

/// `musicTwoRowItemRenderer` → collection, artist or track (cards).
pub fn two_row(r: &Value) -> Option<ShelfItem> {
    let title = r.get("title").and_then(text_opt)?;
    let sub = r.get("subtitle").map(runs).unwrap_or(&[]);
    let meta = meta_from_runs(sub);
    let subtitle = r.get("subtitle").and_then(text).unwrap_or_default();
    let n = r.get("navigationEndpoint").map(nav)?;
    if let Some(vid) = n.video_id.clone() {
        let mut artists = meta.artists;
        if artists.is_empty() {
            if let Some(f) = meta.rest.first() {
                artists = vec![Credit { name: f.clone(), id: None }];
            }
        }
        return Some(ShelfItem::Track(Track {
            id: ItemId::ytm(&vid),
            title,
            artists,
            album: meta.album,
            duration: meta.duration,
            explicit: explicit(r.get("subtitleBadges").unwrap_or(&Value::Null)),
            art: art_of(r),
            kind: kind_of_video(n.video_type.as_deref()),
            plays: meta.plays,
            liked: None,
            playable: true,
        }));
    }
    let bid = n.browse_id.clone()?;
    match n.page_type.as_deref() {
        Some(p) if p.contains("ARTIST") || p == "MUSIC_PAGE_TYPE_USER_CHANNEL" => Some(ShelfItem::Artist(ArtistCard { id: ItemId::ytm(&bid), name: title, subtitle, art: art_of(r) })),
        Some("MUSIC_PAGE_TYPE_ALBUM") | Some("MUSIC_PAGE_TYPE_PLAYLIST") | Some("MUSIC_PAGE_TYPE_AUDIOBOOK") | None => {
            if n.page_type.is_none() && !bid.starts_with("VL") && !bid.starts_with("MPRE") {
                return link_if_supported(title, bid, n.params);
            }
            let kind = collection_kind(&bid, n.page_type.as_deref(), meta.label.as_deref());
            let play_id = find(r.get("thumbnailOverlay").unwrap_or(&Value::Null), "playlistId").and_then(Value::as_str).map(str::to_string);
            Some(ShelfItem::Collection(CollectionCard { id: ItemId::ytm(&bid), kind, title, subtitle, artists: meta.artists, year: meta.year, art: art_of(r), play_id }))
        }
        _ => link_if_supported(title, bid, n.params),
    }
}

/// Only keep links to pages Encore can open.
fn link_if_supported(title: String, bid: String, params: Option<String>) -> Option<ShelfItem> {
    let ok = ["FE", "MPAD", "VL", "UC", "MPTR"].iter().any(|p| bid.starts_with(p));
    ok.then_some(ShelfItem::Link(PageLink { title, browse_id: bid, params, hue: None }))
}

/// `playlistPanelVideoRenderer` (up next / radio).
pub fn panel_video(r: &Value) -> Option<Track> {
    let vid = r.get("videoId").and_then(Value::as_str).or_else(|| find(r.get("navigationEndpoint")?, "videoId").and_then(Value::as_str))?;
    let title = r.get("title").and_then(text_opt)?;
    let meta = meta_from_runs(r.get("longBylineText").map(runs).unwrap_or(&[]));
    let mut artists = meta.artists;
    if artists.is_empty() {
        if let Some(s) = r.get("shortBylineText").and_then(text_opt) {
            artists = vec![Credit { name: s, id: None }];
        }
    }
    let vtype = find(r.get("navigationEndpoint").unwrap_or(&Value::Null), "musicVideoType").and_then(Value::as_str);
    Some(Track {
        id: ItemId::ytm(vid),
        title,
        artists,
        album: meta.album,
        duration: r.get("lengthText").and_then(text_opt).and_then(|t| parse_duration(&t)),
        explicit: explicit(r.get("badges").unwrap_or(&Value::Null)),
        art: art_of(r),
        kind: kind_of_video(vtype),
        plays: None,
        liked: liked(r.get("menu").unwrap_or(&Value::Null)),
        playable: r.get("unplayableText").is_none(),
    })
}

fn nav_button(r: &Value) -> Option<ShelfItem> {
    let title = r.get("buttonText").and_then(text_opt)?;
    let n = nav(r.get("clickCommand")?);
    let hue = r.get("solid").and_then(|s| s.get("leftStripeColor")).and_then(Value::as_u64).map(argb_hue);
    Some(ShelfItem::Link(PageLink { title, browse_id: n.browse_id?, params: n.params, hue }))
}

fn argb_hue(c: u64) -> u16 {
    let r = ((c >> 16) & 0xff) as f64 / 255.0;
    let g = ((c >> 8) & 0xff) as f64 / 255.0;
    let b = (c & 0xff) as f64 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    if d == 0.0 {
        return 0;
    }
    let h = if max == r {
        60.0 * (((g - b) / d) % 6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    ((h + 360.0) % 360.0) as u16
}

/// Parse any item renderer wrapper `{ "<type>Renderer": {...} }`.
pub fn item(wrapper: &Value) -> Option<ShelfItem> {
    let (k, r) = wrapper.as_object()?.iter().next()?;
    match k.as_str() {
        "musicResponsiveListItemRenderer" => list_item(r),
        "musicTwoRowItemRenderer" => two_row(r),
        "playlistPanelVideoRenderer" => panel_video(r).map(ShelfItem::Track),
        // The wrapper holds the requested item and its song/video counterpart;
        // only the primary one is what was asked for (no silent swap, D4).
        "playlistPanelVideoWrapperRenderer" => r.get("primaryRenderer").and_then(|p| p.get("playlistPanelVideoRenderer")).and_then(panel_video).map(ShelfItem::Track),
        "musicNavigationButtonRenderer" => nav_button(r),
        "musicMultiRowListItemRenderer" => multi_row(r),
        _ => None,
    }
}

/// Podcast/episode rows.
fn multi_row(r: &Value) -> Option<ShelfItem> {
    let title = r.get("title").and_then(text_opt)?;
    let vid = find(r, "videoId").and_then(Value::as_str)?;
    Some(ShelfItem::Track(Track {
        id: ItemId::ytm(vid),
        title,
        artists: r.get("secondTitle").and_then(text_opt).map(|n| vec![Credit { name: n, id: None }]).unwrap_or_default(),
        album: None,
        duration: None,
        explicit: false,
        art: art_of(r),
        kind: TrackKind::Episode,
        plays: None,
        liked: None,
        playable: true,
    }))
}

// ---------- shelves ----------

fn more_link(header: &Value) -> Option<PageLink> {
    let ep = find(header, "moreContentButton").and_then(|b| find(b, "navigationEndpoint")).or_else(|| header.get("title").and_then(|t| runs(t).first()).and_then(|r| r.get("navigationEndpoint")))?;
    let n = nav(ep);
    let title = header.get("title").and_then(text_opt).unwrap_or_default();
    Some(PageLink { title, browse_id: n.browse_id?, params: n.params, hue: None })
}

pub fn items_of(contents: &Value) -> Vec<ShelfItem> {
    contents.as_array().map(|a| a.iter().filter_map(item).collect()).unwrap_or_default()
}

/// A section renderer → shelf. Unknown section types return `None`.
pub fn shelf(wrapper: &Value, warnings: &mut u32) -> Option<Shelf> {
    let (k, r) = wrapper.as_object()?.iter().next()?;
    match k.as_str() {
        "musicCarouselShelfRenderer" => {
            let h = r.get("header").and_then(|h| h.get("musicCarouselShelfBasicHeaderRenderer")).unwrap_or(&Value::Null);
            let items = items_of(r.get("contents").unwrap_or(&Value::Null));
            let list = items.iter().all(|i| matches!(i, ShelfItem::Track(_))) && r.get("contents").and_then(|c| c.get(0)).and_then(|c| c.get("musicResponsiveListItemRenderer")).is_some();
            Some(Shelf {
                title: h.get("title").and_then(text_opt).unwrap_or_default(),
                subtitle: h.get("strapline").and_then(text_opt),
                layout: if list { ShelfLayout::List } else { ShelfLayout::Carousel },
                items,
                more: more_link(h),
            })
        }
        "musicShelfRenderer" => {
            let more = r.get("bottomEndpoint").map(nav).and_then(|n| {
                Some(PageLink { title: r.get("bottomText").and_then(text_opt).unwrap_or_else(|| "Show all".into()), browse_id: n.browse_id?, params: n.params, hue: None })
            });
            Some(Shelf { title: r.get("title").and_then(text_opt).unwrap_or_default(), subtitle: None, layout: ShelfLayout::List, items: items_of(r.get("contents").unwrap_or(&Value::Null)), more })
        }
        "musicPlaylistShelfRenderer" => Some(Shelf { title: String::new(), subtitle: None, layout: ShelfLayout::List, items: items_of(r.get("contents").unwrap_or(&Value::Null)), more: None }),
        "gridRenderer" => {
            let title = r.get("header").and_then(|h| find(h, "title")).and_then(text_opt).unwrap_or_default();
            Some(Shelf { title, subtitle: None, layout: ShelfLayout::Grid, items: items_of(r.get("items").unwrap_or(&Value::Null)), more: None })
        }
        "itemSectionRenderer" => {
            // Wrapper: either a single nested section or a flat item list.
            let contents = r.get("contents")?.as_array()?;
            if let Some(inner) = contents.iter().find_map(|c| shelf(c, &mut 0)) {
                return Some(inner);
            }
            let items: Vec<ShelfItem> = contents.iter().filter_map(item).collect();
            (!items.is_empty()).then(|| Shelf { title: String::new(), subtitle: None, layout: ShelfLayout::List, items, more: None })
        }
        "musicCardShelfRenderer" => {
            let mut items = Vec::new();
            if let Some(top) = card_top(r) {
                items.push(top);
            }
            items.extend(items_of(r.get("contents").unwrap_or(&Value::Null)));
            Some(Shelf { title: r.get("header").and_then(|h| find(h, "title")).and_then(text_opt).unwrap_or_else(|| "Top result".into()), subtitle: None, layout: ShelfLayout::List, items, more: None })
        }
        "musicDescriptionShelfRenderer" | "musicTastebuilderShelfRenderer" | "messageRenderer" | "musicNotifierShelfRenderer" | "musicResponsiveHeaderRenderer" => None,
        _ => {
            *warnings += 1;
            None
        }
    }
}

/// The main item of a search "top result" card.
pub fn card_top(r: &Value) -> Option<ShelfItem> {
    let title_run = r.get("title").map(runs).and_then(|r| r.first())?;
    let title = title_run.get("text")?.as_str()?.to_string();
    let n = title_run.get("navigationEndpoint").map(nav).or_else(|| r.get("onTap").map(nav))?;
    let meta = meta_from_runs(r.get("subtitle").map(runs).unwrap_or(&[]));
    let subtitle = r.get("subtitle").and_then(text).unwrap_or_default();
    if let Some(vid) = n.video_id {
        return Some(ShelfItem::Track(Track {
            id: ItemId::ytm(&vid),
            title,
            artists: meta.artists,
            album: meta.album,
            duration: meta.duration,
            explicit: explicit(r.get("subtitleBadges").unwrap_or(&Value::Null)),
            art: art_of(r),
            kind: kind_of_video(n.video_type.as_deref()),
            plays: meta.plays,
            liked: None,
            playable: true,
        }));
    }
    let bid = n.browse_id?;
    match n.page_type.as_deref() {
        Some(p) if p.contains("ARTIST") => Some(ShelfItem::Artist(ArtistCard { id: ItemId::ytm(&bid), name: title, subtitle, art: art_of(r) })),
        _ => Some(ShelfItem::Collection(CollectionCard {
            id: ItemId::ytm(&bid),
            kind: collection_kind(&bid, n.page_type.as_deref(), meta.label.as_deref()),
            title,
            subtitle,
            artists: meta.artists,
            year: meta.year,
            art: art_of(r),
            play_id: None,
        })),
    }
}

pub fn shelves_of(contents: &Value, warnings: &mut u32) -> Vec<Shelf> {
    contents.as_array().map(|a| a.iter().filter_map(|c| shelf(c, warnings)).filter(|s| !s.items.is_empty()).collect()).unwrap_or_default()
}

/// First section list in a browse response (single or two column).
pub fn section_list(resp: &Value) -> Option<&Value> {
    let c = resp.get("contents")?;
    if let Some(s) = c.get("singleColumnBrowseResultsRenderer") {
        return find(s, "sectionListRenderer");
    }
    if let Some(t) = c.get("twoColumnBrowseResultsRenderer") {
        return t.get("secondaryContents").and_then(|s| s.get("sectionListRenderer")).or_else(|| find(t, "sectionListRenderer"));
    }
    if let Some(t) = c.get("tabbedSearchResultsRenderer") {
        return find(t, "sectionListRenderer");
    }
    find(c, "sectionListRenderer")
}

pub fn next_continuation(v: &Value) -> Option<String> {
    // Old style: continuations[].nextContinuationData.continuation
    if let Some(c) = v.get("continuations").and_then(Value::as_array) {
        for x in c {
            if let Some(t) = x.get("nextContinuationData").or_else(|| x.get("nextRadioContinuationData")).and_then(|d| d.get("continuation")).and_then(Value::as_str) {
                return Some(t.to_string());
            }
        }
    }
    None
}

/// New style: a trailing `continuationItemRenderer` in a list.
pub fn item_continuation(list: &Value) -> Option<String> {
    let last = list.as_array()?.last()?;
    last.get("continuationItemRenderer").and_then(|c| find(c, "token")).and_then(Value::as_str).map(str::to_string)
}

pub fn chips(section_list: &Value) -> Vec<Chip> {
    let mut out = Vec::new();
    let Some(cs) = section_list.get("header").and_then(|h| find(h, "chips")).and_then(Value::as_array) else { return out };
    for c in cs {
        let Some(r) = c.get("chipCloudChipRenderer") else { continue };
        let Some(title) = r.get("text").and_then(text_opt) else { continue };
        let n = r.get("navigationEndpoint").map(nav);
        let Some(n) = n else { continue };
        let Some(bid) = n.browse_id else { continue };
        out.push(Chip { title, browse_id: bid, params: n.params, selected: r.get("isSelected").and_then(Value::as_bool).unwrap_or(false) });
    }
    out
}

pub fn is_signed_out(resp: &Value) -> bool {
    // serviceTrackingParams … key "logged_in" value "0"
    let mut params = Vec::new();
    find_all(resp.get("responseContext").unwrap_or(&Value::Null), "params", &mut params);
    params.iter().filter_map(|p| p.as_array()).flatten().any(|kv| kv.get("key").and_then(Value::as_str) == Some("logged_in") && kv.get("value").and_then(Value::as_str) == Some("0"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("5:21"), Some(321.0));
        assert_eq!(parse_duration("1:02:03"), Some(3723.0));
        assert_eq!(parse_duration("2001"), None);
        assert_eq!(parse_duration("a:b"), None);
    }

    #[test]
    fn hue_from_argb() {
        assert_eq!(argb_hue(0xffff0000), 0);
        assert_eq!(argb_hue(0xff00ff00), 120);
        assert_eq!(argb_hue(0xff0000ff), 240);
    }

    #[test]
    fn kinds() {
        assert_eq!(collection_kind("MPREb_x", None, Some("Single")), CollectionKind::Single);
        assert_eq!(collection_kind("VLRDCLAK5uy_x", None, None), CollectionKind::Mix);
        assert_eq!(collection_kind("VLPLx", None, None), CollectionKind::Playlist);
        assert_eq!(collection_kind("VLLM", None, None), CollectionKind::Liked);
    }
}
