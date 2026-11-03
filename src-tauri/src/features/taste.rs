//! Taste (lane D): the block list and "Don't play this again" (D36), "Hide podcasts" (D27), private sessions (D36).
//!
//! Commands go here and are registered in `lib.rs` under
//! `// features: taste and stats`; events the same way in `collect_events!`.
//! Tauri knows a command by its function name alone, across every file,
//! so name them for the area (`blocked_list, private_start`).

use std::sync::Arc;

use encore_core::model::Page;
use tauri::AppHandle;

use crate::core::Core;

/// This area's state, as `core.features.taste`.
#[derive(Default)]
pub struct State {}

/// Once, from `setup`: subscribe to what this area follows.
pub fn install(_app: &AppHandle, _core: &Arc<Core>) {}

/// Home, Explore and browse pages pass through here after the cache
/// (D27 "Hide podcasts", D36 blocked songs and artists), so a change of
/// setting shows at once and the cached copy stays whole. Unchanged until
/// lane D fills it in.
pub fn filter_page(_core: &Core, page: Page) -> Page {
    page
}
