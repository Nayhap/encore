//! Saved things (lane C): bookmarks (D25), saved queues (D33), folders and their local covers (D34).
//!
//! Everything here is local and works offline and signed out. The owner is
//! taken once, when a command starts (`owner`), and every store call of
//! that command uses it, even after a file sheet was open for a minute.

use std::sync::Arc;

use encore_core::error::{AppError, Result};
use encore_core::local::Owner;
use encore_core::local::saved::{Bookmark, BookmarkTarget, FOLDER_FILE_MAX, Folder, FolderFile, SavedQueueInfo};
use encore_core::model::{ArtRef, CollectionCard, ItemId};
use encore_core::queue::Queue;
use tauri::{AppHandle, State as TState};
use tauri_plugin_dialog::DialogExt;

use crate::core::Core;

type S<'a> = TState<'a, Arc<Core>>;

/// This area's state, as `core.features.saved`.
#[derive(Default)]
pub struct State {}

/// Once, from `setup`: subscribe to what this area follows.
pub fn install(_app: &AppHandle, _core: &Arc<Core>) {}

fn owner(core: &Core) -> Owner {
    Owner::of(&core.service().context())
}

// ---------- bookmarks (D25) ----------

/// This account's bookmarks, newest first.
#[tauri::command]
#[specta::specta]
pub async fn bookmarks_list(core: S<'_>) -> Result<Vec<Bookmark>> {
    core.store.bookmarks(&owner(&core))
}

/// Bookmark a page or a song; with `position`, a moment in the song. The
/// same thing twice gives back the bookmark already there.
#[tauri::command]
#[specta::specta]
pub async fn bookmarks_add(core: S<'_>, target: BookmarkTarget, position: Option<f64>, title: String, subtitle: String, art: Option<ArtRef>) -> Result<Bookmark> {
    core.store.bookmark_add(&owner(&core), target, position, &title, &subtitle, art)
}

#[tauri::command]
#[specta::specta]
pub async fn bookmarks_remove(core: S<'_>, id: String) -> Result<()> {
    core.store.bookmark_remove(&owner(&core), &id).map(|_| ())
}

// ---------- saved queues (D33) ----------

#[tauri::command]
#[specta::specta]
pub async fn saved_queues_list(core: S<'_>) -> Result<Vec<SavedQueueInfo>> {
    core.store.saved_queues(&owner(&core))
}

/// Keep the queue as it is now (order, uids, the song and the position in
/// it) under `name`; the same name again replaces that copy.
#[tauri::command]
#[specta::specta]
pub async fn saved_queue_save(core: S<'_>, name: String) -> Result<SavedQueueInfo> {
    let owner = owner(&core);
    let (queue, position) = core.player.queue_and_position();
    core.store.saved_queue_save(&owner, &name, &queue, position)
}

/// Put a saved queue back in place of the current one, paused at its song
/// and position, with its order, uids, shuffle and repeat. Autoplay stays
/// as the user has it now: it's a setting, not part of a queue.
#[tauri::command]
#[specta::specta]
pub async fn saved_queue_restore(core: S<'_>, id: String) -> Result<()> {
    let Some((saved, position)) = core.store.saved_queue_load(&owner(&core), &id)? else {
        return Err(AppError::not_found("That saved queue is gone."));
    };
    let live = core.player.queue_clone();
    let queue = continue_from(saved, &live)?;
    core.player.restore_paused(queue, position)
}

/// The saved queue, numbered after the live one so the UI's splices keep
/// going forward (D13), with the live Autoplay setting.
fn continue_from(saved: Queue, live: &Queue) -> Result<Queue> {
    let bad = |_| AppError::internal("This saved queue can't be read by this version of Encore.");
    let mut v = serde_json::to_value(&saved).map_err(bad)?;
    if let Some(o) = v.as_object_mut() {
        o.insert("revision".into(), (live.revision() + 1).into());
        o.insert("autoplay".into(), live.autoplay.into());
    }
    serde_json::from_value(v).map_err(bad)
}

#[tauri::command]
#[specta::specta]
pub async fn saved_queue_remove(core: S<'_>, id: String) -> Result<()> {
    core.store.saved_queue_remove(&owner(&core), &id).map(|_| ())
}

// ---------- folders (D34) ----------

#[tauri::command]
#[specta::specta]
pub async fn folders_list(core: S<'_>) -> Result<Vec<Folder>> {
    core.store.folders(&owner(&core))
}

/// A new folder, holding `items` to begin with (may be none).
#[tauri::command]
#[specta::specta]
pub async fn folders_create(core: S<'_>, name: String, items: Vec<CollectionCard>) -> Result<Folder> {
    core.store.folder_create(&owner(&core), &name, &items)
}

#[tauri::command]
#[specta::specta]
pub async fn folders_rename(core: S<'_>, id: String, name: String) -> Result<()> {
    core.store.folder_rename(&owner(&core), &id, &name)
}

