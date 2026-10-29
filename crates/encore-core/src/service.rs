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
    async fn playlist_add(&self, ctx: &RequestContext, playlist_id: &str, tracks: &[ItemId]) -> Result<MutationOutcome>;
    async fn playlist_remove(&self, ctx: &RequestContext, playlist_id: &str, entries: &[(ItemId, String)]) -> Result<MutationOutcome>;
    /// Service-native lyrics (timed if possible).
    async fn lyrics(&self, ctx: &RequestContext, id: &ItemId) -> Result<Option<Lyrics>>;
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
    fn cookie_debug_hides_value() {
        let c = CookieRecord { name: "SAPISID".into(), value: "CANARYxyz".into(), domain: ".youtube.com".into(), path: "/".into(), secure: true, http_only: false, expires: None };
        assert!(!format!("{c:?}").contains("CANARY"));
    }
}
