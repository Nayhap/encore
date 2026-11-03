//! Listening stats (lane D): the Stats tab over the play log (D30).
//!
//! Commands go here and are registered in `lib.rs` under
//! `// features: taste and stats`; events the same way in `collect_events!`.
//! Tauri knows a command by its function name alone, across every file,
//! so name them for the area (`stats_month`).

use std::sync::Arc;

use tauri::AppHandle;

use crate::core::Core;

/// This area's state, as `core.features.stats`.
#[derive(Default)]
pub struct State {}

/// Once, from `setup`: subscribe to what this area follows.
pub fn install(_app: &AppHandle, _core: &Arc<Core>) {}
