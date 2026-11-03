//! One `/player` call by one client.
//!
//! Upstream: yt-dlp `yt_dlp/extractor/youtube/_video.py` at the commit in
//! `upstream.toml`: `_extract_player_response` (2914-2967),
//! `_generate_player_context` (2692-2712), `_get_checkok_params`; and
//! `_base.py` `_call_api` (POST `https://<host>/youtubei/v1/player`,
//! `prettyPrint=false`, no key) and `_extract_response` (which errors are
//! final). No PO token is sent: Encore has none.

use std::time::Duration;

use serde_json::{Value, json};

use crate::base::{self, Identity, InnertubeClient};

/// The request body (`_extract_player_response` + `_generate_player_context`).
pub fn body(c: &InnertubeClient, video_id: &str, sts: Option<u32>, ytcfg: Option<&Value>) -> Value {
    let mut cpc = json!({ "html5Preference": "HTML5_PREF_WANTS" });
    if let Some(sts) = sts {
        cpc["signatureTimestamp"] = json!(sts);
    }
    // web_embedded may need these; "no harm in including" them (2945).
    if c.is_embedded()
        && let Some(f) = ytcfg.and_then(base::ytcfg::encrypted_host_flags) {
            cpc["encryptedHostFlags"] = json!(f);
        }
    let mut b = json!({
        "context": base::context(c, ytcfg),
        "videoId": video_id,
        "playbackContext": { "contentPlaybackContext": cpc },
        "contentCheckOk": true,
        "racyCheckOk": true,
    });
    if let Some(p) = &c.player_params {
        b["params"] = json!(p);
    }
    b
}

/// How a call failed before there was a player response to read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallError {
    /// No answer, a timeout or a 5xx.
    Network(String),
    /// 401 with the session: it is no longer valid.
    Unauthorized,
    /// 429, or YouTube's "Sorry" page.
    RateLimited,
    /// Any other refusal (a bare 403 is not a bot check).
    Refused(u16),
    /// Something that isn't a player response.
    Unreadable,
}

/// POST `/youtubei/v1/player` as `id` with client `c`. Returns the player
/// response and the HTTP status.
pub async fn call(http: &reqwest::Client, c: &InnertubeClient, id: &Identity, video_id: &str, sts: Option<u32>, ytcfg: Option<&Value>) -> Result<Value, CallError> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    let url = format!("https://{}/youtubei/v1/player?prettyPrint=false", c.innertube_host);
    let mut rb = http.post(url).timeout(Duration::from_secs(15)).json(&body(c, video_id, sts, ytcfg));
    for (k, v) in base::api_headers(c, id, now, ytcfg) {
        rb = rb.header(k, v);
    }
    let r = rb.send().await.map_err(|e| CallError::Network(if e.is_timeout() { "timed out".into() } else { "unreachable".into() }))?;
    let status = r.status().as_u16();
    let text = r.text().await.map_err(|_| CallError::Network("interrupted".into()))?;
    if text.trim_start().starts_with('<') {
        return Err(if status == 429 || text.contains("<title>Sorry") { CallError::RateLimited } else if status >= 500 { CallError::Network(format!("answered {status}")) } else { CallError::Refused(status) });
    }
    match status {
        200 => {}
        401 if id.is_authenticated() && c.supports_cookies => return Err(CallError::Unauthorized),
        429 => return Err(CallError::RateLimited),
        s if s >= 500 => return Err(CallError::Network(format!("answered {s}"))),
        s => return Err(CallError::Refused(s)),
    }
    serde_json::from_str(&text).map_err(|_| CallError::Unreadable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_is_upstreams() {
        let c = crate::base::client("tv_samsung").unwrap();
        let b = body(c, "4D7u5KF7SP8", Some(20525), None);
        assert_eq!(b["videoId"], "4D7u5KF7SP8");
        assert_eq!(b["contentCheckOk"], true);
        assert_eq!(b["racyCheckOk"], true);
        assert_eq!(b["playbackContext"]["contentPlaybackContext"], json!({ "html5Preference": "HTML5_PREF_WANTS", "signatureTimestamp": 20525 }));
        assert_eq!(b["context"]["client"]["clientName"], "TVHTML5");
        assert_eq!(b["context"]["client"]["deviceMake"], "Samsung");
        assert!(b.get("params").is_none() && b.get("serviceIntegrityDimensions").is_none());
        let b = body(crate::base::client("visionos").unwrap(), "x", None, None);
        let y = json!({ "WEB_PLAYER_CONTEXT_CONFIGS": { "WEB_PLAYER_CONTEXT_CONFIG_ID_EMBEDDED_PLAYER": { "encryptedHostFlags": "f" } } });
        let e = body(crate::base::client("web_embedded").unwrap(), "x", None, Some(&y));
        assert_eq!(e["playbackContext"]["contentPlaybackContext"]["encryptedHostFlags"], "f");
        assert!(b["playbackContext"]["contentPlaybackContext"].get("signatureTimestamp").is_none());
    }
}
