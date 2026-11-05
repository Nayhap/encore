//! Last.fm (D31), with the user's own API key and shared secret: Last.fm
//! gives every app its own key, and Encore has no server to keep one.
//!
//! * Connecting is Last.fm's desktop flow: `auth.getToken`, the user
//!   approves Encore at [`auth_url`] in the browser, then `auth.getSession`
//!   turns the token into a session key that doesn't expire.
//! * Plays that count (the play log's rule, which is Last.fm's) are queued in
//!   `scrobble_queue` and sent in batches by [`flush`], whenever Encore is
//!   online; Last.fm takes timestamps, so plays made offline arrive later.
//!   A song that starts is sent as now playing, never queued.
//! * Play counts come from `user.getTopTracks`, matched by `match_key`ed
//!   artist and title.
//!
//! Every call is a POST with the parameters in the body, so no key, secret
//! or session key is ever part of a URL (or of an error that prints one).
//! Nothing here logs them; the key, secret and session live in the OS
//! credential store (D9), never in SQLite.

use std::sync::Arc;

use async_trait::async_trait;
use encore_core::local::integrations::{LastfmPlays, Scrobble, ScrobbleState};
use encore_core::local::library::match_key;
use encore_core::store::Store;
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const API_URL: &str = "https://ws.audioscrobbler.com/2.0/";
/// Where the user creates their key and secret.
pub const CREATE_URL: &str = "https://www.last.fm/api/account/create";
/// Scrobbles per `track.scrobble` request (Last.fm's limit).
pub const BATCH: usize = 50;

/// An app's key and secret, as the user copied them from Last.fm.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Credentials {
    pub api_key: String,
    pub secret: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Credentials(…)")
    }
}

/// A connected account. `key` never expires unless the user revokes it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub username: String,
    pub key: String,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Session({}, …)", self.username)
    }
}

/// What the credential store item holds (one item, D9).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub credentials: Credentials,
    pub session: Session,
}

/// A key or secret as Last.fm issues them: 32 hexadecimal characters.
pub fn valid_key(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Where the user approves Encore for their account.
pub fn auth_url(api_key: &str, token: &str) -> String {
    format!("https://www.last.fm/api/auth/?api_key={api_key}&token={token}")
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("Couldn't reach Last.fm.")]
    Network,
    #[error("Last.fm said: {message}")]
    Api { code: u32, message: String },
    #[error("Last.fm sent an answer Encore doesn't understand.")]
    Unexpected,
}

impl Error {
    /// Worth trying again later: no connection, or Last.fm busy or down.
    pub fn transient(&self) -> bool {
        matches!(self, Error::Network | Error::Unexpected | Error::Api { code: 8 | 11 | 16 | 29, .. })
    }

    /// The key or the session doesn't work any more: connect again.
    pub fn needs_reconnect(&self) -> bool {
        matches!(self, Error::Api { code: 4 | 9 | 10 | 13 | 26, .. })
    }

    /// `auth.getSession` before the user approved the token.
    pub fn not_approved(&self) -> bool {
        matches!(self, Error::Api { code: 14, .. })
    }
}

/// How requests reach Last.fm; tests answer with fixtures instead.
#[async_trait]
pub trait Transport: Send + Sync {
    async fn post(&self, form: Vec<(String, String)>) -> Result<Value, Error>;
}

/// The real one: HTTPS POSTs to [`API_URL`].
pub struct Http {
    client: reqwest::Client,
}

impl Http {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(concat!("Encore/", env!("CARGO_PKG_VERSION"), " (https://github.com/Nayhap/encore)"))
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .unwrap_or_default();
        Http { client }
    }
}

impl Default for Http {
    fn default() -> Self {
        Self::new()
    }
}

/// A JSON body read to its end but never past `max` bytes; `None` when
/// it was interrupted, larger or not JSON.
async fn json_capped(mut r: reqwest::Response, max: usize) -> Option<Value> {
    if r.content_length().is_some_and(|l| l as usize > max) {
        return None;
    }
    let mut out = Vec::new();
    while let Some(c) = r.chunk().await.ok()? {
        if out.len() + c.len() > max {
            return None;
        }
        out.extend_from_slice(&c);
    }
    serde_json::from_slice(&out).ok()
}

#[async_trait]
impl Transport for Http {
    async fn post(&self, form: Vec<(String, String)>) -> Result<Value, Error> {
        let r = self.client.post(API_URL).form(&form).send().await.map_err(|_| Error::Network)?;
        let status = r.status();
        // A page of 1,000 top tracks is well under 1 MB.
        let body: Value = json_capped(r, 8 * 1024 * 1024).await.ok_or(if status.is_success() { Error::Unexpected } else { Error::Network })?;
        // Errors come as `{ "error": 9, "message": "…" }`, often with a 4xx.
        api_error(&body).map_or(Ok(body), Err)
    }
}

