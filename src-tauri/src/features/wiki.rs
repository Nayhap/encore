//! Wikipedia (lane E): artist extracts with attribution, cached (D29).
//!
//! Commands go here and are registered in `lib.rs` under
//! `// features: integrations`; events the same way in `collect_events!`.
//! Tauri knows a command by its function name alone, across every file,
//! so name them for the area (`wiki_artist`).
//!
//! Asked only for artist pages YouTube gives no description. Answers,
//! including "no article", are kept in `wiki_cache`: an article for 30
//! days, a miss for 7. Offline, or when Wikipedia doesn't answer, the
//! cached article is shown however old it is.

use std::sync::{Arc, OnceLock};

use encore_core::error::Result;
use encore_core::local::integrations::WikiExtract;
use encore_core::store::now;
use tauri::{AppHandle, State as S};

use crate::core::Core;

const FOUND_TTL: i64 = 30 * 24 * 3600;
const MISS_TTL: i64 = 7 * 24 * 3600;

/// This area's state, as `core.features.wiki`.
#[derive(Default)]
pub struct State {
    http: OnceLock<reqwest::Client>,
}

/// Once, from `setup`: subscribe to what this area follows.
pub fn install(_app: &AppHandle, _core: &Arc<Core>) {}

/// The Wikipedia summary of an artist, if one matches strictly.
#[tauri::command]
#[specta::specta]
pub async fn wiki_artist(core: S<'_, Arc<Core>>, artist_id: String, name: String) -> Result<Option<WikiExtract>> {
    let cached = core.store.wiki_get(&artist_id)?;
    if let Some((found, at)) = &cached {
        let ttl = if found.is_some() { FOUND_TTL } else { MISS_TTL };
        if now() - at < ttl {
            return Ok(found.clone());
        }
    }
    let stale = || cached.as_ref().and_then(|c| c.0.clone());
    if !core.connectivity.online() {
        return Ok(stale());
    }
    let http = core.features.wiki.http.get_or_init(encore_wiki::client);
    match encore_wiki::lookup(http, &name).await {
        Ok(found) => {
            core.store.wiki_put(&artist_id, encore_wiki::LANG, found.as_ref())?;
            Ok(found)
        }
        Err(encore_wiki::Error::Network) => Ok(stale()),
    }
}
