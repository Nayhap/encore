//! Parser tests against recorded, scrubbed anonymous responses
//! (`encore-cli record`) and hand-built signed-in shapes.

use encore_core::model::*;
use encore_ytm::pages::{self, Cont};
use serde_json::Value;

fn fx(name: &str) -> Value {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(format!("{name}.json"));
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn fixtures_contain_no_secrets() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    for e in std::fs::read_dir(dir).unwrap() {
        let s = std::fs::read_to_string(e.unwrap().path()).unwrap();
        for bad in ["SAPISID", "__Secure-", "googlevideo.com", "\"visitorData\"", "LOGIN_INFO", "\"trackingParams\""] {
            assert!(!s.contains(bad), "fixture contains {bad}");
        }
        assert_eq!(encore_core::redact::redact(&s), s);
    }
}

#[test]
fn home_has_shelves_chips_and_continuation() {
    let p = pages::browse_page(&fx("home")).unwrap();
    assert!(p.value.shelves.len() >= 2, "shelves {}", p.value.shelves.len());
    assert!(p.value.chips.len() >= 3);
    assert!(p.value.shelves.iter().all(|s| !s.items.is_empty()));
    assert!(matches!(p.cont, Some(Cont::Query(_))));
    let tracks: Vec<&Track> = p.value.shelves.iter().flat_map(|s| &s.items).filter_map(|i| if let ShelfItem::Track(t) = i { Some(t) } else { None }).collect();
    assert!(!tracks.is_empty());
    assert!(tracks.iter().all(|t| !t.title.is_empty() && t.art.is_some()));
}

#[test]
fn explore_has_links_and_new_releases() {
    let p = pages::browse_page(&fx("explore")).unwrap().value;
    let links: Vec<&PageLink> = p.shelves.iter().flat_map(|s| &s.items).filter_map(|i| if let ShelfItem::Link(l) = i { Some(l) } else { None }).collect();
    assert!(links.iter().any(|l| l.browse_id == "FEmusic_new_releases"));
    assert!(links.iter().any(|l| l.browse_id == "FEmusic_moods_and_genres_category" && l.params.is_some()));
    assert!(p.shelves.iter().any(|s| s.items.iter().any(|i| matches!(i, ShelfItem::Collection(c) if c.id.remote().starts_with("MPRE")))));
}

#[test]
fn moods_have_hues() {
    let p = pages::browse_page(&fx("moods")).unwrap().value;
    let n = p.shelves.iter().flat_map(|s| &s.items).filter(|i| matches!(i, ShelfItem::Link(l) if l.hue.is_some())).count();
    assert!(n >= 8, "{n}");
}

#[test]
fn search_all_buckets_results() {
    let r = pages::search(&fx("search_all"), "daft punk").unwrap().value;
    assert!(matches!(r.top, Some(ShelfItem::Artist(ref a)) if a.name == "Daft Punk"));
    assert!(!r.songs.is_empty() && !r.albums.is_empty() && !r.artists.is_empty() && !r.playlists.is_empty());
    // songs under the artist card get the artist stamped in
    assert!(r.songs.iter().all(|t| !t.artists.is_empty()), "{:?}", r.songs.iter().map(|t| &t.title).collect::<Vec<_>>());
    assert!(r.albums.iter().all(|a| a.year.is_some() && a.id.remote().starts_with("MPRE")), "{:?}", r.albums);
}

#[test]
fn search_songs_filter_has_continuation_and_durations() {
    let p = pages::search(&fx("search_songs"), "daft punk").unwrap();
    assert!(p.value.songs.len() >= 10);
    assert!(p.value.songs.iter().filter(|t| t.duration.is_some()).count() >= 10);
    assert!(p.value.songs.iter().all(|t| t.kind == TrackKind::Song));
    assert!(p.value.songs.iter().any(|t| t.album.as_ref().is_some_and(|a| a.id.is_some())));
    assert!(p.cont.is_some());
}

#[test]
fn suggestions_parse() {
    let s = pages::suggestions(&fx("suggestions"));
    assert!(s.iter().any(|x| x.starts_with("daft p")), "{s:?}");
}

