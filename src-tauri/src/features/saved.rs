//! Saved things (lane C): bookmarks (D25), saved queues (D33), folders and their local covers (D34).
//!
//! Commands go here and are registered in `lib.rs` under
//! `// features: saved`; events the same way in `collect_events!`.
//! Tauri knows a command by its function name alone, across every file,
//! so name them for the area (`bookmarks_list, folders_list`).

use std::sync::Arc;

use tauri::AppHandle;

use crate::core::Core;

/// This area's state, as `core.features.saved`.
#[derive(Default)]
pub struct State {}

/// Once, from `setup`: subscribe to what this area follows.
pub fn install(_app: &AppHandle, _core: &Arc<Core>) {}
