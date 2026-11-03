//! Library tools' cache (D34, lane B): release years of albums, looked up
//! one album at a time through the background lane (`crate::paging`) when a
//! list is sorted by year, and kept here so it happens once.

use serde::{Deserialize, Serialize};

/// `year: None`: the album was read and gives no year.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AlbumYear {
    pub album_id: String,
    pub year: Option<u16>,
    pub fetched: i64,
}
