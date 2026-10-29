//! Typed commands (D13). The UI never passes cookies, URLs to resolve or
//! raw InnerTube payloads; it passes ids and domain objects.

use std::sync::Arc;
use std::time::Duration;

use encore_core::error::{AppError, Result};
use encore_core::model::*;
use encore_core::queue::{QueueSource, Repeat};
use encore_player::{PlayerSnapshot, QueueSnapshot};
use encore_resolver::RuntimeStatus;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_specta::Event;

use crate::core::Core;
use crate::events::*;

type S<'a> = State<'a, Arc<Core>>;

#[derive(Serialize, Deserialize, Clone, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    pub version: String,
    pub auth: AuthState,
    pub player: PlayerSnapshot,
    pub queue: QueueSnapshot,
    /// The player components (D22).
    pub runtimes: RuntimeStatus,
    /// Settings → Playback → "Send plays to YouTube history".
    pub report_history: bool,
}

#[derive(Serialize, Deserialize, Clone, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EntriesPage {
    pub entries: Vec<Entry>,
    pub cursor: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CacheInfo {
    pub audio_bytes: f64,
    pub art_bytes: f64,
    pub lyrics_entries: u32,
}

fn strip_page(p: &mut Page) {
    p.cursor = None;
}

// ---------- app ----------

#[tauri::command]
#[specta::specta]
pub async fn bootstrap(core: S<'_>) -> Result<Bootstrap> {
    tracing::info!("command: bootstrap");
    Ok(Bootstrap {
        version: env!("CARGO_PKG_VERSION").to_string(),
        auth: core.accounts.state(),
        player: core.player.snapshot(),
        queue: core.player.queue_snapshot(),
        runtimes: core.resolver.runtimes().status(),
        report_history: core.player.reporting(),
    })
}

#[tauri::command]
#[specta::specta]
pub async fn set_report_history(core: S<'_>, on: bool) -> Result<()> {
    core.player.set_reporting(on);
    core.store.put(crate::core::SETTINGS, crate::core::REPORT_HISTORY, &on, None)?;
    Ok(())
}

fn setup_error(e: encore_resolver::SetupError) -> AppError {
    AppError::new(encore_core::ErrorCode::Unsupported, e.message)
}

/// The player components, with the space they use measured.
#[tauri::command]
#[specta::specta]
pub async fn runtimes_status(core: S<'_>) -> Result<RuntimeStatus> {
    let rt = core.resolver.runtimes().clone();
    Ok(tokio::task::spawn_blocking(move || rt.status_measured()).await.map_err(|_| AppError::internal("status"))?)
}

/// The user approved the download: install (or, with `reinstall`, delete
/// and install again) in the background. Progress arrives as
/// `RuntimesEvent`s.
#[tauri::command]
#[specta::specta]
pub async fn runtimes_install(core: S<'_>, reinstall: bool) -> Result<RuntimeStatus> {
    let rt = core.resolver.runtimes().clone();
    rt.approve().map_err(setup_error)?;
    if reinstall {
        core.resolver.stop_helpers().await;
        rt.remove(true).await.map_err(setup_error)?;
    }
    rt.start();
    Ok(rt.status())
}

#[tauri::command]
#[specta::specta]
pub async fn runtimes_cancel(core: S<'_>) -> Result<()> {
    core.resolver.runtimes().cancel();
    Ok(())
}

/// Delete the downloaded components to free the space. Playback asks for
/// the setup again afterwards.
#[tauri::command]
#[specta::specta]
pub async fn runtimes_remove(core: S<'_>) -> Result<RuntimeStatus> {
    core.resolver.stop_helpers().await;
    let rt = core.resolver.runtimes().clone();
    rt.remove(false).await.map_err(setup_error)?;
    Ok(rt.status_measured())
}

#[tauri::command]
#[specta::specta]
pub async fn cache_info(core: S<'_>) -> Result<CacheInfo> {
    Ok(CacheInfo { audio_bytes: core.audio_cache.total_bytes() as f64, art_bytes: core.art.total_bytes() as f64, lyrics_entries: core.store.count("lyrics") as u32 })
}

