//! Things the user keeps on this computer (lane C): bookmarks (D25), saved
//! queues (D33) and folders (D34). Tables in `local::SCHEMA`; the store
//! methods are lane C's to write here.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::model::{ArtRef, CollectionCard, ItemId, Track};

/// A page that can be opened again: the same routes as the UI's `View`.
/// Links keep their `params` (an artist's "All songs" needs them), never a
/// continuation cursor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "name", rename_all = "camelCase")]
pub enum PageRef {
    Collection {
        id: ItemId,
        params: Option<String>,
    },
    Artist {
        id: ItemId,
    },
    Page {
        #[serde(rename = "browseId")]
        browse_id: String,
        params: Option<String>,
        title: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum BookmarkKind {
    Page,
    Track,
    /// A song at a time: opening it plays from there.
    Moment,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum BookmarkTarget {
    Page(PageRef),
    Track(Track),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Bookmark {
    pub id: String,
    pub kind: BookmarkKind,
    pub target: BookmarkTarget,
    /// Seconds into the song, for a moment.
    pub position: Option<f64>,
    pub title: String,
    pub subtitle: String,
    pub art: Option<ArtRef>,
    /// Unix seconds, as a string on the wire (D13).
    pub created: String,
}

/// A saved queue as listed (the queue itself stays in Rust until restored).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SavedQueueInfo {
    pub id: String,
    pub name: String,
    pub songs: u32,
    pub duration: Option<f64>,
    /// The song it restores to.
    pub current: Option<Track>,
    pub position: f64,
    pub updated: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: String,
    pub name: String,
    /// An opaque local art key (served by `art://`, never a path).
    pub cover: Option<String>,
    pub collapsed: bool,
    pub items: Vec<FolderItem>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FolderItem {
    pub item_id: ItemId,
    pub card: CollectionCard,
}
