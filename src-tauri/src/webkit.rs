//! How WebKitGTK draws on Linux (D17).
//!
//! The AppImage carries the WebKitGTK of the oldest Ubuntu we support
//! (22.04) and draws with the host's much newer Mesa. On some of those hosts
//! (Fedora 44, Mesa 26) WebKit's DMA-BUF renderer aborts its web process and
//! the window goes blank or closes. Turning the renderer off
//! (`WEBKIT_DISABLE_DMABUF_RENDERER`) also turns off accelerated compositing
//! since WebKitGTK 2.44, so every frame is painted on the CPU. The AppImage
//! therefore keeps GPU compositing and only hands frames to the window
//! through shared memory (`WEBKIT_DMABUF_RENDERER_FORCE_SHM`), which never
//! imports a DMA-BUF into the host's Mesa. If the web process still dies in
//! that mode, Encore notes it in the data folder and restarts with the
//! renderer off, and keeps it off until the next version of Encore.
//! A variable the user already exported always wins.
//!
//! Only the choice itself is built off Linux, for its tests.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::path::PathBuf;
use std::sync::OnceLock;

const DISABLE: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";
const FORCE_SHM: &str = "WEBKIT_DMABUF_RENDERER_FORCE_SHM";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Not an AppImage: the system's WebKitGTK, left as it is.
    System,
    /// The user set one of the variables.
    User,
    /// GPU compositing, frames through shared memory.
    SharedMemory,
    /// The renderer off after a crash in shared-memory mode.
    Software,
}

static MODE: OnceLock<Mode> = OnceLock::new();

/// `<data>/webkit-software`: the web process crashed in shared-memory mode
/// under the Encore version written in it.
fn marker() -> PathBuf {
    crate::core::Paths::new().data.join("webkit-software")
}

fn marked() -> bool {
    std::fs::read_to_string(marker()).is_ok_and(|v| v.trim() == env!("CARGO_PKG_VERSION"))
}

/// What to do, from the environment and the marker. Pure, for the tests.
fn decide(appimage: bool, user_set: bool, marked: bool) -> Mode {
    if user_set {
        Mode::User
    } else if !appimage {
        Mode::System
    } else if marked {
        Mode::Software
    } else {
        Mode::SharedMemory
    }
}

#[cfg(target_os = "linux")]
/// Sets the variables. Call first thing in `main`, before any thread starts
/// or GTK/WebKit loads.
pub fn choose() {
    let user_set = std::env::var_os(DISABLE).is_some() || std::env::var_os(FORCE_SHM).is_some();
    let mode = decide(std::env::var_os("APPIMAGE").is_some(), user_set, !user_set && marked());
    // SAFETY: first thing in main, while the process has one thread.
    match mode {
        Mode::SharedMemory => unsafe { std::env::set_var(FORCE_SHM, "1") },
        Mode::Software => unsafe { std::env::set_var(DISABLE, "1") },
        Mode::System | Mode::User => {}
    }
    let _ = MODE.set(mode);
}

#[cfg(target_os = "linux")]
/// Logs the mode once logging is up.
pub fn log_mode() {
    let mode = MODE.get().copied().unwrap_or(Mode::System);
    let var = |k: &str| std::env::var(k).unwrap_or_default();
    tracing::info!(mode = ?mode, disable_dmabuf = %var(DISABLE), force_shm = %var(FORCE_SHM), "webkit renderer");
}

#[cfg(target_os = "linux")]
/// Watches the main window's web process. In shared-memory mode a crash
/// writes the marker and restarts Encore (through the normal exit, so the
/// queue and history are settled); otherwise the page is reloaded, at most
/// twice per launch, rather than leaving a blank window.
pub fn watch(window: &tauri::WebviewWindow) {
    use std::sync::atomic::{AtomicU32, Ordering};
    use tauri::Manager;
    use webkit2gtk::{WebProcessTerminationReason as Reason, WebViewExt};
    static RELOADS: AtomicU32 = AtomicU32::new(0);
    let app = window.app_handle().clone();
    let _ = window.with_webview(move |w| {
        w.inner().connect_web_process_terminated(move |view, reason| {
            let mode = MODE.get().copied().unwrap_or(Mode::System);
            tracing::error!(?reason, ?mode, "the web process ended");
            if reason == Reason::TerminatedByApi {
                return;
            }
            if mode == Mode::SharedMemory && reason == Reason::Crashed {
                let path = marker();
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                match std::fs::write(&path, env!("CARGO_PKG_VERSION")) {
                    Ok(()) => {
                        tracing::warn!("restarting with WebKit's DMA-BUF renderer off");
                        app.request_restart();
                        return;
                    }
                    Err(e) => tracing::warn!(error = %e, "could not save the WebKit fallback"),
                }
            }
            if RELOADS.fetch_add(1, Ordering::Relaxed) < 2 {
                view.reload();
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_users_variable_wins() {
        assert_eq!(decide(true, true, true), Mode::User);
        assert_eq!(decide(false, true, false), Mode::User);
    }

    #[test]
    fn only_the_appimage_is_changed() {
        assert_eq!(decide(false, false, false), Mode::System);
        assert_eq!(decide(false, false, true), Mode::System);
    }

    #[test]
    fn the_appimage_shares_memory_until_it_crashed() {
        assert_eq!(decide(true, false, false), Mode::SharedMemory);
        assert_eq!(decide(true, false, true), Mode::Software);
    }
}
