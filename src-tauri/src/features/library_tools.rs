//! Library tools (lane B): find, sort, select and mass actions, duplicates (D34), the discography and community playlists (D35).
//!
//! Commands go here and are registered in `lib.rs` under
//! `// features: library tools`; events the same way in `collect_events!`.
//! Tauri knows a command by its function name alone, across every file,
//! so name them for the area (`library_years, bulk_add`).
//!
//! Finding, sorting and spotting duplicates happen in the UI over the rows
//! it has. What needs YouTube more than once goes through the background
//! lane (`encore_core::paging`), one request at a time behind playback:
//! release years (one album page each, cached for good), likes and unlikes
//! of a selection (one request per song) and the pages of a discography.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use encore_core::error::{AppError, ErrorCode, Result};
use encore_core::local::Owner;
use encore_core::local::library::match_key;
use encore_core::model::{CollectionCard, ItemId, MutationOutcome, PageLink};
use encore_core::paging::{self, Pace};
use encore_core::service::More;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State as TauriState};
use tauri_specta::Event;

use crate::core::Core;

type S<'a> = TauriState<'a, Arc<Core>>;

/// At most this many albums are looked up for one sort; the rest sort as
/// unknown and the UI says so.
pub const YEARS_LIMIT: usize = 150;
/// A discography stops after this many pages (every link together).
const RELEASE_PAGES: usize = 40;

/// This area's state, as `core.features.library_tools`.
#[derive(Default)]
pub struct State {
    /// The running year lookup; a new one (or a stop) ends the old one.
    years: AtomicU32,
    /// Job numbers for likes, and the ones asked to stop.
    jobs: AtomicU32,
    stopped: Mutex<HashSet<u32>>,
}

/// Once, from `setup`: subscribe to what this area follows.
pub fn install(_app: &AppHandle, _core: &Arc<Core>) {}

// ---------- sorting ----------

/// One row, as far as sorting needs it.
#[derive(Serialize, Deserialize, Clone, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SortRow {
    pub id: ItemId,
    /// The other version of the recording: its plays count too.
    pub counterpart: Option<ItemId>,
    pub title: String,
    pub artist: String,
}

/// What can sort these rows beyond their own fields. Each map only has the
/// songs that have a value; `lastfm` is `None` without Last.fm counts.
#[derive(Serialize, Deserialize, Clone, specta::Type, Default)]
#[serde(rename_all = "camelCase")]
pub struct SortFacts {
    /// Counted plays in Encore (D30), by track id.
    pub plays: HashMap<String, u32>,
    pub lastfm: Option<HashMap<String, u32>>,
    /// Estimated tempo (D28), by track id.
    pub bpm: HashMap<String, f64>,
}

/// Plays, Last.fm counts and tempos for `rows`, from what Encore keeps
/// locally; nothing is fetched. The owner is the account now (D25 rule 7).
#[tauri::command]
#[specta::specta]
pub async fn library_sort_facts(core: S<'_>, rows: Vec<SortRow>) -> Result<SortFacts> {
    let owner = Owner::of(&core.service().context());
    let ids: Vec<&str> = rows.iter().flat_map(|r| std::iter::once(r.id.remote()).chain(r.counterpart.as_ref().map(|c| c.remote()))).collect();
    let played = core.store.counted_plays(&owner, &ids)?;
    let tempos = core.store.tempos(&ids)?;
    let keys: Vec<(String, String)> = rows.iter().map(|r| (match_key(&r.artist), match_key(&r.title))).collect();
    let lastfm = core.store.lastfm_counts(&keys)?;
    let mut f = SortFacts::default();
    for r in &rows {
        let both = || std::iter::once(r.id.remote()).chain(r.counterpart.as_ref().map(|c| c.remote()));
        let n: u32 = both().filter_map(|i| played.get(i)).sum();
        if n > 0 {
            f.plays.insert(r.id.0.clone(), n);
        }
        if let Some(b) = both().find_map(|i| tempos.get(i)) {
            f.bpm.insert(r.id.0.clone(), *b);
        }
    }
    f.lastfm = lastfm.map(|counts| rows.iter().zip(&keys).filter_map(|(r, k)| counts.get(k).map(|n| (r.id.0.clone(), *n))).collect());
    Ok(f)
}

/// A year lookup started: what the cache already knew, and how many albums
/// are being read now (at most `YEARS_LIMIT`; `skipped` more were not).
#[derive(Serialize, Deserialize, Clone, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct YearsStart {
    pub job: u32,
    /// Album id → year; `None`: the album gives no year.
    pub known: HashMap<String, Option<u16>>,
    pub pending: u32,
    pub skipped: u32,
}

