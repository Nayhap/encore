//! InnerTube transport (D10). The only authenticated HTTP boundary.
//!
//! WEB_REMIX with the live client version scraped from music.youtube.com (a
//! pinned fallback otherwise). Signed in: Cookie + SAPISIDHASH (with the brand
//! `_u` variant), `X-Goog-AuthUser`, `X-Goog-PageId` and
//! `context.user.onBehalfOfUser`. Rotated `Set-Cookie`s are merged back into
//! the session jar. Nothing here logs cookies, hashes or bodies.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use encore_auth::{Session, sapisid_authorization};
use encore_core::error::{AppError, ErrorCode, Result};
use parking_lot::{Mutex, RwLock};
use serde_json::{Value, json};

pub const ORIGIN: &str = "https://music.youtube.com";
pub const FALLBACK_VERSION: &str = "1.20261027.10.00";
pub const WEB_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";
const ANDROID_MUSIC_VERSION: &str = "7.27.52";
const ANDROID_MUSIC_UA: &str = "com.google.android.apps.youtube.music/7.27.52 (Linux; U; Android 14) gzip";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Client {
    WebRemix,
    /// Used for timed lyrics only (anonymous).
    AndroidMusic,
}

type SessionHook = Box<dyn Fn(&Session) + Send + Sync>;

struct State {
    version: String,
    version_at: Option<Instant>,
    visitor: Option<String>,
    session: Option<Session>,
}

/// Who requests go out as, frozen at one moment. A sequence of requests that
/// must agree (a play's `/player` call and its pings) uses one snapshot, so a
/// channel switch in between can't mix identities (D2).
#[derive(Clone)]
pub struct Identity {
    session: Option<Session>,
    version: String,
    visitor: Option<String>,
    epoch: u64,
}

impl Identity {
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn version(&self) -> &str {
        &self.version
    }
    /// Signed in with a chosen channel.
    pub fn signed_in(&self) -> bool {
        self.session.as_ref().is_some_and(|s| s.jar.has_login() && s.channel.is_some())
    }
    fn signed(&self) -> bool {
        self.session.as_ref().is_some_and(|s| s.jar.has_login())
    }
}

pub struct InnerTube {
    http: reqwest::Client,
    /// For stats pings: never follows redirects, so the session can't be
    /// forwarded to another host.
    stats_http: reqwest::Client,
    state: RwLock<State>,
    epoch: AtomicU64,
    hl: String,
    gl: String,
    on_session: Mutex<Option<SessionHook>>,
    signed_out_strikes: AtomicU64,
    boot_lock: tokio::sync::Mutex<()>,
}

