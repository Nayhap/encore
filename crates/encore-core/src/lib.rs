//! Encore core: domain types, the `MusicService` trait, the queue, storage,
//! artwork keys and secret redaction. No Tauri, no network.

pub mod art;
pub mod error;
pub mod model;
pub mod queue;
pub mod redact;
pub mod service;
pub mod store;

pub use error::{AppError, ErrorCode, Result};
pub use model::*;