#[tauri::command]
#[specta::specta]
pub async fn cache_clear(core: S<'_>, what: String) -> Result<()> {
    match what.as_str() {
        "audio" => core.audio_cache.clear(),
        "art" => core.art.clear(),
        "lyrics" => {
            core.store.purge_scope("lyrics")?;
        }
        _ => return Err(AppError::invalid("Unknown cache.")),
    }
    Ok(())
}

// ---------- browse ----------

#[tauri::command]
#[specta::specta]
pub async fn home(core: S<'_>) -> Result<Page> {
    tracing::info!("command: home");
    let svc = core.service();
    let ctx = svc.context();
    core.cached("home", || async { svc.home(&ctx).await }, strip_page).await
}

#[tauri::command]
#[specta::specta]
pub async fn explore(core: S<'_>) -> Result<Page> {
    let svc = core.service();
    let ctx = svc.context();
    core.cached("explore", || async { svc.explore(&ctx).await }, strip_page).await
}

#[tauri::command]
#[specta::specta]
pub async fn browse_page(core: S<'_>, browse_id: String, params: Option<String>) -> Result<Page> {
    let svc = core.service();
    let ctx = svc.context();
    let key = format!("page:{browse_id}:{}", params.as_deref().unwrap_or(""));
    core.cached(&key, || async { svc.browse_page(&ctx, &browse_id, params.as_deref()).await }, strip_page).await
}

#[tauri::command]
#[specta::specta]
pub async fn more_shelves(core: S<'_>, cursor: String) -> Result<Page> {
    let svc = core.service();
    svc.more_shelves(&svc.context(), &cursor).await
}

#[tauri::command]
#[specta::specta]
pub async fn search(core: S<'_>, query: String, filter: Option<SearchFilter>) -> Result<SearchResults> {
    let svc = core.service();
    svc.search(&svc.context(), &query, filter).await
}

#[tauri::command]
#[specta::specta]
pub async fn search_more(core: S<'_>, cursor: String) -> Result<SearchResults> {
    let svc = core.service();
    svc.search_more(&svc.context(), &cursor).await
}

#[tauri::command]
#[specta::specta]
pub async fn suggestions(core: S<'_>, query: String) -> Result<Vec<String>> {
    let svc = core.service();
    svc.suggestions(&svc.context(), &query).await
}

#[tauri::command]
#[specta::specta]
pub async fn collection(core: S<'_>, id: ItemId) -> Result<Collection> {
    let svc = core.service();
    let ctx = svc.context();
    let key = format!("collection:{id}");
    core.cached(&key, || async { svc.collection(&ctx, &id).await }, |c: &mut Collection| c.cursor = None).await
}

#[tauri::command]
#[specta::specta]
pub async fn collection_more(core: S<'_>, cursor: String) -> Result<EntriesPage> {
    let svc = core.service();
    let (entries, cursor) = svc.collection_more(&svc.context(), &cursor).await?;
    Ok(EntriesPage { entries, cursor })
}

#[tauri::command]
#[specta::specta]
pub async fn artist(core: S<'_>, id: ItemId) -> Result<Artist> {
    let svc = core.service();
    let ctx = svc.context();
    let key = format!("artist:{id}");
    core.cached(&key, || async { svc.artist(&ctx, &id).await }, |_: &mut Artist| {}).await
}

#[tauri::command]
#[specta::specta]
pub async fn library(core: S<'_>, section: LibrarySection) -> Result<LibraryPage> {
    let svc = core.service();
    let ctx = svc.context();
    let key = format!("library:{section:?}");
    core.cached(&key, || async { svc.library(&ctx, section).await }, |p: &mut LibraryPage| p.cursor = None).await
}

#[tauri::command]
#[specta::specta]
pub async fn library_more(core: S<'_>, section: LibrarySection, cursor: String) -> Result<LibraryPage> {
    let svc = core.service();
    svc.library_more(&svc.context(), section, &cursor).await
}

#[tauri::command]
#[specta::specta]
pub async fn lyrics(core: S<'_>, track: Track) -> Result<Option<Lyrics>> {
    Ok(core.lyrics.get(&track).await)
}

// ---------- playback ----------

#[tauri::command]
#[specta::specta]
pub async fn player_snapshot(core: S<'_>) -> Result<PlayerSnapshot> {
    Ok(core.player.snapshot())
}

