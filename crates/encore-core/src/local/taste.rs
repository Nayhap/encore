//! The block list (D36, lane D): songs and artists that never play on their
//! own. The player asks through `service::Admission`; lane D implements it
//! over this table and the "Hide podcasts" setting (D27).

use std::collections::HashSet;

use rusqlite::params;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::Result;
use crate::local::Owner;
use crate::model::{CollectionKind, ItemId, Page, ShelfItem, Track, TrackKind};
use crate::store::{Store, db_err, now};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum BlockKind {
    Track,
    Artist,
}

impl BlockKind {
    fn as_str(self) -> &'static str {
        match self {
            BlockKind::Track => "track",
            BlockKind::Artist => "artist",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Blocked {
    pub kind: BlockKind,
    pub target: ItemId,
    /// The song's title or the artist's name.
    pub label: String,
    /// A song's artist line.
    pub detail: Option<String>,
    pub created: String,
}

/// One owner's block list in memory, for the questions asked often (inside
/// the player's lock, for every row of a page).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlockSet {
    tracks: HashSet<ItemId>,
    artists: HashSet<ItemId>,
}

impl BlockSet {
    pub fn new(list: &[Blocked]) -> Self {
        let mut s = BlockSet::default();
        for b in list {
            match b.kind {
                BlockKind::Track => s.tracks.insert(b.target.clone()),
                BlockKind::Artist => s.artists.insert(b.target.clone()),
            };
        }
        s
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty() && self.artists.is_empty()
    }

    /// A song is blocked by its own id, its other version's (D21: blocking
    /// the song blocks its music video too), or any credited artist.
    pub fn blocks(&self, t: &Track) -> bool {
        self.tracks.contains(&t.id) || t.counterpart.as_ref().is_some_and(|c| self.tracks.contains(c)) || t.artists.iter().any(|a| a.id.as_ref().is_some_and(|id| self.artists.contains(id)))
    }

    pub fn blocks_artist(&self, id: &ItemId) -> bool {
        self.artists.contains(id)
    }
}

/// What may join the queue (D36, D27): not blocked, and not a podcast
/// episode while podcasts are hidden.
pub fn admits(t: &Track, blocks: &BlockSet, hide_podcasts: bool) -> bool {
    !(hide_podcasts && t.kind == TrackKind::Episode) && !blocks.blocks(t)
}

/// A page as the user wants to see it: blocked songs and artists left out,
/// podcast episodes too while they're hidden, then shelves left empty. The
/// cached copy is never changed, so a setting or an undo shows at once.
pub fn filter_page(mut page: Page, blocks: &BlockSet, hide_podcasts: bool) -> Page {
    if blocks.is_empty() && !hide_podcasts {
        return page;
    }
    for s in &mut page.shelves {
        s.items.retain(|i| match i {
            ShelfItem::Track(t) => admits(t, blocks, hide_podcasts),
            ShelfItem::Artist(a) => !blocks.blocks_artist(&a.id),
            // An album of a blocked artist; playlists (several artists) stay.
            ShelfItem::Collection(c) => !(c.artists.len() == 1 && c.artists[0].id.as_ref().is_some_and(|id| blocks.blocks_artist(id)) && matches!(c.kind, CollectionKind::Album | CollectionKind::Single | CollectionKind::Ep)),
            ShelfItem::Link(_) => true,
        });
    }
    page.shelves.retain(|s| !s.items.is_empty());
    page
}

impl Store {
    /// An owner's block list, newest first.
    pub fn blocked_list(&self, owner: &Owner) -> Result<Vec<Blocked>> {
        let c = self.conn.lock();
        let mut st = c.prepare_cached("SELECT kind, target, label, detail, created FROM blocked WHERE owner = ?1 ORDER BY created DESC, rowid DESC").map_err(db_err)?;
        let rows = st
            .query_map(params![owner.as_str()], |r| {
                let kind: String = r.get(0)?;
                Ok(Blocked { kind: if kind == "artist" { BlockKind::Artist } else { BlockKind::Track }, target: ItemId(r.get(1)?), label: r.get(2)?, detail: r.get(3)?, created: r.get::<_, i64>(4)?.to_string() })
            })
            .map_err(db_err)?;
        rows.collect::<std::result::Result<_, _>>().map_err(db_err)
    }

