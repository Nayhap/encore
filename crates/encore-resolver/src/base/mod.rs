//! InnerTube requests as yt-dlp makes them (D37).
//!
//! Upstream: yt-dlp `yt_dlp/extractor/youtube/_base.py` at the commit in
//! `upstream.toml`: `_extract_context` (753), `_make_sid_authorization`,
//! `_get_sid_authorization_header` (762-822), `_has_auth_cookies` (828),
//! `_generate_cookie_auth_headers` (960-978), `generate_api_headers`
//! (980-999), `_initialize_consent` and `_initialize_pref` (675-692).
//! The client table is [`clients`].

pub mod clients;

use encore_auth::CookieJar;
use encore_core::service::ResolveAuth;
use serde_json::{Value, json};

pub use clients::{InnertubeClient, client};

/// A browser's UA for requests that aren't a client's (the iframe API,
/// player JS), as yt-dlp's default headers have one.
pub const BROWSER_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";

/// Who a resolve runs as, frozen when it starts (D2): every request of
/// the resolve uses it, and its `key` scopes the caches, so a channel
/// switch can neither mix identities nor be served the other's URLs.
#[derive(Clone)]
pub struct Identity {
    /// `anonymous`, or the app's key for account, channel and session.
    pub key: String,
    /// The session's cookies; `None` signed out.
    pub jar: Option<CookieJar>,
    pub auth_user: u32,
    /// `X-Goog-PageId` (delegated session id) of a brand channel.
    pub page_id: Option<String>,
    /// The manager's user session id, for the brand `_u` hash.
    pub user_session_id: Option<String>,
    pub visitor_data: Option<String>,
    /// Signed out: the watch page's cookies (`video::webpage`), with the
    /// consent ones.
    pub anon_cookies: Option<String>,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Identity({}, signed in: {})", self.key, self.is_authenticated())
    }
}

impl Identity {
    pub fn anonymous(visitor_data: Option<String>) -> Self {
        Identity { key: "anonymous".into(), jar: None, auth_user: 0, page_id: None, user_session_id: None, visitor_data, anon_cookies: None }
    }

    pub fn from_auth(auth: Option<&ResolveAuth>, anonymous_visitor: Option<String>) -> Self {
        let Some(a) = auth else { return Self::anonymous(anonymous_visitor) };
        let jar = CookieJar::new(a.cookies.clone());
        if !jar.has_login() {
            return Self::anonymous(anonymous_visitor);
        }
        Identity {
            key: if a.key.is_empty() { format!("session:{}:{}", a.auth_user, a.page_id.as_deref().unwrap_or("")) } else { a.key.clone() },
            jar: Some(jar),
            auth_user: a.auth_user,
            page_id: a.page_id.clone(),
            user_session_id: a.user_session_id.clone(),
            visitor_data: a.visitor_data.clone(),
            anon_cookies: None,
        }
    }

    /// `is_authenticated` / `_has_auth_cookies`: LOGIN_INFO and a SAPISID.
    pub fn is_authenticated(&self) -> bool {
        self.jar.as_ref().is_some_and(|j| j.has_login())
    }
}

/// `_extract_context`: the client's context with language and time zone
/// forced for extraction.
pub fn context(c: &InnertubeClient) -> Value {
    let mut ctx = c.innertube_context.clone();
    let client = &mut ctx["client"];
    client["hl"] = json!("en");
    client["timeZone"] = json!("UTC");
    client["utcOffsetMinutes"] = json!(0);
    ctx
}

/// `generate_api_headers` with `_generate_cookie_auth_headers`, for an API
/// call by client `c` as `id`. `Cookie` included: signed in, the session's
/// cookies for the host (with `PREF` as `_initialize_pref` sets it);
/// signed out, the consent and `PREF` cookies yt-dlp sets.
pub fn api_headers(c: &InnertubeClient, id: &Identity, now: u64) -> Vec<(String, String)> {
    let origin = format!("https://{}", c.innertube_host);
    let mut h: Vec<(String, String)> = vec![
        ("X-YouTube-Client-Name".into(), c.innertube_context_client_name.to_string()),
        ("X-YouTube-Client-Version".into(), c.client_version().into()),
        ("Origin".into(), origin.clone()),
        ("User-Agent".into(), c.user_agent().unwrap_or(BROWSER_UA).into()),
        ("Content-Type".into(), "application/json".into()),
    ];
    if let Some(v) = &id.visitor_data {
        h.push(("X-Goog-Visitor-Id".into(), v.clone()));
    }
    let url = url::Url::parse(&format!("{origin}/youtubei/v1/player")).expect("api url");
    match id.jar.as_ref().filter(|j| j.has_login() && c.supports_cookies) {
        Some(jar) => {
            if let Some(p) = &id.page_id {
                h.push(("X-Goog-PageId".into(), p.clone()));
            }
            h.push(("X-Goog-AuthUser".into(), id.auth_user.to_string()));
            let user = id.page_id.as_ref().and(id.user_session_id.as_deref());
            if let Some(a) = encore_auth::sapisid_authorization(jar, &origin, now, user) {
                h.push(("Authorization".into(), a));
                h.push(("X-Origin".into(), origin.clone()));
            }
            h.push(("Cookie".into(), with_pref(&jar.header_for(&url))));
        }
        None => h.push(("Cookie".into(), id.anon_cookies.clone().unwrap_or_else(|| "SOCS=CAI; PREF=hl=en&tz=UTC".into()))),
    }
    h
}

