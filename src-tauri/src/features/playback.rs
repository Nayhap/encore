//! Playback tools (lane A): the A–B loop and the sleep timer (D26), shuffle cycles and the Autoplay rule (D33).
//!
//! Commands go here and are registered in `lib.rs` under
//! `// features: playback`; events the same way in `collect_events!`.
//! Tauri knows a command by its function name alone, across every file,
//! so name them for the area (`loop_set, sleep_set`).
//!
//! The loop and the timer show in the player snapshot (`loop`, `sleep`);
//! the Autoplay rule and shuffle cycles need no commands of their own.

use std::sync::Arc;

use encore_core::error::Result;
use encore_player::SleepMode;
use tauri::AppHandle;

use crate::core::Core;

type S<'a> = tauri::State<'a, Arc<Core>>;

/// This area's state, as `core.features.playback`.
#[derive(Default)]
pub struct State {}

/// Once, from `setup`: shuffle cycles are kept in the store (D33).
pub fn install(_app: &AppHandle, core: &Arc<Core>) {
    core.player.set_shuffle_cycles(core.store.clone());
}

/// Loop the song playing between `a` and `b` seconds.
#[tauri::command]
#[specta::specta]
pub async fn loop_set(core: S<'_>, a: f64, b: f64) -> Result<()> {
    core.player.set_loop(a, b)
}

#[tauri::command]
#[specta::specta]
pub async fn loop_clear(core: S<'_>) -> Result<()> {
    core.player.clear_loop();
    Ok(())
}

/// Start the sleep timer; `minutes` for `SleepMode::Timer`.
#[tauri::command]
#[specta::specta]
pub async fn sleep_set(core: S<'_>, mode: SleepMode, minutes: Option<f64>) -> Result<()> {
    core.player.set_sleep(mode, minutes)
}

#[tauri::command]
#[specta::specta]
pub async fn sleep_cancel(core: S<'_>) -> Result<()> {
    core.player.cancel_sleep();
    Ok(())
}
