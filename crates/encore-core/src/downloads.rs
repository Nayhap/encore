//! The downloads index (D23), in the same SQLite file as the rest (D11).
//!
//! * `downloads`: one row per song on this device, whoever asked for it,
//!   with the metadata needed to list and play it offline.
//! * `download_pins`: songs someone downloaded on their own, per account.
//! * `download_sets` and `download_members`: kept collections per account,
//!   each an ordered snapshot (the same song may appear twice).
//!
//! A song stays while any pin or member points at it; storage is shared,
//! so the reference count is derived, never stored. Removal marks a row
//! `deleting` first and the files go after, so an interrupted removal is
//! finished at the next start. Every attempt to fetch a song has its own
//! number: cancelling bumps it, and a result is only installed (inside a
//! transaction) while its number is still the current one.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{AppError, Result};
use crate::model::{CollectionCard, Lyrics, Track};
use crate::store::{Store, db_err, now};

pub(crate) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS downloads (
    video_id   TEXT PRIMARY KEY,
    track      TEXT NOT NULL,
    state      TEXT NOT NULL,
    itag       INTEGER,
    ext        TEXT,
    codec      TEXT,
    kbps       INTEGER,
    total      INTEGER,
    bytes      INTEGER NOT NULL DEFAULT 0,
    sha256     TEXT,
    loudness   REAL,
    duration   REAL,
    art_key    TEXT,
    art_base   TEXT,
    art_hue    TEXT,
    art_pinned INTEGER NOT NULL DEFAULT 0,
    error      TEXT,
    attempts   INTEGER NOT NULL DEFAULT 0,
    attempt    INTEGER NOT NULL DEFAULT 0,
    version    INTEGER NOT NULL DEFAULT 0,
    added      INTEGER NOT NULL,
    done_at    INTEGER,
    retry_at   INTEGER
);
CREATE TABLE IF NOT EXISTS download_pins (
    account  TEXT NOT NULL,
    video_id TEXT NOT NULL,
    added    INTEGER NOT NULL,
    PRIMARY KEY (account, video_id)
);
CREATE INDEX IF NOT EXISTS download_pins_video ON download_pins(video_id);
CREATE TABLE IF NOT EXISTS download_sets (
    account    TEXT NOT NULL,
    set_id     TEXT NOT NULL,
    card       TEXT NOT NULL,
    channel    TEXT,
    generation INTEGER NOT NULL,
    added      INTEGER NOT NULL,
    synced     INTEGER,
    error      TEXT,
    gone       INTEGER NOT NULL DEFAULT 0,
    paused     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account, set_id)
);
CREATE TABLE IF NOT EXISTS download_members (
    account   TEXT NOT NULL,
    set_id    TEXT NOT NULL,
    position  INTEGER NOT NULL,
    entry_key TEXT NOT NULL,
    video_id  TEXT NOT NULL,
    PRIMARY KEY (account, set_id, position)
);
CREATE INDEX IF NOT EXISTS download_members_video ON download_members(video_id);
CREATE TABLE IF NOT EXISTS download_lyrics (
    video_id TEXT PRIMARY KEY,
    lyrics   TEXT,
    retry_at INTEGER
);
"#;

const COLS: &str = "video_id, track, state, itag, ext, codec, kbps, total, bytes, sha256, loudness, duration, art_key, art_base, art_hue, art_pinned, error, attempts, attempt, version, added, done_at, retry_at";

/// Something still wants this song: a pin or a kept collection.
const REFERENCED: &str = "(EXISTS (SELECT 1 FROM download_pins p WHERE p.video_id = d.video_id) OR EXISTS (SELECT 1 FROM download_members m WHERE m.video_id = d.video_id))";

/// A new attempt number, unique for this device: microseconds since 1970,
/// then counting up.
pub fn next_attempt() -> u64 {
    static N: OnceLock<AtomicU64> = OnceLock::new();
    let n = N.get_or_init(|| AtomicU64::new(SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_micros() as u64).unwrap_or(1)));
    n.fetch_add(1, Ordering::SeqCst) + 1
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowState {
    Queued,
    Active,
    Ready,
    Failed,
    Deleting,
}

impl RowState {
    fn parse(s: &str) -> Self {
        match s {
            "active" => RowState::Active,
            "ready" => RowState::Ready,
            "failed" => RowState::Failed,
            "deleting" => RowState::Deleting,
            _ => RowState::Queued,
        }
    }
}

/// One song in the index.
#[derive(Clone, Debug)]
pub struct DownloadRow {
    pub video_id: String,
    /// What the UI shows offline; `liked` is cleared (it depends on the account).
    pub track: Track,
    pub state: RowState,
    pub itag: Option<u32>,
    /// `m4a` or `webm`.
    pub ext: Option<String>,
    pub codec: Option<String>,
    pub kbps: Option<u32>,
    /// The stream's length, known once resolved.
    pub total: Option<u64>,
    /// The finished file's size.
    pub bytes: u64,
    pub sha256: Option<String>,
    pub loudness: Option<f32>,
    pub duration: Option<f64>,
    pub art_key: Option<String>,
    pub art_base: Option<String>,
    pub art_hue: Option<(u16, u16)>,
    pub art_pinned: bool,
    /// The last failure, a sentence for people.
    pub error: Option<String>,
    pub attempts: u32,
    pub attempt: u64,
    /// Bumps on every change, for the UI's per-item events.
    pub version: u64,
    pub added: i64,
    pub done_at: Option<i64>,
    pub retry_at: Option<i64>,
}

