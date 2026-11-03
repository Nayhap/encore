//! Discord Rich Presence (lane E): what's playing, over Discord's local IPC (D31).
//!
//! Commands go here and are registered in `lib.rs` under
//! `// features: integrations`; events the same way in `collect_events!`.
//! Tauri knows a command by its function name alone, across every file,
//! so name them for the area (`presence_set`).

use std::sync::Arc;

use tauri::AppHandle;

use crate::core::Core;

/// This area's state, as `core.features.presence`.
#[derive(Default)]
pub struct State {}

/// Once, from `setup`: subscribe to what this area follows.
pub fn install(_app: &AppHandle, _core: &Arc<Core>) {}
