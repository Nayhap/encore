//! Reading a kept collection from the service (D23). Only a list that
//! really ended may remove songs from the saved order; one that stalled,
//! failed halfway or hit the limits keeps the last good order.

use std::sync::Arc;
use std::time::{Duration, Instant};

use encore_audio::cache::sanitize_key;
use encore_core::error::ErrorCode;
use encore_core::model::{Collection, CollectionCard, Entry, ItemId, Track};
use encore_core::service::{GateGuard, More, MusicService, PlaybackGate, RequestContext};

const MAX_ENTRIES: usize = 10_000;
const MAX_PAGES: usize = 250;
const MAX_TIME: Duration = Duration::from_secs(300);
/// Between continuation pages, so a long playlist isn't read in a burst.
const PAGE_GAP: Duration = Duration::from_millis(600);

#[derive(Debug)]
pub enum Traversal {
    /// The whole list.
    Complete { card: CollectionCard, members: Vec<(String, Track)> },
    /// The start of it: the list stalled, failed later or was too long.
    Partial { card: CollectionCard, members: Vec<(String, Track)> },
    /// Private or deleted now.
    Gone,
    Failed,
    /// Offline or the account changed meanwhile: nothing to apply.
    Stopped,
}

pub fn card_of(c: &Collection) -> CollectionCard {
    CollectionCard {
        id: c.id.clone(),
        kind: c.kind,
        title: c.title.clone(),
        subtitle: c.subtitle.clone(),
        artists: c.artists.clone(),
        year: c.year,
        art: c.art.clone(),
        play_id: c.playlist_id.clone(),
    }
}

/// The saved order: (row key, the track that plays) for each playable row.
/// D4: an album row listed by its music video plays the song, and the song
/// is what is downloaded (`entry.track.id`), never the counterpart.
pub fn members_of(entries: &[Entry]) -> Vec<(String, Track)> {
    entries
        .iter()
        .filter(|e| e.track.playable && sanitize_key(e.track.id.remote()).is_some())
        .map(|e| (e.set_video_id.clone().unwrap_or_else(|| e.entry_id.clone()), e.track.clone()))
        .collect()
}

