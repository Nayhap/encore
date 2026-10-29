//! Domain types shared by every service and the UI.
//!
//! Everything here crosses the IPC boundary, so it derives `specta::Type` and
//! serialises camelCase. Times are seconds as `f64`. Ids are
//! `ServiceId + opaque string` (D2), written `"ytm:<remote id>"` on the wire.

use std::fmt;

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ServiceId {
    YoutubeMusic,
}

impl ServiceId {
    pub fn prefix(self) -> &'static str {
        match self {
            ServiceId::YoutubeMusic => "ytm",
        }
    }
}

/// `ServiceId` + opaque remote id, serialised as `"ytm:<remote>"`.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[serde(transparent)]
pub struct ItemId(pub String);

impl ItemId {
    pub fn new(service: ServiceId, remote: &str) -> Self {
        ItemId(format!("{}:{}", service.prefix(), remote))
    }
    pub fn ytm(remote: &str) -> Self {
        Self::new(ServiceId::YoutubeMusic, remote)
    }
    /// Accepts `ytm:xyz` or a bare remote id (treated as YouTube Music).
    pub fn parse(s: &str) -> Self {
        if s.starts_with("ytm:") { ItemId(s.to_string()) } else { Self::ytm(s) }
    }
    pub fn service(&self) -> ServiceId {
        ServiceId::YoutubeMusic
    }
    pub fn remote(&self) -> &str {
        self.0.split_once(':').map(|(_, r)| r).unwrap_or(&self.0)
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A named link to an artist or album. `id` is only set when the service
/// gave a navigable id; it is never invented.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Credit {
    pub name: String,
    pub id: Option<ItemId>,
}

/// Artwork reference. `key` is opaque: the `art://` protocol maps it to the
/// source URL inside Rust (D14). `h1`/`h2` are hues (0..360) once computed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ArtRef {
    pub key: String,
    pub h1: Option<u16>,
    pub h2: Option<u16>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum TrackKind {
    Song,
    Video,
    Episode,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: ItemId,
    pub title: String,
    pub artists: Vec<Credit>,
    pub album: Option<Credit>,
    /// Seconds; `None` when unknown.
    pub duration: Option<f64>,
    pub explicit: bool,
    pub art: Option<ArtRef>,
    pub kind: TrackKind,
    /// Play count as the service writes it ("48M plays").
    pub plays: Option<String>,
    /// `None` = unknown, not "not liked".
    pub liked: Option<bool>,
    pub playable: bool,
    /// The same recording in its other form: the music video when `id` is
    /// the song. Never played on its own; kept for an explicit "play the
    /// video instead" action (D4).
    pub counterpart: Option<ItemId>,
}

impl Track {
    pub fn artist_line(&self) -> String {
        self.artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ")
    }

