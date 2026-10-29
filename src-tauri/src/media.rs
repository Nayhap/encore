//! OS now-playing and media keys via souvlaki (D15): Now Playing on macOS
//! (plus an App Nap activity while audio plays), the System Media Transport
//! Controls on Windows (bound to the main window's HWND) and MPRIS on Linux.

use std::cell::RefCell;
use std::sync::Arc;
use std::time::Duration;

use encore_player::{Phase, PlayerEvent, PlayerSnapshot};
use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig, SeekDirection};
use tauri::{AppHandle, Manager};

use crate::core::Core;

thread_local! {
    static CONTROLS: RefCell<Option<MediaControls>> = const { RefCell::new(None) };
    /// The queue entry whose metadata the OS shows (main thread only).
    static SHOWN: RefCell<Option<String>> = const { RefCell::new(None) };
}

#[cfg(target_os = "macos")]
thread_local! {
    static ACTIVITY: RefCell<Option<objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2_foundation::NSObjectProtocol>>>> = const { RefCell::new(None) };
}

/// SMTC is per window on Windows.
#[cfg(windows)]
fn hwnd(app: &AppHandle) -> Option<*mut std::ffi::c_void> {
    app.get_webview_window("main").and_then(|w| w.hwnd().ok()).map(|h| h.0 as *mut std::ffi::c_void)
}

#[cfg(not(windows))]
fn hwnd(_app: &AppHandle) -> Option<*mut std::ffi::c_void> {
    None
}

/// Must be called on the main thread (Tauri `setup`).
pub fn install(app: &AppHandle, core: Arc<Core>) {
    let hwnd = hwnd(app);
    if cfg!(windows) && hwnd.is_none() {
        tracing::warn!("no window handle; media keys disabled");
        return;
    }
    let mut controls = match MediaControls::new(PlatformConfig { display_name: "Encore", dbus_name: "app.encore.desktop", hwnd }) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = ?e, "media controls unavailable");
            return;
        }
    };
    let p = core.player.clone();
    let rt = tauri::async_runtime::handle();
    let app2 = app.clone();
    let _ = controls.attach(move |ev: MediaControlEvent| {
        let p = p.clone();
        match ev {
            MediaControlEvent::Play => {
                let s = p.snapshot();
                if !s.playing {
                    p.toggle();
                }
            }
            MediaControlEvent::Pause => p.pause(),
            MediaControlEvent::Toggle => p.toggle(),
            MediaControlEvent::Next => {
                rt.spawn(async move { p.next().await });
            }
            MediaControlEvent::Previous => {
                rt.spawn(async move { p.prev().await });
            }
            MediaControlEvent::Stop => p.pause(),
            MediaControlEvent::SetPosition(MediaPosition(d)) => p.seek(d.as_secs_f64()),
            MediaControlEvent::Seek(dir) => {
                let s = p.snapshot();
                p.seek(s.position + if dir == SeekDirection::Forward { 10.0 } else { -10.0 });
            }
            MediaControlEvent::SeekBy(dir, d) => {
                let s = p.snapshot();
                let d = d.as_secs_f64();
                p.seek(s.position + if dir == SeekDirection::Forward { d } else { -d });
            }
            MediaControlEvent::Raise => {
                if let Some(w) = app2.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            _ => {}
        }
    });
    CONTROLS.with(|c| *c.borrow_mut() = Some(controls));

    // Feed player events to Now Playing on the main thread.
    let mut rx = core.player.subscribe();
    let player = core.player.clone();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut last_track: Option<(String, bool)> = None;
        let mut last_phase: Option<(Phase, bool)> = None;
        loop {
            let ev = match rx.recv().await {
                Ok(e) => e,
                // Missed events: catch up from a snapshot.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => PlayerEvent::State(player.snapshot()),
                Err(_) => break,
            };
            let s = match ev {
                PlayerEvent::State(s) => s,
                // An automatic skip changes the song before the next state
                // event; follow it at once so the OS never shows the old one.
                PlayerEvent::TrackChanged(_) => player.snapshot(),
                _ => continue,
            };
            // Metadata is re-sent when the track changes and when its
            // duration becomes known (Now Playing shows progress then).
            let tid = s.current.as_ref().map(|c| (c.uid.clone(), s.duration.is_some()));
            let changed_track = tid != last_track;
            let key = (s.phase, s.playing);
            if !changed_track && last_phase == Some(key) {
                continue;
            }
            last_track = tid;
            last_phase = Some(key);
            let core = app.state::<Arc<Core>>().inner().clone();
            let art_key = s.current.as_ref().and_then(|c| c.track.art.as_ref()).map(|a| a.key.clone());
            let cover = art_key.as_ref().and_then(|k| core.art.path_for(k, 544)).filter(|p| p.exists()).map(|p| file_url(&p));
            if changed_track && cover.is_none() {
                if let Some(key) = art_key.clone() {
                    // Fetch the cover (same cache the UI uses), then refresh Now Playing.
                    let (app2, snap2) = (app.clone(), s.clone());
                    tauri::async_runtime::spawn(async move {
                        if core.art.get(&key, 544).await.is_ok() {
                            if let Some(p) = core.art.path_for(&key, 544) {
                                let url = file_url(&p);
                                // Only if that song is still the one shown: a quick skip
                                // must not be overwritten by an older song's cover.
                                let _ = app2.run_on_main_thread(move || {
                                    let uid = snap2.current.as_ref().map(|c| c.uid.clone());
                                    if SHOWN.with(|s| *s.borrow() == uid) {
                                        set_metadata(&snap2, Some(url));
                                    }
                                });
                            }
                        }
                    });
                }
            }
            let snap = s.clone();
            let _ = app.run_on_main_thread(move || update(&snap, cover, changed_track));
        }
    });
}

