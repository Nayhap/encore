//! A client's ytcfg: the page config some clients need before `/player`.
//!
//! Upstream: yt-dlp `yt_dlp/extractor/youtube/_base.py` at the commit in
//! `upstream.toml`: `_download_ytcfg` (1013-1048: which page per client,
//! the `_fix_embedded_ytcfg` and the signed-out `tv` `appInstallData`
//! workaround), `extract_ytcfg` (951) and the `_ytcfg_get_safe` lookups of
//! `generate_api_headers`; `_video.py:2948-2953` (`encryptedHostFlags`).
//! Without it, `web_embedded` answers "Error code: 152" and `tv` "The page
//! needs to be reloaded".

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use super::{Identity, InnertubeClient};

const MAX_PAGE_BYTES: usize = 8 << 20;
static YTCFG_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"ytcfg\.set\s*\(\s*(\{.+?\})\s*\)\s*;").unwrap());

/// `_download_ytcfg`'s page for a client (only those Encore asks).
pub fn page_url(client: &str, video_id: &str) -> Option<String> {
    match client {
        "web_embedded" => Some(format!("https://www.youtube.com/embed/{video_id}?html5=1")),
        "tv" => Some("https://www.youtube.com/tv".into()),
        _ => None,
    }
}

/// `extract_ytcfg`: the first `ytcfg.set({...});` that parses.
pub fn extract(page: &str) -> Option<Value> {
    YTCFG_RE.captures_iter(page).find_map(|c| serde_json::from_str::<Value>(&c[1]).ok()).filter(|v| v.is_object())
}

/// What `_download_ytcfg` does to the page's config.
pub fn fix(c: &InnertubeClient, ytcfg: &mut Value, authenticated: bool) {
    if c.is_embedded() && ytcfg["INNERTUBE_CONTEXT"].is_object() {
        ytcfg["INNERTUBE_CONTEXT"]["thirdParty"] = c.innertube_context["thirdParty"].clone();
    }
    // yt-dlp issue 12563: not effective when logged in.
    if c.key == "tv" && !authenticated {
        if let Some(ci) = ytcfg["INNERTUBE_CONTEXT"]["client"]["configInfo"].as_object_mut() {
            ci.remove("appInstallData");
        }
    }
}

/// `encryptedHostFlags` of the embedded player's context config.
pub fn encrypted_host_flags(ytcfg: &Value) -> Option<String> {
    ytcfg["WEB_PLAYER_CONTEXT_CONFIGS"]["WEB_PLAYER_CONTEXT_CONFIG_ID_EMBEDDED_PLAYER"]["encryptedHostFlags"].as_str().map(str::to_string)
}

/// Load and fix a client's ytcfg. `None` when the client has no page or
/// it gave none (the table's defaults are used then, as upstream).
pub async fn download(http: &reqwest::Client, c: &InnertubeClient, id: &Identity, video_id: &str) -> Option<Value> {
    let url = page_url(&c.key, video_id)?;
    if c.require_auth && !id.is_authenticated() {
        return None;
    }
    let u = url::Url::parse(&url).ok()?;
    let mut rb = http.get(u.clone()).header("User-Agent", c.user_agent().unwrap_or(super::BROWSER_UA));
    if let Some(r) = c.innertube_context["thirdParty"]["embedUrl"].as_str() {
        rb = rb.header("Referer", r);
    }
    let cookie = match id.jar.as_ref().filter(|j| j.has_login() && c.supports_cookies) {
        Some(j) => j.header_for(&u),
        None => id.anon_cookies.clone().unwrap_or_else(|| "SOCS=CAI; PREF=hl=en&tz=UTC".into()),
    };
    let r = rb.header("Cookie", cookie).send().await.ok()?;
    if !r.status().is_success() {
        return None;
    }
    let body = r.bytes().await.ok()?;
    let mut cfg = extract(&String::from_utf8_lossy(&body[..body.len().min(MAX_PAGE_BYTES)]))?;
    fix(c, &mut cfg, id.is_authenticated());
    Some(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_config_is_found_fixed_and_read() {
        let page = r#"<script>ytcfg.set({"INNERTUBE_CLIENT_VERSION":"1.2","INNERTUBE_CONTEXT":{"client":{"clientName":"WEB_EMBEDDED_PLAYER","clientVersion":"1.2","configInfo":{"appInstallData":"x"}}},
            "WEB_PLAYER_CONTEXT_CONFIGS":{"WEB_PLAYER_CONTEXT_CONFIG_ID_EMBEDDED_PLAYER":{"encryptedHostFlags":"flags"}}});</script><script>ytcfg.set({"B":1});</script>"#;
        let mut cfg = extract(&page.replace('\n', "")).unwrap();
        assert_eq!(encrypted_host_flags(&cfg).as_deref(), Some("flags"));
        let emb = crate::base::client("web_embedded").unwrap();
        fix(emb, &mut cfg, false);
        assert_eq!(cfg["INNERTUBE_CONTEXT"]["thirdParty"]["embedUrl"], "https://www.reddit.com/");
        let mut tv = cfg.clone();
        fix(crate::base::client("tv").unwrap(), &mut tv, false);
        assert!(tv["INNERTUBE_CONTEXT"]["client"]["configInfo"].get("appInstallData").is_none());
        assert_eq!(page_url("web_embedded", "abc").as_deref(), Some("https://www.youtube.com/embed/abc?html5=1"));
        assert_eq!(page_url("tv_samsung", "abc"), None);
        assert_eq!(extract("no config"), None);
    }
}
