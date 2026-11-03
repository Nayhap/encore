//! The local play log (D30): one row per play, written by the player's
//! observer (`encore_player::plays`) whether or not the play is reported to
//! YouTube (D18), online or offline. Lane D builds stats and "your plays"
//! on it; lane E's scrobbler uses the same counting rule.

use rusqlite::params;

use crate::error::Result;
use crate::local::Owner;
use crate::model::Track;
use crate::store::{Store, db_err, now};

/// The longest a play has to be heard before it counts, whatever its length.
pub const COUNT_CAP: f64 = 240.0;

/// A play counts once it was heard for half the song or four minutes,
/// whichever comes first (Last.fm's rule too). Unknown length: four minutes.
pub fn counts(heard: f64, duration: Option<f64>) -> bool {
    let need = duration.filter(|d| d.is_finite() && *d > 0.0).map(|d| (d / 2.0).min(COUNT_CAP)).unwrap_or(COUNT_CAP);
    heard >= need
}

/// What is known when a play begins. The owner is the account it started
/// under, and stays so if the account changes while it plays.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayStart {
    /// A v4 UUID; every write for this play uses it, so writes repeat safely.
    pub id: String,
    pub owner: Owner,
    pub track: Track,
    /// Unix seconds.
    pub started: i64,
    pub offline: bool,
    pub private: bool,
    /// `QueueSource::collection_id` of the queue it played from.
    pub source: Option<String>,
}

/// Where the observer writes. `Store` is the real one; tests use their own.
pub trait PlaySink: Send + Sync {
    fn begin(&self, play: &PlayStart);
    /// Checkpoint: never lowers `heard` or un-counts a play.
    fn update(&self, id: &str, heard: f64, counted: bool, ended: bool);
}

impl Store {
    /// Record a play that became audible, and remember the song's metadata.
    pub fn play_begin(&self, p: &PlayStart) -> Result<()> {
        let mut track = p.track.clone();
        track.liked = None;
        let json = serde_json::to_string(&track).map_err(|e| crate::error::AppError::internal(e.to_string()))?;
        let c = self.conn.lock();
        // The UTC offset in force now, as SQLite's own clock sees it.
        c.execute(
            "INSERT INTO plays (id, owner, video_id, started, utc_offset, duration, offline, private, source)
             VALUES (?1, ?2, ?3, ?4, CAST(strftime('%s', ?4, 'unixepoch', 'localtime') AS INTEGER) - ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO NOTHING",
            params![p.id, p.owner.as_str(), p.track.id.remote(), p.started, p.track.duration, p.offline, p.private, p.source],
        )
        .map_err(db_err)?;
        c.execute(
            "INSERT INTO track_meta (video_id, track, updated) VALUES (?1, ?2, ?3)
             ON CONFLICT(video_id) DO UPDATE SET track = excluded.track, updated = excluded.updated",
            params![p.track.id.remote(), json, now()],
        )
        .map_err(db_err)?;
        Ok(())
    }

    pub fn play_update(&self, id: &str, heard: f64, counted: bool, ended: bool) -> Result<()> {
        self.conn
            .lock()
            .execute(
                "UPDATE plays SET heard = MAX(heard, ?2), counted = MAX(counted, ?3), ended = CASE WHEN ?4 THEN COALESCE(ended, ?5) ELSE ended END WHERE id = ?1",
                params![id, heard, counted, ended, now()],
            )
            .map_err(db_err)?;
        Ok(())
    }
}

impl PlaySink for Store {
    fn begin(&self, play: &PlayStart) {
        if let Err(e) = self.play_begin(play) {
            tracing::warn!(error = %e.message, "couldn't record a play");
        }
    }

    fn update(&self, id: &str, heard: f64, counted: bool, ended: bool) {
        if let Err(e) = self.play_update(id, heard, counted, ended) {
            tracing::warn!(error = %e.message, "couldn't update a play");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ItemId, TrackKind};

    fn track() -> Track {
        Track { id: ItemId::ytm("v1"), title: "Song".into(), artists: vec![], album: None, duration: Some(200.0), explicit: false, art: None, kind: TrackKind::Song, plays: None, liked: Some(true), playable: true, counterpart: None }
    }

    #[test]
    fn the_counting_rule() {
        assert!(!counts(99.0, Some(200.0)));
        assert!(counts(100.0, Some(200.0)));
        assert!(counts(240.0, Some(1200.0)), "four minutes is enough for a long one");
        assert!(!counts(239.0, None));
    }

    #[test]
    fn plays_are_written_once_and_only_grow() {
        let s = Store::open_memory().unwrap();
        let p = PlayStart { id: "p1".into(), owner: Owner::new("acc"), track: track(), started: 1_800_000_000, offline: false, private: false, source: None };
        s.play_begin(&p).unwrap();
        s.play_begin(&p).unwrap();
        s.play_update("p1", 50.0, false, false).unwrap();
        s.play_update("p1", 30.0, true, true).unwrap();
        let c = s.conn.lock();
        let (n, heard, counted, ended): (i64, f64, bool, Option<i64>) = c.query_row("SELECT COUNT(*), MAX(heard), MAX(counted), MAX(ended) FROM plays", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).unwrap();
        assert_eq!((n, heard, counted), (1, 50.0, true));
        assert!(ended.is_some());
        let meta: String = c.query_row("SELECT track FROM track_meta WHERE video_id = 'v1'", [], |r| r.get(0)).unwrap();
        assert!(meta.contains("\"liked\":null"), "likes depend on the account: {meta}");
    }
}
