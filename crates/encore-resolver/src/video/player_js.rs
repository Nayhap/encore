//! The player JavaScript: which one, its signature timestamp, a cache.
//!
//! Upstream: yt-dlp `yt_dlp/extractor/youtube/_video.py` at the commit in
//! `upstream.toml`: `_PLAYER_JS_VARIANT_MAP` (1885-1897),
//! `_construct_player_url` (2104), `_download_player_url` (2157, the
//! iframe API fallback, which is how Encore always finds the player: it
//! fetches no watch page or ytcfg), `_extract_player_info` (2195),
//! `_extract_signature_timestamp` (2228-2257). Encore always uses the
//! `main` variant, as yt-dlp does by default (`_DEFAULT_PLAYER_JS_VARIANT`).

use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use regex::Regex;

use crate::jsc::PlayerScript;

/// `_PLAYER_JS_VARIANT_MAP['main']`.
pub const MAIN_VARIANT: (&str, &str) = ("main", "player_ias.vflset/en_US/base.js");
const IFRAME_API: &str = "https://www.youtube.com/iframe_api";
/// How long a player id from the iframe API is used before asking again.
const PLAYER_ID_TTL: Duration = Duration::from_secs(3600);
/// Player files kept on disk (they are about 2.5 MB each).
const KEPT_ON_DISK: usize = 3;
/// A player larger than this isn't one.
const MAX_PLAYER_BYTES: usize = 12 << 20;

/// The upstream patterns, verbatim.
static IFRAME_PLAYER_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"player\\?/([0-9a-fA-F]{8})\\?/").unwrap());
static STS_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:signatureTimestamp|sts)\s*:\s*(?P<sts>[0-9]{5})").unwrap());

/// `_download_player_url`: the player id in the iframe API script.
pub fn player_id_from_iframe_api(js: &str) -> Option<String> {
    IFRAME_PLAYER_RE.captures(js).map(|c| c[1].to_string())
}

/// `_extract_signature_timestamp` from the player's code.
pub fn signature_timestamp(code: &str) -> Option<u32> {
    STS_RE.captures(code).and_then(|c| c["sts"].parse().ok())
}

/// `_construct_player_url(player_id=...)` for the main variant.
pub fn player_url(id: &str) -> String {
    format!("https://www.youtube.com/s/player/{id}/{}", MAIN_VARIANT.1)
}

/// A player ready to solve with.
#[derive(Clone, Debug)]
pub struct Player {
    pub script: PlayerScript,
    pub sts: Option<u32>,
}

/// Errors finding or fetching the player: `Network` when YouTube didn't
/// answer, `Missing` when it answered with nothing usable (a layout
/// Encore doesn't know: an update problem, not a connection one).
#[derive(Debug, Clone)]
pub enum PlayerJsError {
    Network(String),
    Missing(String),
}

pub struct PlayerJs {
    http: reqwest::Client,
    dir: PathBuf,
    id: Mutex<Option<(String, Instant)>>,
    current: Mutex<Option<Player>>,
}

fn valid_id(id: &str) -> bool {
    (8..=16).contains(&id.len()) && id.bytes().all(|c| c.is_ascii_hexdigit())
}

impl PlayerJs {
    pub fn new(http: reqwest::Client, cache_dir: PathBuf) -> Self {
        PlayerJs { http, dir: cache_dir.join("player"), id: Mutex::new(None), current: Mutex::new(None) }
    }

    /// The id of the player in use, if one was fetched (Settings, CLI).
    pub fn current_id(&self) -> Option<String> {
        self.current.lock().as_ref().map(|p| p.script.id.clone())
    }