impl DownloadRow {
    /// The file name of the finished download, `<videoId>-<itag>.<ext>`.
    pub fn file_name(&self) -> Option<String> {
        Some(format!("{}-{}.{}", self.video_id, self.itag?, self.ext.as_deref()?))
    }
}

/// The stream a download fetches, recorded when it starts so a later run
/// can tell whether a partial file may be continued.
#[derive(Clone, Debug, Default)]
pub struct Format {
    pub itag: u32,
    pub ext: String,
    pub codec: String,
    pub kbps: Option<u32>,
    pub total: u64,
}

/// A checked, complete download.
#[derive(Clone, Debug)]
pub struct Finished {
    pub bytes: u64,
    pub sha256: String,
    pub codec: String,
    pub duration: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Commit {
    Installed,
    /// Cancelled, removed or superseded meanwhile: the file is not wanted.
    Stale,
}

/// A kept collection.
#[derive(Clone, Debug)]
pub struct KeptRow {
    pub account: String,
    pub set_id: String,
    pub card: CollectionCard,
    /// The channel that keeps it, for "Kept by …".
    pub channel: Option<String>,
    pub generation: u64,
    pub added: i64,
    pub synced: Option<i64>,
    pub error: Option<String>,
    /// The collection is private or deleted on the service now.
    pub gone: bool,
    pub paused: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SetCounts {
    pub songs: u32,
    pub ready: u32,
    pub failed: u32,
    pub bytes: u64,
}

fn conv<E: std::error::Error + Send + Sync + 'static>(i: usize, e: E) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(i, rusqlite::types::Type::Text, Box::new(e))
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<DownloadRow> {
    let track: String = r.get(1)?;
    let track: Track = serde_json::from_str(&track).map_err(|e| conv(1, e))?;
    let hue: Option<String> = r.get(14)?;
    let art_hue = hue.and_then(|h| {
        let (a, b) = h.split_once(',')?;
        Some((a.parse().ok()?, b.parse().ok()?))
    });
    Ok(DownloadRow {
        video_id: r.get(0)?,
        track,
        state: RowState::parse(&r.get::<_, String>(2)?),
        itag: r.get::<_, Option<i64>>(3)?.map(|v| v as u32),
        ext: r.get(4)?,
        codec: r.get(5)?,
        kbps: r.get::<_, Option<i64>>(6)?.map(|v| v as u32),
        total: r.get::<_, Option<i64>>(7)?.map(|v| v as u64),
        bytes: r.get::<_, i64>(8)? as u64,
        sha256: r.get(9)?,
        loudness: r.get::<_, Option<f64>>(10)?.map(|v| v as f32),
        duration: r.get(11)?,
        art_key: r.get(12)?,
        art_base: r.get(13)?,
        art_hue,
        art_pinned: r.get(15)?,
        error: r.get(16)?,
        attempts: r.get::<_, i64>(17)? as u32,
        attempt: r.get::<_, i64>(18)? as u64,
        version: r.get::<_, i64>(19)? as u64,
        added: r.get(20)?,
        done_at: r.get(21)?,
        retry_at: r.get(22)?,
    })
}

fn kept(r: &rusqlite::Row) -> rusqlite::Result<KeptRow> {
    let card: String = r.get(2)?;
    Ok(KeptRow {
        account: r.get(0)?,
        set_id: r.get(1)?,
        card: serde_json::from_str(&card).map_err(|e| conv(2, e))?,
        channel: r.get(3)?,
        generation: r.get::<_, i64>(4)? as u64,
        added: r.get(5)?,
        synced: r.get(6)?,
        error: r.get(7)?,
        gone: r.get(8)?,
        paused: r.get(9)?,
    })
}

const KEPT_COLS: &str = "account, set_id, card, channel, generation, added, synced, error, gone, paused";

fn json<T: serde::Serialize>(v: &T) -> Result<String> {
    serde_json::to_string(v).map_err(|e| AppError::internal(e.to_string()))
}

/// Make sure `track` has a row; a song that was being removed is wanted
/// again. With `retry_failed`, a failed song is queued again too. Returns
/// whether the song is (now) waiting to be fetched.
fn ensure_row(c: &Connection, track: &Track, retry_failed: bool) -> rusqlite::Result<bool> {
    let vid = track.id.remote().to_string();
    let state: Option<String> = c.query_row("SELECT state FROM downloads WHERE video_id = ?1", params![vid], |r| r.get(0)).optional()?;
    let n = now();
    match state.as_deref().map(RowState::parse) {
        None => {
            let mut t = track.clone();
            t.liked = None;
            let t = serde_json::to_string(&t).map_err(|e| conv(1, e))?;
            let art = track.art.as_ref().and_then(|a| crate::art::lookup(&a.key).map(|e| (a.key.clone(), e.base, e.hue)));
            let (key, base, hue) = match art {
                Some((k, b, h)) => (Some(k), Some(b), h.map(|(a, b)| format!("{a},{b}"))),
                None => (None, None, None),
            };
            c.execute(
                "INSERT INTO downloads (video_id, track, state, art_key, art_base, art_hue, attempt, added) VALUES (?1, ?2, 'queued', ?3, ?4, ?5, ?6, ?7)",
                params![vid, t, key, base, hue, next_attempt() as i64, n],
            )?;
            Ok(true)
        }
        Some(RowState::Deleting) => {
            c.execute(
                "UPDATE downloads SET state = 'queued', error = NULL, attempts = 0, retry_at = NULL, attempt = ?2, version = version + 1, added = ?3 WHERE video_id = ?1",
                params![vid, next_attempt() as i64, n],
            )?;
            Ok(true)
        }
        Some(RowState::Failed) if retry_failed => {
            c.execute("UPDATE downloads SET state = 'queued', error = NULL, attempts = 0, retry_at = NULL, version = version + 1 WHERE video_id = ?1", params![vid])?;
            Ok(true)
        }
        Some(s) => Ok(matches!(s, RowState::Queued | RowState::Active)),
    }
}

impl Store {
    // ---------- songs ----------