/// Delete a folder and its cover; the playlists and albums in it stay.
#[tauri::command]
#[specta::specta]
pub async fn folders_remove(core: S<'_>, id: String) -> Result<()> {
    if let Some(key) = core.store.folder_remove(&owner(&core), &id)? {
        core.art.remove_local(&key);
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn folders_set_collapsed(core: S<'_>, id: String, collapsed: bool) -> Result<()> {
    core.store.folder_set_collapsed(&owner(&core), &id, collapsed)
}

/// File a playlist or album in a folder (out of any other one).
#[tauri::command]
#[specta::specta]
pub async fn folders_add(core: S<'_>, id: String, card: CollectionCard) -> Result<()> {
    core.store.folder_add(&owner(&core), &id, &card)
}

#[tauri::command]
#[specta::specta]
pub async fn folders_remove_item(core: S<'_>, id: String, item: ItemId) -> Result<()> {
    core.store.folder_remove_item(&owner(&core), &id, &item)
}

#[tauri::command]
#[specta::specta]
pub async fn folders_move_item(core: S<'_>, id: String, item: ItemId, before: Option<ItemId>) -> Result<()> {
    core.store.folder_move_item(&owner(&core), &id, &item, before.as_ref())
}

/// Pick a file in the system's open sheet; `None` when cancelled. The sheet
/// runs in Rust, so the webview never names a path.
async fn pick(app: &AppHandle, title: &str, filter: (&str, &[&str])) -> Option<std::path::PathBuf> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().set_title(title).add_filter(filter.0, filter.1).pick_file(move |p| {
        let _ = tx.send(p);
    });
    rx.await.ok().flatten().and_then(|p| p.into_path().ok())
}

/// Ask for a picture and make it the folder's cover (a local art key).
/// `false` when the user cancelled.
#[tauri::command]
#[specta::specta]
pub async fn folders_choose_cover(app: AppHandle, core: S<'_>, id: String) -> Result<bool> {
    let owner = owner(&core);
    if core.store.folder(&owner, &id)?.is_none() {
        return Err(AppError::not_found("That folder is gone."));
    }
    let Some(path) = pick(&app, "Choose a cover", ("Pictures", &["jpg", "jpeg", "png", "webp"])).await else { return Ok(false) };
    let art = core.art.clone();
    let key = tokio::task::spawn_blocking(move || art.import_local(&path)).await.map_err(|_| AppError::internal("Encore couldn't read that picture."))?.map_err(AppError::invalid)?;
    match core.store.folder_set_cover(&owner, &id, Some(&key)) {
        Ok(old) => {
            if let Some(old) = old {
                core.art.remove_local(&old);
            }
            Ok(true)
        }
        Err(e) => {
            // Deleted while the sheet was open.
            core.art.remove_local(&key);
            Err(e)
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn folders_clear_cover(core: S<'_>, id: String) -> Result<()> {
    if let Some(old) = core.store.folder_set_cover(&owner(&core), &id, None)? {
        core.art.remove_local(&old);
    }
    Ok(())
}

/// A file name from a folder name, safe on every system.
fn file_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_control() || r#"/\:*?"<>|"#.contains(c) { ' ' } else { c }).collect();
    let s = s.trim().trim_start_matches('.').trim();
    format!("{}.encorefolder", if s.is_empty() { "Folder" } else { s })
}

/// Save a folder's links as a `.encorefolder` file where the user says.
/// `false` when the user cancelled.
#[tauri::command]
#[specta::specta]
pub async fn folders_export(app: AppHandle, core: S<'_>, id: String) -> Result<bool> {
    let Some(folder) = core.store.folder(&owner(&core), &id)? else {
        return Err(AppError::not_found("That folder is gone."));
    };
    let bytes = serde_json::to_vec_pretty(&FolderFile::of(&folder)).map_err(|e| AppError::internal(e.to_string()))?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().set_title("Export folder").set_file_name(file_name(&folder.name)).add_filter("Encore folder", &["encorefolder"]).save_file(move |p| {
        let _ = tx.send(p);
    });
    let Some(path) = rx.await.ok().flatten().and_then(|p| p.into_path().ok()) else { return Ok(false) };
    tokio::fs::write(&path, bytes).await.map_err(|e| AppError::internal(format!("Encore couldn't save the file ({}).", e.kind())))?;
    Ok(true)
}

/// Open a `.encorefolder` file and make a new folder from its links.
/// `None` when the user cancelled.
#[tauri::command]
#[specta::specta]
pub async fn folders_import(app: AppHandle, core: S<'_>) -> Result<Option<Folder>> {
    let owner = owner(&core);
    let Some(path) = pick(&app, "Import folder", ("Encore folder", &["encorefolder", "json"])).await else { return Ok(None) };
    let len = tokio::fs::metadata(&path).await.map_err(|_| AppError::invalid("Encore couldn't open that file."))?.len();
    if len as usize > FOLDER_FILE_MAX {
        return Err(AppError::invalid("This file is too large to be an Encore folder."));
    }
    let bytes = tokio::fs::read(&path).await.map_err(|_| AppError::invalid("Encore couldn't open that file."))?;
    let (name, cards) = FolderFile::parse(&bytes)?;
    if cards.is_empty() {
        return Err(AppError::invalid("This folder has no YouTube Music playlists or albums in it."));
    }
    // An import never files lists away from folders the user made.
    let mine: std::collections::HashSet<String> = core.store.folders(&owner)?.into_iter().flat_map(|f| f.items.into_iter().map(|i| i.item_id.0)).collect();
    let cards: Vec<CollectionCard> = cards.into_iter().filter(|c| !mine.contains(&c.id.0)).collect();
    if cards.is_empty() {
        return Err(AppError::invalid("Everything in this folder is already in your folders."));
    }
    core.store.folder_create(&owner, &name, &cards).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_safe() {
        assert_eq!(file_name("Road trip"), "Road trip.encorefolder");
        assert_eq!(file_name("a/b:c"), "a b c.encorefolder");
        assert_eq!(file_name("..."), "Folder.encorefolder");
    }

    #[test]
    fn a_restored_queue_continues_the_numbering() {
        let mut live = Queue::default();
        live.set_autoplay(false);
        for _ in 0..5 {
            live.clear_up_next();
        }
        let restored = continue_from(Queue::default(), &live).unwrap();
        assert_eq!(restored.revision(), live.revision() + 1);
        assert!(!restored.autoplay);
    }
}