    /// Forget the player and its id (Reset player cache, or after a solve
    /// that failed in a way a newer player may fix).
    pub fn forget(&self, files_too: bool) {
        *self.id.lock() = None;
        *self.current.lock() = None;
        if files_too
            && let Ok(rd) = std::fs::read_dir(&self.dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    // Only our own files: `<hex id>-main.js`.
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.ends_with("-main.js") && e.file_type().is_ok_and(|t| t.is_file()) {
                        let _ = std::fs::remove_file(p);
                    }
                }
            }
    }

    async fn get_text(&self, url: &str) -> Result<String, PlayerJsError> {
        let r = self.http.get(url).header("User-Agent", crate::base::BROWSER_UA).send().await.map_err(|e| PlayerJsError::Network(if e.is_timeout() { "timed out".into() } else { "unreachable".into() }))?;
        let st = r.status().as_u16();
        if st >= 500 || st == 429 {
            return Err(PlayerJsError::Network(format!("answered {st}")));
        }
        if st != 200 {
            return Err(PlayerJsError::Missing(format!("answered {st}")));
        }
        let body = r.bytes().await.map_err(|_| PlayerJsError::Network("interrupted".into()))?;
        if body.len() > MAX_PLAYER_BYTES {
            return Err(PlayerJsError::Missing("too large".into()));
        }
        String::from_utf8(body.to_vec()).map_err(|_| PlayerJsError::Missing("not text".into()))
    }

    async fn player_id(&self) -> Result<String, PlayerJsError> {
        if let Some((id, at)) = self.id.lock().clone()
            && at.elapsed() < PLAYER_ID_TTL {
                return Ok(id);
            }
        let js = self.get_text(IFRAME_API).await?;
        let id = player_id_from_iframe_api(&js).filter(|i| valid_id(i)).ok_or_else(|| PlayerJsError::Missing("no player id in the iframe API".into()))?;
        *self.id.lock() = Some((id.clone(), Instant::now()));
        Ok(id)
    }

    /// The current player, fetched or read from the disk cache.
    pub async fn get(&self) -> Result<Player, PlayerJsError> {
        let id = self.player_id().await?;
        let key = format!("{id}/{}", MAIN_VARIANT.0);
        if let Some(p) = self.current.lock().clone().filter(|p| p.script.id == key) {
            return Ok(p);
        }
        let file = self.dir.join(format!("{id}-{}.js", MAIN_VARIANT.0));
        let code = match std::fs::read_to_string(&file) {
            Ok(c) if signature_timestamp(&c).is_some() => c,
            _ => {
                let c = self.get_text(&player_url(&id)).await?;
                if signature_timestamp(&c).is_none() {
                    return Err(PlayerJsError::Missing("the player has no signature timestamp".into()));
                }
                self.store(&file, &c);
                c
            }
        };
        let p = Player { sts: signature_timestamp(&code), script: PlayerScript { id: key, code: Arc::from(code) } };
        *self.current.lock() = Some(p.clone());
        Ok(p)
    }

    /// Write a player by temp file and rename, then keep only the newest.
    fn store(&self, file: &std::path::Path, code: &str) {
        if std::fs::create_dir_all(&self.dir).is_err() {
            return;
        }
        let tmp = file.with_extension("part");
        if std::fs::write(&tmp, code).is_ok() && std::fs::rename(&tmp, file).is_ok() {
            let mut files: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&self.dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().ends_with("-main.js"))
                .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
                .collect();
            files.sort();
            while files.len() > KEPT_ON_DISK {
                let _ = std::fs::remove_file(files.remove(0).1);
            }
        } else {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_player_id_comes_from_the_iframe_api() {
        let js = r#"var scriptUrl = 'https:\/\/www.youtube.com\/s\/player\/9fcf08e8\/www-widgetapi.vflset\/www-widgetapi.js';"#;
        assert_eq!(player_id_from_iframe_api(js).as_deref(), Some("9fcf08e8"));
        assert_eq!(player_id_from_iframe_api("https://www.youtube.com/s/player/0123abcd/x.js").as_deref(), Some("0123abcd"));
        assert_eq!(player_id_from_iframe_api("player/xyz"), None);
        assert_eq!(player_url("9fcf08e8"), "https://www.youtube.com/s/player/9fcf08e8/player_ias.vflset/en_US/base.js");
    }

    #[test]
    fn the_signature_timestamp_is_read_from_the_player() {
        assert_eq!(signature_timestamp("a={signatureTimestamp:20525,b:1}"), Some(20525));
        assert_eq!(signature_timestamp("x.sts : 19999;"), Some(19999));
        assert_eq!(signature_timestamp("sts:123456"), Some(12345));
        assert_eq!(signature_timestamp("nothing"), None);
    }
}