impl InnerTube {
    pub fn new(hl: &str, gl: &str) -> Arc<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .gzip(true)
            .brotli(true)
            .build()
            .expect("http client");
        let stats_http = reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(15)).redirect(reqwest::redirect::Policy::none()).build().expect("stats client");
        Arc::new(Self {
            http,
            stats_http,
            state: RwLock::new(State { version: FALLBACK_VERSION.into(), version_at: None, visitor: None, session: None }),
            epoch: AtomicU64::new(1),
            hl: hl.into(),
            gl: gl.into(),
            on_session: Mutex::new(None),
            signed_out_strikes: AtomicU64::new(0),
            boot_lock: tokio::sync::Mutex::new(()),
        })
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    /// Replace the session (sign-in, channel switch, sign-out). Bumps the
    /// epoch so in-flight work for the old identity is dropped.
    pub fn set_session(&self, s: Option<Session>) {
        let mut st = self.state.write();
        if let Some(v) = s.as_ref().and_then(|s| s.visitor_data.clone()) {
            st.visitor = Some(v);
        } else if s.is_none() {
            st.visitor = None;
        }
        st.session = s;
        self.epoch.fetch_add(1, Ordering::SeqCst);
        self.signed_out_strikes.store(0, Ordering::Relaxed);
    }

    pub fn session(&self) -> Option<Session> {
        self.state.read().session.clone()
    }

    pub fn signed_in(&self) -> bool {
        self.state.read().session.as_ref().is_some_and(|s| s.jar.has_login() && s.channel.is_some())
    }

    pub fn has_cookies(&self) -> bool {
        self.state.read().session.as_ref().is_some_and(|s| s.jar.has_login())
    }

    pub fn on_session_change(&self, f: impl Fn(&Session) + Send + Sync + 'static) {
        *self.on_session.lock() = Some(Box::new(f));
    }

    pub fn version(&self) -> String {
        self.state.read().version.clone()
    }

    /// Scrape the live WEB_REMIX version and visitor data (every 6 h).
    pub async fn bootstrap(&self) {
        let _one = self.boot_lock.lock().await;
        {
            let st = self.state.read();
            if st.version_at.is_some_and(|t| t.elapsed() < Duration::from_secs(6 * 3600)) {
                return;
            }
        }
        let t0 = Instant::now();
        let res = self.http.get(format!("{ORIGIN}/")).header("User-Agent", WEB_UA).header("Cookie", "SOCS=CAI").header("Accept-Language", "en-US,en;q=0.9").send().await;
        let body = match res {
            Ok(r) => r.text().await.unwrap_or_default(),
            Err(_) => String::new(),
        };
        tracing::info!(ms = t0.elapsed().as_millis() as u64, bytes = body.len(), "innertube bootstrap");
        let mut st = self.state.write();
        st.version_at = Some(Instant::now());
        if let Some(v) = scrape(&body, "\"INNERTUBE_CLIENT_VERSION\":\"") {
            if v.starts_with("1.") && v.len() < 32 {
                st.version = v;
            }
        }
        if st.visitor.is_none() {
            if let Some(v) = scrape(&body, "\"VISITOR_DATA\":\"") {
                if v.len() < 200 {
                    st.visitor = Some(v);
                }
            }
        }
    }

    /// The current identity, to pin a sequence of requests to.
    pub fn identity(&self) -> Identity {
        let st = self.state.read();
        Identity { session: st.session.clone(), version: st.version.clone(), visitor: st.visitor.clone(), epoch: self.epoch() }
    }

    fn context(&self, client: Client, st: &Identity, signed: bool) -> Value {
        let mut c = match client {
            Client::WebRemix => json!({ "clientName": "WEB_REMIX", "clientVersion": st.version, "hl": self.hl, "gl": self.gl }),
            Client::AndroidMusic => json!({ "clientName": "ANDROID_MUSIC", "clientVersion": ANDROID_MUSIC_VERSION, "androidSdkVersion": 34, "osName": "Android", "osVersion": "14", "hl": self.hl, "gl": self.gl }),
        };
        if let Some(v) = &st.visitor {
            c["visitorData"] = json!(v);
        }
        let mut user = json!({ "lockedSafetyMode": false });
        if signed {
            if let Some(p) = st.session.as_ref().and_then(|s| s.channel.as_ref()).and_then(|c| c.page_id.clone()) {
                user["onBehalfOfUser"] = json!(p);
            }
        }
        json!({ "client": c, "user": user })
    }

    /// POST an InnerTube endpoint. `query` is appended (continuations).
    pub async fn post(&self, client: Client, endpoint: &str, body: Value, query: &[(&str, &str)]) -> Result<Value> {
        self.bootstrap().await;
        let id = self.identity();
        self.post_as(&id, client, endpoint, body, query).await
    }

    /// POST as a pinned identity (see [`Identity`]).
    pub async fn post_as(&self, id: &Identity, client: Client, endpoint: &str, mut body: Value, query: &[(&str, &str)]) -> Result<Value> {
        let epoch = id.epoch;
        let (url, req) = {
            let st = id;
            let signed = client == Client::WebRemix && st.signed();
            body["context"] = self.context(client, &st, signed);
            let host = ORIGIN;
            let mut url = url::Url::parse(&format!("{host}/youtubei/v1/{endpoint}")).map_err(|e| AppError::internal(e.to_string()))?;
            url.query_pairs_mut().append_pair("prettyPrint", "false");
            for (k, v) in query {
                url.query_pairs_mut().append_pair(k, v);
            }
            let mut rb = self.http.post(url.clone()).header("Content-Type", "application/json").header("X-Goog-Api-Format-Version", "1");
            match client {
                Client::WebRemix => {
                    rb = rb
                        .header("User-Agent", WEB_UA)
                        .header("X-YouTube-Client-Name", "67")
                        .header("X-YouTube-Client-Version", st.version.as_str())
                        .header("Origin", ORIGIN)
                        .header("X-Origin", ORIGIN)
                        .header("Referer", format!("{ORIGIN}/"));
                }
                Client::AndroidMusic => {
                    rb = rb.header("User-Agent", ANDROID_MUSIC_UA).header("X-YouTube-Client-Name", "21").header("X-YouTube-Client-Version", ANDROID_MUSIC_VERSION);
                }
            }
            if let Some(v) = &st.visitor {
                rb = rb.header("X-Goog-Visitor-Id", v.as_str());
            }
            if signed {
                let s = st.session.as_ref().unwrap();
                let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
                let ch = s.channel.as_ref();
                let user = ch.and_then(|c| c.page_id.as_ref().and(c.user_session_id.as_deref()));
                if let Some(auth) = sapisid_authorization(&s.jar, ORIGIN, ts, user) {
                    rb = rb.header("Authorization", auth);
                }
                rb = rb.header("Cookie", s.jar.header_for(&url)).header("X-Goog-AuthUser", ch.map(|c| c.auth_user).unwrap_or(0).to_string());
                if let Some(p) = ch.and_then(|c| c.page_id.as_ref()) {
                    rb = rb.header("X-Goog-PageId", p.as_str());
                }
            } else {
                rb = rb.header("Cookie", "SOCS=CAI");
            }
            (url, rb.json(&body))
        };
        let t0 = Instant::now();
        let resp = req.send().await.map_err(|e| {
            if e.is_timeout() {
                AppError::network("YouTube Music did not answer in time.")
            } else {
                AppError::network("Could not reach YouTube Music. Check your connection.")
            }
        })?;
        let status = resp.status().as_u16();
        // Rotated cookies belong to the identity that made the request.
        if self.epoch() == epoch {
            self.merge_cookies(&url, resp.headers());
        }
        let text = resp.text().await.map_err(|_| AppError::network("The response from YouTube Music was interrupted."))?;
        tracing::info!(endpoint, status, ms = t0.elapsed().as_millis() as u64, bytes = text.len(), signed = self.has_cookies(), "innertube");
        if self.epoch() != epoch {
            return Err(AppError::stale());
        }
        if text.trim_start().starts_with('<') {
            if text.contains("<title>Sorry") || status == 429 {
                return Err(AppError::new(ErrorCode::Challenged, "YouTube asked for a verification. Wait a minute and try again."));
            }
            return Err(AppError::network(format!("YouTube Music answered with an unexpected page ({status}).")));
        }
        let v: Value = serde_json::from_str(&text).map_err(|_| AppError::parse("YouTube Music sent a response Encore could not read."))?;
        if let Some(err) = v.get("error") {
            let code = err.get("code").and_then(Value::as_u64).unwrap_or(status as u64);
            let msg = err.get("message").and_then(Value::as_str).unwrap_or("request failed").to_string();
            return Err(match code {
                401 => AppError::session_expired(),
                403 if self.has_cookies() => AppError::new(ErrorCode::SessionExpired, "YouTube Music refused the signed-in request. The session may have expired.").with_action("reimport"),
                403 => AppError::new(ErrorCode::Rejected, "YouTube Music refused the request."),
                404 => AppError::not_found("YouTube Music could not find that."),
                429 => AppError::new(ErrorCode::Challenged, "YouTube is rate limiting requests. Try again shortly."),
                400 => AppError::new(ErrorCode::InvalidArgument, format!("YouTube Music rejected the request: {}", short(&msg))),
                _ => AppError::network(format!("YouTube Music error {code}: {}", short(&msg))),
            });
        }
        if client == Client::WebRemix && self.has_cookies() {
            if crate::parse::is_signed_out(&v) {
                let n = self.signed_out_strikes.fetch_add(1, Ordering::Relaxed) + 1;
                if n >= 2 {
                    return Err(AppError::session_expired());
                }
            } else {
                self.signed_out_strikes.store(0, Ordering::Relaxed);
            }
        }
        Ok(v)
    }

    fn merge_cookies(&self, url: &url::Url, headers: &reqwest::header::HeaderMap) {
        let set: Vec<&str> = headers.get_all("set-cookie").iter().filter_map(|h| h.to_str().ok()).collect();
        if set.is_empty() {
            return;
        }
        let changed = {
            let mut st = self.state.write();
            let Some(s) = st.session.as_mut() else { return };
            let mut changed = false;
            for h in set {
                changed |= s.jar.apply_set_cookie(url, h);
            }
            s.jar.retain_needed();
            changed.then(|| s.clone())
        };
        if let Some(s) = changed {
            if let Some(f) = self.on_session.lock().as_ref() {
                f(&s);
            }
        }
    }

    /// GET a playback stats URL (history reporting, D18) as `id`. Only https
    /// `s.youtube.com` / `www.youtube.com` / `music.youtube.com` URLs under
    /// `/api/stats/` are accepted, redirects are not followed and response
    /// cookies are not merged. Returns the HTTP status.
    pub async fn stats_get(&self, id: &Identity, url: &url::Url) -> Result<u16> {
        if !stats_url_ok(url) {
            return Err(AppError::invalid("Refused to send a stats ping to an unexpected address."));
        }
        if id.epoch != self.epoch() {
            return Err(AppError::stale());
        }
        let s = id.session.as_ref().filter(|s| s.jar.has_login()).ok_or_else(AppError::auth_required)?;
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        let ch = s.channel.as_ref();
        let user = ch.and_then(|c| c.page_id.as_ref().and(c.user_session_id.as_deref()));
        let mut rb = self
            .stats_http
            .get(url.clone())
            .header("User-Agent", WEB_UA)
            .header("Accept", "*/*")
            .header("X-YouTube-Client-Name", "67")
            .header("X-YouTube-Client-Version", id.version.as_str())
            .header("Origin", ORIGIN)
            .header("X-Origin", ORIGIN)
            .header("Referer", format!("{ORIGIN}/"))
            .header("Cookie", s.jar.header_for(url))
            .header("X-Goog-AuthUser", ch.map(|c| c.auth_user).unwrap_or(0).to_string());
        if let Some(a) = sapisid_authorization(&s.jar, ORIGIN, ts, user) {
            rb = rb.header("Authorization", a);
        }
        if let Some(p) = ch.and_then(|c| c.page_id.as_ref()) {
            rb = rb.header("X-Goog-PageId", p.as_str());
        }
        if let Some(v) = &id.visitor {
            rb = rb.header("X-Goog-Visitor-Id", v.as_str());
        }
        let t0 = Instant::now();
        let resp = rb.send().await.map_err(|_| AppError::network("Could not reach YouTube for the play report."))?;
        let status = resp.status().as_u16();
        // The path says which ping it was; the query (ids, nonce) stays out of the log.
        tracing::info!(path = url.path(), status, ms = t0.elapsed().as_millis() as u64, "stats ping");
        Ok(status)
    }

    /// Keep-alive: GET /verify_session and merge rotated cookies. Returns
    /// whether any cookie changed.
    pub async fn verify_session(&self) -> Result<bool> {
        let (url, req) = {
            let st = self.state.read();
            let Some(s) = st.session.as_ref() else { return Err(AppError::auth_required()) };
            let url = url::Url::parse(&format!("{ORIGIN}/verify_session")).unwrap();
            let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
            let mut rb = self
                .http
                .get(url.clone())
                .header("User-Agent", WEB_UA)
                .header("Accept", "*/*")
                .header("Origin", ORIGIN)
                .header("X-Origin", ORIGIN)
                .header("Referer", format!("{ORIGIN}/"))
                .header("Cookie", s.jar.header_for(&url));
            if let Some(a) = sapisid_authorization(&s.jar, ORIGIN, ts, None) {
                rb = rb.header("Authorization", a);
            }
            (url, rb)
        };
        let before = self.session().map(|s| s.jar);
        let resp = req.send().await.map_err(|_| AppError::network("Could not reach YouTube Music for the session check."))?;
        let status = resp.status().as_u16();
        self.merge_cookies(&url, resp.headers());
        if status == 401 {
            return Err(AppError::session_expired());
        }
        if !(200..300).contains(&status) {
            return Err(AppError::network(format!("Session check answered {status}.")));
        }
        Ok(self.session().map(|s| s.jar) != before)
    }
}

