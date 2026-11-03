//! Taste (lane D): the block list and "Don't play this again" (D36), "Hide
//! podcasts" (D27), private sessions (D36).
//!
//! The block list is per owner (D2 account key, `anonymous` signed out) and
//! held in memory per owner, so the player's `Admission` and every page
//! filter answer without the database. Blocking a song also tells YouTube
//! "Not interested" when a row of it carried that token this session, once
//! the undo window has passed. A private session lasts an hour, or until
//! Encore quits; the player keeps its clock, and its flag stops history
//! reporting (D18), scrobbles and presence (D31). Local stats still record (D30).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use encore_core::error::Result;
use encore_core::local::Owner;
use encore_core::local::taste::{self as blocks, BlockKind, BlockSet, Blocked};
use encore_core::model::{ItemId, MutationOutcome, Page, Track};
use encore_core::service::{Admission, MusicService};
use encore_core::store::Store;
use encore_ytm::YtMusic;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_specta::Event;

use crate::core::{Core, SETTINGS};

type S<'a> = tauri::State<'a, Arc<Core>>;

const HIDE_PODCASTS: &str = "hide_podcasts";
/// How long "Undo" is offered before YouTube hears "Not interested".
const UNDO_WINDOW: Duration = Duration::from_secs(10);
/// A private session's length.
const PRIVATE_FOR: Duration = Duration::from_secs(3600);

/// This area's state, as `core.features.taste`.
#[derive(Default)]
pub struct State {
    shared: Arc<Shared>,
    /// Per blocked song, bumped by every block or unblock of it: a pending
    /// "Not interested" from before an undo is never sent.
    feedback_gen: Mutex<HashMap<ItemId, u64>>,
}

/// What the player's admission shares with the commands.
#[derive(Default)]
struct Shared {
    sets: RwLock<HashMap<String, Arc<BlockSet>>>,
    hide_podcasts: AtomicBool,
}

impl Shared {
    fn set(&self, store: &Store, owner: &Owner) -> Arc<BlockSet> {
        if let Some(s) = self.sets.read().get(owner.as_str()) {
            return s.clone();
        }
        let s = Arc::new(BlockSet::new(&store.blocked_list(owner).unwrap_or_default()));
        self.sets.write().insert(owner.as_str().to_string(), s.clone());
        s
    }

    fn reload(&self, store: &Store, owner: &Owner) -> Result<Vec<Blocked>> {
        let list = store.blocked_list(owner)?;
        self.sets.write().insert(owner.as_str().to_string(), Arc::new(BlockSet::new(&list)));
        Ok(list)
    }
}

/// The player asks this before a song joins the queue (D36, D27).
struct Gate {
    shared: Arc<Shared>,
    store: Arc<Store>,
    ytm: Arc<YtMusic>,
}

impl Admission for Gate {
    fn admits(&self, t: &Track) -> bool {
        let owner = Owner::of(&self.ytm.context());
        blocks::admits(t, &self.shared.set(&self.store, &owner), self.shared.hide_podcasts.load(Ordering::Relaxed))
    }
}

#[derive(Serialize, Deserialize, Clone, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TasteState {
    /// The current owner's block list, newest first.
    pub blocked: Vec<Blocked>,
    pub hide_podcasts: bool,
}

/// `taste`: the block list or a setting changed.
#[derive(Serialize, Deserialize, Clone, specta::Type, Event)]
pub struct TasteEvent(pub TasteState);

/// Once, from `setup`.
pub fn install(_app: &AppHandle, core: &Arc<Core>) {
    let t = &core.features.taste;
    t.shared.hide_podcasts.store(core.store.get::<bool>(SETTINGS, HIDE_PODCASTS).ok().flatten().unwrap_or(false), Ordering::Relaxed);
    core.player.set_admission(Arc::new(Gate { shared: t.shared.clone(), store: core.store.clone(), ytm: core.ytm.clone() }));
}

fn owner(core: &Core) -> Owner {
    Owner::of(&core.service().context())
}

