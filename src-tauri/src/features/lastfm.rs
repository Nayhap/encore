//! Last.fm (lane E): connecting, scrobbling and play counts (D31).
//!
//! Commands go here and are registered in `lib.rs` under
//! `// features: integrations`; events the same way in `collect_events!`.
//! Tauri knows a command by its function name alone, across every file,
//! so name them for the area (`lastfm_status, lastfm_begin`).
//!
//! The connection belongs to this computer, whichever YouTube account is
//! signed in. Its key, secret and session key are one item in the OS
//! credential store (D9, `lastfm-session`); the settings only hold the
//! Last.fm username, so a launch without Last.fm never touches the store.
//!
//! Plays come from the play log (`core.plays`, D30): a play that starts is
//! sent as now playing, a play that counts is queued and sent. Plays in a
//! private session (D36) are neither. The queue is sent when something is
//! added, when the connection comes back, and every five minutes.

use std::sync::Arc;
use std::time::{Duration, Instant};

use encore_auth::store::{LASTFM_ACCOUNT, OsStore, STORE_NAME, SecretStore};
use encore_core::error::{AppError, Result};
use encore_core::local::integrations::{Scrobble, ScrobbleState};
use encore_core::model::{Track, TrackKind};
use encore_core::local::library::match_key;
use encore_lastfm::{Client, Credentials, Http, Play, Saved, Session};
use encore_player::plays::PlayEvent;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State as S};
use tauri_specta::Event;

use crate::core::{Core, SETTINGS};

/// The connected Last.fm username, in the settings (not secret).
const USER_KEY: &str = "lastfm_user";
/// Counts older than this are read again before sorting by them.
const COUNTS_TTL: i64 = 3600;
/// At most this many pages (1,000 songs each) of counts.
const COUNT_PAGES: u32 = 5;

/// This area's state, as `core.features.lastfm`.
pub struct State {
    inner: Mutex<Inner>,
    wake: tokio::sync::Notify,
    http: Arc<Http>,
    app: Mutex<Option<AppHandle>>,
}

impl Default for State {
    fn default() -> Self {
        State { inner: Mutex::default(), wake: tokio::sync::Notify::new(), http: Arc::new(Http::new()), app: Mutex::default() }
    }
}

#[derive(Default)]
struct Inner {
    /// The connected account, from the settings.
    username: Option<String>,
    /// Its keys, once read from the credential store; `None` also while
    /// Last.fm refuses the session (then `error` says so).
    saved: Option<Saved>,
    /// Connecting: the key, secret and token waiting for the user's approval.
    pending: Option<(Credentials, String)>,
    error: Option<String>,
    counts_read: Option<Instant>,
}

/// What Settings shows.
#[derive(Serialize, Deserialize, Clone, Default, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LastfmStatus {
    /// Connected as this Last.fm user.
    pub username: Option<String>,
    /// Waiting for the user to approve Encore in the browser.
    pub approving: bool,
    /// Scrobbles waiting to be sent, and ones Last.fm refused for good.
    pub queued: u32,
    pub failed: u32,
    /// Why scrobbling is stopped, if it is.
    pub error: Option<String>,
}

/// The status changed.
#[derive(Serialize, Deserialize, Clone, specta::Type, Event)]
pub struct LastfmEvent(pub LastfmStatus);

fn secrets() -> OsStore {
    OsStore::named(LASTFM_ACCOUNT)
}

fn status(core: &Core) -> LastfmStatus {
    let i = core.features.lastfm.inner.lock();
    let (queued, failed) = i.username.as_deref().and_then(|u| core.store.scrobble_counts(u).ok()).unwrap_or((0, 0));
    LastfmStatus { username: i.username.clone(), approving: i.pending.is_some(), queued, failed, error: i.error.clone() }
}

fn changed(core: &Core) {
    if let Some(app) = core.features.lastfm.app.lock().clone() {
        let _ = LastfmEvent(status(core)).emit(&app);
    }
}

fn client(core: &Core, creds: Credentials) -> Client {
    Client::new(core.features.lastfm.http.clone(), creds)
}

/// The words for what Last.fm said, when connecting.
fn connect_error(e: encore_lastfm::Error) -> AppError {
    use encore_lastfm::Error as E;
    match e {
        E::Network => AppError::network("Couldn't reach Last.fm. Check your connection and try again."),
        E::Api { code: 10 | 26, .. } => AppError::invalid("Last.fm doesn't accept that API key. Check that you copied all of it."),
        E::Api { code: 13, .. } => AppError::invalid("That shared secret doesn't belong to this API key. Copy both again from your Last.fm API account."),
        e if e.not_approved() => AppError::invalid("Last.fm hasn't been told yet. In the browser, choose “Yes, allow access”, then come back and choose Finish."),
        E::Api { code: 15, .. } => AppError::invalid("That approval ran out. Start connecting again."),
        e => AppError::internal(e.to_string()),
    }
}

/// The play as Last.fm knows songs: first artist, title, album (songs only).
fn play_of(t: &Track, started: i64) -> Option<Play> {
    let artist = t.artists.first()?.name.clone();
    let album = (t.kind == TrackKind::Song).then(|| t.album.as_ref().map(|a| a.name.clone())).flatten();
    Some(Play { artist, track: t.title.clone(), album, album_artist: None, duration: t.duration, timestamp: started })
}