    /// Block a song or an artist; blocking one again keeps the first date.
    pub fn block(&self, owner: &Owner, kind: BlockKind, target: &ItemId, label: &str, detail: Option<&str>) -> Result<()> {
        self.conn
            .lock()
            .execute(
                "INSERT INTO blocked (owner, kind, target, label, detail, created) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(owner, kind, target) DO UPDATE SET label = excluded.label, detail = excluded.detail",
                params![owner.as_str(), kind.as_str(), target.0, label, detail, now()],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// Whether anything was removed.
    pub fn unblock(&self, owner: &Owner, kind: BlockKind, target: &ItemId) -> Result<bool> {
        let n = self.conn.lock().execute("DELETE FROM blocked WHERE owner = ?1 AND kind = ?2 AND target = ?3", params![owner.as_str(), kind.as_str(), target.0]).map_err(db_err)?;
        Ok(n > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CollectionCard, Credit, Shelf, ShelfLayout};

    fn track(id: &str, artist: &str, kind: TrackKind) -> Track {
        Track { id: ItemId::ytm(id), title: id.into(), artists: vec![Credit { name: artist.into(), id: Some(ItemId::ytm(artist)) }], album: None, duration: Some(200.0), explicit: false, art: None, kind, plays: None, liked: None, playable: true, counterpart: None }
    }

    #[test]
    fn the_list_is_per_owner_and_kept_once() {
        let s = Store::open_memory().unwrap();
        let (me, other) = (Owner::new("me"), Owner::anonymous());
        s.block(&me, BlockKind::Track, &ItemId::ytm("v1"), "Song", Some("Band")).unwrap();
        s.block(&me, BlockKind::Track, &ItemId::ytm("v1"), "Song", Some("Band")).unwrap();
        s.block(&me, BlockKind::Artist, &ItemId::ytm("UC1"), "Band", None).unwrap();
        assert_eq!(s.blocked_list(&me).unwrap().len(), 2);
        assert!(s.blocked_list(&other).unwrap().is_empty());
        assert!(s.unblock(&me, BlockKind::Track, &ItemId::ytm("v1")).unwrap());
        assert!(!s.unblock(&me, BlockKind::Track, &ItemId::ytm("v1")).unwrap());
        let left = s.blocked_list(&me).unwrap();
        assert_eq!((left.len(), left[0].kind), (1, BlockKind::Artist));
    }

    #[test]
    fn songs_videos_and_artists_are_blocked() {
        let s = Store::open_memory().unwrap();
        let me = Owner::new("me");
        s.block(&me, BlockKind::Track, &ItemId::ytm("song"), "Song", None).unwrap();
        s.block(&me, BlockKind::Artist, &ItemId::ytm("UCx"), "X", None).unwrap();
        let set = BlockSet::new(&s.blocked_list(&me).unwrap());
        assert!(set.blocks(&track("song", "UCa", TrackKind::Song)));
        let video = Track { counterpart: Some(ItemId::ytm("song")), ..track("video", "UCa", TrackKind::Video) };
        assert!(set.blocks(&video), "the song's music video");
        assert!(set.blocks(&track("other", "UCx", TrackKind::Song)), "by the artist");
        assert!(!set.blocks(&track("other", "UCa", TrackKind::Song)));
        let ep = track("ep", "UCa", TrackKind::Episode);
        assert!(admits(&ep, &set, false) && !admits(&ep, &set, true));
    }

    #[test]
    fn pages_lose_hidden_rows_and_empty_shelves() {
        let mut set = BlockSet::default();
        set.artists.insert(ItemId::ytm("UCx"));
        let shelf = |items| Shelf { title: "s".into(), subtitle: None, layout: ShelfLayout::Carousel, items, more: None };
        let album = |artist: &str, kind| ShelfItem::Collection(CollectionCard { id: ItemId::ytm("MPRE"), kind, title: "A".into(), subtitle: String::new(), artists: vec![Credit { name: artist.into(), id: Some(ItemId::ytm(artist)) }], year: None, art: None, play_id: None });
        let page = Page {
            title: None,
            chips: vec![],
            cursor: None,
            shelves: vec![
                shelf(vec![ShelfItem::Track(track("a", "UCa", TrackKind::Song)), ShelfItem::Track(track("b", "UCx", TrackKind::Song))]),
                shelf(vec![ShelfItem::Track(track("ep", "UCp", TrackKind::Episode))]),
                shelf(vec![album("UCx", CollectionKind::Album), album("UCx", CollectionKind::Playlist)]),
            ],
        };
        let out = filter_page(page.clone(), &set, true);
        assert_eq!(out.shelves.len(), 2, "the podcast shelf is gone");
        assert_eq!(out.shelves[0].items.len(), 1);
        assert_eq!(out.shelves[1].items.len(), 1, "the album goes, the playlist stays");
        assert_eq!(filter_page(page.clone(), &BlockSet::default(), false), page);
    }
}