/// `_initialize_pref`: the session's `PREF` with `hl=en` and `tz=UTC`.
fn with_pref(cookie: &str) -> String {
    let mut pref: Vec<(String, String)> = Vec::new();
    let mut rest: Vec<&str> = Vec::new();
    for part in cookie.split("; ").filter(|p| !p.is_empty()) {
        match part.strip_prefix("PREF=") {
            Some(v) => pref.extend(url::form_urlencoded::parse(v.as_bytes()).map(|(k, v)| (k.into_owned(), v.into_owned()))),
            None => rest.push(part),
        }
    }
    pref.retain(|(k, _)| k != "hl" && k != "tz");
    pref.push(("hl".into(), "en".into()));
    pref.push(("tz".into(), "UTC".into()));
    let pref = format!("PREF={}", url::form_urlencoded::Serializer::new(String::new()).extend_pairs(pref).finish());
    rest.push(&pref);
    rest.join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use encore_core::service::CookieRecord;

    fn signed_in(page: Option<&str>) -> Identity {
        let c = |n: &str, v: &str| CookieRecord { name: n.into(), value: v.into(), domain: ".youtube.com".into(), path: "/".into(), secure: true, http_only: false, expires: None };
        let auth = ResolveAuth {
            cookies: vec![c("SAPISID", "sapi"), c("LOGIN_INFO", "li"), c("PREF", "f6=40000000&hl=de")],
            auth_user: 1,
            page_id: page.map(str::to_string),
            user_session_id: Some("1234".into()),
            visitor_data: Some("visitor".into()),
            key: "acct:1".into(),
        };
        Identity::from_auth(Some(&auth), None)
    }

    fn get<'a>(h: &'a [(String, String)], k: &str) -> Option<&'a str> {
        h.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    }

    #[test]
    fn signed_out_requests_carry_consent_and_no_auth() {
        let h = api_headers(client("tv_samsung").unwrap(), &Identity::anonymous(None), 1_700_000_000);
        assert_eq!(get(&h, "X-YouTube-Client-Name"), Some("7"));
        assert_eq!(get(&h, "X-YouTube-Client-Version"), Some("5.20260707"));
        assert_eq!(get(&h, "Origin"), Some("https://www.youtube.com"));
        assert!(get(&h, "User-Agent").unwrap().contains("Tizen"));
        assert_eq!(get(&h, "Cookie"), Some("SOCS=CAI; PREF=hl=en&tz=UTC"));
        assert!(get(&h, "Authorization").is_none() && get(&h, "X-Goog-AuthUser").is_none());
    }

    #[test]
    fn signed_in_requests_hash_for_the_api_origin() {
        let id = signed_in(None);
        assert_eq!(id.key, "acct:1");
        let h = api_headers(client("web_embedded").unwrap(), &id, 1_700_000_000);
        let auth = get(&h, "Authorization").unwrap();
        assert!(auth.starts_with("SAPISIDHASH 1700000000_") && !auth.ends_with("_u"), "{auth}");
        assert_eq!(get(&h, "X-Origin"), Some("https://www.youtube.com"));
        assert_eq!(get(&h, "X-Goog-AuthUser"), Some("1"));
        assert!(get(&h, "X-Goog-PageId").is_none());
        assert_eq!(get(&h, "X-Goog-Visitor-Id"), Some("visitor"));
        let cookie = get(&h, "Cookie").unwrap();
        assert!(cookie.contains("SAPISID=sapi") && cookie.ends_with("PREF=f6=40000000&hl=en&tz=UTC"), "{cookie}");

        // A brand channel: the page id and the manager's `_u` hash.
        let h = api_headers(client("tv_samsung").unwrap(), &signed_in(Some("page")), 1_700_000_000);
        assert_eq!(get(&h, "X-Goog-PageId"), Some("page"));
        assert!(get(&h, "Authorization").unwrap().ends_with("_u"));

        // A client without cookie support never gets the session.
        let h = api_headers(client("visionos").unwrap(), &id, 1_700_000_000);
        assert!(get(&h, "Authorization").is_none());
        assert_eq!(get(&h, "Cookie"), Some("SOCS=CAI; PREF=hl=en&tz=UTC"));
    }

    #[test]
    fn the_context_forces_language_and_time_zone() {
        let c = context(client("web_embedded").unwrap());
        assert_eq!(c["client"]["clientName"], "WEB_EMBEDDED_PLAYER");
        assert_eq!(c["client"]["timeZone"], "UTC");
        assert_eq!(c["client"]["utcOffsetMinutes"], 0);
        assert!(c["thirdParty"]["embedUrl"].is_string());
    }
}