fn api_error(v: &Value) -> Option<Error> {
    let code = v.get("error")?.as_u64()? as u32;
    let message = v.get("message").and_then(Value::as_str).unwrap_or("an error").to_string();
    Some(Error::Api { code, message })
}

/// The `api_sig` of a request: every parameter but `format` and
/// `callback`, sorted by name, names and values run together, the secret
/// appended, MD5 in hex.
pub fn sign(params: &[(String, String)], secret: &str) -> String {
    let mut p: Vec<&(String, String)> = params.iter().filter(|(k, _)| k != "format" && k != "callback").collect();
    p.sort_by(|a, b| a.0.cmp(&b.0));
    let mut h = Md5::new();
    for (k, v) in p {
        h.update(k.as_bytes());
        h.update(v.as_bytes());
    }
    h.update(secret.as_bytes());
    hex::encode(h.finalize())
}

/// One play, as Last.fm wants it.
#[derive(Clone, Debug, PartialEq)]
pub struct Play {
    pub artist: String,
    pub track: String,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub duration: Option<f64>,
    /// When it started, unix seconds.
    pub timestamp: i64,
}

impl From<&Scrobble> for Play {
    fn from(s: &Scrobble) -> Self {
        Play { artist: s.artist.clone(), track: s.track.clone(), album: s.album.clone(), album_artist: s.album_artist.clone(), duration: s.duration, timestamp: s.timestamp }
    }
}

/// Last.fm scrobbles songs longer than 30 seconds; an unknown length is
/// left for Last.fm to judge.
pub fn scrobblable(duration: Option<f64>) -> bool {
    duration.is_none_or(|d| d > 30.0)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Accepted,
    /// Last.fm took the request but not this scrobble (too old, a filtered
    /// artist): its reason.
    Ignored(String),
}

/// One row of `user.getTopTracks`.
#[derive(Clone, Debug, PartialEq)]
pub struct TopTrack {
    pub artist: String,
    pub title: String,
    pub plays: u32,
}

pub struct Client {
    t: Arc<dyn Transport>,
    creds: Credentials,
}

impl Client {
    pub fn new(t: Arc<dyn Transport>, creds: Credentials) -> Self {
        Client { t, creds }
    }

    async fn call(&self, method: &str, params: Vec<(&str, String)>, session: Option<&str>, signed: bool) -> Result<Value, Error> {
        let mut form: Vec<(String, String)> = params.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        form.push(("method".into(), method.into()));
        form.push(("api_key".into(), self.creds.api_key.clone()));
        if let Some(sk) = session {
            form.push(("sk".into(), sk.into()));
        }
        if signed {
            let sig = sign(&form, &self.creds.secret);
            form.push(("api_sig".into(), sig));
        }
        form.push(("format".into(), "json".into()));
        self.t.post(form).await
    }

    /// Step one of connecting; also proves the key and secret work.
    pub async fn token(&self) -> Result<String, Error> {
        let v = self.call("auth.getToken", vec![], None, true).await?;
        v.get("token").and_then(Value::as_str).map(str::to_string).ok_or(Error::Unexpected)
    }

    /// Step two, after the user approved `token` in the browser.
    pub async fn session(&self, token: &str) -> Result<Session, Error> {
        let v = self.call("auth.getSession", vec![("token", token.to_string())], None, true).await?;
        let s = v.get("session").ok_or(Error::Unexpected)?;
        let username = s.get("name").and_then(Value::as_str).ok_or(Error::Unexpected)?.to_string();
        let key = s.get("key").and_then(Value::as_str).ok_or(Error::Unexpected)?.to_string();
        Ok(Session { username, key })
    }

    pub async fn now_playing(&self, sk: &str, p: &Play) -> Result<(), Error> {
        let mut params = vec![("artist", p.artist.clone()), ("track", p.track.clone())];
        if let Some(a) = &p.album {
            params.push(("album", a.clone()));
        }
        if let Some(a) = &p.album_artist {
            params.push(("albumArtist", a.clone()));
        }
        if let Some(d) = p.duration {
            params.push(("duration", (d.round() as i64).to_string()));
        }
        self.call("track.updateNowPlaying", params, Some(sk), true).await.map(|_| ())
    }

