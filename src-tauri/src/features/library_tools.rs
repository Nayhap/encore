//! Library tools (lane B): find, sort, select and mass actions, duplicates (D34), the discography and community playlists (D35).
//!
//! Commands go here and are registered in `lib.rs` under
//! `// features: library tools`; events the same way in `collect_events!`.
//! Tauri knows a command by its function name alone, across every file,
//! so name them for the area (`library_years, bulk_add`).

use std::sync::Arc;

use tauri::AppHandle;

use crate::core::Core;

/// This area's state, as `core.features.library_tools`.
#[derive(Default)]
pub struct State {}

/// Once, from `setup`: subscribe to what this area follows.
pub fn install(_app: &AppHandle, _core: &Arc<Core>) {}
