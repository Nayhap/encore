//! Shuffle cycles (D33, lane A): per owner and collection, the songs already
//! played in the current cycle, so a shuffle doesn't repeat a song until
//! every song has played, across sessions. The table is in `local::SCHEMA`;
//! the queue (`crate::queue`) and these store methods are lane A's.

use serde::{Deserialize, Serialize};

/// One collection's cycle, as stored (`shuffle_cycles.played` is the JSON
/// list of video ids).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ShuffleCycle {
    pub collection_id: String,
    /// Starts at 1; a new cycle begins once every song has played.
    pub cycle: u32,
    pub played: Vec<String>,
}
