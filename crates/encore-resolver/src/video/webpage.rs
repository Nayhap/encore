//! The signed-out session: the watch page's cookies and visitor.
//!
//! Upstream: yt-dlp `yt_dlp/extractor/youtube/_video.py` at the commit in
//! `upstream.toml`: `_download_initial_webpage` (3805-3820: the watch page
//! with `bpctr=9999999999&has_verified=1`), and `_base.py`
//! `_initialize_consent` / `_initialize_pref` (675-692), `extract_ytcfg`
//! and `_extract_visitor_data` (941-958). yt-dlp's cookie jar keeps what
//! the page sets (`YSC`, `VISITOR_INFO1_LIVE`, `__Secure-YNID`, …) and
//! sends it with every `/player` call; without them YouTube answers a
//! signed-out TV client with a bot check.
//!
//! Divergence: yt-dlp loads the page for every video; Encore loads it once
//! and keeps the session (cookies and visitor) in memory for a while, as a
//! device does, and starts a new one after a bot check.

use std::sync::LazyLock;
use std::time::{Duration, Instant};

use encore_auth::CookieJar;
use encore_core::service::CookieRecord;
use regex::Regex;

/// How long a signed-out session is used before a new one.
pub const SESSION_TTL: Duration = Duration::from_secs(6 * 3600);
const MAX_PAGE_BYTES: usize = 8 << 20;

static YTCFG_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""VISITOR_DATA"\s*:\s*"([^"]{8,256})""#).unwrap());

/// What `_initialize_consent` and `_initialize_pref` set.
pub fn consent_jar() -> CookieJar {
    let c = |n: &str, v: &str| CookieRecord { name: n.into(), value: v.into(), domain: ".youtube.com".into(), path: "/".into(), secure: true, http_only: false, expires: None };
    CookieJar::new(vec![c("SOCS", "CAI"), c("PREF", "hl=en&tz=UTC")])
}

/// The visitor in a page's ytcfg (`VISITOR_DATA`).
pub fn visitor_data(page: &str) -> Option<String> {
    YTCFG_RE.captures(page).map(|c| c[1].to_string())
}

/// A signed-out session: its cookies and visitor, and when it began.
#[derive(Clone)]
pub struct AnonSession {
    pub jar: CookieJar,
    pub visitor: Option<String>,
    pub at: Instant,
}

impl AnonSession {
    pub fn fresh(&self) -> bool {
        self.at.elapsed() < SESSION_TTL
    }

    /// The `Cookie` header for the API host.
    pub fn cookie_header(&self) -> String {
        let url = url::Url::parse("https://www.youtube.com/youtubei/v1/player").expect("url");
        self.jar.header_for(&url)
    }
}

/// Load the watch page of `video_id` as a browser does, keeping its cookies
/// and visitor. `None` when YouTube didn't answer usefully.
pub async fn load(http: &reqwest::Client, video_id: &str) -> Option<AnonSession> {
    let url = url::Url::parse(&format!("https://www.youtube.com/watch?v={video_id}&bpctr=9999999999&has_verified=1")).ok()?;
    let mut jar = consent_jar();
    let r = http
        .get(url.clone())
        .header("Cookie", jar.header_for(&url))
        .header("User-Agent", crate::base::BROWSER_UA)
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .header("Accept-Language", "en-us,en;q=0.5")
        .header("Sec-Fetch-Mode", "navigate")
        .send()
        .await
        .ok()?;
    for h in r.headers().get_all("set-cookie") {
        if let Ok(h) = h.to_str() {
            jar.apply_set_cookie(&url, h);
        }
    }
    if !r.status().is_success() {
        return None;
    }
    let body = r.bytes().await.ok()?;
    let page = String::from_utf8_lossy(&body[..body.len().min(MAX_PAGE_BYTES)]);
    Some(AnonSession { jar, visitor: visitor_data(&page), at: Instant::now() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_visitor_is_read_from_ytcfg() {
        assert_eq!(visitor_data(r#"ytcfg.set({"VISITOR_DATA":"CgtABCDEFGH%3D%3D","X":1})"#).as_deref(), Some("CgtABCDEFGH%3D%3D"));
        assert_eq!(visitor_data("nothing"), None);
    }

    #[test]
    fn page_cookies_join_consent() {
        let mut jar = consent_jar();
        let url = url::Url::parse("https://www.youtube.com/watch?v=x").unwrap();
        jar.apply_set_cookie(&url, "YSC=abc; Domain=.youtube.com; Path=/; Secure; HttpOnly; SameSite=none; Partitioned");
        jar.apply_set_cookie(&url, "VISITOR_INFO1_LIVE=def; Domain=.youtube.com; Expires=Sun, 02-May-2099 16:14:47 GMT; Path=/; Secure; HttpOnly");
        let s = AnonSession { jar, visitor: None, at: Instant::now() };
        let h = s.cookie_header();
        for c in ["SOCS=CAI", "PREF=hl=en&tz=UTC", "YSC=abc", "VISITOR_INFO1_LIVE=def"] {
            assert!(h.contains(c), "{h}");
        }
    }
}