    /// `id` and, when known, `counterpart`: every ID this recording goes by.
    pub fn recording_ids(&self) -> impl Iterator<Item = &ItemId> {
        std::iter::once(&self.id).chain(self.counterpart.as_ref())
    }
}

/// One row of a playlist. Duplicate tracks are distinct entries; the
/// `setVideoId` belongs to the entry, not the track (D10).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub entry_id: String,
    pub track: Track,
    pub set_video_id: Option<String>,
    /// The video ID the service lists for this row. Playlist edits pair it
    /// with `set_video_id`. It differs from `track.id` when the row was
    /// resolved to its song (album rows that list the music video).
    pub video_id: ItemId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum CollectionKind {
    Album,
    Single,
    Ep,
    Playlist,
    Mix,
    Liked,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CollectionCard {
    pub id: ItemId,
    pub kind: CollectionKind,
    pub title: String,
    /// "Artist" or "Album • 2001" style line, as given.
    pub subtitle: String,
    pub artists: Vec<Credit>,
    pub year: Option<u16>,
    pub art: Option<ArtRef>,
    /// Watch-playlist id for the play button (OLAK…, RD…), if known.
    pub play_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ArtistCard {
    pub id: ItemId,
    pub name: String,
    pub subtitle: String,
    pub art: Option<ArtRef>,
}

/// A link to another browse page (moods, charts, new releases, "more").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PageLink {
    pub title: String,
    pub browse_id: String,
    pub params: Option<String>,
    /// Hue for coloured mood buttons.
    pub hue: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ShelfItem {
    Track(Track),
    Collection(CollectionCard),
    Artist(ArtistCard),
    Link(PageLink),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ShelfLayout {
    Carousel,
    List,
    Grid,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Shelf {
    pub title: String,
    pub subtitle: Option<String>,
    pub layout: ShelfLayout,
    pub items: Vec<ShelfItem>,
    pub more: Option<PageLink>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Chip {
    pub title: String,
    pub browse_id: String,
    pub params: Option<String>,
    pub selected: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub title: Option<String>,
    pub shelves: Vec<Shelf>,
    pub chips: Vec<Chip>,
    /// Opaque cursor id for more shelves (in-memory, TTL; D10).
    pub cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Collection {
    pub id: ItemId,
    pub kind: CollectionKind,
    pub title: String,
    pub subtitle: String,
    pub artists: Vec<Credit>,
    pub year: Option<u16>,
    pub art: Option<ArtRef>,
    pub description: Option<String>,
    /// "14 songs • 1 hour, 1 minute" as given.
    pub stats: Option<String>,
    pub track_count: Option<u32>,
    pub entries: Vec<Entry>,
    pub cursor: Option<String>,
    /// Playlist id used for playback/radio/save (OLAK… for albums).
    pub playlist_id: Option<String>,
    pub saved: Option<bool>,
    pub editable: bool,
    /// Extra shelves shown under an album ("Other versions", "More by").
    pub related: Vec<Shelf>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Artist {
    pub id: ItemId,
    pub name: String,
    pub subscribers: Option<String>,
    pub description: Option<String>,
    pub art: Option<ArtRef>,
    pub top_songs: Vec<Track>,
    /// Browse link to the full song list, if the service offers one.
    pub top_songs_more: Option<PageLink>,
    pub shelves: Vec<Shelf>,
    pub subscribed: Option<bool>,
    pub channel_id: Option<String>,
    pub radio_playlist: Option<String>,
    pub shuffle_playlist: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SearchFilter {
    Songs,
    Videos,
    Albums,
    Artists,
    CommunityPlaylists,
    FeaturedPlaylists,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    pub query: String,
    pub top: Option<ShelfItem>,
    pub songs: Vec<Track>,
    pub videos: Vec<Track>,
    pub albums: Vec<CollectionCard>,
    pub artists: Vec<ArtistCard>,
    pub playlists: Vec<CollectionCard>,
    pub did_you_mean: Option<String>,
    pub cursor: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum LibrarySection {
    Playlists,
    Albums,
    Artists,
    Songs,
    Liked,
    Subscriptions,
    History,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LibraryPage {
    pub section: LibrarySection,
    pub items: Vec<ShelfItem>,
    pub cursor: Option<String>,
}

/// What a radio / up-next request is seeded with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RadioSeed {
    Track { id: ItemId },
    Playlist { playlist_id: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpNext {
    pub tracks: Vec<Track>,
    pub playlist_id: Option<String>,
    pub cursor: Option<String>,
}

// ---------- lyrics ----------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Word {
    pub t: f64,
    pub end: Option<f64>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LyricLine {
    pub t: f64,
    pub end: Option<f64>,
    pub text: String,
    pub words: Option<Vec<Word>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum LyricsSync {
    Line,
    Word,
    None,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Lyrics {
    pub track_id: ItemId,
    pub sync: LyricsSync,
    /// Timed lines; empty for plain lyrics.
    pub lines: Vec<LyricLine>,
    /// Untimed text when no timing exists.
    pub plain: Option<String>,
    /// Provider name shown to the user ("YouTube Music", "LRCLIB").
    pub source: String,
    pub attribution: Option<String>,
}

// ---------- accounts ----------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[serde(rename = "AccountChannel")]
pub struct Channel {
    /// Stable key for selection: "<authUser>:<pageId or 'primary'>".
    pub key: String,
    pub name: String,
    pub handle: Option<String>,
    pub page_id: Option<String>,
    pub auth_user: u32,
    pub is_brand: bool,
    pub photo: Option<ArtRef>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum AuthState {
    Anonymous,
    /// Cookies imported or signed in; waiting for the user to choose a channel.
    ChoosingChannel { channels: Vec<Channel>, source: String },
    SignedIn { channel: Channel, channels: Vec<Channel>, source: String, premium: Option<bool> },
    /// The stored session stopped working; offer re-import / sign-in.
    Expired { source: String, reason: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BrowserProfile {
    pub id: String,
    pub browser: String,
    pub name: String,
}

/// Result of a write. HTTP 200 is not proof; ambiguous results refetch (D10).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum MutationOutcome {
    Done,
    Rejected,
    OutcomeUnknown,
}