/// Stats pings carry the session, so they only go to YouTube's own stats paths.
pub fn stats_url_ok(u: &url::Url) -> bool {
    u.scheme() == "https"
        && matches!(u.host_str(), Some("s.youtube.com" | "www.youtube.com" | "music.youtube.com"))
        && u.port().is_none()
        && u.username().is_empty()
        && u.password().is_none()
        && matches!(u.path(), "/api/stats/playback" | "/api/stats/watchtime")
}

fn short(s: &str) -> String {
    let s = encore_core::redact::redact(s);
    if s.chars().count() > 120 { format!("{}…", s.chars().take(120).collect::<String>()) } else { s }
}

fn scrape(body: &str, marker: &str) -> Option<String> {
    let i = body.find(marker)? + marker.len();
    let rest = &body[i..];
    let j = rest.find('"')?;
    Some(rest[..j].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_urls_are_restricted() {
        let ok = |s: &str| stats_url_ok(&url::Url::parse(s).unwrap());
        assert!(ok("https://s.youtube.com/api/stats/playback?docid=x"));
        assert!(ok("https://s.youtube.com/api/stats/watchtime?docid=x"));
        assert!(!ok("http://s.youtube.com/api/stats/playback"));
        assert!(!ok("https://s.youtube.com.evil.example/api/stats/playback"));
        assert!(!ok("https://evil.example/api/stats/playback"));
        assert!(!ok("https://s.youtube.com:8443/api/stats/playback"));
        assert!(!ok("https://user@s.youtube.com/api/stats/playback"));
        assert!(!ok("https://s.youtube.com/api/stats/qoe"));
        assert!(!ok("https://s.youtube.com/redirect?to=x"));
    }
}
