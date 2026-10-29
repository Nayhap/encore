//! One error type for every service, sent to the UI as `AppError`.
//! Messages are written for people and never carry raw HTTP bodies, headers,
//! cookies or signed URLs.

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    Network,
    AuthRequired,
    SessionExpired,
    NotFound,
    Parse,
    /// YouTube asked for a bot check / verification.
    Challenged,
    /// The track or page is unavailable (region, removed, private, premium).
    Unavailable,
    /// Superseded by a newer request or an account switch.
    Stale,
    Unsupported,
    OutcomeUnknown,
    Rejected,
    InvalidArgument,
    Keychain,
    Internal,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{message}")]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
    /// A short action hint for the UI ("reimport", "signIn", "retry").
    pub action: Option<String>,
}

pub type Result<T, E = AppError> = std::result::Result<T, E>;

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        let retryable = matches!(code, ErrorCode::Network | ErrorCode::Challenged | ErrorCode::OutcomeUnknown);
        AppError { code, message: crate::redact::redact(&message.into()), retryable, action: None }
    }
    pub fn with_action(mut self, a: &str) -> Self {
        self.action = Some(a.to_string());
        self
    }
    pub fn network(m: impl Into<String>) -> Self {
        Self::new(ErrorCode::Network, m)
    }
    pub fn parse(m: impl Into<String>) -> Self {
        Self::new(ErrorCode::Parse, m)
    }
    pub fn auth_required() -> Self {
        Self::new(ErrorCode::AuthRequired, "Sign in to YouTube Music to use this.").with_action("signIn")
    }
    pub fn session_expired() -> Self {
        Self::new(ErrorCode::SessionExpired, "Your YouTube Music session has expired. Import it again or sign in.").with_action("reimport")
    }
    pub fn not_found(m: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, m)
    }
    pub fn internal(m: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, m)
    }
    pub fn stale() -> Self {
        Self::new(ErrorCode::Stale, "Superseded by a newer request.")
    }
    pub fn invalid(m: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidArgument, m)
    }
}
