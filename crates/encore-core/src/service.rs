//! The service abstraction (D2) and the stream resolver seam (D3).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::Result;
use crate::model::*;
use crate::redact::Secret;

/// Immutable per-request account context. A request started under one
/// `epoch` whose result arrives after an account switch is dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestContext {
    pub account: String,
    pub epoch: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub signed_in: bool,
    pub library: bool,
    pub mutations: bool,
    pub radio: bool,
    pub lyrics: bool,
}

#[async_trait]
pub trait MusicService: Send + Sync {
    fn service(&self) -> ServiceId;
    fn capabilities(&self) -> Capabilities;
    /// The context new requests should run under.
    fn context(&self) -> RequestContext;

    async fn home(&self, ctx: &RequestContext) -> Result<Page>;
    async fn explore(&self, ctx: &RequestContext) -> Result<Page>;
    async fn browse_page(&self, ctx: &RequestContext, browse_id: &str, params: Option<&str>) -> Result<Page>;
    async fn more_shelves(&self, ctx: &RequestContext, cursor: &str) -> Result<Page>;
    async fn search(&self, ctx: &RequestContext, query: &str, filter: Option<SearchFilter>) -> Result<SearchResults>;
    async fn search_more(&self, ctx: &RequestContext, cursor: &str) -> Result<SearchResults>;
    async fn suggestions(&self, ctx: &RequestContext, query: &str) -> Result<Vec<String>>;
    async fn collection(&self, ctx: &RequestContext, id: &ItemId) -> Result<Collection>;
    async fn collection_more(&self, ctx: &RequestContext, cursor: &str) -> Result<(Vec<Entry>, Option<String>)>;
    async fn artist(&self, ctx: &RequestContext, id: &ItemId) -> Result<Artist>;
    async fn library(&self, ctx: &RequestContext, section: LibrarySection) -> Result<LibraryPage>;
    async fn library_more(&self, ctx: &RequestContext, section: LibrarySection, cursor: &str) -> Result<LibraryPage>;
    async fn track(&self, ctx: &RequestContext, id: &ItemId) -> Result<Track>;
    async fn up_next(&self, ctx: &RequestContext, seed: &RadioSeed, cursor: Option<&str>) -> Result<UpNext>;
    async fn set_like(&self, ctx: &RequestContext, id: &ItemId, liked: bool) -> Result<MutationOutcome>;
    async fn set_saved(&self, ctx: &RequestContext, playlist_id: &str, saved: bool) -> Result<MutationOutcome>;
    async fn set_subscribed(&self, ctx: &RequestContext, channel_id: &str, subscribed: bool) -> Result<MutationOutcome>;
    /// Add tracks. Unless `allow_duplicates`, a song already in the playlist
    /// is refused with `Duplicate` and nothing is added.
    async fn playlist_add(&self, ctx: &RequestContext, playlist_id: &str, tracks: &[ItemId], allow_duplicates: bool) -> Result<MutationOutcome>;
    async fn playlist_remove(&self, ctx: &RequestContext, playlist_id: &str, entries: &[(ItemId, String)]) -> Result<MutationOutcome>;
    /// Service-native lyrics (timed if possible).
    async fn lyrics(&self, ctx: &RequestContext, id: &ItemId) -> Result<Option<Lyrics>>;