/// One album read, or the lookup ending (`finished`, with `stopped` when
/// it ended early: offline, another account, a failure, or asked to).
#[derive(Serialize, Deserialize, Clone, specta::Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct LibraryYearEvent {
    pub job: u32,
    pub album_id: Option<String>,
    pub year: Option<u16>,
    pub done: u32,
    pub total: u32,
    pub finished: bool,
    pub stopped: bool,
}

/// Release years of albums (`ytm:MPRE…`) for sorting by year: cached years
/// at once, the rest one album page at a time in the background (D34).
#[tauri::command]
#[specta::specta]
pub async fn library_years(app: AppHandle, core: S<'_>, album_ids: Vec<ItemId>) -> Result<YearsStart> {
    let mut ids: Vec<ItemId> = Vec::new();
    for id in album_ids {
        if id.remote().starts_with("MPRE") && !ids.contains(&id) {
            ids.push(id);
        }
    }
    let remotes: Vec<&str> = ids.iter().map(ItemId::remote).collect();
    let cached = core.store.album_years(&remotes)?;
    let known: HashMap<String, Option<u16>> = ids.iter().filter_map(|i| cached.get(i.remote()).map(|y| (i.0.clone(), *y))).collect();
    let missing: Vec<ItemId> = ids.iter().filter(|i| !cached.contains_key(i.remote())).cloned().collect();
    let skipped = missing.len().saturating_sub(YEARS_LIMIT) as u32;
    let todo: Vec<ItemId> = missing.into_iter().take(YEARS_LIMIT).collect();
    let job = core.features.library_tools.years.fetch_add(1, Ordering::SeqCst) + 1;
    let start = YearsStart { job, known, pending: todo.len() as u32, skipped };
    if todo.is_empty() || !core.connectivity.online() {
        return Ok(YearsStart { pending: 0, skipped: skipped + todo.len() as u32, ..start });
    }
    let core = core.inner().clone();
    tauri::async_runtime::spawn(async move {
        let svc = core.service();
        let ctx = svc.context();
        let total = todo.len() as u32;
        let go_on = || core.features.library_tools.years.load(Ordering::SeqCst) == job && core.connectivity.online() && svc.context() == ctx;
        let mut done = 0;
        let mut stopped = false;
        for id in &todo {
            let Some(_turn) = paging::turn(&core.player.gate(), Pace::Background, &go_on).await else {
                stopped = true;
                break;
            };
            match svc.collection(&ctx, id).await {
                Ok(c) => {
                    let _ = core.store.set_album_year(id.remote(), c.year);
                    done += 1;
                    let _ = LibraryYearEvent { job, album_id: Some(id.0.clone()), year: c.year, done, total, finished: false, stopped: false }.emit(&app);
                }
                // Gone or unreadable: not a year, but not worth stopping for.
                Err(e) if matches!(e.code, ErrorCode::NotFound | ErrorCode::Parse) => done += 1,
                Err(e) => {
                    core.net::<()>(Err(e)).ok();
                    stopped = true;
                    break;
                }
            }
        }
        let _ = LibraryYearEvent { job, album_id: None, year: None, done, total, finished: true, stopped }.emit(&app);
    });
    Ok(start)
}

/// Stop looking up years (the list was left or sorted otherwise).
#[tauri::command]
#[specta::specta]
pub async fn library_years_stop(core: S<'_>) -> Result<()> {
    core.features.library_tools.years.fetch_add(1, Ordering::SeqCst);
    Ok(())
}

// ---------- likes of a selection ----------

/// One song of a like job answered, or the job ended.
#[derive(Serialize, Deserialize, Clone, specta::Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct LibraryLikeEvent {
    pub job: u32,
    pub track_id: Option<ItemId>,
    pub outcome: Option<MutationOutcome>,
    pub done: u32,
    pub total: u32,
    pub finished: bool,
    /// Why it ended early, for the user.
    pub error: Option<String>,
}

