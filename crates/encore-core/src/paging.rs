//! The background lane for reading long lists (D10, D23): every
//! continuation page of a collection goes through the `PlaybackGate`, one
//! request at a time, after any playback resolve, and spaced from the last
//! background request, whoever made it (downloads, collection checks, year
//! lookups). A list is complete only when the service says it ended
//! (`More::Ended`); a stalled answer, an error or a limit is a partial list,
//! and says so.
//!
//! Two paces: `Interactive` when someone is looking at the list or waiting
//! for it to play (a page a second), `Background` for everything else (as
//! far apart as collection checks).

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::error::Result;
use crate::model::Entry;
use crate::service::{GateGuard, More, MusicService, PlaybackGate, RequestContext};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pace {
    /// Someone is waiting: the open page, or a shuffle about to start.
    Interactive,
    /// Nobody is waiting: the rest of a queue, lookups, checks.
    Background,
}

impl Pace {
    /// The least time since the last background request.
    pub fn gap(self) -> Duration {
        if cfg!(test) {
            return Duration::ZERO;
        }
        match self {
            Pace::Interactive => Duration::from_millis(1000),
            Pace::Background => Duration::from_millis(2500),
        }
    }
}

/// How long a whole read may go on before it stops as partial.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub entries: usize,
    pub pages: usize,
    pub time: Duration,
}

impl Limits {
    /// What `play_collection` enumerates before shuffling (D6).
    pub const SHUFFLE: Limits = Limits { entries: 10_000, pages: 200, time: Duration::from_secs(90) };
    /// The rest of a queue loaded behind the first page.
    pub const QUEUE: Limits = Limits { entries: 20_000, pages: 200, time: Duration::from_secs(1800) };
}

/// Why a read stopped before the end.
#[derive(Clone, Debug, PartialEq)]
pub enum Stop {
    /// The service sent no progress (`More::Stalled`).
    Stalled,
    /// A request failed.
    Failed(crate::error::AppError),
    /// Too many songs, pages or too long.
    Limit,
    /// `go_on` said to stop (offline, another account, the queue changed).
    Cancelled,
}

/// Wait for a turn to ask the service; `None` once `go_on` says to stop.
pub async fn turn(gate: &Arc<PlaybackGate>, pace: Pace, go_on: &(dyn Fn() -> bool + Send + Sync)) -> Option<GateGuard> {
    loop {
        if !go_on() {
            return None;
        }
        if let Some(g) = gate.try_background_after(pace.gap()) {
            return Some(g);
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

/// One continuation page, in turn. `None` when cancelled while waiting.
pub async fn page(service: &dyn MusicService, ctx: &RequestContext, cursor: &str, gate: &Arc<PlaybackGate>, pace: Pace, go_on: &(dyn Fn() -> bool + Send + Sync)) -> Option<Result<(Vec<Entry>, More)>> {
    let _turn = turn(gate, pace, go_on).await?;
    Some(service.collection_more_checked(ctx, cursor).await)
}

/// Read from `cursor` to the end of the list, page by page, handing each
/// page to `each` as it arrives. Returns how it ended: `Ok(())` only after
/// `More::Ended`.
#[allow(clippy::too_many_arguments)]
pub async fn read_rest(
    service: &dyn MusicService,
    ctx: &RequestContext,
    cursor: String,
    gate: &Arc<PlaybackGate>,
    pace: Pace,
    limits: Limits,
    go_on: &(dyn Fn() -> bool + Send + Sync),
    each: &mut (dyn FnMut(Vec<Entry>) + Send),
) -> std::result::Result<(), Stop> {
    let t0 = Instant::now();
    let (mut entries, mut pages) = (0usize, 0usize);
    let mut cursor = cursor;
    loop {
        if entries >= limits.entries || pages >= limits.pages || t0.elapsed() > limits.time {
            return Err(Stop::Limit);
        }
        let Some(r) = page(service, ctx, &cursor, gate, pace, go_on).await else { return Err(Stop::Cancelled) };
        pages += 1;
        let (more, next) = r.map_err(Stop::Failed)?;
        entries += more.len();
        // The answer may land after the caller moved on.
        if !go_on() {
            return Err(Stop::Cancelled);
        }
        each(more);
        match next {
            More::Next(c) => cursor = c,
            More::Ended => return Ok(()),
            More::Stalled => return Err(Stop::Stalled),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn playback_goes_first() {
        let gate = Arc::new(PlaybackGate::default());
        let busy = gate.enter();
        let g = gate.clone();
        let waiting = tokio::spawn(async move { turn(&g, Pace::Interactive, &|| true).await.is_some() });
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(!waiting.is_finished(), "no page while a song resolves");
        drop(busy);
        assert!(waiting.await.unwrap());
    }

    #[tokio::test]
    async fn a_cancelled_wait_ends() {
        let gate = Arc::new(PlaybackGate::default());
        let _busy = gate.enter();
        assert!(turn(&gate, Pace::Background, &|| false).await.is_none());
    }
}
