//! Listening stats (lane D): the Stats tab over the play log (D30), and
//! the play counts other views sort by.
//!
//! Stats belong to the owner the plays started under (D2 account key, or
//! `anonymous`), never leave the device and work offline. Private plays
//! count too: a private session is about YouTube and the outside (D36).

use std::collections::HashMap;
use std::sync::Arc;

use encore_core::error::{AppError, Result};
use encore_core::local::Owner;
use encore_core::local::plays::{ListeningStats, StatsOverview, StatsPeriod};
use encore_core::model::ItemId;
use encore_player::plays::PlayEvent;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_specta::Event;

use crate::core::Core;

type S<'a> = tauri::State<'a, Arc<Core>>;

/// This area's state, as `core.features.stats`.
#[derive(Default)]
pub struct State {}

/// `stats`: a play was counted or ended, so open stats can refresh.
#[derive(Serialize, Deserialize, Clone, specta::Type, Event)]
pub struct StatsEvent;

/// Once, from `setup`: close plays a quit left open, and tell the UI when
/// the log grows.
pub fn install(app: &AppHandle, core: &Arc<Core>) {
    if let Err(e) = core.store.plays_close_open(encore_core::store::now() - 5) {
        tracing::warn!(error = %e.message, "couldn't close open plays");
    }
    let mut rx = core.plays.subscribe();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(PlayEvent::Counted { .. } | PlayEvent::Ended { .. }) => {
                    let _ = StatsEvent.emit(&app);
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
    });
}

fn owner(core: &Core) -> Owner {
    Owner::of(&core.service().context())
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f).await.map_err(|_| AppError::internal("stats"))?
}

/// The months with plays, and since when Encore has counted.
#[tauri::command]
#[specta::specta]
pub async fn stats_overview(core: S<'_>) -> Result<StatsOverview> {
    let (store, owner) = (core.store.clone(), owner(&core));
    blocking(move || store.stats_overview(&owner)).await
}

#[tauri::command]
#[specta::specta]
pub async fn stats_period(core: S<'_>, period: StatsPeriod) -> Result<ListeningStats> {
    if period.month.is_some_and(|m| !(1..=12).contains(&m)) {
        return Err(AppError::invalid("No such month."));
    }
    let (store, owner) = (core.store.clone(), owner(&core));
    blocking(move || store.stats(&owner, period)).await
}

/// Counted plays per song (by `ItemId`), for "your plays" sorting.
#[tauri::command]
#[specta::specta]
pub async fn stats_play_counts(core: S<'_>, ids: Vec<ItemId>) -> Result<HashMap<String, u32>> {
    let (store, owner) = (core.store.clone(), owner(&core));
    let remote: Vec<String> = ids.iter().map(|i| i.remote().to_string()).collect();
    let counts = blocking(move || store.play_counts(&owner, &remote)).await?;
    Ok(ids.into_iter().filter_map(|i| Some((i.0.clone(), *counts.get(i.remote())?))).collect())
}

/// Forget the current owner's plays (Settings → Storage).
#[tauri::command]
#[specta::specta]
pub async fn stats_clear(app: AppHandle, core: S<'_>) -> Result<u32> {
    let (store, owner) = (core.store.clone(), owner(&core));
    let n = blocking(move || store.plays_clear(&owner)).await?;
    let _ = StatsEvent.emit(&app);
    Ok(n as u32)
}
