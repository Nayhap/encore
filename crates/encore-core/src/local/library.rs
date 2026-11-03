//! Library tools' cache (D34, lane B): release years of albums, looked up
//! one album at a time through the background lane (`crate::paging`) when a
//! list is sorted by year, and kept here so it happens once.
//!
//! Sorting also reads what other areas keep, without writing to it: the
//! play log (`plays`, lane D) for "Your plays", Last.fm's counts
//! (`lastfm_plays`, lane E) and tempo estimates (`audio_analysis`, lane E).
//! Each answer only holds the songs that have a value, so the UI offers a
//! sort only when there is something to sort by.

use std::collections::HashMap;

use rusqlite::params_from_iter;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::local::Owner;
use crate::store::{Store, db_err, now};

/// `year: None`: the album was read and gives no year.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AlbumYear {
    pub album_id: String,
    pub year: Option<u16>,
    pub fetched: i64,
}

/// SQLite takes a bounded number of `?`s per statement.
const CHUNK: usize = 400;

fn marks(n: usize) -> String {
    vec!["?"; n].join(",")
}

/// How Last.fm's counts are matched to a row: case, spacing and the
/// punctuation around words don't matter ("Daft Punk" = "daft  punk").
pub fn match_key(s: &str) -> String {
    let s: String = s.to_lowercase().chars().filter(|c| !matches!(c, '’' | '\'')).map(|c| if matches!(c, '"' | '.' | ',' | '!' | '?') { ' ' } else { c }).collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Store {
    /// Cached years of these albums (remote ids, `MPRE…`). Albums never read
    /// are missing; albums read without a year are `None`.
    pub fn album_years(&self, ids: &[&str]) -> Result<HashMap<String, Option<u16>>> {
        let c = self.conn.lock();
        let mut out = HashMap::new();
        for part in ids.chunks(CHUNK) {
            let mut st = c.prepare(&format!("SELECT album_id, year FROM album_years WHERE album_id IN ({})", marks(part.len()))).map_err(db_err)?;
            let rows = st.query_map(params_from_iter(part.iter()), |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<u16>>(1)?))).map_err(db_err)?;
            for r in rows {
                let (id, y) = r.map_err(db_err)?;
                out.insert(id, y);
            }
        }
        Ok(out)
    }

    pub fn set_album_year(&self, album_id: &str, year: Option<u16>) -> Result<()> {
        self.conn
            .lock()
            .execute(
                "INSERT INTO album_years (album_id, year, fetched) VALUES (?1, ?2, ?3)
                 ON CONFLICT(album_id) DO UPDATE SET year = excluded.year, fetched = excluded.fetched",
                rusqlite::params![album_id, year, now()],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// Counted plays in Encore (D30) of these songs (remote ids), for
    /// `owner`. Songs never counted are missing.
    pub fn counted_plays(&self, owner: &Owner, ids: &[&str]) -> Result<HashMap<String, u32>> {
        let c = self.conn.lock();
        let mut out = HashMap::new();
        for part in ids.chunks(CHUNK) {
            let sql = format!("SELECT video_id, COUNT(*) FROM plays WHERE owner = ? AND counted = 1 AND video_id IN ({}) GROUP BY video_id", marks(part.len()));
            let mut st = c.prepare(&sql).map_err(db_err)?;
            let args = std::iter::once(owner.as_str()).chain(part.iter().copied());
            let rows = st.query_map(params_from_iter(args), |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?))).map_err(db_err)?;
            for r in rows {
                let (id, n) = r.map_err(db_err)?;
                out.insert(id, n);
            }
        }
        Ok(out)
    }

    /// Estimated tempos (D28) of these songs. Only confident estimates are
    /// stored with a value, so a missing song hasn't been analysed or wasn't
    /// clear enough to say.
    pub fn tempos(&self, ids: &[&str]) -> Result<HashMap<String, f64>> {
        let c = self.conn.lock();
        let mut out = HashMap::new();
        for part in ids.chunks(CHUNK) {
            let mut st = c.prepare(&format!("SELECT video_id, bpm FROM audio_analysis WHERE bpm IS NOT NULL AND video_id IN ({})", marks(part.len()))).map_err(db_err)?;
            let rows = st.query_map(params_from_iter(part.iter()), |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))).map_err(db_err)?;
            for r in rows {
                let (id, b) = r.map_err(db_err)?;
                out.insert(id, b);
            }
        }
        Ok(out)
    }

    /// Last.fm play counts (D31) of `(artist, title)` pairs, keyed as
    /// [`match_key`] keys them, for the Last.fm account counted most
    /// recently. `None` when Encore has no Last.fm counts at all.
    pub fn lastfm_counts(&self, pairs: &[(String, String)]) -> Result<Option<HashMap<(String, String), u32>>> {
        let c = self.conn.lock();
        let user: Option<String> = c.query_row("SELECT username FROM lastfm_plays ORDER BY updated DESC LIMIT 1", [], |r| r.get(0)).ok();
        let Some(user) = user else { return Ok(None) };
        let mut out = HashMap::new();
        let mut st = c.prepare_cached("SELECT plays FROM lastfm_plays WHERE username = ?1 AND artist_key = ?2 AND title_key = ?3").map_err(db_err)?;
        for (a, t) in pairs {
            if let Ok(n) = st.query_row(rusqlite::params![user, a, t], |r| r.get::<_, u32>(0)) {
                out.insert((a.clone(), t.clone()), n);
            }
        }
        Ok(Some(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn years_are_cached_with_their_absence() {
        let s = Store::open_memory().unwrap();
        s.set_album_year("MPREa", Some(2013)).unwrap();
        s.set_album_year("MPREb", None).unwrap();
        s.set_album_year("MPREa", Some(2014)).unwrap();
        let y = s.album_years(&["MPREa", "MPREb", "MPREc"]).unwrap();
        assert_eq!(y.get("MPREa"), Some(&Some(2014)));
        assert_eq!(y.get("MPREb"), Some(&None), "read, no year");
        assert!(!y.contains_key("MPREc"), "never read");
    }

    #[test]
    fn many_ids_go_in_chunks() {
        let s = Store::open_memory().unwrap();
        let ids: Vec<String> = (0..1000).map(|i| format!("MPRE{i}")).collect();
        s.set_album_year("MPRE999", Some(1999)).unwrap();
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        assert_eq!(s.album_years(&refs).unwrap().get("MPRE999"), Some(&Some(1999)));
    }

    #[test]
    fn counted_plays_are_the_owners_own() {
        let s = Store::open_memory().unwrap();
        {
            let c = s.conn.lock();
            for (id, owner, video, counted) in [("1", "me", "v1", 1), ("2", "me", "v1", 1), ("3", "me", "v1", 0), ("4", "you", "v1", 1), ("5", "me", "v2", 1)] {
                c.execute("INSERT INTO plays (id, owner, video_id, started, counted) VALUES (?1, ?2, ?3, 0, ?4)", rusqlite::params![id, owner, video, counted]).unwrap();
            }
        }
        let n = s.counted_plays(&Owner::new("me"), &["v1", "v2", "v3"]).unwrap();
        assert_eq!(n.get("v1"), Some(&2), "uncounted plays and other owners don't add up");
        assert_eq!(n.get("v2"), Some(&1));
        assert!(!n.contains_key("v3"));
    }

    #[test]
    fn tempos_only_where_known() {
        let s = Store::open_memory().unwrap();
        {
            let c = s.conn.lock();
            c.execute("INSERT INTO audio_analysis (video_id, source, version, bpm, analysed) VALUES ('v1', '140:1', 1, 124.0, 0), ('v2', '140:1', 1, NULL, 0)", []).unwrap();
        }
        let t = s.tempos(&["v1", "v2"]).unwrap();
        assert_eq!(t.get("v1"), Some(&124.0));
        assert!(!t.contains_key("v2"), "not confident: no tempo");
    }

    #[test]
    fn lastfm_counts_for_the_latest_account() {
        let s = Store::open_memory().unwrap();
        assert_eq!(s.lastfm_counts(&[]).unwrap(), None, "not connected");
        {
            let c = s.conn.lock();
            c.execute("INSERT INTO lastfm_plays VALUES ('old', 'daft punk', 'voyager', 3, 1), ('new', 'daft punk', 'voyager', 12, 2)", []).unwrap();
        }
        let key = (match_key("Daft  Punk"), match_key("Voyager"));
        let n = s.lastfm_counts(&[key.clone(), ("x".into(), "y".into())]).unwrap().unwrap();
        assert_eq!(n.get(&key), Some(&12));
        assert_eq!(n.len(), 1);
    }

    #[test]
    fn match_keys_ignore_case_and_spacing() {
        assert_eq!(match_key("  Don't  Stop, Me Now! "), "dont stop me now");
        assert_eq!(match_key("Daft Punk"), match_key("daft punk"));
    }
}
