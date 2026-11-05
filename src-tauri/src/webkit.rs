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
//! A variable the user already exported always wins. Encore marks its own
//! choice in `ENCORE_WEBKIT_ENV`, because a restart (after a crash, or to
//! update) starts the new Encore with this one's environment: a variable
//! Encore set is then dropped and decided again, not taken for the user's.
//!
//! Only the choice itself is built off Linux, for its tests.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::path::PathBuf;
use std::sync::OnceLock;

const DISABLE: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";
const FORCE_SHM: &str = "WEBKIT_DMABUF_RENDERER_FORCE_SHM";
/// Which of the two Encore set itself: `shm` or `software`.
const OURS: &str = "ENCORE_WEBKIT_ENV";

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

/// Which variables the user set: those present, less the one an earlier
/// Encore set (named by `ours`). Pure, for the tests.
fn users(ours: Option<&str>, disable: bool, shm: bool) -> (bool, bool) {
    (disable && ours != Some("software"), shm && ours != Some("shm"))
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
    let ours = std::env::var(OURS).ok();
    let (disable, shm) = users(ours.as_deref(), std::env::var_os(DISABLE).is_some(), std::env::var_os(FORCE_SHM).is_some());
    // SAFETY (each env call): first thing in main, while the process has
    // one thread.
    if ours.is_some() {
        // An earlier Encore's choice (this one restarted): decided again.
        if !disable {
            unsafe { std::env::remove_var(DISABLE) };
        }
        if !shm {
            unsafe { std::env::remove_var(FORCE_SHM) };
        }
        unsafe { std::env::remove_var(OURS) };
    }
    let user_set = disable || shm;
    let mode = decide(std::env::var_os("APPIMAGE").is_some(), user_set, !user_set && marked());
    match mode {
        Mode::SharedMemory => unsafe {
            std::env::set_var(FORCE_SHM, "1");
            std::env::set_var(OURS, "shm");
        },
        Mode::Software => unsafe {
            std::env::set_var(DISABLE, "1");
            std::env::set_var(OURS, "software");
        },
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

    /// One launch as `choose` makes it, from what it inherited: the mode
    /// and what it leaves for the next (restarted) one.
    fn launch(env: (Option<&str>, bool, bool), marked: bool) -> (Mode, (Option<&'static str>, bool, bool)) {
        let (disable, shm) = users(env.0, env.1, env.2);
        let mode = decide(true, disable || shm, !(disable || shm) && marked);
        let next = match mode {
            Mode::SharedMemory => (Some("shm"), disable, true),
            Mode::Software => (Some("software"), true, shm),
            _ => (None, disable, shm),
        };
        (mode, next)
    }

    #[test]
    fn a_restart_after_a_crash_takes_the_fallback() {
        let (first, inherited) = launch((None, false, false), false);
        assert_eq!(first, Mode::SharedMemory);
        // It crashed: the marker is written and Encore restarts with this
        // environment.
        let (second, inherited) = launch(inherited, true);
        assert_eq!(second, Mode::Software, "Encore's own FORCE_SHM is not the user's");
        assert_eq!(launch(inherited, true).0, Mode::Software, "and it stays so on the next restart");
    }

    #[test]
    fn a_restart_into_a_new_version_tries_shared_memory_again() {
        // Restart to update from a version that fell back; the new one has
        // no marker of its own.
        assert_eq!(launch((Some("software"), true, false), false).0, Mode::SharedMemory);
    }

    #[test]
    fn a_users_variable_still_wins_after_a_restart() {
        assert_eq!(launch((None, false, true), true).0, Mode::User);
        assert_eq!(launch((None, true, false), false).0, Mode::User);
        // The user exported DISABLE; Encore had set nothing then.
        assert_eq!(users(Some("shm"), true, true), (true, false));
    }

    #[test]
    fn the_appimage_shares_memory_until_it_crashed() {
        assert_eq!(decide(true, false, false), Mode::SharedMemory);
        assert_eq!(decide(true, false, true), Mode::Software);
    }
}