fn state(core: &Core, list: Option<Vec<Blocked>>) -> Result<TasteState> {
    let t = &core.features.taste;
    let blocked = match list {
        Some(l) => l,
        None => core.store.blocked_list(&owner(core))?,
    };
    Ok(TasteState { blocked, hide_podcasts: t.shared.hide_podcasts.load(Ordering::Relaxed) })
}

fn changed(app: &AppHandle, s: &TasteState) {
    let _ = TasteEvent(s.clone()).emit(app);
}

/// Home, Explore and browse pages pass through here after the cache
/// (D27 "Hide podcasts", D36 blocked songs and artists), so a change of
/// setting shows at once and the cached copy stays whole.
pub fn filter_page(core: &Core, page: Page) -> Page {
    let t = &core.features.taste;
    blocks::filter_page(page, &t.shared.set(&core.store, &owner(core)), t.shared.hide_podcasts.load(Ordering::Relaxed))
}

#[tauri::command]
#[specta::specta]
pub async fn taste_state(core: S<'_>) -> Result<TasteState> {
    state(&core, None)
}

/// Block a song or an artist. A song that is playing (or an artist's)
/// is skipped at once; a song's "Not interested" goes to YouTube after the
/// undo window, if a row of it carried the token.
#[tauri::command]
#[specta::specta]
pub async fn taste_block(app: AppHandle, core: S<'_>, kind: BlockKind, target: ItemId, label: String, detail: Option<String>) -> Result<TasteState> {
    let svc = core.service();
    let ctx = svc.context();
    let owner = Owner::of(&ctx);
    core.store.block(&owner, kind, &target, &label, detail.as_deref())?;
    let list = core.features.taste.shared.reload(&core.store, &owner)?;
    let s = state(&core, Some(list))?;
    changed(&app, &s);

    // Blocked songs leave Up next at once; the one playing moves on (D36).
    core.player.admission_changed().await;
    if kind == BlockKind::Track {
        let my = {
            let mut g = core.features.taste.feedback_gen.lock();
            let n = g.entry(target.clone()).or_default();
            *n += 1;
            *n
        };
        let core = core.inner().clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(UNDO_WINDOW).await;
            if core.features.taste.feedback_gen.lock().get(&target) != Some(&my) || svc.context() != ctx {
                return;
            }
            match svc.not_interested(&ctx, target.remote()).await {
                Ok(Some(MutationOutcome::Done)) => tracing::info!(video = %target, "not interested: sent"),
                Ok(Some(o)) => tracing::info!(video = %target, outcome = ?o, "not interested: not confirmed"),
                Ok(None) => {}
                Err(e) => tracing::warn!(video = %target, error = %e.message, "not interested failed"),
            }
        });
    }
    Ok(s)
}

#[tauri::command]
#[specta::specta]
pub async fn taste_unblock(app: AppHandle, core: S<'_>, kind: BlockKind, target: ItemId) -> Result<TasteState> {
    let owner = owner(&core);
    core.store.unblock(&owner, kind, &target)?;
    if let Some(n) = core.features.taste.feedback_gen.lock().get_mut(&target) {
        *n += 1;
    }
    let list = core.features.taste.shared.reload(&core.store, &owner)?;
    let s = state(&core, Some(list))?;
    changed(&app, &s);
    core.player.admission_changed().await;
    Ok(s)
}

#[tauri::command]
#[specta::specta]
pub async fn taste_set_hide_podcasts(app: AppHandle, core: S<'_>, on: bool) -> Result<TasteState> {
    core.store.put(SETTINGS, HIDE_PODCASTS, &on, None)?;
    core.features.taste.shared.hide_podcasts.store(on, Ordering::Relaxed);
    let s = state(&core, None)?;
    changed(&app, &s);
    Ok(s)
}

/// Start a private session (an hour) or end it. The player keeps the clock
/// and ends it on time (D36); its snapshot carries the time left.
#[tauri::command]
#[specta::specta]
pub async fn taste_private(core: S<'_>, on: bool) -> Result<()> {
    if on {
        core.player.start_private_session(Some(PRIVATE_FOR));
    } else {
        core.player.end_private_session();
    }
    Ok(())
}
