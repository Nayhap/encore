//! The local play log (D30): one row per play, written by the player's
//! observer (`encore_player::plays`) whether or not the play is reported to
//! YouTube (D18), online or offline. Lane D builds stats and "your plays"
//! on it; lane E's scrobbler uses the same counting rule.

use std::collections::HashMap;

use rusqlite::params;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::Result;
use crate::local::Owner;
use crate::model::{ArtRef, ItemId, Track};
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

// ---------- stats (the History page's Stats tab) ----------

/// A month (`month` 1–12) or a whole year, in the local time each play
/// started in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StatsPeriod {
    pub year: u32,
    pub month: Option<u32>,
}

impl StatsPeriod {
    /// `[start, end)` in local seconds (unix seconds shifted by the offset).
    fn bounds(self) -> (i64, i64) {
        let (y, m) = (self.year as i64, self.month.unwrap_or(1) as i64);
        let start = days_from_civil(y, m, 1) * 86_400;
        let end = match self.month {
            Some(12) | None => days_from_civil(y + 1, 1, 1),
            Some(m) => days_from_civil(y, m as i64 + 1, 1),
        } * 86_400;
        (start, end)
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TopTrack {
    pub track: Track,
    pub plays: u32,
    pub minutes: f64,
}

/// An artist or an album, ranked.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TopCredit {
    pub name: String,
    pub id: Option<ItemId>,
    /// The cover of its most played song.
    pub art: Option<ArtRef>,
    /// An album's artist line.
    pub detail: Option<String>,
    pub plays: u32,
    pub minutes: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListeningStats {
    pub period: StatsPeriod,
    /// Everything heard, counted or not.
    pub minutes: f64,
    /// Plays that met the counting rule.
    pub plays: u32,
    /// Different songs among them.
    pub songs: u32,
    pub top_songs: Vec<TopTrack>,
    pub top_artists: Vec<TopCredit>,
    pub top_albums: Vec<TopCredit>,
    /// Minutes heard by the local hour a play started in, 0–23.
    pub by_hour: Vec<f64>,
    /// Songs counted for the first time in this period.
    pub new_songs: u32,
    /// The most played of them.
    pub new_top: Vec<Track>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StatsOverview {
    /// Months with plays, newest first.
    pub months: Vec<StatsPeriod>,
    /// Unix seconds of the first play, if any.
    pub since: Option<String>,
    pub plays: u32,
}

/// Top lists are this long.
const TOP: usize = 10;
/// A play's start can be this far from UTC (UTC+14, UTC−12, with room).
const MAX_OFFSET: i64 = 15 * 3600;

#[derive(Default)]
struct Tally {
    plays: u32,
    heard: f64,
}

impl Tally {
    fn add(&mut self, counted: bool, heard: f64) {
        self.plays += counted as u32;
        self.heard += heard;
    }
}

fn rank<K>(m: HashMap<K, Tally>) -> Vec<(K, Tally)> {
    let mut v: Vec<_> = m.into_iter().filter(|(_, t)| t.plays > 0).collect();
    v.sort_by(|a, b| b.1.plays.cmp(&a.1.plays).then(b.1.heard.total_cmp(&a.1.heard)));
    v
}

impl Store {
    fn track_meta(&self, video_id: &str) -> Option<Track> {
        let c = self.conn.lock();
        let json: String = c.query_row("SELECT track FROM track_meta WHERE video_id = ?1", params![video_id], |r| r.get(0)).ok()?;
        serde_json::from_str(&json).ok()
    }

    /// The months an owner played something in, and since when.
    pub fn stats_overview(&self, owner: &Owner) -> Result<StatsOverview> {
        let c = self.conn.lock();
        let (since, plays): (Option<i64>, u32) = c.query_row("SELECT MIN(started), COUNT(*) FROM plays WHERE owner = ?1 AND counted = 1", params![owner.as_str()], |r| Ok((r.get(0)?, r.get(1)?))).map_err(db_err)?;
        let mut st = c
            .prepare_cached("SELECT DISTINCT CAST(strftime('%Y', started + utc_offset, 'unixepoch') AS INTEGER), CAST(strftime('%m', started + utc_offset, 'unixepoch') AS INTEGER) FROM plays WHERE owner = ?1 ORDER BY 1 DESC, 2 DESC")
            .map_err(db_err)?;
        let months = st.query_map(params![owner.as_str()], |r| Ok(StatsPeriod { year: r.get(0)?, month: Some(r.get(1)?) })).map_err(db_err)?.collect::<std::result::Result<_, _>>().map_err(db_err)?;
        Ok(StatsOverview { months, since: since.map(|s| s.to_string()), plays })
    }

    /// An owner's listening in one month or year.
    pub fn stats(&self, owner: &Owner, period: StatsPeriod) -> Result<ListeningStats> {
        let (start, end) = period.bounds();
        // (video id, local start, heard, counted), in the period.
        let rows: Vec<(String, i64, f64, bool)> = {
            let c = self.conn.lock();
            let mut st = c.prepare_cached("SELECT video_id, started + utc_offset, heard, counted FROM plays WHERE owner = ?1 AND started >= ?2 AND started < ?3").map_err(db_err)?;
            st.query_map(params![owner.as_str(), start - MAX_OFFSET, end + MAX_OFFSET], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                .map_err(db_err)?
                .filter_map(|r| r.ok())
                .filter(|r| r.1 >= start && r.1 < end)
                .collect()
        };
        let mut by_song: HashMap<String, Tally> = HashMap::new();
        let mut by_hour = vec![0.0; 24];
        let (mut heard, mut plays) = (0.0, 0);
        for (vid, local, h, counted) in &rows {
            by_song.entry(vid.clone()).or_default().add(*counted, *h);
            by_hour[(local.rem_euclid(86_400) / 3600) as usize] += h / 60.0;
            heard += h;
            plays += *counted as u32;
        }
        let songs = rank(by_song);
        let meta: HashMap<String, Track> = songs.iter().filter_map(|(v, _)| Some((v.clone(), self.track_meta(v)?))).collect();

        // Artists (the first credited one) and albums, from the songs' metadata.
        let mut artists: HashMap<String, (Tally, &Track)> = HashMap::new();
        let mut albums: HashMap<String, (Tally, &Track)> = HashMap::new();
        for (v, t) in &songs {
            let Some(track) = meta.get(v) else { continue };
            if let Some(a) = track.artists.first() {
                let e = artists.entry(a.id.as_ref().map(|i| i.0.clone()).unwrap_or_else(|| a.name.to_lowercase())).or_insert((Tally::default(), track));
                e.0.plays += t.plays;
                e.0.heard += t.heard;
            }
            if let Some(al) = &track.album {
                let e = albums.entry(al.id.as_ref().map(|i| i.0.clone()).unwrap_or_else(|| al.name.to_lowercase())).or_insert((Tally::default(), track));
                e.0.plays += t.plays;
                e.0.heard += t.heard;
            }
        }
        let credit = |m: HashMap<String, (Tally, &Track)>, album: bool| -> Vec<TopCredit> {
            let mut v: Vec<_> = m.into_values().collect();
            v.sort_by(|a, b| b.0.plays.cmp(&a.0.plays).then(b.0.heard.total_cmp(&a.0.heard)));
            v.into_iter()
                .take(TOP)
                .map(|(t, track)| {
                    let c = if album { track.album.clone().unwrap() } else { track.artists[0].clone() };
                    TopCredit { name: c.name, id: c.id, art: track.art.clone(), detail: album.then(|| track.artist_line()), plays: t.plays, minutes: t.heard / 60.0 }
                })
                .collect()
        };
        let top_artists = credit(artists, false);
        let top_albums = credit(albums, true);

        // New: counted for the first time ever inside the period.
        let first: HashMap<String, i64> = {
            let c = self.conn.lock();
            let mut st = c.prepare_cached("SELECT video_id, MIN(started + utc_offset) FROM plays WHERE owner = ?1 AND counted = 1 GROUP BY video_id").map_err(db_err)?;
            st.query_map(params![owner.as_str()], |r| Ok((r.get(0)?, r.get(1)?))).map_err(db_err)?.filter_map(|r| r.ok()).collect()
        };
        let new: Vec<&String> = songs.iter().map(|(v, _)| v).filter(|v| first.get(*v).is_some_and(|t| *t >= start && *t < end)).collect();
        let new_top = new.iter().filter_map(|v| meta.get(*v).cloned()).take(8).collect();

        let top_songs = songs.iter().filter_map(|(v, t)| Some(TopTrack { track: meta.get(v)?.clone(), plays: t.plays, minutes: t.heard / 60.0 })).take(TOP).collect();
        Ok(ListeningStats { period, minutes: heard / 60.0, plays, songs: songs.len() as u32, top_songs, top_artists, top_albums, by_hour, new_songs: new.len() as u32, new_top })
    }

    /// Counted plays of each song, for sorting by "your plays".
    pub fn play_counts(&self, owner: &Owner, video_ids: &[String]) -> Result<HashMap<String, u32>> {
        let c = self.conn.lock();
        let mut st = c.prepare_cached("SELECT COUNT(*) FROM plays WHERE owner = ?1 AND video_id = ?2 AND counted = 1").map_err(db_err)?;
        let mut out = HashMap::new();
        for v in video_ids {
            let n: u32 = st.query_row(params![owner.as_str(), v], |r| r.get(0)).map_err(db_err)?;
            if n > 0 {
                out.insert(v.clone(), n);
            }
        }
        Ok(out)
    }

    /// Forget an owner's plays (Settings → Storage). Song metadata no other
    /// owner's play needs goes too.
    pub fn plays_clear(&self, owner: &Owner) -> Result<usize> {
        let mut c = self.conn.lock();
        let tx = c.transaction().map_err(db_err)?;
        let n = tx.execute("DELETE FROM plays WHERE owner = ?1", params![owner.as_str()]).map_err(db_err)?;
        tx.execute("DELETE FROM track_meta WHERE video_id NOT IN (SELECT video_id FROM plays)", []).map_err(db_err)?;
        tx.commit().map_err(db_err)?;
        Ok(n)
    }

    /// Plays left open when Encore last stopped (it quit mid-song) are over:
    /// close those that began before `before`.
    pub fn plays_close_open(&self, before: i64) -> Result<usize> {
        self.conn.lock().execute("UPDATE plays SET ended = started + CAST(heard AS INTEGER) WHERE ended IS NULL AND started < ?1", params![before]).map_err(db_err)
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

    fn song(id: &str, artist: &str, album: &str) -> Track {
        use crate::model::Credit;
        let credit = |n: &str| Credit { name: n.into(), id: Some(ItemId::ytm(n)) };
        Track { id: ItemId::ytm(id), title: id.into(), artists: vec![credit(artist)], album: Some(credit(album)), ..track() }
    }

    /// A play written straight in, at a given local time.
    fn put(s: &Store, n: &mut u32, t: &Track, local: i64, offset: i64, heard: f64) {
        *n += 1;
        let p = PlayStart { id: format!("p{n}"), owner: Owner::new("me"), track: t.clone(), started: local - offset, offline: false, private: false, source: None };
        s.play_begin(&p).unwrap();
        s.conn.lock().execute("UPDATE plays SET utc_offset = ?2 WHERE id = ?1", params![p.id, offset]).unwrap();
        s.play_update(&p.id, heard, counts(heard, t.duration), true).unwrap();
    }

    #[test]
    fn periods_are_local_months_and_years() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2026, 11, 1), 20_758);
        let (a, b) = StatsPeriod { year: 2026, month: Some(12) }.bounds();
        assert_eq!((b - a) / 86_400, 31);
        let (a, b) = StatsPeriod { year: 2024, month: None }.bounds();
        assert_eq!((b - a) / 86_400, 366);
    }

    #[test]
    fn stats_rank_count_and_find_new_songs() {
        let s = Store::open_memory().unwrap();
        let nov = days_from_civil(2026, 11, 1) * 86_400;
        let oct = days_from_civil(2026, 10, 1) * 86_400;
        let (a, b, c) = (song("a", "Ann", "First"), song("b", "Ann", "Second"), song("c", "Cat", "Third"));
        let mut n = 0;
        put(&s, &mut n, &a, oct + 3600, 0, 200.0);
        for i in 0..3 {
            put(&s, &mut n, &a, nov + 22 * 3600 + i * 600, 3600, 120.0);
        }
        put(&s, &mut n, &b, nov + 9 * 3600, -18_000, 150.0);
        put(&s, &mut n, &c, nov + 9 * 3600, 0, 30.0); // not counted
        // 23:30 local on 31 October, though already November in UTC.
        put(&s, &mut n, &c, nov - 1800, -7200, 100.0);
        let st = s.stats(&Owner::new("me"), StatsPeriod { year: 2026, month: Some(11) }).unwrap();
        assert_eq!((st.plays, st.songs), (4, 2));
        assert!((st.minutes - (360.0 + 150.0 + 30.0) / 60.0).abs() < 1e-9);
        assert_eq!(st.top_songs.iter().map(|t| (t.track.title.as_str(), t.plays)).collect::<Vec<_>>(), vec![("a", 3), ("b", 1)]);
        assert_eq!((st.top_artists[0].name.as_str(), st.top_artists[0].plays), ("Ann", 4));
        assert_eq!(st.top_albums.len(), 2);
        assert_eq!(st.top_albums[1].detail.as_deref(), Some("Ann"));
        assert!((st.by_hour[22] - 6.0).abs() < 1e-9 && (st.by_hour[9] - 3.0).abs() < 1e-9);
        assert_eq!(st.new_songs, 1, "a was first played in October");
        assert_eq!(st.new_top[0].title, "b");
        let oct_st = s.stats(&Owner::new("me"), StatsPeriod { year: 2026, month: Some(10) }).unwrap();
        assert_eq!(oct_st.top_songs.iter().map(|t| t.track.title.as_str()).collect::<Vec<_>>(), vec!["a", "c"], "a tie goes to the longer listen");
        let year = s.stats(&Owner::new("me"), StatsPeriod { year: 2026, month: None }).unwrap();
        assert_eq!((year.plays, year.new_songs), (6, 3));
        let ov = s.stats_overview(&Owner::new("me")).unwrap();
        assert_eq!(ov.months, vec![StatsPeriod { year: 2026, month: Some(11) }, StatsPeriod { year: 2026, month: Some(10) }]);
        assert_eq!(ov.plays, 6);
        assert!(s.stats_overview(&Owner::anonymous()).unwrap().months.is_empty());
        assert_eq!(s.play_counts(&Owner::new("me"), &["a".into(), "z".into()]).unwrap(), HashMap::from([("a".to_string(), 4)]));
        assert_eq!(s.plays_clear(&Owner::new("me")).unwrap(), 7);
        assert_eq!(s.conn.lock().query_row("SELECT COUNT(*) FROM track_meta", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
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
