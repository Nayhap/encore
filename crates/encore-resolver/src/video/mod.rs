//! The video extractor, the part of it Encore needs: one audio URL (D37).
//!
//! Upstream: yt-dlp `yt_dlp/extractor/youtube/_video.py` at the commit in
//! `upstream.toml`, split by what it does: [`plan`] (which clients),
//! [`player`] (the `/player` call), [`playability`] (what the answer
//! means), [`formats`] (the audio formats and their URL), [`player_js`]
//! (the player script and its signature timestamp), [`webpage`] (the
//! signed-out session the watch page starts).

pub mod formats;
pub mod plan;
pub mod playability;
pub mod player;
pub mod player_js;
pub mod webpage;
