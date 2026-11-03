//! Outside services and estimates (lane E): the Last.fm scrobble queue and
//! play counts (D31), Wikipedia extracts (D29) and the tempo and key
//! estimated from a file (D28). Last.fm keys and session keys live in the
//! credential store (D9), never here.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::Result;
use crate::local::Owner;
use crate::store::{Store, db_err, now};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScrobbleState {
    Queued,
    Sent,
    /// Last.fm accepted the request but ignored the scrobble (its own rules).
    Ignored,
    Failed,
}

impl ScrobbleState {
    fn as_str(self) -> &'static str {
        match self {
            ScrobbleState::Queued => "queued",
            ScrobbleState::Sent => "sent",
            ScrobbleState::Ignored => "ignored",
            ScrobbleState::Failed => "failed",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "sent" => ScrobbleState::Sent,
            "ignored" => ScrobbleState::Ignored,
            "failed" => ScrobbleState::Failed,
            _ => ScrobbleState::Queued,
        }
    }
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

/// One song's play count on Last.fm (`user.getTopTracks`), under the keys
/// [`match_key`](super::library::match_key) makes.
#[derive(Clone, Debug, PartialEq)]
pub struct LastfmPlays {
    pub artist_key: String,
    pub title_key: String,
    pub plays: u32,
}

/// What `audio_analysis` holds for a song: the estimate and the file it
/// came from ("<itag>:<bytes>"), so another file is analysed again.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredAnalysis {
    pub source: String,
    pub version: u32,
    pub analysis: AudioAnalysis,
}

impl Store {
    // ---------- scrobble queue (D31) ----------

    /// Queue a counted play; a play is queued once, however often it's asked.
    pub fn scrobble_enqueue(&self, s: &Scrobble) -> Result<bool> {
        let n = self
            .conn
            .lock()
            .execute(
                "INSERT INTO scrobble_queue (play_id, owner, username, artist, track, album, album_artist, duration, timestamp, state, attempts, updated)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'queued', 0, ?10) ON CONFLICT(play_id) DO NOTHING",
                params![s.play_id, s.owner, s.username, s.artist, s.track, s.album, s.album_artist, s.duration, s.timestamp, now()],
            )
            .map_err(db_err)?;
        Ok(n > 0)
    }

