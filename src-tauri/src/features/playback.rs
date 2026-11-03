//! Playback tools (lane A): the A–B loop and the sleep timer (D26), shuffle cycles and the Autoplay rule (D33).
//!
//! Commands go here and are registered in `lib.rs` under
//! `// features: playback`; events the same way in `collect_events!`.
//! Tauri knows a command by its function name alone, across every file,
//! so name them for the area (`loop_set, sleep_set`).

use std::sync::Arc;

use tauri::AppHandle;

use crate::core::Core;

/// This area's state, as `core.features.playback`.
#[derive(Default)]
pub struct State {}

/// Once, from `setup`: subscribe to what this area follows.
pub fn install(_app: &AppHandle, _core: &Arc<Core>) {}
