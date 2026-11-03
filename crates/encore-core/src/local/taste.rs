//! The block list (D36, lane D): songs and artists that never play on their
//! own. The player asks through `service::Admission`; lane D implements it
//! over this table and the "Hide podcasts" setting (D27).

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::model::ItemId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum BlockKind {
    Track,
    Artist,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Blocked {
    pub kind: BlockKind,
    pub target: ItemId,
    /// The song's title or the artist's name.
    pub label: String,
    /// A song's artist line.
    pub detail: Option<String>,
    pub created: String,
}