    /// Up to [`BATCH`] plays in one request; an outcome for each, in order.
    pub async fn scrobble(&self, sk: &str, plays: &[Play]) -> Result<Vec<Outcome>, Error> {
        let mut params = Vec::new();
        for (i, p) in plays.iter().take(BATCH).enumerate() {
            let n = |k: &str| format!("{k}[{i}]");
            params.push((n("artist"), p.artist.clone()));
            params.push((n("track"), p.track.clone()));
            params.push((n("timestamp"), p.timestamp.to_string()));
            if let Some(a) = &p.album {
                params.push((n("album"), a.clone()));
            }
            if let Some(a) = &p.album_artist {
                params.push((n("albumArtist"), a.clone()));
            }
            if let Some(d) = p.duration {
                params.push((n("duration"), (d.round() as i64).to_string()));
            }
        }
        let refs: Vec<(&str, String)> = params.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        let v = self.call("track.scrobble", refs, Some(sk), true).await?;
        let list = v.get("scrobbles").and_then(|s| s.get("scrobble")).ok_or(Error::Unexpected)?;
        // One scrobble comes back as an object, several as an array.
        let rows: Vec<&Value> = match list {
            Value::Array(a) => a.iter().collect(),
            o => vec![o],
        };
        let n = plays.len().min(BATCH);
        if rows.len() != n {
            return Err(Error::Unexpected);
        }
        Ok(rows
            .into_iter()
            .map(|r| {
                let m = r.get("ignoredMessage");
                let code = m.and_then(|m| m.get("code")).map(|c| c.as_str().map(str::to_string).unwrap_or_else(|| c.to_string())).unwrap_or_else(|| "0".into());
                if code == "0" {
                    Outcome::Accepted
                } else {
                    let why = m.and_then(|m| m.get("#text")).and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("ignored by Last.fm");
                    Outcome::Ignored(why.to_string())
                }
            })
            .collect())
    }

    /// One page (1,000 songs) of an account's most played songs, all time,
    /// and how many pages there are.
    pub async fn top_tracks(&self, user: &str, page: u32) -> Result<(Vec<TopTrack>, u32), Error> {
        let params = vec![("user", user.to_string()), ("period", "overall".into()), ("limit", "1000".into()), ("page", page.to_string())];
        let v = self.call("user.getTopTracks", params, None, false).await?;
        let top = v.get("toptracks").ok_or(Error::Unexpected)?;
        let num = |v: Option<&Value>| v.and_then(|x| x.as_str().and_then(|s| s.parse().ok()).or_else(|| x.as_u64().map(|n| n as u32)));
        let pages = num(top.get("@attr").and_then(|a| a.get("totalPages"))).unwrap_or(1);
        let rows = match top.get("track") {
            Some(Value::Array(a)) => a.iter().collect(),
            Some(o @ Value::Object(_)) => vec![o],
            _ => vec![],
        };
        let tracks = rows
            .into_iter()
            .filter_map(|t| {
                Some(TopTrack {
                    title: t.get("name")?.as_str()?.to_string(),
                    artist: t.get("artist")?.get("name")?.as_str()?.to_string(),
                    plays: num(t.get("playcount"))?,
                })
            })
            .collect();
        Ok((tracks, pages))
    }
}

/// Rows for `lastfm_plays` from a read of the top tracks.
pub fn play_rows(top: &[TopTrack]) -> Vec<LastfmPlays> {
    top.iter().map(|t| LastfmPlays { artist_key: match_key(&t.artist), title_key: match_key(&t.title), plays: t.plays }).collect()
}

/// What a [`flush`] did.
#[derive(Debug, Default, PartialEq)]
pub struct Flushed {
    pub sent: usize,
    pub ignored: usize,
    /// Why it stopped early, if it did; queued scrobbles stay queued.
    pub stopped: Option<Error>,
}

/// Plays Last.fm turns down as bad requests are retried this often, then
/// marked failed.
const MAX_ATTEMPTS: u32 = 5;

