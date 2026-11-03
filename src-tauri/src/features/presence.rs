//! Discord Rich Presence (lane E): what's playing, over Discord's local IPC (D31).
//!
//! Commands go here and are registered in `lib.rs` under
//! `// features: integrations`; events the same way in `collect_events!`.
//! Tauri knows a command by its function name alone, across every file,
//! so name them for the area (`presence_status, presence_set`).
//!
//! Off by default. Encore's Discord application id is public, not a
//! secret; it is built in from `ENCORE_DISCORD_APP_ID` and without one the
//! feature is absent (the setting is hidden). While on, the song, artist,
//! album cover and elapsed time are shown while a song plays; nothing while
//! paused, stopped or in a private session (D36). One thread talks to
//! Discord, at most one update every two seconds (Discord's own limit is
//! five in twenty), and looks for Discord again every 30 seconds while it
//! isn't running.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use encore_core::error::{AppError, Result};
use encore_discord::{Activity, Ipc};
use encore_player::{Phase, PlayerEvent, PlayerSnapshot};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State as S};
use tauri_specta::Event;

use crate::core::{Core, SETTINGS};

/// Built in by whoever builds Encore (D31): the owner of the Discord
/// application named "Encore".
pub const APP_ID: Option<&str> = option_env!("ENCORE_DISCORD_APP_ID");
const ON_KEY: &str = "discord_presence";
const RETRY: Duration = Duration::from_secs(30);
const MIN_GAP: Duration = Duration::from_secs(2);

enum Msg {
    Enabled(bool),
    Want(Option<Activity>),
}

/// This area's state, as `core.features.presence`.
#[derive(Default)]
pub struct State {
    tx: OnceLock<mpsc::Sender<Msg>>,
    enabled: AtomicBool,
    connected: AtomicBool,
}

#[derive(Serialize, Deserialize, Clone, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PresenceStatus {
    /// This build has an application id; without one the row is hidden.
    pub available: bool,
    pub enabled: bool,
    /// Discord is running and Encore is talking to it.
    pub connected: bool,
}

#[derive(Serialize, Deserialize, Clone, specta::Type, Event)]
pub struct PresenceEvent(pub PresenceStatus);

fn status(core: &Core) -> PresenceStatus {
    let me = &core.features.presence;
    PresenceStatus { available: APP_ID.is_some_and(|id| !id.is_empty()), enabled: me.enabled.load(Ordering::SeqCst), connected: me.connected.load(Ordering::SeqCst) }
}

/// An https cover from an allow-listed host (D14), at a size Discord shows well.
fn cover(key: &str) -> Option<String> {
    let base = encore_core::art::lookup(key)?.base;
    let u = url::Url::parse(&base).ok()?;
    (u.scheme() == "https" && encore_core::art::host_allowed(u.host_str()?)).then(|| encore_core::art::sized_url(&base, 544))
}

/// What to show for a snapshot: only a song that is playing, outside a
/// private session.
fn activity(s: &PlayerSnapshot, private: bool, now_ms: i64) -> Option<Activity> {
    let e = s.current.as_ref()?;
    if !s.playing || s.phase != Phase::Playing || private || s.private.is_some() {
        return None;
    }
    let t = &e.track;
    let start_ms = now_ms - (s.position * 1000.0) as i64;
    let duration = s.duration.or(t.duration).filter(|d| d.is_finite() && *d > 0.0);
    Some(Activity {
        details: t.title.clone(),
        state: t.artist_line(),
        start_ms,
        end_ms: duration.map(|d| start_ms + (d * 1000.0) as i64),
        image: t.art.as_ref().and_then(|a| cover(&a.key)),
        image_text: t.album.as_ref().map(|a| a.name.clone()),
    })
}

/// The same thing on screen: a drift of a second or two isn't worth an update.
fn same(a: &Option<Activity>, b: &Option<Activity>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.details == b.details && a.state == b.state && a.image == b.image && (a.start_ms - b.start_ms).abs() < 2000 && a.end_ms.is_some() == b.end_ms.is_some(),
        _ => false,
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Once, from `setup`: subscribe to what this area follows.
pub fn install(app: &AppHandle, core: &Arc<Core>) {
    let Some(id) = APP_ID.filter(|id| !id.is_empty()) else { return };
    let me = &core.features.presence;
    let on = core.store.get::<bool>(SETTINGS, ON_KEY).ok().flatten().unwrap_or(false);
    me.enabled.store(on, Ordering::SeqCst);
    let (tx, rx) = mpsc::channel();
    let _ = tx.send(Msg::Enabled(on));
    let _ = me.tx.set(tx.clone());
    let (c, a) = (core.clone(), app.clone());
    let _ = std::thread::Builder::new().name("encore-discord".into()).spawn(move || worker(id.to_string(), rx, c, a));
    let mut events = core.player.subscribe();
    let core = core.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            match events.recv().await {
                Ok(PlayerEvent::State(s)) => {
                    let _ = tx.send(Msg::Want(activity(&s, core.player.private(), now_ms())));
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    let _ = tx.send(Msg::Want(activity(&core.player.snapshot(), core.player.private(), now_ms())));
                }
                Err(_) => break,
            }
        }
    });
}

fn apply(m: Msg, enabled: &mut bool, want: &mut Option<Activity>, tried: &mut Option<Instant>) {
    match m {
        Msg::Enabled(on) => {
            *enabled = on;
            *tried = None;
        }
        Msg::Want(a) => *want = a,
    }
}