/// Last.fm's play count of a song, from the cached counts.
pub fn plays_for(core: &Core, t: &Track) -> Option<u32> {
    let user = core.features.lastfm.inner.lock().username.clone()?;
    let artist = t.artists.first()?;
    core.store.lastfm_plays(&user, &match_key(&artist.name), &match_key(&t.title)).ok().flatten()
}

/// Once, from `setup`: subscribe to what this area follows.
pub fn install(app: &AppHandle, core: &Arc<Core>) {
    let me = &core.features.lastfm;
    *me.app.lock() = Some(app.clone());
    let user: Option<String> = core.store.get(SETTINGS, USER_KEY).ok().flatten();
    me.inner.lock().username = user.clone();
    // Only a connected computer reads the credential store.
    if let Some(user) = user {
        let c = core.clone();
        tauri::async_runtime::spawn(async move {
            let r = tokio::task::spawn_blocking(|| secrets().load()).await;
            {
                let mut i = c.features.lastfm.inner.lock();
                match r {
                    Ok(Ok(Some(v))) => match serde_json::from_str::<Saved>(&v) {
                        Ok(s) if s.session.username == user => i.saved = Some(s),
                        _ => i.error = Some("Encore's saved Last.fm connection is damaged. Connect again.".into()),
                    },
                    Ok(Ok(None)) => i.error = Some(format!("Encore's Last.fm key is no longer in {STORE_NAME}. Connect again.")),
                    Ok(Err(e)) => i.error = Some(format!("Couldn't read your Last.fm key: {e} Plays wait until it can.")),
                    Err(_) => {}
                }
            }
            changed(&c);
            c.features.lastfm.wake.notify_one();
        });
    }
    follow_plays(core.clone());
    send_queue(core.clone());
    let c = core.clone();
    core.connectivity.on_change(move |on| {
        if on {
            c.features.lastfm.wake.notify_one();
        }
    });
}

fn follow_plays(core: Arc<Core>) {
    let mut rx = core.plays.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            let ev = match rx.recv().await {
                Ok(e) => e,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            };
            match ev {
                PlayEvent::Started(p) if !p.private => {
                    let saved = core.features.lastfm.inner.lock().saved.clone();
                    let Some(saved) = saved.filter(|_| core.connectivity.online() && encore_lastfm::scrobblable(p.track.duration)) else { continue };
                    let Some(play) = play_of(&p.track, p.started) else { continue };
                    let c = client(&core, saved.credentials);
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = c.now_playing(&saved.session.key, &play).await {
                            tracing::debug!(error = %e, "Last.fm now playing");
                        }
                    });
                }
                PlayEvent::Counted { play: p, .. } if !p.private => {
                    let user = core.features.lastfm.inner.lock().username.clone();
                    let Some(user) = user.filter(|_| encore_lastfm::scrobblable(p.track.duration)) else { continue };
                    let Some(play) = play_of(&p.track, p.started) else { continue };
                    let s = Scrobble {
                        play_id: p.id.clone(),
                        owner: p.owner.to_string(),
                        username: user,
                        artist: play.artist,
                        track: play.track,
                        album: play.album,
                        album_artist: None,
                        duration: play.duration,
                        timestamp: play.timestamp,
                        state: ScrobbleState::Queued,
                        attempts: 0,
                    };
                    match core.store.scrobble_enqueue(&s) {
                        Ok(_) => {
                            changed(&core);
                            core.features.lastfm.wake.notify_one();
                        }
                        Err(e) => tracing::warn!(error = %e.message, "couldn't queue a scrobble"),
                    }
                }
                _ => {}
            }
        }
    });
}

fn send_queue(core: Arc<Core>) {
    tauri::async_runtime::spawn(async move {
        loop {
            let _ = tokio::time::timeout(Duration::from_secs(300), core.features.lastfm.wake.notified()).await;
            // A moment for more plays and the connection to settle.
            tokio::time::sleep(Duration::from_secs(2)).await;
            let saved = core.features.lastfm.inner.lock().saved.clone();
            let Some(saved) = saved.filter(|_| core.connectivity.online()) else { continue };
            let r = encore_lastfm::flush(&client(&core, saved.credentials), &saved.session, &core.store).await;
            if r.sent + r.ignored > 0 {
                tracing::info!(sent = r.sent, ignored = r.ignored, "scrobbled");
            }
            if let Some(e) = &r.stopped {
                if e.needs_reconnect() {
                    let mut i = core.features.lastfm.inner.lock();
                    i.saved = None;
                    i.error = Some("Last.fm no longer accepts Encore's connection. Connect again to send your plays.".into());
                } else {
                    tracing::info!(error = %e, "scrobbles wait");
                }
            }
            changed(&core);
        }
    });
}

#[tauri::command]
#[specta::specta]
pub async fn lastfm_status(core: S<'_, Arc<Core>>) -> Result<LastfmStatus> {
    Ok(status(&core))
}

