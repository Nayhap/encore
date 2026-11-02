//! Typed events (D13). The UI subscribes before calling `bootstrap`, and
//! applies only newer revisions.

use encore_core::model::{AuthState, Lyrics};
use encore_downloads::{DownloadItem, DownloadsSummary};
use encore_player::{PlayerSnapshot, PositionAnchor, QueueSplice};
use encore_resolver::RuntimeStatus;
use serde::{Deserialize, Serialize};
use tauri_specta::Event;

/// `player:state`, sent on every change.
#[derive(Serialize, Deserialize, Clone, specta::Type, Event)]
pub struct PlayerStateEvent(pub PlayerSnapshot);

/// `player:position`, a 2 Hz anchor (also right after seek/pause).
#[derive(Serialize, Deserialize, Clone, specta::Type, Event)]
pub struct PlayerPositionEvent(pub PositionAnchor);

/// Queue splice against the previous revision.
#[derive(Serialize, Deserialize, Clone, specta::Type, Event)]
pub struct QueueEvent(pub QueueSplice);

#[derive(Serialize, Deserialize, Clone, specta::Type, Event)]
pub struct AuthEvent(pub AuthState);

/// Lyrics for the current track (or `None` when nothing was found).
#[derive(Serialize, Deserialize, Clone, specta::Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct LyricsEvent {
    pub track_id: String,
    pub lyrics: Option<Lyrics>,
}

/// A cover's dominant hues, computed once when the art is first fetched.
#[derive(Serialize, Deserialize, Clone, specta::Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct ArtHueEvent {
    pub key: String,
    pub h1: u16,
    pub h2: u16,
}

/// A user-facing notice ("Skipped X: …", "Session expired").
#[derive(Serialize, Deserialize, Clone, specta::Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct NoticeEvent {
    pub text: String,
    pub level: String,
    pub action: Option<String>,
}

/// Progress of the in-app sign-in window.
#[derive(Serialize, Deserialize, Clone, specta::Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct SigninEvent {
    pub stage: String,
    pub message: Option<String>,
}

/// A playlist was created, changed or deleted: sidebar, Library, the open
/// playlist and the "Add to playlist" menu refresh.
#[derive(Serialize, Deserialize, Clone, specta::Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistsEvent {
    pub playlist_id: Option<String>,
}

/// A play was reported to the history: the History page refreshes if open.
#[derive(Serialize, Deserialize, Clone, specta::Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEvent {
    pub track_id: String,
}

/// The player components changed: install progress, done, failed, removed
/// (D22).
#[derive(Serialize, Deserialize, Clone, specta::Type, Event)]
pub struct RuntimesEvent(pub RuntimeStatus);

/// A song couldn't start because the player components aren't installed:
/// the UI shows the setup.
#[derive(Serialize, Deserialize, Clone, specta::Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct SetupNeededEvent {
    pub track_id: Option<String>,
}

/// Downloads: counts, progress and waits (at most four a second). A newer
/// revision than the UI has seen means it missed a change: it takes a
/// snapshot (D23).
#[derive(Serialize, Deserialize, Clone, specta::Type, Event)]
pub struct DownloadsEvent(pub DownloadsSummary);

/// One download changed state.
#[derive(Serialize, Deserialize, Clone, specta::Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct DownloadItemEvent {
    /// The downloads revision this change made.
    pub revision: String,
    pub item: DownloadItem,
}

/// The connection was lost, or came back.
#[derive(Serialize, Deserialize, Clone, specta::Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct ConnectivityEvent {
    pub online: bool,
}