/// Like (or unlike) many songs: one request each, paced through the
/// background lane so YouTube sees a person, not a script (D34). Returns
/// the job; answers arrive as `LibraryLikeEvent`s.
#[tauri::command]
#[specta::specta]
pub async fn library_like(app: AppHandle, core: S<'_>, track_ids: Vec<ItemId>, liked: bool) -> Result<u32> {
    core.require_online("change your likes")?;
    if !core.service().capabilities().mutations {
        return Err(AppError::auth_required());
    }
    let st = &core.features.library_tools;
    let job = st.jobs.fetch_add(1, Ordering::SeqCst) + 1;
    let core = core.inner().clone();
    tauri::async_runtime::spawn(async move {
        let svc = core.service();
        let ctx = svc.context();
        let st = &core.features.library_tools;
        let go_on = || !st.stopped.lock().contains(&job) && core.connectivity.online() && svc.context() == ctx;
        let total = track_ids.len() as u32;
        let (mut done, mut changed, mut error) = (0, false, None);
        for id in &track_ids {
            let Some(turn) = paging::turn(&core.player.gate(), Pace::Background, &go_on).await else {
                if svc.context() != ctx {
                    error = Some("Stopped: the account changed.".to_string());
                } else if !core.connectivity.online() {
                    error = Some("Stopped: you're offline.".to_string());
                }
                break;
            };
            let r = svc.set_like(&ctx, id, liked).await;
            core.wrote();
            drop(turn);
            match r {
                Ok(o) => {
                    done += 1;
                    if o == MutationOutcome::Done {
                        core.player.update_track(id.as_str(), |t| t.liked = Some(liked));
                        changed = true;
                    }
                    let _ = LibraryLikeEvent { job, track_id: Some(id.clone()), outcome: Some(o), done, total, finished: false, error: None }.emit(&app);
                }
                Err(e) => {
                    core.net::<()>(Err(e.clone())).ok();
                    error = Some(e.message);
                    break;
                }
            }
        }
        // Liked songs kept downloaded follow, once (D23).
        if changed && let Some(d) = &core.downloads {
            d.collection_changed(&ItemId::ytm("VLLM"));
        }
        st.stopped.lock().remove(&job);
        let _ = LibraryLikeEvent { job, track_id: None, outcome: None, done, total, finished: true, error }.emit(&app);
    });
    Ok(job)
}

/// Stop a like job; songs already answered stay as they are.
#[tauri::command]
#[specta::specta]
pub async fn library_like_stop(core: S<'_>, job: u32) -> Result<()> {
    core.features.library_tools.stopped.lock().insert(job);
    Ok(())
}

// ---------- discography (D35) ----------

/// Every release behind an artist's release links, newest first.
/// `complete`: every link was read to its end; otherwise YouTube stopped
/// early (or a limit did) and there may be more.
#[derive(Serialize, Deserialize, Clone, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Discography {
    pub releases: Vec<CollectionCard>,
    pub complete: bool,
}

/// Read an artist's releases from the links its page gave (the `MPAD…`
/// "Albums" and "Singles and EPs" shelves' `more`, params and all). The
/// first page of each is read in turn like any page someone opened; the
/// rest through the background lane (D10, D35).
#[tauri::command]
#[specta::specta]
pub async fn library_discography(core: S<'_>, links: Vec<PageLink>) -> Result<Discography> {
    let links: Vec<PageLink> = links.into_iter().filter(|l| l.browse_id.starts_with("MPAD")).collect();
    if links.is_empty() {
        return Err(AppError::invalid("This artist has no releases to list."));
    }
    let key = format!("discography:{}", links.iter().map(|l| format!("{}:{}", l.browse_id, l.params.as_deref().unwrap_or(""))).collect::<Vec<_>>().join("|"));
    let c = core.inner().clone();
    core.cached(&key, || read_releases(c, links), |_: &mut Discography| {}).await
}

async fn read_releases(core: Arc<Core>, links: Vec<PageLink>) -> Result<Discography> {
    let ytm = core.ytm.clone();
    let svc = core.service();
    let ctx = svc.context();
    let same = || svc.context() == ctx;
    let gate = core.player.gate();
    let mut seen = HashSet::new();
    let mut releases = Vec::new();
    let mut complete = true;
    let mut pages = 0;
    let mut add = |cards: Vec<CollectionCard>| {
        for c in cards {
            if seen.insert(c.id.clone()) {
                releases.push(c);
            }
        }
    };
    for link in &links {
        let _turn = paging::turn(&gate, Pace::Interactive, &same).await.ok_or_else(AppError::stale)?;
        let (cards, mut more) = ytm.releases(&ctx, link).await?;
        drop(_turn);
        add(cards);
        pages += 1;
        loop {
            match more {
                More::Ended => break,
                More::Stalled => {
                    complete = false;
                    break;
                }
                More::Next(_) if pages >= RELEASE_PAGES => {
                    complete = false;
                    break;
                }
                More::Next(cursor) => {
                    let Some(_turn) = paging::turn(&gate, Pace::Background, &same).await else { return Err(AppError::stale()) };
                    match ytm.releases_more(&ctx, &cursor).await {
                        Ok((cards, next)) => {
                            add(cards);
                            more = next;
                            pages += 1;
                        }
                        // What arrived stays; the list says it may go on.
                        Err(e) => {
                            core.net::<()>(Err(e)).ok();
                            complete = false;
                            break;
                        }
                    }
                }
            }
        }
    }
    // Newest first; undated releases last, in YouTube's order.
    releases.sort_by_key(|c| std::cmp::Reverse(c.year.unwrap_or(0)));
    Ok(Discography { releases, complete })
}
