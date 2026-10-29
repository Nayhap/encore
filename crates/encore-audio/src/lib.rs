//! Encore audio engine: cpal output, symphonia decoding (AAC/MP4, Opus/WebM
//! via libopus), rtrb PCM rings, rubato resampling, an HTTP range source with a
//! disk LRU cache, and near-gapless preloading.

pub mod cache;
pub mod decode;
pub mod engine;
pub mod source;

pub use cache::AudioCache;
pub use engine::{Engine, EngineEvent, EngineStatus};
pub use source::{HttpSpec, Refresher, SourceSpec};