/// Step one: check the key and secret with Last.fm and return the page
/// where the user approves Encore.
#[tauri::command]
#[specta::specta]
pub async fn lastfm_begin(core: S<'_, Arc<Core>>, api_key: String, secret: String) -> Result<String> {
    let creds = Credentials { api_key: api_key.trim().to_ascii_lowercase(), secret: secret.trim().to_ascii_lowercase() };
    if !encore_lastfm::valid_key(&creds.api_key) {
        return Err(AppError::invalid("That isn't a Last.fm API key: it has 32 letters and digits."));
    }
    if !encore_lastfm::valid_key(&creds.secret) {
        return Err(AppError::invalid("That isn't a Last.fm shared secret: it has 32 letters and digits."));
    }
    core.require_online("connect Last.fm")?;
    let token = client(&core, creds.clone()).token().await.map_err(connect_error)?;
    let url = encore_lastfm::auth_url(&creds.api_key, &token);
    core.features.lastfm.inner.lock().pending = Some((creds, token));
    changed(&core);
    Ok(url)
}

/// Step two, once the user approved Encore: keep the session.
#[tauri::command]
#[specta::specta]
pub async fn lastfm_finish(core: S<'_, Arc<Core>>) -> Result<LastfmStatus> {
    let Some((creds, token)) = core.features.lastfm.inner.lock().pending.clone() else {
        return Err(AppError::invalid("Start connecting again."));
    };
    let session: Session = client(&core, creds.clone()).session(&token).await.map_err(connect_error)?;
    let saved = Saved { credentials: creds, session };
    let json = serde_json::to_string(&saved).map_err(|e| AppError::internal(e.to_string()))?;
    tokio::task::spawn_blocking(move || secrets().save(&json))
        .await
        .map_err(|_| AppError::internal("couldn't save"))?
        .map_err(|e| AppError::new(encore_core::error::ErrorCode::Keychain, format!("Couldn't keep the Last.fm key: {e}")))?;
    let user = saved.session.username.clone();
    core.store.put(SETTINGS, USER_KEY, &user, None)?;
    {
        let mut i = core.features.lastfm.inner.lock();
        i.pending = None;
        i.error = None;
        i.counts_read = None;
        i.username = Some(user.clone());
        i.saved = Some(saved);
    }
    tracing::info!("Last.fm connected");
    changed(&core);
    core.features.lastfm.wake.notify_one();
    Ok(status(&core))
}

#[tauri::command]
#[specta::specta]
pub async fn lastfm_cancel(core: S<'_, Arc<Core>>) -> Result<LastfmStatus> {
    core.features.lastfm.inner.lock().pending = None;
    changed(&core);
    Ok(status(&core))
}

/// Forget the connection. Scrobbles not sent yet stay queued for that
/// Last.fm account, and go if it is connected again.
#[tauri::command]
#[specta::specta]
pub async fn lastfm_disconnect(core: S<'_, Arc<Core>>) -> Result<LastfmStatus> {
    tokio::task::spawn_blocking(|| secrets().delete())
        .await
        .map_err(|_| AppError::internal("couldn't remove"))?
        .map_err(|e| AppError::new(encore_core::error::ErrorCode::Keychain, format!("Couldn't remove the Last.fm key: {e}")))?;
    core.store.delete(SETTINGS, USER_KEY)?;
    *core.features.lastfm.inner.lock() = Inner::default();
    tracing::info!("Last.fm disconnected");
    changed(&core);
    Ok(status(&core))
}

/// Last.fm play counts for songs, in order (`None`: not matched), for
/// sorting by them (D34). Counts more than an hour old are read again first.
#[tauri::command]
#[specta::specta]
pub async fn lastfm_plays(core: S<'_, Arc<Core>>, tracks: Vec<Track>) -> Result<Vec<Option<u32>>> {
    let (saved, read) = {
        let i = core.features.lastfm.inner.lock();
        (i.saved.clone(), i.counts_read)
    };
    let Some(saved) = saved else { return Ok(vec![None; tracks.len()]) };
    let user = saved.session.username.clone();
    let stale = read.is_none_or(|t| t.elapsed().as_secs() as i64 > COUNTS_TTL) && core.store.lastfm_plays_updated(&user)?.is_none_or(|t| encore_core::store::now() - t > COUNTS_TTL);
    if stale && core.connectivity.online() {
        core.features.lastfm.inner.lock().counts_read = Some(Instant::now());
        let c = client(&core, saved.credentials);
        let mut all = Vec::new();
        let mut page = 1;
        let r = loop {
            match c.top_tracks(&user, page).await {
                Ok((rows, pages)) => {
                    all.extend(rows);
                    if page >= pages.min(COUNT_PAGES) {
                        break Ok(());
                    }
                    page += 1;
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                Err(e) => break Err(e),
            }
        };
        match r {
            Ok(()) => core.store.lastfm_plays_replace(&user, &encore_lastfm::play_rows(&all))?,
            // The last counts stay; they're better than none.
            Err(e) => tracing::info!(error = %e, "couldn't read Last.fm counts"),
        }
    }
    Ok(tracks.iter().map(|t| plays_for(&core, t)).collect())
}