/// Wait for a turn to ask YouTube: never while playback resolves.
/// `None` when `go_on` says to stop.
async fn admitted(gate: &Arc<PlaybackGate>, go_on: &(dyn Fn() -> bool + Send + Sync)) -> Option<GateGuard> {
    loop {
        if !go_on() {
            return None;
        }
        if let Some(g) = gate.try_background() {
            return Some(g);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Read a collection to its end. `first` is its first page when the
/// caller already has it. `go_on` is asked before every request (online,
/// same account).
pub async fn traverse(service: &dyn MusicService, ctx: &RequestContext, id: &ItemId, first: Option<Collection>, gate: &Arc<PlaybackGate>, go_on: &(dyn Fn() -> bool + Send + Sync)) -> Traversal {
    let c = match first {
        Some(c) => c,
        None => {
            let Some(_turn) = admitted(gate, go_on).await else { return Traversal::Stopped };
            match service.collection(ctx, id).await {
                Ok(c) => c,
                Err(e) if matches!(e.code, ErrorCode::NotFound | ErrorCode::Unavailable) => return Traversal::Gone,
                Err(e) => {
                    tracing::info!(collection = %id, code = ?e.code, "couldn't check a kept collection");
                    return Traversal::Failed;
                }
            }
        }
    };
    let card = card_of(&c);
    let mut entries = c.entries;
    let mut cursor = c.cursor;
    let t0 = Instant::now();
    let mut pages = 0;
    let complete = loop {
        let Some(cur) = cursor.take() else { break true };
        if entries.len() >= MAX_ENTRIES || pages >= MAX_PAGES || t0.elapsed() > MAX_TIME {
            break false;
        }
        tokio::time::sleep(PAGE_GAP).await;
        let Some(_turn) = admitted(gate, go_on).await else { return Traversal::Stopped };
        pages += 1;
        match service.collection_more_checked(ctx, &cur).await {
            Ok((more, More::Next(next))) => {
                entries.extend(more);
                cursor = Some(next);
            }
            Ok((more, More::Ended)) => {
                entries.extend(more);
                break true;
            }
            Ok((more, More::Stalled)) => {
                entries.extend(more);
                break false;
            }
            Err(e) => {
                tracing::info!(collection = %id, code = ?e.code, pages, "a kept collection stopped halfway");
                break false;
            }
        }
    };
    let members = members_of(&entries);
    if complete { Traversal::Complete { card, members } } else { Traversal::Partial { card, members } }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use async_trait::async_trait;
    use encore_core::error::{AppError, Result};
    use encore_core::model::*;
    use encore_core::service::Capabilities;

    pub fn track(id: &str) -> Track {
        Track { id: ItemId::ytm(id), title: id.into(), artists: vec![], album: None, duration: Some(100.0), explicit: false, art: None, kind: TrackKind::Song, plays: None, liked: None, playable: true, counterpart: None }
    }

    fn entry(id: &str) -> Entry {
        Entry { entry_id: format!("e-{id}"), track: track(id), set_video_id: Some(format!("s-{id}")), video_id: ItemId::ytm(id) }
    }

    /// A playlist of two pages; the second ends as `end` says.
    pub struct Pages {
        pub end: More,
    }

    #[async_trait]
    impl MusicService for Pages {
        fn service(&self) -> ServiceId {
            ServiceId::YoutubeMusic
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities::default()
        }
        fn context(&self) -> RequestContext {
            RequestContext { account: "a".into(), epoch: 1 }
        }
        async fn home(&self, _: &RequestContext) -> Result<Page> {
            Err(AppError::internal("unused"))
        }
        async fn explore(&self, _: &RequestContext) -> Result<Page> {
            Err(AppError::internal("unused"))
        }
        async fn browse_page(&self, _: &RequestContext, _: &str, _: Option<&str>) -> Result<Page> {
            Err(AppError::internal("unused"))
        }
        async fn more_shelves(&self, _: &RequestContext, _: &str) -> Result<Page> {
            Err(AppError::internal("unused"))
        }
        async fn search(&self, _: &RequestContext, _: &str, _: Option<SearchFilter>) -> Result<SearchResults> {
            Err(AppError::internal("unused"))
        }
        async fn search_more(&self, _: &RequestContext, _: &str) -> Result<SearchResults> {
            Err(AppError::internal("unused"))
        }
        async fn suggestions(&self, _: &RequestContext, _: &str) -> Result<Vec<String>> {
            Ok(vec![])
        }
        async fn collection(&self, _: &RequestContext, id: &ItemId) -> Result<Collection> {
            Ok(Collection {
                id: id.clone(),
                kind: CollectionKind::Playlist,
                title: "List".into(),
                subtitle: String::new(),
                artists: vec![],
                year: None,
                art: None,
                description: None,
                stats: None,
                track_count: None,
                entries: vec![entry("a"), entry("b")],
                cursor: Some("c1".into()),
                playlist_id: None,
                saved: None,
                editable: false,
                privacy: None,
                related: vec![],
            })
        }
        async fn collection_more(&self, _: &RequestContext, _: &str) -> Result<(Vec<Entry>, Option<String>)> {
            Ok((vec![], None))
        }
        async fn collection_more_checked(&self, _: &RequestContext, _: &str) -> Result<(Vec<Entry>, More)> {
            Ok((vec![entry("c")], self.end.clone()))
        }
        async fn artist(&self, _: &RequestContext, _: &ItemId) -> Result<Artist> {
            Err(AppError::internal("unused"))
        }
        async fn library(&self, _: &RequestContext, _: LibrarySection) -> Result<LibraryPage> {
            Err(AppError::internal("unused"))
        }
        async fn library_more(&self, _: &RequestContext, _: LibrarySection, _: &str) -> Result<LibraryPage> {
            Err(AppError::internal("unused"))
        }
        async fn track(&self, _: &RequestContext, _: &ItemId) -> Result<Track> {
            Err(AppError::internal("unused"))
        }
        async fn up_next(&self, _: &RequestContext, _: &RadioSeed, _: Option<&str>) -> Result<UpNext> {
            Err(AppError::internal("unused"))
        }
        async fn set_like(&self, _: &RequestContext, _: &ItemId, _: bool) -> Result<MutationOutcome> {
            Err(AppError::internal("unused"))
        }
        async fn set_saved(&self, _: &RequestContext, _: &str, _: bool) -> Result<MutationOutcome> {
            Err(AppError::internal("unused"))
        }
        async fn set_subscribed(&self, _: &RequestContext, _: &str, _: bool) -> Result<MutationOutcome> {
            Err(AppError::internal("unused"))
        }
        async fn playlist_add(&self, _: &RequestContext, _: &str, _: &[ItemId], _: bool) -> Result<MutationOutcome> {
            Err(AppError::internal("unused"))
        }
        async fn playlist_remove(&self, _: &RequestContext, _: &str, _: &[(ItemId, String)]) -> Result<MutationOutcome> {
            Err(AppError::internal("unused"))
        }
        async fn lyrics(&self, _: &RequestContext, _: &ItemId) -> Result<Option<Lyrics>> {
            Ok(None)
        }
    }

    fn ids(m: &[(String, Track)]) -> Vec<&str> {
        m.iter().map(|(_, t)| t.id.remote()).collect()
    }

    #[tokio::test]
    async fn an_ended_list_is_complete() {
        let t = traverse(&Pages { end: More::Ended }, &RequestContext { account: "a".into(), epoch: 1 }, &ItemId::ytm("VLPL1"), None, &Arc::new(PlaybackGate::default()), &|| true).await;
        let members = match t {
            Traversal::Complete { members, .. } => members,
            other => panic!("{other:?}"),
        };
        assert_eq!(ids(&members), vec!["a", "b", "c"]);
        assert_eq!(members[0].0, "s-a", "rows are keyed by their own id");
    }

    #[tokio::test]
    async fn a_stalled_list_is_only_partial() {
        let t = traverse(&Pages { end: More::Stalled }, &RequestContext { account: "a".into(), epoch: 1 }, &ItemId::ytm("VLPL1"), None, &Arc::new(PlaybackGate::default()), &|| true).await;
        assert!(matches!(t, Traversal::Partial { .. }), "{t:?}");
    }

    #[test]
    fn rows_download_the_song_that_plays() {
        let mut e = entry("song");
        e.video_id = ItemId::ytm("video");
        e.track.counterpart = Some(ItemId::ytm("video"));
        let mut gone = entry("x");
        gone.track.playable = false;
        let m = members_of(&[e, gone]);
        assert_eq!(ids(&m), vec!["song"]);
    }
}
