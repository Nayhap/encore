//! Song details (lane E): the details sheet and tempo and key estimated from a file (D28).
//!
//! Commands go here and are registered in `lib.rs` under
//! `// features: integrations`; events the same way in `collect_events!`.
//! Tauri knows a command by its function name alone, across every file,
//! so name them for the area (`details_track, details_analyse`).
//!
//! The estimate reads only a file Encore already has: the song's download,
//! or its complete copy in the audio cache. Nothing is fetched for it and
//! playback is never tapped. One estimate runs at a time; a new one, or
//! `details_cancel`, stops the last.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use encore_core::error::{AppError, Result};
use encore_core::local::Owner;
use encore_core::local::integrations::AudioAnalysis;
use encore_core::model::Track;
use encore_core::service::LocalAudio;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State as S};

use crate::core::Core;

/// This area's state, as `core.features.details`.
#[derive(Default)]
pub struct State {
    /// The running estimate's stop flag.
    cancel: parking_lot::Mutex<Arc<AtomicBool>>,
}

/// Once, from `setup`: subscribe to what this area follows.
pub fn install(_app: &AppHandle, _core: &Arc<Core>) {}

/// The audio file Encore has of a song.
#[derive(Serialize, Deserialize, Clone, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SongFile {
    /// Downloaded (kept), rather than in the cache (may be evicted).
    pub downloaded: bool,
    /// "AAC", "Opus".
    pub format: String,
    pub kbps: Option<u32>,
    pub bytes: f64,
}

#[derive(Serialize, Deserialize, Clone, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SongDetails {
    pub file: Option<SongFile>,
    /// Counted plays in Encore's play log, for this account (D30).
    pub plays: u32,
    /// From Last.fm's counts, when connected and the song matched (D31).
    pub lastfm_plays: Option<u32>,
    /// The last estimate, if any.
    pub analysis: Option<AudioAnalysis>,
    /// A file is here that hasn't been estimated (or a newer one is).
    pub can_analyse: bool,
}

struct Found {
    path: PathBuf,
    source: String,
    file: SongFile,
}

/// YouTube's audio formats: codec and nominal bitrate.
fn itag_format(itag: u32) -> (&'static str, Option<u32>) {
    match itag {
        139 => ("AAC", Some(48)),
        140 => ("AAC", Some(128)),
        141 => ("AAC", Some(256)),
        249 => ("Opus", Some(50)),
        250 => ("Opus", Some(70)),
        251 => ("Opus", Some(160)),
        774 => ("Opus", Some(256)),
        _ => ("Audio", None),
    }
}

fn format_name(codec: &str, itag: u32) -> String {
    match codec.to_ascii_lowercase().as_str() {
        "aac" | "mp4a" | "m4a" => "AAC".into(),
        "opus" | "webm" => "Opus".into(),
        _ => itag_format(itag).0.into(),
    }
}

/// The song's download, else its complete copy in the audio cache.
fn find(core: &Core, video_id: &str) -> Option<Found> {
    if let Some(f) = core.downloads.as_ref().and_then(|d| d.lookup(video_id)) {
        let file = SongFile { downloaded: true, format: format_name(&f.codec, f.itag), kbps: f.kbps.or(itag_format(f.itag).1), bytes: f.bytes as f64 };
        return Some(Found { source: format!("{}:{}", f.itag, f.bytes), path: f.path, file });
    }
    let (path, itag) = core.audio_cache.find_video(video_id)?;
    let bytes = std::fs::metadata(&path).ok()?.len();
    let (format, kbps) = itag_format(itag);
    Some(Found { source: format!("{itag}:{bytes}"), path, file: SongFile { downloaded: false, format: format.into(), kbps, bytes: bytes as f64 } })
}

/// What the details sheet shows beyond the track itself (the album's year
/// comes from the album, through `collection`).
#[tauri::command]
#[specta::specta]
pub async fn details_track(core: S<'_, Arc<Core>>, track: Track) -> Result<SongDetails> {
    let id = track.id.remote().to_string();
    let owner = Owner::of(&core.service().context());
    let found = find(&core, &id);
    let stored = core.store.analysis_get(&id)?.filter(|s| s.version == encore_analysis::VERSION);
    let can_analyse = found.as_ref().is_some_and(|f| stored.as_ref().is_none_or(|s| s.source != f.source));
    Ok(SongDetails {
        plays: core.store.plays_of(&owner, &id)?,
        lastfm_plays: super::lastfm::plays_for(&core, &track),
        analysis: stored.map(|s| s.analysis),
        file: found.map(|f| f.file),
        can_analyse,
    })
}

/// Estimate the song's tempo and key from its file (D28).
#[tauri::command]
#[specta::specta]
pub async fn details_analyse(core: S<'_, Arc<Core>>, track_id: String) -> Result<AudioAnalysis> {
    let id = encore_core::model::ItemId(track_id).remote().to_string();
    let Some(found) = find(&core, &id) else {
        return Err(AppError::not_found("Encore has no file of this song yet. Play it through, or download it, then try again."));
    };
    let flag = Arc::new(AtomicBool::new(false));
    std::mem::replace(&mut *core.features.details.cancel.lock(), flag.clone()).store(true, Ordering::SeqCst);
    let path = found.path.clone();
    let f2 = flag.clone();
    let r = tokio::task::spawn_blocking(move || encore_analysis::analyse_file(&path, &f2)).await.map_err(|_| AppError::internal("the estimate stopped"))?;
    match r {
        Ok(e) => {
            let a = AudioAnalysis { bpm: e.bpm, key: e.key, confidence: Some(e.tempo_confidence.min(e.key_confidence)) };
            core.store.analysis_put(&id, &found.source, encore_analysis::VERSION, &a)?;
            tracing::info!(bpm = ?a.bpm, key = ?a.key, "estimated tempo and key");
            Ok(a)
        }
        Err(encore_analysis::Error::Cancelled) => Err(AppError::stale()),
        Err(encore_analysis::Error::TooShort) => {
            let a = AudioAnalysis { bpm: None, key: None, confidence: Some(0.0) };
            core.store.analysis_put(&id, &found.source, encore_analysis::VERSION, &a)?;
            Ok(a)
        }
        Err(e) => Err(AppError::internal(format!("Encore {e}."))),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn details_cancel(core: S<'_, Arc<Core>>) -> Result<()> {
    core.features.details.cancel.lock().store(true, Ordering::SeqCst);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_by_itag_or_codec() {
        assert_eq!(itag_format(140), ("AAC", Some(128)));
        assert_eq!(itag_format(251), ("Opus", Some(160)));
        assert_eq!(format_name("opus", 0), "Opus");
        assert_eq!(format_name("mystery", 141), "AAC");
    }
}