#[tauri::command]
#[specta::specta]
pub async fn queue_snapshot(core: S<'_>) -> Result<QueueSnapshot> {
    Ok(core.player.queue_snapshot())
}

#[tauri::command]
#[specta::specta]
pub async fn play_tracks(core: S<'_>, tracks: Vec<Track>, start: u32, source: Option<QueueSource>, shuffle: bool) -> Result<()> {
    core.player.play_tracks(tracks, start as usize, source, shuffle)
}

#[tauri::command]
#[specta::specta]
pub async fn play_collection(core: S<'_>, id: ItemId, start: u32, shuffle: bool) -> Result<()> {
    core.player.play_collection(&id, start as usize, shuffle).await
}

#[tauri::command]
#[specta::specta]
pub async fn play_track(core: S<'_>, track: Track) -> Result<()> {
    core.player.play_track(track)
}

#[tauri::command]
#[specta::specta]
pub async fn start_radio(core: S<'_>, playlist_id: String, label: String) -> Result<()> {
    let svc = core.service();
    let u = svc.up_next(&svc.context(), &RadioSeed::Playlist { playlist_id: playlist_id.clone() }, None).await?;
    core.player.play_tracks(u.tracks, 0, Some(QueueSource { label, collection_id: None, radio_playlist: u.playlist_id.or(Some(playlist_id)) }), false)
}

