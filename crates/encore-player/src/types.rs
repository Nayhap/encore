//! Wire types for playback state (D13). u64 counters are strings on the wire.

use encore_core::queue::{QueueEntry, QueueSource, Repeat};
use encore_core::service::PlayFailure;
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Idle,
    Loading,
    Playing,
    Paused,
    Buffering,
    Ended,
    Error,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StreamInfo {
    pub itag: u32,
    pub codec: String,
    pub kbps: Option<u32>,
    pub from_cache: bool,
    pub seekable: bool,
    pub authenticated: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlayerSnapshot {
    pub revision: String,
    pub track_gen: String,
    pub seek_gen: String,
    pub phase: Phase,
    pub playing: bool,
    pub current: Option<QueueEntry>,
    pub index: Option<u32>,
    pub position: f64,
    pub duration: Option<f64>,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub autoplay: bool,
    pub volume: f64,
    pub muted: bool,
    pub source: Option<QueueSource>,
    pub stream: Option<StreamInfo>,
    pub error: Option<PlayFailure>,
    /// Auto-skips in a row (D4: at most 3).
    pub skipped_in_a_row: u32,
    pub queue_loading: bool,
    /// No audio output could be opened: playback runs silent until one works.
    pub no_output: bool,
}

/// A queue change as a splice of the previous revision (D13). `base` is the
/// revision it applies to; a UI on another revision asks for a snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QueueSplice {
    pub base: String,
    pub revision: String,
    pub start: u32,
    pub delete: u32,
    pub insert: Vec<QueueEntry>,
    pub index: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QueueSnapshot {
    pub revision: String,
    pub entries: Vec<QueueEntry>,
    pub index: Option<u32>,
}

/// 2 Hz position anchor; the UI interpolates between anchors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PositionAnchor {
    pub track_gen: String,
    pub seek_gen: String,
    pub position: f64,
    pub duration: Option<f64>,
    pub playing: bool,
}

#[derive(Clone, Debug)]
pub enum PlayerEvent {
    State(PlayerSnapshot),
    Queue(QueueSplice),
    Position(PositionAnchor),
    /// The current track changed (for lyrics, Now Playing, history).
    TrackChanged(Option<QueueEntry>),
    /// A user-visible notice ("Skipped X: unavailable in your country").
    Notice(String),
    /// Resolution reported an expired session.
    SessionExpired,
    /// A song couldn't start because the player components aren't installed.
    SetupNeeded,
}

/// Compute the splice that turns `old` into `new` (by entry uid).
pub fn diff(old: &[QueueEntry], new: &[QueueEntry]) -> (usize, usize, Vec<QueueEntry>) {
    let pre = old.iter().zip(new.iter()).take_while(|(a, b)| a.uid == b.uid && a.track == b.track).count();
    let max_suf = old.len().min(new.len()) - pre;
    let suf = old.iter().rev().zip(new.iter().rev()).take(max_suf).take_while(|(a, b)| a.uid == b.uid && a.track == b.track).count();
    let delete = old.len() - pre - suf;
    let insert = new[pre..new.len() - suf].to_vec();
    (pre, delete, insert)
}

#[cfg(test)]
mod tests {
    use super::*;
    use encore_core::model::*;
    use encore_core::queue::Origin;

    fn e(uid: &str) -> QueueEntry {
        QueueEntry {
            uid: uid.into(),
            track: Track { id: ItemId::ytm(uid), title: uid.into(), artists: vec![], album: None, duration: None, explicit: false, art: None, kind: TrackKind::Song, plays: None, liked: None, playable: true, counterpart: None },
            origin: Origin::Manual,
        }
    }

    fn apply(old: &[QueueEntry], s: (usize, usize, Vec<QueueEntry>)) -> Vec<String> {
        let mut v: Vec<QueueEntry> = old.to_vec();
        v.splice(s.0..s.0 + s.1, s.2);
        v.into_iter().map(|e| e.uid).collect()
    }

    #[test]
    fn splices_roundtrip() {
        let cases: Vec<(Vec<&str>, Vec<&str>)> = vec![
            (vec!["a", "b", "c"], vec!["a", "x", "b", "c"]),
            (vec!["a", "b", "c"], vec!["a", "c"]),
            (vec!["a", "b", "c"], vec!["c", "b", "a"]),
            (vec![], vec!["a"]),
            (vec!["a"], vec![]),
            (vec!["a", "b"], vec!["a", "b", "c", "d"]),
        ];
        for (o, n) in cases {
            let old: Vec<QueueEntry> = o.iter().map(|x| e(x)).collect();
            let new: Vec<QueueEntry> = n.iter().map(|x| e(x)).collect();
            assert_eq!(apply(&old, diff(&old, &new)), n);
        }
    }
}
