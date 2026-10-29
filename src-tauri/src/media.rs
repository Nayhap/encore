//! macOS Now Playing and media keys via souvlaki (D15), plus an App Nap
//! activity while audio plays.

use std::cell::RefCell;
use std::sync::Arc;
use std::time::Duration;

use encore_player::{Phase, PlayerEvent, PlayerSnapshot};
use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig, SeekDirection};
use tauri::{AppHandle, Manager};

use crate::core::Core;

thread_local! {
    static CONTROLS: RefCell<Option<MediaControls>> = const { RefCell::new(None) };
    static ACTIVITY: RefCell<Option<objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2_foundation::NSObjectProtocol>>>> = const { RefCell::new(None) };
}

/// Must be called on the main thread (Tauri `setup`).
pub fn install(app: &AppHandle, core: Arc<Core>) {
    let mut controls = match MediaControls::new(PlatformConfig { display_name: "Encore", dbus_name: "app.encore.desktop", hwnd: None }) {
        Ok(c) => c,
        Err(_) => return,
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
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut last_track: Option<(String, bool)> = None;
        let mut last_phase: Option<(Phase, bool)> = None;
        loop {
            let ev = match rx.recv().await {
                Ok(e) => e,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            };
            let PlayerEvent::State(s) = ev else { continue };
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
            let cover = art_key.as_ref().and_then(|k| core.art.path_for(k, 544)).filter(|p| p.exists()).map(|p| format!("file://{}", p.display()));
            if changed_track && cover.is_none() {
                if let Some(key) = art_key.clone() {
                    // Fetch the cover (same cache the UI uses), then refresh Now Playing.
                    let (app2, snap2) = (app.clone(), s.clone());
                    tauri::async_runtime::spawn(async move {
                        if core.art.get(&key, 544).await.is_ok() {
                            if let Some(p) = core.art.path_for(&key, 544) {
                                let url = format!("file://{}", p.display());
                                let _ = app2.run_on_main_thread(move || update(&snap2, Some(url), true));
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

fn update(s: &PlayerSnapshot, cover: Option<String>, changed_track: bool) {
    CONTROLS.with(|c| {
        let mut b = c.borrow_mut();
        let Some(ctl) = b.as_mut() else { return };
        if changed_track {
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
        }
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

/// Hold an NSProcessInfo activity while playing so App Nap does not
/// throttle the audio and decode threads.
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