/// Send `username`'s queued scrobbles, oldest first, batch by batch. A
/// transient failure or a dead session stops it, keeping the rest queued.
pub async fn flush(client: &Client, session: &Session, store: &Store) -> Flushed {
    let mut done = Flushed::default();
    loop {
        let batch = match store.scrobbles_queued(&session.username, BATCH as u32) {
            Ok(b) if !b.is_empty() => b,
            _ => return done,
        };
        let plays: Vec<Play> = batch.iter().map(Play::from).collect();
        match client.scrobble(&session.key, &plays).await {
            Ok(outcomes) => {
                for (s, o) in batch.iter().zip(outcomes) {
                    let r = match o {
                        Outcome::Accepted => {
                            done.sent += 1;
                            store.scrobble_mark(&s.play_id, ScrobbleState::Sent, None)
                        }
                        Outcome::Ignored(why) => {
                            done.ignored += 1;
                            store.scrobble_mark(&s.play_id, ScrobbleState::Ignored, Some(&why))
                        }
                    };
                    if r.is_err() {
                        // Not marked: stop rather than send it twice.
                        return done;
                    }
                }
            }
            Err(e) if e.transient() || e.needs_reconnect() => {
                done.stopped = Some(e);
                return done;
            }
            Err(e) => {
                // Last.fm refused the request itself: count a try for each,
                // and give up on a play after a few.
                tracing::warn!(error = %e, "Last.fm refused scrobbles");
                for s in &batch {
                    let state = if s.attempts + 1 >= MAX_ATTEMPTS { ScrobbleState::Failed } else { ScrobbleState::Queued };
                    let _ = store.scrobble_mark(&s.play_id, state, Some(&e.to_string()));
                }
                done.stopped = Some(e);
                return done;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;

    /// Answers from a list, and keeps what was sent.
    #[derive(Default)]
    struct Fake {
        answers: Mutex<Vec<Result<Value, Error>>>,
        sent: Mutex<Vec<Vec<(String, String)>>>,
    }

    impl Fake {
        fn with(answers: Vec<Result<Value, Error>>) -> Arc<Self> {
            Arc::new(Fake { answers: Mutex::new(answers), sent: Mutex::default() })
        }
    }

    #[async_trait]
    impl Transport for Fake {
        async fn post(&self, form: Vec<(String, String)>) -> Result<Value, Error> {
            self.sent.lock().push(form);
            let mut a = self.answers.lock();
            // Error bodies become errors, as `Http` makes them.
            if a.is_empty() { Err(Error::Network) } else { a.remove(0).and_then(|v| api_error(&v).map_or(Ok(v), Err)) }
        }
    }

    fn fixture(name: &str) -> Value {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(name);
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    fn creds() -> Credentials {
        Credentials { api_key: "b25b959554ed76058ac220b7b2e0a026".into(), secret: "SECRET".into() }
    }

    fn get<'a>(form: &'a [(String, String)], k: &str) -> Option<&'a str> {
        form.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    }

    #[test]
    fn signs_like_last_fm() {
        let p = vec![("token".to_string(), "abcdef".to_string()), ("method".into(), "auth.getSession".into()), ("api_key".into(), "b25b959554ed76058ac220b7b2e0a026".into()), ("format".into(), "json".into())];
        // md5("api_key…methodauth.getSessiontokenabcdefSECRET"), format left out.
        assert_eq!(sign(&p, "SECRET"), "f92565fd4dfb212d90d5f73e87da7684");
    }

    #[test]
    fn keys_look_like_keys() {
        assert!(valid_key("b25b959554ed76058ac220b7b2e0a026"));
        assert!(!valid_key("b25b959554ed76058ac220b7b2e0a02"));
        assert!(!valid_key("z25b959554ed76058ac220b7b2e0a026"));
        assert!(!format!("{:?}", creds()).contains("SECRET"), "Debug never prints secrets");
        assert!(!format!("{:?}", Session { username: "rj".into(), key: "sk123".into() }).contains("sk123"));
    }

    #[tokio::test]
    async fn connects_in_two_steps() {
        let fake = Fake::with(vec![Ok(serde_json::json!({ "token": "tok" })), Ok(fixture("error_not_approved.json")), Ok(fixture("session.json"))]);
        let c = Client::new(fake.clone(), creds());
        let token = c.token().await.unwrap();
        assert_eq!(token, "tok");
        assert!(auth_url(&creds().api_key, &token).starts_with("https://www.last.fm/api/auth/?api_key=b25b"));
        let early = c.session(&token).await.unwrap_err();
        assert!(early.not_approved(), "{early:?}");
        let s = c.session(&token).await.unwrap();
        assert_eq!(s.username, "RJ");
        let sent = fake.sent.lock();
        assert_eq!(get(&sent[0], "method"), Some("auth.getToken"));
        assert!(get(&sent[0], "api_sig").is_some());
        assert_eq!(get(&sent[2], "token"), Some("tok"));
        assert!(sent.iter().all(|f| get(f, "secret").is_none() && !f.iter().any(|(_, v)| v == "SECRET")), "the secret is never sent");
    }

    #[tokio::test]
    async fn reads_scrobble_outcomes() {
        let fake = Fake::with(vec![Ok(fixture("scrobble_one.json")), Ok(fixture("scrobble_two.json"))]);
        let c = Client::new(fake.clone(), creds());
        let p = Play { artist: "Lumen Drive".into(), track: "Daybreak Signal".into(), album: Some("Night Roads".into()), album_artist: None, duration: Some(201.4), timestamp: 1_800_000_000 };
        assert_eq!(c.scrobble("sk", std::slice::from_ref(&p)).await.unwrap(), vec![Outcome::Accepted]);
        let two = c.scrobble("sk", &[p.clone(), p]).await.unwrap();
        assert_eq!(two, vec![Outcome::Accepted, Outcome::Ignored("Timestamp too old".into())]);
        let sent = fake.sent.lock();
        assert_eq!(get(&sent[0], "artist[0]"), Some("Lumen Drive"));
        assert_eq!(get(&sent[0], "duration[0]"), Some("201"));
        assert_eq!(get(&sent[0], "sk"), Some("sk"));
        assert_eq!(get(&sent[1], "track[1]"), Some("Daybreak Signal"));
    }

    #[tokio::test]
    async fn reads_top_tracks() {
        let fake = Fake::with(vec![Ok(fixture("toptracks.json"))]);
        let c = Client::new(fake.clone(), creds());
        let (top, pages) = c.top_tracks("rj", 1).await.unwrap();
        assert_eq!(pages, 3);
        assert_eq!(top[0], TopTrack { artist: "Lumen Drive".into(), title: "Daybreak Signal".into(), plays: 42 });
        let rows = play_rows(&top);
        assert_eq!((rows[1].artist_key.as_str(), rows[1].title_key.as_str(), rows[1].plays), ("pale harbor", "glass weather (remastered)", 7));
        assert!(get(&fake.sent.lock()[0], "api_sig").is_none(), "a public read isn't signed");
    }

    #[test]
    fn normalises_for_matching() {
        assert_eq!(match_key("  Daft  Punk "), "daft punk");
        assert_eq!(match_key("Don’t Stop Me Now!"), "dont stop me now");
        assert_eq!(match_key("Glass Weather (Remastered)"), "glass weather (remastered)", "brackets stay, as the sort's keys keep them");
        assert_eq!(match_key("夜行列車"), "夜行列車");
        assert!(scrobblable(None) && scrobblable(Some(31.0)) && !scrobblable(Some(30.0)));
    }

    fn queued(store: &Store, id: &str, ts: i64) {
        store
            .scrobble_enqueue(&Scrobble { play_id: id.into(), owner: "acc".into(), username: "RJ".into(), artist: "A".into(), track: id.into(), album: None, album_artist: None, duration: Some(200.0), timestamp: ts, state: ScrobbleState::Queued, attempts: 0 })
            .unwrap();
    }

    #[tokio::test]
    async fn flush_sends_in_order_and_keeps_what_failed() {
        let store = Store::open_memory().unwrap();
        let session = Session { username: "RJ".into(), key: "sk".into() };
        queued(&store, "p1", 10);
        queued(&store, "p2", 20);
        // Offline: nothing changes.
        let c = Client::new(Fake::with(vec![]), creds());
        let r = flush(&c, &session, &store).await;
        assert_eq!(r.stopped, Some(Error::Network));
        assert_eq!(store.scrobble_counts("RJ").unwrap(), (2, 0));
        // Back: one accepted, one ignored, nothing left.
        let fake = Fake::with(vec![Ok(fixture("scrobble_two.json"))]);
        let c = Client::new(fake.clone(), creds());
        let r = flush(&c, &session, &store).await;
        assert_eq!((r.sent, r.ignored, r.stopped), (1, 1, None));
        assert_eq!(store.scrobble_counts("RJ").unwrap(), (0, 0));
        assert_eq!(get(&fake.sent.lock()[0], "track[0]"), Some("p1"), "oldest first");
        // A dead session stops, and says so.
        queued(&store, "p3", 30);
        let c = Client::new(Fake::with(vec![Ok(fixture("error_session.json"))]), creds());
        let r = flush(&c, &session, &store).await;
        assert!(r.stopped.unwrap().needs_reconnect());
        assert_eq!(store.scrobble_counts("RJ").unwrap(), (1, 0));
    }

    #[tokio::test]
    async fn a_refused_play_fails_after_a_few_tries() {
        let store = Store::open_memory().unwrap();
        let session = Session { username: "RJ".into(), key: "sk".into() };
        queued(&store, "p1", 10);
        for _ in 0..MAX_ATTEMPTS {
            let c = Client::new(Fake::with(vec![Err(Error::Api { code: 6, message: "Invalid parameters".into() })]), creds());
            flush(&c, &session, &store).await;
        }
        assert_eq!(store.scrobble_counts("RJ").unwrap(), (0, 1));
    }
}