#[tauri::command]
#[specta::specta]
pub async fn toggle(core: S<'_>) -> Result<()> {
    core.player.toggle();
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn next(core: S<'_>) -> Result<()> {
    core.player.next().await;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn prev(core: S<'_>) -> Result<()> {
    core.player.prev().await;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn seek(core: S<'_>, seconds: f64) -> Result<()> {
    core.player.seek(seconds);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn jump_to(core: S<'_>, uid: String) -> Result<()> {
    core.player.jump(&uid)
}

#[tauri::command]
#[specta::specta]
pub async fn play_next(core: S<'_>, tracks: Vec<Track>) -> Result<()> {
    core.player.play_next(tracks);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn add_to_queue(core: S<'_>, tracks: Vec<Track>) -> Result<()> {
    core.player.add_to_queue(tracks);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn queue_remove(core: S<'_>, uid: String) -> Result<()> {
    core.player.remove(&uid)
}

#[tauri::command]
#[specta::specta]
pub async fn queue_move(core: S<'_>, uid: String, before: Option<String>) -> Result<()> {
    core.player.move_entry(&uid, before.as_deref())
}

#[tauri::command]
#[specta::specta]
pub async fn clear_up_next(core: S<'_>) -> Result<()> {
    core.player.clear_up_next();
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn set_shuffle(core: S<'_>, on: bool) -> Result<()> {
    core.player.set_shuffle(on);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn set_repeat(core: S<'_>, mode: Repeat) -> Result<()> {
    core.player.set_repeat(mode);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn set_autoplay(core: S<'_>, on: bool) -> Result<()> {
    core.player.set_autoplay(on);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn set_volume(core: S<'_>, value: f64) -> Result<()> {
    core.player.set_volume(value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn set_muted(core: S<'_>, muted: bool) -> Result<()> {
    core.player.set_muted(muted);
    Ok(())
}

// ---------- library writes ----------

#[tauri::command]
#[specta::specta]
pub async fn set_like(core: S<'_>, track_id: ItemId, liked: bool) -> Result<MutationOutcome> {
    let svc = core.service();
    let r = svc.set_like(&svc.context(), &track_id, liked).await?;
    if r == MutationOutcome::Done {
        core.player.update_track(track_id.as_str(), |t| t.liked = Some(liked));
    }
    Ok(r)
}

#[tauri::command]
#[specta::specta]
pub async fn set_saved(core: S<'_>, playlist_id: String, saved: bool) -> Result<MutationOutcome> {
    let svc = core.service();
    svc.set_saved(&svc.context(), &playlist_id, saved).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_subscribed(core: S<'_>, channel_id: String, subscribed: bool) -> Result<MutationOutcome> {
    let svc = core.service();
    svc.set_subscribed(&svc.context(), &channel_id, subscribed).await
}

/// After a playlist write: drop the cached copies (the cache only serves on
/// network failure, but must never bring back a deleted playlist or an old
/// order) and tell every playlist surface to refresh.
fn playlists_changed(app: &AppHandle, core: &Core, playlist_id: Option<&str>, outcome: MutationOutcome) {
    if outcome == MutationOutcome::Rejected || outcome == MutationOutcome::Duplicate {
        return;
    }
    let scope = core.account_scope();
    let _ = core.store.delete(&scope, "library:Playlists");
    if let Some(p) = playlist_id {
        let bare = p.strip_prefix("VL").unwrap_or(p);
        for key in [format!("collection:ytm:VL{bare}"), format!("collection:ytm:{bare}")] {
            let _ = core.store.delete(&scope, &key);
        }
    }
    let _ = PlaylistsEvent { playlist_id: playlist_id.map(|p| format!("ytm:VL{}", p.strip_prefix("VL").unwrap_or(p))) }.emit(app);
}

#[tauri::command]
#[specta::specta]
pub async fn playlist_add(app: AppHandle, core: S<'_>, playlist_id: String, track_ids: Vec<ItemId>, allow_duplicates: bool) -> Result<MutationOutcome> {
    let svc = core.service();
    let r = svc.playlist_add(&svc.context(), &playlist_id, &track_ids, allow_duplicates).await?;
    playlists_changed(&app, &core, Some(&playlist_id), r);
    Ok(r)
}

#[tauri::command]
#[specta::specta]
pub async fn playlist_create(app: AppHandle, core: S<'_>, title: String, description: String, privacy: Privacy, track_ids: Vec<ItemId>) -> Result<PlaylistCreated> {
    let svc = core.service();
    let r = svc.playlist_create(&svc.context(), &title, &description, privacy, &track_ids).await?;
    playlists_changed(&app, &core, r.playlist_id.as_deref(), r.outcome);
    Ok(r)
}

#[tauri::command]
#[specta::specta]
pub async fn playlist_edit(app: AppHandle, core: S<'_>, playlist_id: String, edit: PlaylistEdit) -> Result<MutationOutcome> {
    let svc = core.service();
    let r = svc.playlist_edit(&svc.context(), &playlist_id, &edit).await?;
    playlists_changed(&app, &core, Some(&playlist_id), r);
    Ok(r)
}

#[tauri::command]
#[specta::specta]
pub async fn playlist_delete(app: AppHandle, core: S<'_>, playlist_id: String) -> Result<MutationOutcome> {
    let svc = core.service();
    let r = svc.playlist_delete(&svc.context(), &playlist_id).await?;
    playlists_changed(&app, &core, Some(&playlist_id), r);
    Ok(r)
}

#[tauri::command]
#[specta::specta]
pub async fn playlist_move(app: AppHandle, core: S<'_>, playlist_id: String, set_video_id: String, before: Option<String>) -> Result<MutationOutcome> {
    let svc = core.service();
    let r = svc.playlist_move(&svc.context(), &playlist_id, &set_video_id, before.as_deref()).await?;
    playlists_changed(&app, &core, Some(&playlist_id), r);
    Ok(r)
}

// ---------- history ----------

/// Not cached: history changes with every play, and an old copy would offer
/// removal tokens that no longer work (D19).
#[tauri::command]
#[specta::specta]
pub async fn history(core: S<'_>) -> Result<HistoryPage> {
    let svc = core.service();
    svc.history(&svc.context()).await
}

#[tauri::command]
#[specta::specta]
pub async fn history_more(core: S<'_>, cursor: String) -> Result<HistoryPage> {
    let svc = core.service();
    svc.history_more(&svc.context(), &cursor).await
}

#[tauri::command]
#[specta::specta]
pub async fn history_remove(core: S<'_>, row_ids: Vec<String>) -> Result<MutationOutcome> {
    let svc = core.service();
    svc.history_remove(&svc.context(), &row_ids).await
}

// ---------- song / video ----------

#[tauri::command]
#[specta::specta]
pub async fn counterpart(core: S<'_>, track: Track) -> Result<Option<Counterpart>> {
    let svc = core.service();
    svc.counterpart(&svc.context(), &track).await
}

/// Switch a queue entry to its other version. The pair is looked up here,
/// not taken from the UI (D4).
#[tauri::command]
#[specta::specta]
pub async fn switch_version(core: S<'_>, uid: String) -> Result<()> {
    let entry = core.player.queue_snapshot().entries.into_iter().find(|e| e.uid == uid).ok_or_else(|| AppError::new(encore_core::ErrorCode::Stale, "That song is no longer in the queue."))?;
    let svc = core.service();
    let pair = svc.counterpart(&svc.context(), &entry.track).await?.ok_or_else(|| AppError::not_found(no_version(&entry.track)))?;
    core.player.switch_version(&uid, &pair)
}

/// Play the other version of a track that is not playing, keeping the queue.
#[tauri::command]
#[specta::specta]
pub async fn play_version(core: S<'_>, track: Track) -> Result<()> {
    let svc = core.service();
    let pair = svc.counterpart(&svc.context(), &track).await?.ok_or_else(|| AppError::not_found(no_version(&track)))?;
    let mut t = pair.track;
    t.counterpart = Some(track.id.clone());
    core.player.play_now(t)
}

fn no_version(t: &Track) -> &'static str {
    if t.kind == TrackKind::Video { "YouTube Music has no song version of this video." } else { "YouTube Music has no video version of this song." }
}

#[tauri::command]
#[specta::specta]
pub async fn playlist_remove(app: AppHandle, core: S<'_>, playlist_id: String, entries: Vec<Entry>) -> Result<MutationOutcome> {
    let svc = core.service();
    // The row's own ID, not the track's: an album row may list the video
    // while the track plays the song (D10).
    let pairs: Vec<(ItemId, String)> = entries.into_iter().filter_map(|e| Some((e.video_id, e.set_video_id?))).collect();
    if pairs.is_empty() {
        return Err(AppError::invalid("These songs cannot be removed from this playlist."));
    }
    let r = svc.playlist_remove(&svc.context(), &playlist_id, &pairs).await?;
    playlists_changed(&app, &core, Some(&playlist_id), r);
    Ok(r)
}

// ---------- auth ----------

fn after_auth_change(app: &AppHandle, core: &Core) {
    let _ = AuthEvent(core.accounts.state()).emit(app);
}

#[tauri::command]
#[specta::specta]
pub async fn auth_state(core: S<'_>) -> Result<AuthState> {
    Ok(core.accounts.state())
}

#[tauri::command]
#[specta::specta]
pub async fn browser_profiles() -> Result<Vec<BrowserProfile>> {
    Ok(tokio::task::spawn_blocking(encore_auth::import::list_profiles).await.unwrap_or_default())
}

#[tauri::command]
#[specta::specta]
pub async fn import_browser(app: AppHandle, core: S<'_>, profile_id: String) -> Result<AuthState> {
    core.player.stop();
    let st = core.accounts.import_browser(&profile_id, Duration::from_secs(120)).await?;
    after_auth_change(&app, &core);
    Ok(st)
}

#[tauri::command]
#[specta::specta]
pub async fn select_channel(app: AppHandle, core: S<'_>, key: String) -> Result<AuthState> {
    let st = core.accounts.select_channel(&key).await?;
    core.player.stop();
    core.switch_queue();
    after_auth_change(&app, &core);
    Ok(st)
}

#[tauri::command]
#[specta::specta]
pub async fn reload_channels(core: S<'_>) -> Result<Vec<Channel>> {
    core.accounts.load_channels().await
}

#[tauri::command]
#[specta::specta]
pub async fn sign_out(app: AppHandle, core: S<'_>) -> Result<()> {
    core.player.stop();
    let scope = core.account_scope();
    core.accounts.sign_out()?;
    let _ = core.store.purge_scope(&scope);
    core.resolver_forget_all();
    core.switch_queue();
    after_auth_change(&app, &core);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn begin_signin(app: AppHandle) -> Result<()> {
    crate::signin::open(&app)
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_signin(app: AppHandle) -> Result<()> {
    crate::signin::close(&app);
    Ok(())
}

impl Core {
    pub fn resolver_forget_all(&self) {
        self.resolver.clear_cache();
    }
}