#[test]
fn album_parses_header_and_tracks() {
    let c = pages::collection(&fx("album"), &ItemId::ytm("MPREb_7ltM34kr0mH")).unwrap().value;
    assert_eq!(c.title, "Discovery");
    assert_eq!(c.kind, CollectionKind::Album);
    assert_eq!(c.year, Some(2001));
    assert_eq!(c.entries.len(), 14);
    assert_eq!(c.track_count, Some(14));
    assert!(c.playlist_id.as_deref().is_some_and(|p| p.starts_with("OLAK5uy_")));
    assert_eq!(c.artists[0].name, "Daft Punk");
    let first = &c.entries[0].track;
    assert_eq!(first.title, "One More Time");
    assert_eq!(first.duration, Some(321.0));
    assert_eq!(first.artists[0].name, "Daft Punk", "artist stamped from header");
    assert_eq!(first.album.as_ref().unwrap().name, "Discovery");
    assert!(first.art.is_some());
}

#[test]
fn long_playlist_paginates() {
    let id = ItemId::ytm("VLPLgVLQgm4lkANkwLnIYZiwf6o2jKIc5Th3");
    let p = pages::collection(&fx("playlist_long"), &id).unwrap();
    assert_eq!(p.value.entries.len(), 100);
    assert!(matches!(p.cont, Some(Cont::Body(_))));
    assert!(p.value.artists.iter().any(|a| !a.name.is_empty()), "author from facepile");
    assert!(p.value.entries.iter().all(|e| e.set_video_id.is_some()), "setVideoId belongs to the entry");
    let more = pages::collection_continuation(&fx("playlist_continuation"), 100).unwrap();
    assert_eq!(more.value.len(), 100);
    assert!(more.cont.is_some());
}

#[test]
fn artist_parses_sections() {
    let a = pages::artist(&fx("artist"), &ItemId::ytm("UCRr1xG_2WIDs18a6cIiCxeA")).unwrap().value;
    assert_eq!(a.name, "Daft Punk");
    assert!(a.subscribers.is_some());
    assert!(a.top_songs.len() >= 5);
    assert!(a.channel_id.is_some());
    assert!(a.radio_playlist.is_some());
    assert!(a.shelves.iter().any(|s| s.title == "Albums"));
    assert!(a.shelves.iter().any(|s| s.items.iter().any(|i| matches!(i, ShelfItem::Artist(_)))));
    assert!(a.description.is_some());
}

#[test]
fn radio_queue_and_tabs() {
    let n = pages::next(&fx("next_radio")).unwrap();
    assert!(n.tracks.len() >= 20);
    assert_eq!(n.tracks[0].id.remote(), "4D7u5KF7SP8");
    assert!(n.tracks[0].duration.is_some());
    assert!(n.cont.is_some());
    assert!(n.lyrics_browse_id.as_deref().is_some_and(|b| b.starts_with("MPLY")));
}

#[test]
fn timed_and_plain_lyrics() {
    let id = ItemId::ytm("4D7u5KF7SP8");
    let t = pages::timed_lyrics(&fx("lyrics_android"), &id).unwrap();
    assert_eq!(t.sync, LyricsSync::Line);
    assert!(t.lines.len() > 20);
    assert!((t.lines[0].t - 30.0).abs() < 0.01, "ms → s and ♪ intro dropped: {}", t.lines[0].t);
    assert!(t.lines.windows(2).all(|w| w[0].t <= w[1].t));
    let p = pages::plain_lyrics(&fx("lyrics_web"), &id).unwrap();
    assert_eq!(p.sync, LyricsSync::None);
    assert!(p.lines.is_empty());
    assert!(p.plain.unwrap().contains("Like the legend of the phoenix"));
}

#[test]
fn accounts_list_brand_and_logins() {
    let a = pages::accounts(&fx("accounts_list_synthetic"));
    assert_eq!(a.len(), 3);
    assert!(a[0].selected);
    assert_eq!(a[0].selection.page_id, None);
    assert_eq!(a[0].selection.user_session_id, None);
    assert_eq!(a[0].channel.handle.as_deref(), Some("@testperson"));
    assert_eq!(a[1].selection.page_id.as_deref(), Some("112233445566778899000"));
    assert_eq!(a[1].selection.user_session_id.as_deref(), Some("100000000000000000001"));
    assert!(a[1].channel.is_brand);
    assert_eq!(a[2].selection.auth_user, 1);
    assert_eq!(a[2].channel.key, "1:primary");
}

#[test]
fn library_grid_skips_pseudo_items() {
    let p = pages::library(&fx("library_playlists_synthetic"), LibrarySection::Playlists).unwrap();
    let titles: Vec<String> = p.value.iter().map(|i| match i {
        ShelfItem::Collection(c) => c.title.clone(),
        other => format!("{other:?}"),
    }).collect();
    assert_eq!(titles, vec!["Liked music", "Synthetic Road Trip"]);
    assert!(matches!(p.cont, Some(Cont::Query(_))));
}
