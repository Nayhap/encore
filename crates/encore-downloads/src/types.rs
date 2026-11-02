//! Wire types for downloads (D13, D23). u64 counters are strings; sizes are
//! `f64` bytes, as in `CacheInfo` and `RuntimeStatus`.

use encore_core::model::{CollectionCard, ItemId, Track};
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum DownloadState {
    Queued,
    Downloading,
    Ready,
    Failed,
    Removed,
}

/// Why downloads are waiting, when it isn't the user's pause.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum DownloadWait {
    Offline,
    BotCheck,
    Stopped,
    DiskFull,
    SetupNeeded,
    Elsewhere,
    Locked,
}

/// One downloaded, or downloading, song.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DownloadItem {
    pub track: Track,
    pub state: DownloadState,
    /// Why it failed, as a sentence.
    pub error: Option<String>,
    /// Size on disk once downloaded.
    pub bytes: f64,
    pub codec: Option<String>,
    pub kbps: Option<u32>,
    pub itag: Option<u32>,
    /// Unix seconds when it was asked for.
    pub added: f64,
    /// Bumps on every change of this song.
    pub version: String,
    /// The attempt progress belongs to.
    pub attempt: String,
}

/// The song being downloaded right now.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ActiveDownload {
    pub track_id: ItemId,
    pub attempt: String,
    pub bytes: f64,
    pub total: Option<f64>,
}

/// Counts, progress and waits, sent at most four times a second.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DownloadsSummary {
    /// Bumps on every change of a song or a kept collection; a gap means
    /// the UI missed one and asks for a snapshot.
    pub revision: String,
    /// Downloads work on this computer.
    pub available: bool,
    pub songs: u32,
    /// Waiting or downloading.
    pub waiting: u32,
    pub failed: u32,
    /// Space used: finished songs, partial ones and covers.
    pub bytes: f64,
    pub active: Option<ActiveDownload>,
    /// Paused by the user.
    pub paused: bool,
    pub wait: Option<DownloadWait>,
    /// What the wait (or `available: false`) means, as a sentence.
    pub message: Option<String>,
    pub online: bool,
    /// Kept collections being checked for changes.
    pub syncing: Vec<ItemId>,
}

/// A playlist, album or liked songs kept downloaded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct KeptCollection {
    pub card: CollectionCard,
    /// Whose copy this is (an opaque account key): two accounts may keep
    /// the same playlist.
    pub account: String,
    /// The channel that keeps it.
    pub channel: Option<String>,
    /// Kept by the account in use, so it stays up to date.
    pub mine: bool,
    pub songs: u32,
    pub ready: u32,
    pub failed: u32,
    pub bytes: f64,
    /// Unix seconds of the last check for changes.
    pub synced: Option<f64>,
    /// Why the last check didn't work, as a sentence.
    pub error: Option<String>,
    /// Private or deleted on YouTube Music now; the songs stay.
    pub gone: bool,
    pub paused: bool,
}

/// Everything the UI shows about downloads, at once.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DownloadsSnapshot {
    pub summary: DownloadsSummary,
    pub items: Vec<DownloadItem>,
    pub collections: Vec<KeptCollection>,
}

/// What "Remove download" did.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DownloadRemoved {
    pub removed: u32,
    /// Songs that stay because a kept collection needs them, with its title.
    pub kept: Vec<KeptBy>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct KeptBy {
    pub track_id: ItemId,
    /// The kept collection that has it; `None` when another account
    /// downloaded it too.
    pub collection: Option<String>,
}

/// Settings → Downloads, measured on disk.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DownloadsStorage {
    pub songs: u32,
    pub bytes: f64,
    pub location: String,
}
