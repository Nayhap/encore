//! The feature wave after 0.4.0: one file per area, each owned by one lane
//! (see foundation.md in the notes). Every area gets the same three seams,
//! so no lane edits another's file or this one:
//!
//! * commands: `#[tauri::command] #[specta::specta]` functions in the area's
//!   file, registered in `lib.rs` under that area's `// features:` line.
//!   Tauri knows commands by function name alone, so names start with the
//!   area (`bookmarks_list`, `lastfm_connect`, ...);
//! * events: `tauri_specta::Event` types in the area's file, registered the
//!   same way in `collect_events!`;
//! * state: the area's `State` (a field of [`Features`], built with
//!   `Default` inside `Core::new`) and its `install`, called once from
//!   `setup` after the core is managed and the player's events flow.

use std::sync::Arc;

use tauri::AppHandle;

use crate::core::Core;

pub mod details;
pub mod lastfm;
pub mod library_tools;
pub mod playback;
pub mod presence;
pub mod saved;
pub mod stats;
pub mod taste;
pub mod wiki;

/// Every area's state, reachable as `core.features.<area>`.
#[derive(Default)]
#[allow(dead_code)] // until the lanes fill their areas in
pub struct Features {
    /// Lane A: loop, sleep timer, shuffle cycles.
    pub playback: playback::State,
    /// Lane B: find, sort, select, duplicates, discography.
    pub library_tools: library_tools::State,
    /// Lane C: bookmarks, saved queues, folders.
    pub saved: saved::State,
    /// Lane D: the block list, hidden podcasts, private sessions.
    pub taste: taste::State,
    /// Lane D: the play log's stats.
    pub stats: stats::State,
    /// Lane E: Last.fm.
    pub lastfm: lastfm::State,
    /// Lane E: Discord Rich Presence.
    pub presence: presence::State,
    /// Lane E: Wikipedia on artist pages.
    pub wiki: wiki::State,
    /// Lane E: song details and tempo and key estimates.
    pub details: details::State,
}

/// Once, from `setup`.
pub fn install(app: &AppHandle, core: &Arc<Core>) {
    playback::install(app, core);
    library_tools::install(app, core);
    saved::install(app, core);
    taste::install(app, core);
    stats::install(app, core);
    lastfm::install(app, core);
    presence::install(app, core);
    wiki::install(app, core);
    details::install(app, core);
}