    /// Create a playlist, optionally with its first songs, in one request.
    async fn playlist_create(&self, _ctx: &RequestContext, _title: &str, _description: &str, _privacy: Privacy, _tracks: &[ItemId]) -> Result<PlaylistCreated> {
        Err(unsupported())
    }
    async fn playlist_edit(&self, _ctx: &RequestContext, _playlist_id: &str, _edit: &PlaylistEdit) -> Result<MutationOutcome> {
        Err(unsupported())
    }
    async fn playlist_delete(&self, _ctx: &RequestContext, _playlist_id: &str) -> Result<MutationOutcome> {
        Err(unsupported())
    }
    /// Move the entry `set_video_id` before `before` (another entry's
    /// `setVideoId`), or to the end when `None`.
    async fn playlist_move(&self, _ctx: &RequestContext, _playlist_id: &str, _set_video_id: &str, _before: Option<&str>) -> Result<MutationOutcome> {
        Err(unsupported())
    }
    /// The signed-in listening history, grouped as the service groups it.
    async fn history(&self, _ctx: &RequestContext) -> Result<HistoryPage> {
        Err(unsupported())
    }
    async fn history_more(&self, _ctx: &RequestContext, _cursor: &str) -> Result<HistoryPage> {
        Err(unsupported())
    }
    /// Remove rows (by `HistoryRow::row_id`) from the history.
    async fn history_remove(&self, _ctx: &RequestContext, _row_ids: &[String]) -> Result<MutationOutcome> {
        Err(unsupported())
    }
    /// The other version of `track` (song ↔ music video), if the service
    /// pairs one with it.
    async fn counterpart(&self, _ctx: &RequestContext, _track: &Track) -> Result<Option<Counterpart>> {
        Ok(None)
    }
}

fn unsupported() -> crate::error::AppError {
    crate::error::AppError::new(crate::error::ErrorCode::Unsupported, "This service does not support that.")
}

// ---------- play reporting (history) ----------

/// When to send progress for a play: at each `at` second of the play's wall
/// time, then every `every` seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct FlushSchedule {
    pub at: Vec<f64>,
    pub every: f64,
}

impl Default for FlushSchedule {
    fn default() -> Self {
        FlushSchedule { at: vec![10.0, 20.0, 30.0], every: 40.0 }
    }
}

impl FlushSchedule {
    /// The `n`th flush time (0-based) in seconds since the play started.
    pub fn due(&self, n: usize) -> f64 {
        match self.at.get(n) {
            Some(t) => *t,
            None => {
                let last = self.at.last().copied().unwrap_or(0.0);
                last + self.every.max(1.0) * (n + 1 - self.at.len()) as f64
            }
        }
    }
}

/// What a reported play tells the service. Times are seconds; `rt` is wall
/// time since the play started.
#[derive(Clone, Debug, PartialEq)]
pub enum Ping {
    /// The play started (this creates the history entry).
    Playback { cmt: f64, rt: f64 },
    /// Heard stretches since the last ping (`st[i]..et[i]`).
    Watchtime { st: Vec<f64>, et: Vec<f64>, cmt: f64, rt: f64, playing: bool, last: bool },
}

/// One reported play, bound to the account it started under.
#[async_trait]
pub trait PlayReport: Send + Sync {
    fn schedule(&self) -> FlushSchedule;
    /// Send a ping. Dropped (Ok) if the account changed since the play began.
    async fn send(&self, ping: &Ping, itag: Option<u32>) -> Result<()>;
}

/// Starts reports for plays that became audible (D18 in decisions.md).
#[async_trait]
pub trait PlayReporter: Send + Sync {
    /// `None` when this play can't be reported (signed out, no tracking).
    async fn open_report(&self, id: &ItemId) -> Result<Option<std::sync::Arc<dyn PlayReport>>>;
}

// ---------- stream resolution ----------

/// A cookie as it travels between the auth layer and the resolver. The value
/// is secret; `Debug` never prints it.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct CookieRecord {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    /// Unix seconds; `None` = session cookie.
    pub expires: Option<i64>,
}

impl std::fmt::Debug for CookieRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Cookie({} @ {}{} = <redacted>)", self.name, self.domain, self.path)
    }
}