fn worker(id: String, rx: mpsc::Receiver<Msg>, core: Arc<Core>, app: AppHandle) {
    let mut enabled = false;
    let mut want: Option<Activity> = None;
    let mut shown: Option<Activity> = None;
    let mut ipc: Option<Ipc> = None;
    let mut tried: Option<Instant> = None;
    let mut sent_at = Instant::now() - MIN_GAP;
    let set_connected = |on: bool| {
        if core.features.presence.connected.swap(on, Ordering::SeqCst) != on {
            let _ = PresenceEvent(status(&core)).emit(&app);
        }
    };
    loop {
        // Wait for news; while Discord is missing, look again now and then.
        let waiting = enabled && ipc.is_none() && want.is_some();
        let msg = if waiting { rx.recv_timeout(RETRY) } else { rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected) };
        match msg {
            Ok(m) => apply(m, &mut enabled, &mut want, &mut tried),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        // Take what else arrived meanwhile: only the latest counts.
        while let Ok(m) = rx.try_recv() {
            apply(m, &mut enabled, &mut want, &mut tried);
        }
        if !enabled {
            if let Some(i) = ipc.take() {
                i.close();
            }
            shown = None;
            set_connected(false);
            continue;
        }
        if ipc.is_some() && same(&want, &shown) {
            continue;
        }
        if ipc.is_none() {
            if want.is_none() || tried.is_some_and(|t| t.elapsed() < RETRY) {
                continue;
            }
            tried = Some(Instant::now());
            match Ipc::connect(&id) {
                Ok(i) => {
                    tracing::info!("connected to Discord");
                    ipc = Some(i);
                    shown = None;
                    set_connected(true);
                }
                Err(e) => {
                    tracing::debug!(error = %e, "no Discord");
                    continue;
                }
            }
        }
        let gap = sent_at.elapsed();
        if gap < MIN_GAP {
            std::thread::sleep(MIN_GAP - gap);
            while let Ok(m) = rx.try_recv() {
                apply(m, &mut enabled, &mut want, &mut tried);
            }
            if !enabled {
                if let Some(i) = ipc.take() {
                    i.close();
                }
                shown = None;
                set_connected(false);
                continue;
            }
        }
        let Some(i) = ipc.as_mut() else { continue };
        sent_at = Instant::now();
        match i.set_activity(want.as_ref()) {
            Ok(()) => shown = want.clone(),
            Err(e) => {
                tracing::info!(error = %e, "Discord went away");
                ipc = None;
                shown = None;
                tried = Some(Instant::now());
                set_connected(false);
            }
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn presence_status(core: S<'_, Arc<Core>>) -> Result<PresenceStatus> {
    Ok(status(&core))
}

/// Settings → Connections → Discord.
#[tauri::command]
#[specta::specta]
pub async fn presence_set(core: S<'_, Arc<Core>>, on: bool) -> Result<PresenceStatus> {
    let me = &core.features.presence;
    let tx = me.tx.get().ok_or_else(|| AppError::invalid("This build of Encore can't show your music on Discord."))?;
    core.store.put(SETTINGS, ON_KEY, &on, None)?;
    me.enabled.store(on, Ordering::SeqCst);
    let _ = tx.send(Msg::Enabled(on));
    if on {
        let _ = tx.send(Msg::Want(activity(&core.player.snapshot(), core.player.private(), now_ms())));
    }
    Ok(status(&core))
}

#[cfg(test)]
mod tests {
    use super::*;
    use encore_core::model::{Credit, ItemId, Track, TrackKind};
    use encore_core::queue::{Origin, QueueEntry, Repeat};

    fn snap(playing: bool, position: f64) -> PlayerSnapshot {
        let track = Track {
            id: ItemId::ytm("v"),
            title: "Daybreak Signal".into(),
            artists: vec![Credit { name: "Lumen Drive".into(), id: None }],
            album: Some(Credit { name: "Night Roads".into(), id: None }),
            duration: Some(200.0),
            explicit: false,
            art: None,
            kind: TrackKind::Song,
            plays: None,
            liked: None,
            playable: true,
            counterpart: None,
        };
        PlayerSnapshot {
            revision: "1".into(),
            track_gen: "1".into(),
            seek_gen: "0".into(),
            phase: if playing { Phase::Playing } else { Phase::Paused },
            playing,
            current: Some(QueueEntry { uid: "u".into(), track, origin: Origin::Manual }),
            index: Some(0),
            position,
            duration: Some(200.0),
            shuffle: false,
            repeat: Repeat::Off,
            autoplay: false,
            volume: 1.0,
            muted: false,
            source: None,
            stream: None,
            error: None,
            skipped_in_a_row: 0,
            queue_loading: false,
            no_output: false,
            ab_loop: None,
            sleep: None,
            private: None,
        }
    }

    #[test]
    fn shows_only_what_plays() {
        let a = activity(&snap(true, 30.0), false, 100_000).unwrap();
        assert_eq!((a.details.as_str(), a.state.as_str(), a.start_ms, a.end_ms), ("Daybreak Signal", "Lumen Drive", 70_000, Some(270_000)));
        assert_eq!(a.image_text.as_deref(), Some("Night Roads"));
        assert!(activity(&snap(false, 30.0), false, 100_000).is_none(), "paused");
        assert!(activity(&snap(true, 30.0), true, 100_000).is_none(), "private session");
        let later = activity(&snap(true, 31.0), false, 101_200);
        assert!(same(&Some(a.clone()), &later), "a little drift is the same thing");
        let seeked = activity(&snap(true, 90.0), false, 101_000);
        assert!(!same(&Some(a), &seeked));
    }
}