    /// Every song except those being removed, oldest first.
    pub fn download_rows(&self) -> Result<Vec<DownloadRow>> {
        let c = self.conn.lock();
        let mut st = c.prepare(&format!("SELECT {COLS} FROM downloads WHERE state != 'deleting' ORDER BY added, rowid")).map_err(db_err)?;
        let rows = st.query_map([], row).map_err(db_err)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn download_row(&self, video_id: &str) -> Result<Option<DownloadRow>> {
        self.conn.lock().query_row(&format!("SELECT {COLS} FROM downloads WHERE video_id = ?1"), params![video_id], row).optional().map_err(db_err)
    }

    /// Rows of any state, for the recovery check at startup.
    pub fn download_count(&self) -> Result<usize> {
        self.conn.lock().query_row("SELECT COUNT(*) FROM downloads", [], |r| r.get::<_, i64>(0)).map(|n| n as usize).map_err(db_err)
    }

    /// (ready, waiting, failed, bytes of the ready songs).
    pub fn download_counts(&self) -> Result<(u32, u32, u32, u64)> {
        self.conn
            .lock()
            .query_row(
                "SELECT SUM(state = 'ready'), SUM(state IN ('queued', 'active')), SUM(state = 'failed'), SUM(CASE WHEN state = 'ready' THEN bytes ELSE 0 END) FROM downloads",
                [],
                |r| {
                    let n = |i: usize| r.get::<_, Option<i64>>(i).map(|v| v.unwrap_or(0));
                    Ok((n(0)? as u32, n(1)? as u32, n(2)? as u32, n(3)? as u64))
                },
            )
            .map_err(db_err)
    }

    /// `account` downloads these songs. Returns how many are now waiting.
    pub fn download_request(&self, account: &str, tracks: &[Track]) -> Result<usize> {
        let mut c = self.conn.lock();
        let tx = c.transaction().map_err(db_err)?;
        let mut waiting = 0;
        for t in tracks {
            if ensure_row(&tx, t, true).map_err(db_err)? {
                waiting += 1;
            }
            tx.execute("INSERT OR IGNORE INTO download_pins (account, video_id, added) VALUES (?1, ?2, ?3)", params![account, t.id.remote(), now()]).map_err(db_err)?;
        }
        tx.commit().map_err(db_err)?;
        Ok(waiting)
    }

    /// The next song to fetch: queued, due, and wanted by a pin or by a kept
    /// collection that isn't paused.
    pub fn download_next(&self, at: i64) -> Result<Option<DownloadRow>> {
        let sql = format!(
            "SELECT {COLS} FROM downloads d WHERE d.state = 'queued' AND (d.retry_at IS NULL OR d.retry_at <= ?1)
             AND (EXISTS (SELECT 1 FROM download_pins p WHERE p.video_id = d.video_id)
                  OR EXISTS (SELECT 1 FROM download_members m JOIN download_sets s ON s.account = m.account AND s.set_id = m.set_id WHERE m.video_id = d.video_id AND s.paused = 0))
             ORDER BY d.added, d.rowid LIMIT 1"
        );
        self.conn.lock().query_row(&sql, params![at], row).optional().map_err(db_err)
    }

    /// When the next song that is waiting to retry is due.
    pub fn download_next_retry(&self) -> Result<Option<i64>> {
        let sql = format!("SELECT MIN(d.retry_at) FROM downloads d WHERE d.state = 'queued' AND d.retry_at IS NOT NULL AND {REFERENCED}");
        self.conn.lock().query_row(&sql, [], |r| r.get::<_, Option<i64>>(0)).map_err(db_err)
    }

    /// Start a new attempt at a queued song.
    pub fn download_start(&self, video_id: &str, attempt: u64) -> Result<bool> {
        let n = self
            .conn
            .lock()
            .execute("UPDATE downloads SET state = 'active', attempt = ?2, error = NULL, version = version + 1 WHERE video_id = ?1 AND state = 'queued'", params![video_id, attempt as i64])
            .map_err(db_err)?;
        Ok(n == 1)
    }

    /// Record the stream being fetched (itag, container, length).
    pub fn download_format(&self, video_id: &str, attempt: u64, f: &Format) -> Result<()> {
        self.conn
            .lock()
            .execute(
                "UPDATE downloads SET itag = ?3, ext = ?4, codec = ?5, kbps = ?6, total = ?7 WHERE video_id = ?1 AND attempt = ?2",
                params![video_id, attempt as i64, f.itag as i64, f.ext, f.codec, f.kbps.map(|k| k as i64), f.total as i64],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// Back to the queue (paused, offline, or to retry at `retry_at`).
    pub fn download_requeue(&self, video_id: &str, attempt: u64, retry_at: Option<i64>, count: bool, error: Option<&str>) -> Result<()> {
        self.conn
            .lock()
            .execute(
                "UPDATE downloads SET state = 'queued', retry_at = ?3, attempts = attempts + ?4, error = ?5, version = version + 1 WHERE video_id = ?1 AND attempt = ?2 AND state = 'active'",
                params![video_id, attempt as i64, retry_at, count as i64, error],
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn download_fail(&self, video_id: &str, attempt: u64, error: &str) -> Result<()> {
        self.conn
            .lock()
            .execute(
                "UPDATE downloads SET state = 'failed', error = ?3, attempts = attempts + 1, retry_at = NULL, version = version + 1 WHERE video_id = ?1 AND attempt = ?2 AND state = 'active'",
                params![video_id, attempt as i64, error],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// Install a finished download: only while `attempt` is still the
    /// song's current one and something still wants the song. `install`
    /// puts the file in place; it runs inside the transaction.
    pub fn download_commit(&self, video_id: &str, attempt: u64, done: &Finished, install: impl FnOnce() -> std::io::Result<()>) -> Result<Commit> {
        let mut c = self.conn.lock();
        let tx = c.transaction().map_err(db_err)?;
        let current: Option<(String, i64)> = tx.query_row("SELECT state, attempt FROM downloads WHERE video_id = ?1", params![video_id], |r| Ok((r.get(0)?, r.get(1)?))).optional().map_err(db_err)?;
        let referenced: bool = tx.query_row(&format!("SELECT {REFERENCED} FROM downloads d WHERE d.video_id = ?1"), params![video_id], |r| r.get(0)).optional().map_err(db_err)?.unwrap_or(false);
        if current != Some(("active".to_string(), attempt as i64)) || !referenced {
            return Ok(Commit::Stale);
        }
        install().map_err(|e| AppError::internal(format!("could not keep the download: {}", e.kind())))?;
        tx.execute(
            "UPDATE downloads SET state = 'ready', bytes = ?2, sha256 = ?3, codec = ?4, duration = COALESCE(?5, duration), error = NULL, retry_at = NULL, done_at = ?6, version = version + 1 WHERE video_id = ?1",
            params![video_id, done.bytes as i64, done.sha256, done.codec, done.duration, now()],
        )
        .map_err(db_err)?;
        tx.commit().map_err(db_err)?;
        Ok(Commit::Installed)
    }

    pub fn download_set_loudness(&self, video_id: &str, loudness: Option<f32>) -> Result<()> {
        self.conn.lock().execute("UPDATE downloads SET loudness = ?2 WHERE video_id = ?1", params![video_id, loudness.map(|l| l as f64)]).map_err(db_err)?;
        Ok(())
    }

    /// The cover for `key` is kept (or gone) for every song that uses it,
    /// with its hues when known (they survive a restart offline).
    pub fn download_set_art_pinned(&self, key: &str, pinned: bool, hue: Option<(u16, u16)>) -> Result<()> {
        let hue = hue.map(|(a, b)| format!("{a},{b}"));
        self.conn.lock().execute("UPDATE downloads SET art_pinned = ?2, art_hue = COALESCE(?3, art_hue) WHERE art_key = ?1", params![key, pinned, hue]).map_err(db_err)?;
        Ok(())
    }

    /// Whether a song that isn't being removed still uses this cover.
    pub fn download_art_in_use(&self, key: &str) -> Result<bool> {
        self.conn.lock().query_row("SELECT EXISTS (SELECT 1 FROM downloads WHERE art_key = ?1 AND state != 'deleting')", params![key], |r| r.get(0)).map_err(db_err)
    }

    /// A ready song whose file is missing or damaged.
    pub fn download_mark_damaged(&self, video_id: &str, error: &str) -> Result<()> {
        self.conn
            .lock()
            .execute("UPDATE downloads SET state = 'failed', error = ?2, version = version + 1 WHERE video_id = ?1 AND state = 'ready'", params![video_id, error])
            .map_err(db_err)?;
        Ok(())
    }

    /// Drop everyone's pin on these songs. Returns, for songs a kept
    /// collection still needs, the titles of those collections.
    pub fn download_unpin(&self, video_ids: &[String]) -> Result<Vec<(String, Vec<String>)>> {
        let mut c = self.conn.lock();
        let tx = c.transaction().map_err(db_err)?;
        let mut kept = Vec::new();
        for v in video_ids {
            tx.execute("DELETE FROM download_pins WHERE video_id = ?1", params![v]).map_err(db_err)?;
            let titles = set_titles(&tx, v).map_err(db_err)?;
            if !titles.is_empty() {
                kept.push((v.clone(), titles));
            }
            // A song nothing wants any more stops at once (D23 commit barrier).
            tx.execute(&format!("UPDATE downloads SET attempt = ?2 WHERE video_id = ?1 AND state IN ('queued', 'active') AND NOT {REFERENCED_BY_ID}"), params![v, next_attempt() as i64]).map_err(db_err)?;
        }
        tx.commit().map_err(db_err)?;
        Ok(kept)
    }

    /// Songs nothing wants any more: marked `deleting` (which also cancels
    /// a running attempt) and returned, for their files to be removed.
    pub fn download_release(&self) -> Result<Vec<DownloadRow>> {
        let mut c = self.conn.lock();
        let tx = c.transaction().map_err(db_err)?;
        let rows: Vec<DownloadRow> = {
            let mut st = tx.prepare(&format!("SELECT {COLS} FROM downloads d WHERE d.state != 'deleting' AND NOT {REFERENCED}")).map_err(db_err)?;
            let r = st.query_map([], row).map_err(db_err)?;
            r.filter_map(|r| r.ok()).collect()
        };
        let attempt = next_attempt() as i64;
        for r in &rows {
            tx.execute("UPDATE downloads SET state = 'deleting', attempt = ?2, version = version + 1 WHERE video_id = ?1", params![r.video_id, attempt]).map_err(db_err)?;
        }
        tx.commit().map_err(db_err)?;
        Ok(rows.into_iter().map(|mut r| {
            r.state = RowState::Deleting;
            r.attempt = attempt as u64;
            r
        })
        .collect())
    }

    /// Rows whose removal didn't finish (retried at startup).
    pub fn download_deleting(&self) -> Result<Vec<DownloadRow>> {
        let c = self.conn.lock();
        let mut st = c.prepare(&format!("SELECT {COLS} FROM downloads WHERE state = 'deleting'")).map_err(db_err)?;
        let rows = st.query_map([], row).map_err(db_err)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// The files are gone: forget the row, unless it was wanted again.
    pub fn download_forget(&self, video_id: &str, attempt: u64) -> Result<bool> {
        let mut c = self.conn.lock();
        let tx = c.transaction().map_err(db_err)?;
        let n = tx.execute("DELETE FROM downloads WHERE video_id = ?1 AND state = 'deleting' AND attempt = ?2", params![video_id, attempt as i64]).map_err(db_err)?;
        if n == 1 {
            tx.execute("DELETE FROM download_lyrics WHERE video_id = ?1", params![video_id]).map_err(db_err)?;
        }
        tx.commit().map_err(db_err)?;
        Ok(n == 1)
    }

    /// At startup: attempts of a previous run are over.
    pub fn download_reset_active(&self) -> Result<()> {
        self.conn.lock().execute("UPDATE downloads SET state = 'queued', version = version + 1 WHERE state = 'active'", []).map_err(db_err)?;
        Ok(())
    }

    /// Queue failed songs again, and those waiting for a retry right away.
    pub fn download_retry_failed(&self) -> Result<usize> {
        let c = self.conn.lock();
        let n = c.execute("UPDATE downloads SET state = 'queued', error = NULL, attempts = 0, retry_at = NULL, version = version + 1 WHERE state = 'failed'", []).map_err(db_err)?;
        c.execute("UPDATE downloads SET retry_at = NULL WHERE state = 'queued' AND retry_at IS NOT NULL", []).map_err(db_err)?;
        Ok(n)
    }

    /// Forget every pin and kept collection, and mark every song for removal.
    pub fn download_remove_all(&self) -> Result<Vec<DownloadRow>> {
        {
            let mut c = self.conn.lock();
            let tx = c.transaction().map_err(db_err)?;
            tx.execute_batch("DELETE FROM download_pins; DELETE FROM download_members; DELETE FROM download_sets;").map_err(db_err)?;
            tx.commit().map_err(db_err)?;
        }
        self.download_release()
    }

    /// A song found on disk after the index was lost (`Store::open` set a
    /// damaged database aside): ready again, pinned under `account`.
    pub fn download_recover(&self, account: &str, r: &DownloadRow) -> Result<()> {
        let mut t = r.track.clone();
        t.liked = None;
        let t = json(&t)?;
        let hue = r.art_hue.map(|(a, b)| format!("{a},{b}"));
        let mut c = self.conn.lock();
        let tx = c.transaction().map_err(db_err)?;
        tx.execute(
            "INSERT OR REPLACE INTO downloads (video_id, track, state, itag, ext, codec, kbps, total, bytes, sha256, loudness, duration, art_key, art_base, art_hue, attempt, added, done_at)
             VALUES (?1, ?2, 'ready', ?3, ?4, ?5, ?6, ?7, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15)",
            params![
                r.video_id,
                t,
                r.itag.map(|v| v as i64),
                r.ext,
                r.codec,
                r.kbps.map(|v| v as i64),
                r.bytes as i64,
                r.sha256,
                r.loudness.map(|l| l as f64),
                r.duration,
                r.art_key,
                r.art_base,
                hue,
                next_attempt() as i64,
                r.added,
            ],
        )
        .map_err(db_err)?;
        tx.execute("INSERT OR IGNORE INTO download_pins (account, video_id, added) VALUES (?1, ?2, ?3)", params![account, r.video_id, now()]).map_err(db_err)?;
        tx.commit().map_err(db_err)?;
        Ok(())
    }

    // ---------- kept collections ----------

    /// Keep a collection for `account` (again). Returns the generation a
    /// sync must still find when it applies its result.
    pub fn kept_put(&self, account: &str, card: &CollectionCard, channel: Option<&str>) -> Result<u64> {
        let generation = next_attempt();
        self.conn
            .lock()
            .execute(
                "INSERT INTO download_sets (account, set_id, card, channel, generation, added) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(account, set_id) DO UPDATE SET card = excluded.card, channel = COALESCE(excluded.channel, channel), paused = 0",
                params![account, card.id.as_str(), json(card)?, channel, generation as i64, now()],
            )
            .map_err(db_err)?;
        Ok(self.kept_row(account, card.id.as_str())?.map(|k| k.generation).unwrap_or(generation))
    }

    pub fn kept_rows(&self) -> Result<Vec<KeptRow>> {
        let c = self.conn.lock();
        let mut st = c.prepare(&format!("SELECT {KEPT_COLS} FROM download_sets ORDER BY added")).map_err(db_err)?;
        let rows = st.query_map([], kept).map_err(db_err)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn kept_row(&self, account: &str, set_id: &str) -> Result<Option<KeptRow>> {
        self.conn
            .lock()
            .query_row(&format!("SELECT {KEPT_COLS} FROM download_sets WHERE account = ?1 AND set_id = ?2"), params![account, set_id], kept)
            .optional()
            .map_err(db_err)
    }

    /// Stop keeping a collection. Its songs go unless something else wants
    /// them (`download_release`).
    pub fn kept_remove(&self, account: &str, set_id: &str) -> Result<()> {
        let mut c = self.conn.lock();
        let tx = c.transaction().map_err(db_err)?;
        tx.execute("DELETE FROM download_members WHERE account = ?1 AND set_id = ?2", params![account, set_id]).map_err(db_err)?;
        tx.execute("DELETE FROM download_sets WHERE account = ?1 AND set_id = ?2", params![account, set_id]).map_err(db_err)?;
        tx.commit().map_err(db_err)?;
        Ok(())
    }

    pub fn kept_pause(&self, account: &str, set_id: &str, paused: bool) -> Result<()> {
        self.conn.lock().execute("UPDATE download_sets SET paused = ?3 WHERE account = ?1 AND set_id = ?2", params![account, set_id, paused]).map_err(db_err)?;
        Ok(())
    }

    pub fn kept_counts(&self, account: &str, set_id: &str) -> Result<SetCounts> {
        self.conn
            .lock()
            .query_row(
                "SELECT COUNT(*),
                        SUM(CASE WHEN d.state = 'ready' THEN 1 ELSE 0 END),
                        SUM(CASE WHEN d.state = 'failed' THEN 1 ELSE 0 END),
                        SUM(CASE WHEN d.state = 'ready' THEN d.bytes ELSE 0 END)
                 FROM download_members m LEFT JOIN downloads d ON d.video_id = m.video_id
                 WHERE m.account = ?1 AND m.set_id = ?2",
                params![account, set_id],
                |r| {
                    Ok(SetCounts {
                        songs: r.get::<_, i64>(0)? as u32,
                        ready: r.get::<_, Option<i64>>(1)?.unwrap_or(0) as u32,
                        failed: r.get::<_, Option<i64>>(2)?.unwrap_or(0) as u32,
                        bytes: r.get::<_, Option<i64>>(3)?.unwrap_or(0) as u64,
                    })
                },
            )
            .map_err(db_err)
    }

    /// Apply a sync. With `members`, they replace the saved order (and new
    /// songs are queued); without, the old snapshot stays. Only while the
    /// collection is still kept under `generation`; returns whether it was.
    pub fn kept_apply(&self, account: &str, set_id: &str, generation: u64, members: Option<&[(String, Track)]>, card: Option<&CollectionCard>, error: Option<&str>, gone: bool) -> Result<bool> {
        let mut c = self.conn.lock();
        let tx = c.transaction().map_err(db_err)?;
        let current: Option<i64> = tx.query_row("SELECT generation FROM download_sets WHERE account = ?1 AND set_id = ?2", params![account, set_id], |r| r.get(0)).optional().map_err(db_err)?;
        if current != Some(generation as i64) {
            return Ok(false);
        }
        if let Some(members) = members {
            tx.execute("DELETE FROM download_members WHERE account = ?1 AND set_id = ?2", params![account, set_id]).map_err(db_err)?;
            for (i, (key, track)) in members.iter().enumerate() {
                ensure_row(&tx, track, false).map_err(db_err)?;
                tx.execute(
                    "INSERT INTO download_members (account, set_id, position, entry_key, video_id) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![account, set_id, i as i64, key, track.id.remote()],
                )
                .map_err(db_err)?;
            }
        }
        if let Some(card) = card {
            tx.execute("UPDATE download_sets SET card = ?3 WHERE account = ?1 AND set_id = ?2", params![account, set_id, json(card)?]).map_err(db_err)?;
        }
        tx.execute(
            "UPDATE download_sets SET synced = ?3, error = ?4, gone = ?5 WHERE account = ?1 AND set_id = ?2",
            params![account, set_id, now(), error, gone],
        )
        .map_err(db_err)?;
        tx.commit().map_err(db_err)?;
        Ok(true)
    }

    /// Whether the collection has a saved order yet.
    pub fn kept_has_members(&self, account: &str, set_id: &str) -> Result<bool> {
        self.conn
            .lock()
            .query_row("SELECT EXISTS (SELECT 1 FROM download_members WHERE account = ?1 AND set_id = ?2)", params![account, set_id], |r| r.get(0))
            .map_err(db_err)
    }

    /// The saved order of a kept collection: (entry key, track) per row.
    pub fn kept_snapshot(&self, account: &str, set_id: &str) -> Result<Option<(CollectionCard, Vec<(String, Track)>)>> {
        let Some(k) = self.kept_row(account, set_id)? else { return Ok(None) };
        let c = self.conn.lock();
        let mut st = c
            .prepare("SELECT m.entry_key, d.track FROM download_members m JOIN downloads d ON d.video_id = m.video_id WHERE m.account = ?1 AND m.set_id = ?2 ORDER BY m.position")
            .map_err(db_err)?;
        let rows = st
            .query_map(params![account, set_id], |r| {
                let t: String = r.get(1)?;
                Ok((r.get::<_, String>(0)?, serde_json::from_str::<Track>(&t).map_err(|e| conv(1, e))?))
            })
            .map_err(db_err)?;
        Ok(Some((k.card, rows.filter_map(|r| r.ok()).collect())))
    }

    /// Titles of the kept collections a song belongs to.
    pub fn kept_titles_for(&self, video_id: &str) -> Result<Vec<String>> {
        set_titles(&self.conn.lock(), video_id).map_err(db_err)
    }

    // ---------- lyrics kept with downloads ----------

    /// `Some(Some(l))`: kept lyrics. `Some(None)`: the providers had none.
    /// `None`: nothing kept for this song.
    pub fn pinned_lyrics(&self, video_id: &str) -> Result<Option<Option<Lyrics>>> {
        let v: Option<Option<String>> = self.conn.lock().query_row("SELECT lyrics FROM download_lyrics WHERE video_id = ?1", params![video_id], |r| r.get(0)).optional().map_err(db_err)?;
        Ok(v.map(|l| l.and_then(|s| serde_json::from_str(&s).ok())))
    }

    /// Keep lyrics with a download, or remember there were none and when to
    /// ask again.
    pub fn pin_lyrics(&self, video_id: &str, lyrics: Option<&Lyrics>, retry_at: Option<i64>) -> Result<()> {
        let l = match lyrics {
            Some(l) => Some(json(l)?),
            None => None,
        };
        self.conn
            .lock()
            .execute(
                "INSERT INTO download_lyrics (video_id, lyrics, retry_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(video_id) DO UPDATE SET lyrics = excluded.lyrics, retry_at = excluded.retry_at",
                params![video_id, l, if lyrics.is_some() { None } else { retry_at }],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// Ready songs whose lyrics haven't been looked up, or whose "none"
    /// is due to be checked again.
    pub fn lyrics_due(&self, at: i64, limit: usize) -> Result<Vec<DownloadRow>> {
        let c = self.conn.lock();
        let mut st = c
            .prepare(&format!(
                "SELECT {COLS} FROM downloads d WHERE d.state = 'ready' AND NOT EXISTS (
                    SELECT 1 FROM download_lyrics l WHERE l.video_id = d.video_id AND (l.lyrics IS NOT NULL OR l.retry_at IS NULL OR l.retry_at > ?1)
                 ) ORDER BY d.done_at LIMIT ?2"
            ))
            .map_err(db_err)?;
        let rows = st.query_map(params![at, limit as i64], row).map_err(db_err)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Ready songs whose cover isn't kept yet.
    pub fn art_due(&self, limit: usize) -> Result<Vec<DownloadRow>> {
        let c = self.conn.lock();
        let mut st = c.prepare(&format!("SELECT {COLS} FROM downloads WHERE state = 'ready' AND art_key IS NOT NULL AND art_pinned = 0 ORDER BY done_at LIMIT ?1")).map_err(db_err)?;
        let rows = st.query_map(params![limit as i64], row).map_err(db_err)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
}

/// Same as `REFERENCED`, for a statement on the `downloads` table itself.
const REFERENCED_BY_ID: &str = "(EXISTS (SELECT 1 FROM download_pins p WHERE p.video_id = downloads.video_id) OR EXISTS (SELECT 1 FROM download_members m WHERE m.video_id = downloads.video_id))";

fn set_titles(c: &Connection, video_id: &str) -> rusqlite::Result<Vec<String>> {
    let mut st = c.prepare(
        "SELECT s.card FROM download_sets s WHERE EXISTS (SELECT 1 FROM download_members m WHERE m.account = s.account AND m.set_id = s.set_id AND m.video_id = ?1) ORDER BY s.added",
    )?;
    let rows = st.query_map(params![video_id], |r| r.get::<_, String>(0))?;
    Ok(rows.filter_map(|r| r.ok()).filter_map(|c| serde_json::from_str::<CollectionCard>(&c).ok()).map(|c| c.title).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CollectionKind, ItemId, TrackKind};

    fn t(id: &str) -> Track {
        Track { id: ItemId::ytm(id), title: id.into(), artists: vec![], album: None, duration: Some(100.0), explicit: false, art: None, kind: TrackKind::Song, plays: None, liked: Some(true), playable: true, counterpart: None }
    }

    fn card(id: &str) -> CollectionCard {
        CollectionCard { id: ItemId::ytm(id), kind: CollectionKind::Playlist, title: format!("List {id}"), subtitle: String::new(), artists: vec![], year: None, art: None, play_id: None }
    }

    fn done() -> Finished {
        Finished { bytes: 10, sha256: "00".into(), codec: "aac".into(), duration: None }
    }

    #[test]
    fn a_song_is_stored_once_and_kept_while_anything_wants_it() {
        let s = Store::open_memory().unwrap();
        s.download_request("a", &[t("x")]).unwrap();
        s.download_request("b", &[t("x")]).unwrap();
        let g = s.kept_put("a", &card("PL1"), Some("Channel A")).unwrap();
        assert!(s.kept_apply("a", "ytm:PL1", g, Some(&[("e1".into(), t("x")), ("e2".into(), t("y")), ("e3".into(), t("x"))]), None, None, false).unwrap());
        assert_eq!(s.download_rows().unwrap().len(), 2, "x once, y once");
        assert_eq!(s.download_rows().unwrap()[0].track.liked, None, "like state isn't shared");
        let snap = s.kept_snapshot("a", "ytm:PL1").unwrap().unwrap().1;
        assert_eq!(snap.iter().map(|(_, t)| t.title.as_str()).collect::<Vec<_>>(), vec!["x", "y", "x"], "duplicates keep their place");

        let kept = s.download_unpin(&["x".into()]).unwrap();
        assert_eq!(kept, vec![("x".to_string(), vec!["List PL1".to_string()])]);
        assert!(s.download_release().unwrap().is_empty(), "the kept playlist still wants x");

        s.kept_remove("a", "ytm:PL1").unwrap();
        let gone: Vec<String> = s.download_release().unwrap().into_iter().map(|r| r.video_id).collect();
        assert_eq!(gone.len(), 2);
        assert!(s.download_rows().unwrap().is_empty());
        assert_eq!(s.download_deleting().unwrap().len(), 2, "removed after their files");
    }

    #[test]
    fn a_cancelled_attempt_is_not_installed() {
        let s = Store::open_memory().unwrap();
        s.download_request("a", &[t("x")]).unwrap();
        let first = next_attempt();
        assert!(s.download_start("x", first).unwrap());
        // Removed while the last chunk arrives.
        s.download_unpin(&["x".into()]).unwrap();
        s.download_release().unwrap();
        let mut installed = false;
        assert_eq!(s.download_commit("x", first, &done(), || Ok(installed = true)).unwrap(), Commit::Stale);
        assert!(!installed, "the file is never put in place");

        // Asked for again: a new attempt installs.
        s.download_request("a", &[t("x")]).unwrap();
        let second = next_attempt();
        assert!(s.download_start("x", second).unwrap());
        assert_eq!(s.download_commit("x", first, &done(), || Ok(())).unwrap(), Commit::Stale, "the old attempt still can't");
        assert_eq!(s.download_commit("x", second, &done(), || Ok(())).unwrap(), Commit::Installed);
        assert_eq!(s.download_row("x").unwrap().unwrap().state, RowState::Ready);
    }

    #[test]
    fn a_stale_sync_changes_nothing() {
        let s = Store::open_memory().unwrap();
        let g = s.kept_put("a", &card("PL1"), None).unwrap();
        s.kept_remove("a", "ytm:PL1").unwrap();
        let g2 = s.kept_put("a", &card("PL1"), None).unwrap();
        assert_ne!(g, g2);
        assert!(!s.kept_apply("a", "ytm:PL1", g, Some(&[("e".into(), t("x"))]), None, None, false).unwrap());
        assert!(s.download_rows().unwrap().is_empty());
    }

    #[test]
    fn paused_collections_wait() {
        let s = Store::open_memory().unwrap();
        let g = s.kept_put("a", &card("PL1"), None).unwrap();
        s.kept_apply("a", "ytm:PL1", g, Some(&[("e".into(), t("x"))]), None, None, false).unwrap();
        s.kept_pause("a", "ytm:PL1", true).unwrap();
        assert!(s.download_next(now()).unwrap().is_none());
        s.kept_pause("a", "ytm:PL1", false).unwrap();
        assert_eq!(s.download_next(now()).unwrap().unwrap().video_id, "x");
    }

    #[test]
    fn lyrics_misses_are_asked_again_later() {
        let s = Store::open_memory().unwrap();
        s.download_request("a", &[t("x")]).unwrap();
        let a = next_attempt();
        s.download_start("x", a).unwrap();
        s.download_commit("x", a, &done(), || Ok(())).unwrap();
        assert_eq!(s.lyrics_due(now(), 10).unwrap().len(), 1);
        s.pin_lyrics("x", None, Some(now() + 3600)).unwrap();
        assert!(s.lyrics_due(now(), 10).unwrap().is_empty());
        assert_eq!(s.pinned_lyrics("x").unwrap(), Some(None));
        assert_eq!(s.lyrics_due(now() + 7200, 10).unwrap().len(), 1);
    }
}
