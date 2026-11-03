//! Outside services and estimates (lane E): the Last.fm scrobble queue and
//! play counts (D31), Wikipedia extracts (D29) and the tempo and key
//! estimated from a file (D28). Last.fm keys and session keys live in the
//! credential store (D9), never here.

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScrobbleState {
    Queued,
    Sent,
    /// Last.fm accepted the request but ignored the scrobble (its own rules).
    Ignored,
    Failed,
}

/// One counted play waiting for Last.fm (`scrobble_queue`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scrobble {
    pub play_id: String,
    pub owner: String,
    pub username: String,
    pub artist: String,
    pub track: String,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub duration: Option<f64>,
    pub timestamp: i64,
    pub state: ScrobbleState,
    pub attempts: u32,
}

/// An artist's Wikipedia extract, with what the attribution line needs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WikiExtract {
    pub title: String,
    pub description: Option<String>,
    pub extract: String,
    pub url: String,
    pub lang: String,
}

/// Tempo and key estimated from a downloaded or cached file. `None` where
/// the estimate wasn't confident enough to show.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AudioAnalysis {
    pub bpm: Option<f64>,
    pub key: Option<String>,
    pub confidence: Option<f64>,
}
