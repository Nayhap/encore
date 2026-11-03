//! Shuffle cycles (D33, lane A): per owner and collection, the songs already
//! played in the current cycle, so a shuffle doesn't repeat a song until
//! every song has played, across sessions. The table is in `local::SCHEMA`;
//! the queue (`crate::queue`) and these store methods are lane A's.

use std::collections::HashSet;

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::local::Owner;
use crate::store::{Store, db_err, now};

/// One collection's cycle, as stored (`shuffle_cycles.played` is the JSON
/// list of video ids).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ShuffleCycle {
    pub collection_id: String,
    /// Starts at 1; a new cycle begins once every song has played.
    pub cycle: u32,
    pub played: Vec<String>,
}

impl ShuffleCycle {
    pub fn new(collection_id: &str) -> Self {
        ShuffleCycle { collection_id: collection_id.into(), cycle: 1, played: vec![] }
    }

    /// Fit the cycle to the collection as it is now: songs that left it
    /// leave the cycle (new ones simply haven't played yet), and once every
    /// song has played the next cycle starts empty. Returns the set of
    /// songs already played in the cycle that is current afterwards.
    pub fn fit(&mut self, members: &HashSet<String>) -> HashSet<String> {
        self.played.retain(|v| members.contains(v));
        let mut seen = HashSet::new();
        self.played.retain(|v| seen.insert(v.clone()));
        if !members.is_empty() && seen.len() >= members.len() {
            self.cycle += 1;
            self.played.clear();
            seen.clear();
        }
        seen
    }

    /// Note that `video_id` played. Returns false if it already had.
    pub fn mark(&mut self, video_id: &str) -> bool {
        if self.played.iter().any(|v| v == video_id) {
            return false;
        }
        self.played.push(video_id.into());
        true
    }
}

/// Where the player keeps cycles. `Store` is the real one; tests use their own.
pub trait ShuffleCycles: Send + Sync {
    fn load(&self, owner: &Owner, collection_id: &str) -> Option<ShuffleCycle>;
    fn save(&self, owner: &Owner, cycle: &ShuffleCycle);
}

impl Store {
    pub fn shuffle_cycle(&self, owner: &Owner, collection_id: &str) -> Result<Option<ShuffleCycle>> {
        let row: Option<(u32, String)> = self
            .conn
            .lock()
            .query_row("SELECT cycle, played FROM shuffle_cycles WHERE owner = ?1 AND collection_id = ?2", params![owner.as_str(), collection_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
            .map_err(db_err)?;
        Ok(row.map(|(cycle, played)| ShuffleCycle { collection_id: collection_id.into(), cycle: cycle.max(1), played: serde_json::from_str(&played).unwrap_or_default() }))
    }

    pub fn put_shuffle_cycle(&self, owner: &Owner, c: &ShuffleCycle) -> Result<()> {
        let played = serde_json::to_string(&c.played).map_err(|e| crate::error::AppError::internal(e.to_string()))?;
        self.conn
            .lock()
            .execute(
                "INSERT INTO shuffle_cycles (owner, collection_id, cycle, played, updated) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(owner, collection_id) DO UPDATE SET cycle = excluded.cycle, played = excluded.played, updated = excluded.updated",
                params![owner.as_str(), c.collection_id, c.cycle, played, now()],
            )
            .map_err(db_err)?;
        Ok(())
    }
}

impl ShuffleCycles for Store {
    fn load(&self, owner: &Owner, collection_id: &str) -> Option<ShuffleCycle> {
        self.shuffle_cycle(owner, collection_id).unwrap_or_else(|e| {
            tracing::warn!(error = %e.message, "couldn't read a shuffle cycle");
            None
        })
    }

    fn save(&self, owner: &Owner, cycle: &ShuffleCycle) {
        if let Err(e) = self.put_shuffle_cycle(owner, cycle) {
            tracing::warn!(error = %e.message, "couldn't save a shuffle cycle");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(v: &[&str]) -> HashSet<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_cycle_follows_the_collection() {
        let mut c = ShuffleCycle { collection_id: "PL".into(), cycle: 1, played: vec!["a".into(), "gone".into(), "a".into()] };
        assert_eq!(c.fit(&set(&["a", "b", "c"])), set(&["a"]), "removed songs leave, duplicates collapse");
        assert!(c.mark("b"));
        assert!(!c.mark("b"));
        assert_eq!(c.fit(&set(&["a", "b", "c", "new"])), set(&["a", "b"]), "a new song joins the current cycle unplayed");
        c.mark("c");
        c.mark("new");
        assert!(c.fit(&set(&["a", "b", "c", "new"])).is_empty(), "every song played: the next cycle starts");
        assert_eq!(c.cycle, 2);
    }

    #[test]
    fn cycles_are_kept_per_owner() {
        let s = Store::open_memory().unwrap();
        let (me, other) = (Owner::new("me"), Owner::new("other"));
        let mut c = ShuffleCycle::new("PL1");
        c.mark("v1");
        s.put_shuffle_cycle(&me, &c).unwrap();
        c.mark("v2");
        s.put_shuffle_cycle(&me, &c).unwrap();
        assert_eq!(s.shuffle_cycle(&me, "PL1").unwrap().unwrap().played, vec!["v1", "v2"]);
        assert!(s.shuffle_cycle(&other, "PL1").unwrap().is_none());
        assert!(s.shuffle_cycle(&me, "PL2").unwrap().is_none());
    }
}