    /// The oldest queued scrobbles for a Last.fm account.
    pub fn scrobbles_queued(&self, username: &str, limit: u32) -> Result<Vec<Scrobble>> {
        let c = self.conn.lock();
        let mut st = c
            .prepare(
                "SELECT play_id, owner, username, artist, track, album, album_artist, duration, timestamp, state, attempts
                 FROM scrobble_queue WHERE username = ?1 AND state = 'queued' ORDER BY timestamp LIMIT ?2",
            )
            .map_err(db_err)?;
        let rows = st
            .query_map(params![username, limit], |r| {
                Ok(Scrobble {
                    play_id: r.get(0)?,
                    owner: r.get(1)?,
                    username: r.get(2)?,
                    artist: r.get(3)?,
                    track: r.get(4)?,
                    album: r.get(5)?,
                    album_artist: r.get(6)?,
                    duration: r.get(7)?,
                    timestamp: r.get(8)?,
                    state: ScrobbleState::parse(&r.get::<_, String>(9)?),
                    attempts: r.get(10)?,
                })
            })
            .map_err(db_err)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Record what Last.fm made of a scrobble. `Queued` counts an attempt
    /// and leaves it for the next try.
    pub fn scrobble_mark(&self, play_id: &str, state: ScrobbleState, error: Option<&str>) -> Result<()> {
        self.conn
            .lock()
            .execute(
                "UPDATE scrobble_queue SET state = ?2, error = ?3, attempts = attempts + 1, updated = ?4 WHERE play_id = ?1",
                params![play_id, state.as_str(), error, now()],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// Scrobbles waiting and failed for a Last.fm account.
    pub fn scrobble_counts(&self, username: &str) -> Result<(u32, u32)> {
        self.conn
            .lock()
            .query_row(
                "SELECT COALESCE(SUM(state = 'queued'), 0), COALESCE(SUM(state = 'failed'), 0) FROM scrobble_queue WHERE username = ?1",
                params![username],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(db_err)
    }

    // ---------- Last.fm play counts (D31) ----------

    /// Replace an account's counts with a fresh read.
    pub fn lastfm_plays_replace(&self, username: &str, rows: &[LastfmPlays]) -> Result<()> {
        let mut c = self.conn.lock();
        let tx = c.transaction().map_err(db_err)?;
        tx.execute("DELETE FROM lastfm_plays WHERE username = ?1", params![username]).map_err(db_err)?;
        {
            // One song can come twice (two spellings of a title): add them up.
            let mut st = tx
                .prepare(
                    "INSERT INTO lastfm_plays (username, artist_key, title_key, plays, updated) VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(username, artist_key, title_key) DO UPDATE SET plays = plays + excluded.plays",
                )
                .map_err(db_err)?;
            let t = now();
            for r in rows {
                st.execute(params![username, r.artist_key, r.title_key, r.plays, t]).map_err(db_err)?;
            }
        }
        tx.commit().map_err(db_err)
    }

    /// When an account's counts were last read, if ever.
    pub fn lastfm_plays_updated(&self, username: &str) -> Result<Option<i64>> {
        self.conn
            .lock()
            .query_row("SELECT MAX(updated) FROM lastfm_plays WHERE username = ?1", params![username], |r| r.get(0))
            .map_err(db_err)
    }

    pub fn lastfm_plays(&self, username: &str, artist_key: &str, title_key: &str) -> Result<Option<u32>> {
        self.conn
            .lock()
            .query_row(
                "SELECT plays FROM lastfm_plays WHERE username = ?1 AND artist_key = ?2 AND title_key = ?3",
                params![username, artist_key, title_key],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_err)
    }

    // ---------- Wikipedia (D29) ----------

    /// The cached answer for an artist, with when it was fetched:
    /// `Some((None, _))` means no article matched.
    pub fn wiki_get(&self, artist_id: &str) -> Result<Option<(Option<WikiExtract>, i64)>> {
        self.conn
            .lock()
            .query_row(
                "SELECT lang, title, description, extract, url, fetched FROM wiki_cache WHERE artist_id = ?1",
                params![artist_id],
                |r| {
                    let lang: String = r.get(0)?;
                    let found = match (r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(3)?, r.get::<_, Option<String>>(4)?) {
                        (Some(title), Some(extract), Some(url)) => Some(WikiExtract { title, description: r.get(2)?, extract, url, lang }),
                        _ => None,
                    };
                    Ok((found, r.get(5)?))
                },
            )
            .optional()
            .map_err(db_err)
    }

    pub fn wiki_put(&self, artist_id: &str, lang: &str, found: Option<&WikiExtract>) -> Result<()> {
        self.conn
            .lock()
            .execute(
                "INSERT INTO wiki_cache (artist_id, lang, title, description, extract, url, fetched) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(artist_id) DO UPDATE SET lang = excluded.lang, title = excluded.title, description = excluded.description,
                 extract = excluded.extract, url = excluded.url, fetched = excluded.fetched",
                params![artist_id, lang, found.map(|f| &f.title), found.and_then(|f| f.description.as_ref()), found.map(|f| &f.extract), found.map(|f| &f.url), now()],
            )
            .map_err(db_err)?;
        Ok(())
    }

    // ---------- tempo and key (D28) ----------

    pub fn analysis_get(&self, video_id: &str) -> Result<Option<StoredAnalysis>> {
        self.conn
            .lock()
            .query_row(
                "SELECT source, version, bpm, music_key, confidence FROM audio_analysis WHERE video_id = ?1",
                params![video_id],
                |r| Ok(StoredAnalysis { source: r.get(0)?, version: r.get(1)?, analysis: AudioAnalysis { bpm: r.get(2)?, key: r.get(3)?, confidence: r.get(4)? } }),
            )
            .optional()
            .map_err(db_err)
    }

    pub fn analysis_put(&self, video_id: &str, source: &str, version: u32, a: &AudioAnalysis) -> Result<()> {
        self.conn
            .lock()
            .execute(
                "INSERT INTO audio_analysis (video_id, source, version, bpm, music_key, confidence, analysed) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(video_id) DO UPDATE SET source = excluded.source, version = excluded.version, bpm = excluded.bpm,
                 music_key = excluded.music_key, confidence = excluded.confidence, analysed = excluded.analysed",
                params![video_id, source, version, a.bpm, a.key, a.confidence, now()],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// Counted plays of a song in the play log (D30), for its details.
    pub fn plays_of(&self, owner: &Owner, video_id: &str) -> Result<u32> {
        self.conn
            .lock()
            .query_row("SELECT COUNT(*) FROM plays WHERE owner = ?1 AND video_id = ?2 AND counted = 1", params![owner.as_str(), video_id], |r| r.get(0))
            .map_err(db_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scrobble(id: &str, user: &str, ts: i64) -> Scrobble {
        Scrobble { play_id: id.into(), owner: "acc".into(), username: user.into(), artist: "Artist".into(), track: "Song".into(), album: None, album_artist: None, duration: Some(200.0), timestamp: ts, state: ScrobbleState::Queued, attempts: 0 }
    }

    #[test]
    fn scrobbles_queue_once_and_leave_when_sent() {
        let s = Store::open_memory().unwrap();
        assert!(s.scrobble_enqueue(&scrobble("p2", "rj", 20)).unwrap());
        assert!(s.scrobble_enqueue(&scrobble("p1", "rj", 10)).unwrap());
        assert!(!s.scrobble_enqueue(&scrobble("p1", "rj", 10)).unwrap(), "a play is queued once");
        s.scrobble_enqueue(&scrobble("p3", "other", 5)).unwrap();
        let q = s.scrobbles_queued("rj", 10).unwrap();
        assert_eq!(q.iter().map(|x| x.play_id.as_str()).collect::<Vec<_>>(), ["p1", "p2"], "oldest first, this account only");
        s.scrobble_mark("p1", ScrobbleState::Sent, None).unwrap();
        s.scrobble_mark("p2", ScrobbleState::Queued, Some("offline")).unwrap();
        let q = s.scrobbles_queued("rj", 10).unwrap();
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].attempts, 1);
        s.scrobble_mark("p2", ScrobbleState::Failed, Some("bad")).unwrap();
        assert_eq!(s.scrobble_counts("rj").unwrap(), (0, 1));
        assert_eq!(s.scrobble_counts("nobody").unwrap(), (0, 0));
    }

    #[test]
    fn lastfm_counts_are_replaced_whole() {
        let s = Store::open_memory().unwrap();
        assert_eq!(s.lastfm_plays_updated("rj").unwrap(), None);
        let row = |a: &str, t: &str, n| LastfmPlays { artist_key: a.into(), title_key: t.into(), plays: n };
        s.lastfm_plays_replace("rj", &[row("a", "x", 3), row("a", "x", 2), row("b", "y", 7)]).unwrap();
        assert_eq!(s.lastfm_plays("rj", "a", "x").unwrap(), Some(5), "two spellings of one song add up");
        s.lastfm_plays_replace("rj", &[row("b", "y", 8)]).unwrap();
        assert_eq!(s.lastfm_plays("rj", "a", "x").unwrap(), None);
        assert_eq!(s.lastfm_plays("rj", "b", "y").unwrap(), Some(8));
        assert!(s.lastfm_plays_updated("rj").unwrap().is_some());
    }

    #[test]
    fn wiki_remembers_misses_too() {
        let s = Store::open_memory().unwrap();
        assert_eq!(s.wiki_get("ytm:UC1").unwrap(), None);
        s.wiki_put("ytm:UC1", "en", None).unwrap();
        assert_eq!(s.wiki_get("ytm:UC1").unwrap().unwrap().0, None);
        let w = WikiExtract { title: "Daft Punk".into(), description: Some("French electronic music duo".into()), extract: "Daft Punk were…".into(), url: "https://en.wikipedia.org/wiki/Daft_Punk".into(), lang: "en".into() };
        s.wiki_put("ytm:UC1", "en", Some(&w)).unwrap();
        assert_eq!(s.wiki_get("ytm:UC1").unwrap().unwrap().0, Some(w));
    }

    #[test]
    fn analysis_round_trips() {
        let s = Store::open_memory().unwrap();
        let a = AudioAnalysis { bpm: Some(123.5), key: None, confidence: Some(0.4) };
        s.analysis_put("v1", "140:1234", 1, &a).unwrap();
        let got = s.analysis_get("v1").unwrap().unwrap();
        assert_eq!((got.source.as_str(), got.version, got.analysis), ("140:1234", 1, a));
        assert_eq!(s.plays_of(&Owner::new("acc"), "v1").unwrap(), 0);
    }
}