/// souvlaki takes `file://` plus the native path on every OS (on Windows it
/// strips the prefix and opens the path; MPRIS wants `file:///home/...`).
fn file_url(p: &std::path::Path) -> String {
    format!("file://{}", p.display())
}

fn set_metadata(s: &PlayerSnapshot, cover: Option<String>) {
    SHOWN.with(|x| *x.borrow_mut() = s.current.as_ref().map(|c| c.uid.clone()));
    CONTROLS.with(|c| {
        let mut b = c.borrow_mut();
        let Some(ctl) = b.as_mut() else { return };
        match &s.current {
            Some(e) => {
                let artist = e.track.artist_line();
                let _ = ctl.set_metadata(MediaMetadata {
                    title: Some(&e.track.title),
                    artist: Some(&artist),
                    album: e.track.album.as_ref().map(|a| a.name.as_str()),
                    cover_url: cover.as_deref(),
                    duration: s.duration.map(Duration::from_secs_f64),
                });
            }
            None => {
                let _ = ctl.set_metadata(MediaMetadata::default());
            }
        }
    });
}

fn update(s: &PlayerSnapshot, cover: Option<String>, changed_track: bool) {
    if changed_track {
        set_metadata(s, cover);
    }
    CONTROLS.with(|c| {
        let mut b = c.borrow_mut();
        let Some(ctl) = b.as_mut() else { return };
        let progress = Some(MediaPosition(Duration::from_secs_f64(s.position.max(0.0))));
        let pb = match (s.current.is_some(), s.playing) {
            (false, _) => MediaPlayback::Stopped,
            (true, true) => MediaPlayback::Playing { progress },
            (true, false) => MediaPlayback::Paused { progress },
        };
        let _ = ctl.set_playback(pb);
    });
    app_nap(s.playing);
}

#[cfg(not(target_os = "macos"))]
fn app_nap(_playing: bool) {}

/// Hold an NSProcessInfo activity while playing so App Nap does not
/// throttle the audio and decode threads.
#[cfg(target_os = "macos")]
fn app_nap(playing: bool) {
    use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};
    ACTIVITY.with(|a| {
        let mut a = a.borrow_mut();
        let info = NSProcessInfo::processInfo();
        match (playing, a.is_some()) {
            (true, false) => {
                let reason = NSString::from_str("Playing audio");
                let act = info.beginActivityWithOptions_reason(NSActivityOptions::UserInitiated | NSActivityOptions::LatencyCritical, &reason);
                *a = Some(act);
            }
            (false, true) => {
                if let Some(act) = a.take() {
                    unsafe { info.endActivity(&act) };
                }
            }
            _ => {}
        }
    });
}

/// Refresh Now Playing's elapsed time after a seek.
pub fn position_changed(app: &AppHandle, s: PlayerSnapshot) {
    let _ = app.run_on_main_thread(move || update(&s, None, false));
}