/// Identity a resolve runs under.
#[derive(Clone, Debug, Default)]
pub struct ResolveAuth {
    pub cookies: Vec<CookieRecord>,
    pub auth_user: u32,
    pub page_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ResolvedStream {
    pub video_id: String,
    /// Signed URL. Secret.
    pub url: Secret<String>,
    pub headers: Vec<(String, String)>,
    pub itag: u32,
    pub codec: String,
    pub mime: String,
    pub bitrate_kbps: Option<u32>,
    pub content_length: Option<u64>,
    pub duration: Option<f64>,
    pub loudness_db: Option<f32>,
    /// Unix seconds when the URL expires.
    pub expires_at: Option<i64>,
    /// false = deep ranges failed the probe: play sequentially (D4).
    pub range_safe: bool,
    pub authenticated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum PlayFailureKind {
    Challenged,
    Unavailable,
    SessionExpired,
    Network,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{message}")]
pub struct PlayFailure {
    pub kind: PlayFailureKind,
    pub message: String,
}

impl PlayFailure {
    pub fn new(kind: PlayFailureKind, message: impl Into<String>) -> Self {
        PlayFailure { kind, message: crate::redact::redact(&message.into()) }
    }

    /// Classify a free-text failure from an extractor (D4).
    pub fn classify(msg: &str, authenticated: bool) -> Self {
        let m = msg.to_ascii_lowercase();
        let kind = if m.contains("sign in to confirm") || m.contains("not a bot") || m.contains("captcha") || m.contains("verify") || m.contains("429") {
            PlayFailureKind::Challenged
        } else if (m.contains("sign in") || m.contains("login required") || m.contains("cookies are no longer valid")) && authenticated {
            PlayFailureKind::SessionExpired
        } else if m.contains("unavailable")
            || m.contains("private video")
            || m.contains("not available")
            || m.contains("removed")
            || m.contains("blocked")
            || m.contains("copyright")
            || m.contains("premium")
            || m.contains("age")
            || m.contains("country")
            || m.contains("no audio formats")
            || m.contains("drm")
        {
            PlayFailureKind::Unavailable
        } else if m.contains("timed out") || m.contains("timeout") || m.contains("connection") || m.contains("network") || m.contains("resolve host") || m.contains("temporary failure") {
            PlayFailureKind::Network
        } else {
            PlayFailureKind::Unknown
        };
        PlayFailure::new(kind, msg)
    }
}

#[async_trait]
pub trait StreamResolver: Send + Sync {
    async fn resolve(&self, video_id: &str, auth: Option<&ResolveAuth>) -> std::result::Result<ResolvedStream, PlayFailure>;
    /// Drop any cached URL for this video (expired / refused).
    fn forget(&self, _video_id: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_failures() {
        assert_eq!(PlayFailure::classify("Sign in to confirm you're not a bot", false).kind, PlayFailureKind::Challenged);
        assert_eq!(PlayFailure::classify("Video unavailable. This video is not available in your country", false).kind, PlayFailureKind::Unavailable);
        assert_eq!(PlayFailure::classify("Sign in to view this video. Login required", true).kind, PlayFailureKind::SessionExpired);
        assert_eq!(PlayFailure::classify("Connection timed out", false).kind, PlayFailureKind::Network);
        assert_eq!(PlayFailure::classify("something odd", false).kind, PlayFailureKind::Unknown);
    }

    #[test]
    fn flush_schedule_then_interval() {
        let s = FlushSchedule::default();
        let due: Vec<f64> = (0..5).map(|n| s.due(n)).collect();
        assert_eq!(due, vec![10.0, 20.0, 30.0, 70.0, 110.0]);
        let s = FlushSchedule { at: vec![], every: 30.0 };
        assert_eq!(s.due(0), 30.0);
        assert_eq!(s.due(1), 60.0);
    }

    #[test]
    fn cookie_debug_hides_value() {
        let c = CookieRecord { name: "SAPISID".into(), value: "CANARYxyz".into(), domain: ".youtube.com".into(), path: "/".into(), secure: true, http_only: false, expires: None };
        assert!(!format!("{c:?}").contains("CANARY"));
    }
}
