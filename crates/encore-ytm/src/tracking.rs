//! Plays reported to YouTube Music history (D18).
//!
//! Encore makes its own signed-in `WEB_REMIX` `/player` call for a play that
//! became audible and reads only `playbackTracking` from it (streams still
//! come from yt-dlp, D3). The `playback` URL registers the play; `watchtime`
//! URLs report the stretches actually heard. Both keep every parameter the
//! service put in them and only set the ones the player fills in.
//!
//! Tracking URLs carry account-bound event ids: they are secret-ish, never
//! logged, never sent to the UI and never stored.

use std::sync::Arc;

use async_trait::async_trait;
use encore_core::error::{AppError, Result};
use encore_core::service::{FlushSchedule, Ping, PlayReport};
use serde_json::Value;
use url::Url;

use crate::client::{Identity, InnerTube, stats_url_ok};

/// The tracking part of one signed-in `/player` answer.
#[derive(Clone)]
pub struct PlaybackTracking {
    pub playback: Url,
    pub watchtime: Option<Url>,
    pub schedule: FlushSchedule,
}

impl std::fmt::Debug for PlaybackTracking {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlaybackTracking").field("playback", &self.playback.path()).field("watchtime", &self.watchtime.as_ref().map(|u| u.path())).field("schedule", &self.schedule).finish()
    }
}

fn base_url(pt: &Value, key: &str) -> Option<Url> {
    let u = Url::parse(pt.get(key)?.get("baseUrl")?.as_str()?).ok()?;
    stats_url_ok(&u).then_some(u)
}

fn seconds(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok())).filter(|x| x.is_finite() && *x > 0.0)
}

/// The flush settings, wherever this response keeps them (in
/// `playbackTracking`, or in `playerConfig`'s stats config).
fn schedule_of(resp: &Value, pt: &Value) -> FlushSchedule {
    let pick = |key: &str| pt.get(key).or_else(|| resp.get("playerConfig").and_then(|c| crate::parse::find(c, key)));
    let mut s = FlushSchedule::default();
    if let Some(a) = pick("videostatsScheduledFlushWalltimeSeconds").and_then(Value::as_array) {
        let at: Vec<f64> = a.iter().filter_map(seconds).collect();
        if !at.is_empty() && at.windows(2).all(|w| w[0] < w[1]) {
            s.at = at;
        }
    }
    if let Some(e) = pick("videostatsDefaultFlushIntervalSeconds").and_then(seconds) {
        s.every = e.max(5.0);
    }
    s
}

impl PlaybackTracking {
    /// `None` when the video isn't playable for this account or the answer
    /// has no usable playback URL (a verdict, not an error).
    pub fn from_player(resp: &Value) -> Option<Self> {
        let status = resp.get("playabilityStatus").and_then(|p| p.get("status")).and_then(Value::as_str);
        if status != Some("OK") {
            return None;
        }
        let pt = resp.get("playbackTracking")?;
        Some(PlaybackTracking { playback: base_url(pt, "videostatsPlaybackUrl")?, watchtime: base_url(pt, "videostatsWatchtimeUrl"), schedule: schedule_of(resp, pt) })
    }
}

/// A client playback nonce: 16 random characters of the URL-safe alphabet,
/// one per play.
pub fn cpn() -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let r = uuid::Uuid::new_v4().into_bytes();
    let r2 = uuid::Uuid::new_v4().into_bytes();
    r.iter().chain(r2.iter()).take(16).map(|b| ABC[(*b & 63) as usize] as char).collect()
}

fn secs(x: f64) -> String {
    format!("{:.3}", x.max(0.0))
}

/// Replace (or add) query parameters, keeping the rest in order.
fn set_params(base: &Url, set: &[(&str, String)]) -> Url {
    let keep: Vec<(String, String)> = base.query_pairs().filter(|(k, _)| !set.iter().any(|(s, _)| s == k)).map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
    let mut u = base.clone();
    {
        let mut q = u.query_pairs_mut();
        q.clear();
        for (k, v) in &keep {
            q.append_pair(k, v);
        }
        for (k, v) in set {
            q.append_pair(k, v);
        }
    }
    u
}

/// The URL for one ping of a play.
pub fn ping_url(t: &PlaybackTracking, ping: &Ping, cpn: &str, version: &str, itag: Option<u32>) -> Option<Url> {
    let mut p: Vec<(&str, String)> = vec![("ver", "2".into()), ("cpn", cpn.into()), ("c", "WEB_REMIX".into()), ("cver", version.into())];
    if let Some(i) = itag.filter(|i| *i > 0) {
        p.push(("fmt", i.to_string()));
    }
    match ping {
        Ping::Playback { cmt, rt } => {
            p.push(("cmt", secs(*cmt)));
            p.push(("rt", secs(*rt)));
            Some(set_params(&t.playback, &p))
        }
        Ping::Watchtime { st, et, cmt, rt, playing, last } => {
            let base = t.watchtime.as_ref()?;
            if st.is_empty() || st.len() != et.len() {
                return None;
            }
            p.push(("st", st.iter().map(|x| secs(*x)).collect::<Vec<_>>().join(",")));
            p.push(("et", et.iter().map(|x| secs(*x)).collect::<Vec<_>>().join(",")));
            p.push(("cmt", secs(*cmt)));
            p.push(("rt", secs(*rt)));
            p.push(("state", if *playing { "playing" } else { "paused" }.into()));
            if *last {
                p.push(("final", "1".into()));
            }
            Some(set_params(base, &p))
        }
    }
}

/// One play being reported, pinned to the identity it started under.
pub struct YtReport {
    pub(crate) it: Arc<InnerTube>,
    pub(crate) id: Identity,
    pub(crate) tracking: PlaybackTracking,
    pub(crate) cpn: String,
}

#[async_trait]
impl PlayReport for YtReport {
    fn schedule(&self) -> FlushSchedule {
        self.tracking.schedule.clone()
    }

    async fn send(&self, ping: &Ping, itag: Option<u32>) -> Result<()> {
        // A switch or sign-out since the play began: the play's reports stop
        // (they never move to the new identity, and the old one is done).
        if self.id.epoch() != self.it.epoch() {
            return Ok(());
        }
        let Some(url) = ping_url(&self.tracking, ping, &self.cpn, self.id.version(), itag) else { return Ok(()) };
        let status = self.it.stats_get(&self.id, &url).await?;
        if (200..300).contains(&status) { Ok(()) } else { Err(AppError::network(format!("The play report was refused ({status})."))) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tracking() -> PlaybackTracking {
        PlaybackTracking::from_player(&json!({
            "playabilityStatus": { "status": "OK" },
            "playbackTracking": {
                "videostatsPlaybackUrl": { "baseUrl": "https://s.youtube.com/api/stats/playback?cl=1&docid=vid&ei=EVENT&fexp=1&ns=yt&plid=P&el=detailpage&len=200&of=OF&vm=VM" },
                "videostatsWatchtimeUrl": { "baseUrl": "https://s.youtube.com/api/stats/watchtime?cl=1&docid=vid&ei=EVENT&ns=yt&el=detailpage&len=200&of=OF&vm=VM&cmt=999" },
                "videostatsScheduledFlushWalltimeSeconds": [10, 20, 30],
                "videostatsDefaultFlushIntervalSeconds": 40
            }
        }))
        .unwrap()
    }

    fn param(u: &Url, k: &str) -> Option<String> {
        u.query_pairs().find(|(x, _)| x == k).map(|(_, v)| v.into_owned())
    }

    #[test]
    fn reads_tracking_and_schedule() {
        let t = tracking();
        assert_eq!(t.playback.path(), "/api/stats/playback");
        assert!(t.watchtime.is_some());
        assert_eq!(t.schedule, FlushSchedule { at: vec![10.0, 20.0, 30.0], every: 40.0 });
        // Debug never shows the event ids.
        assert!(!format!("{t:?}").contains("EVENT"));
    }

    #[test]
    fn schedule_from_player_config_or_default() {
        let t = PlaybackTracking::from_player(&json!({
            "playabilityStatus": { "status": "OK" },
            "playbackTracking": { "videostatsPlaybackUrl": { "baseUrl": "https://s.youtube.com/api/stats/playback?docid=v" } },
            "playerConfig": { "audioConfig": {}, "statsConfig": { "videostatsScheduledFlushWalltimeSeconds": ["5", "15"], "videostatsDefaultFlushIntervalSeconds": "30" } }
        }))
        .unwrap();
        assert_eq!(t.schedule, FlushSchedule { at: vec![5.0, 15.0], every: 30.0 });
        assert!(t.watchtime.is_none());
        let t = PlaybackTracking::from_player(&json!({ "playabilityStatus": { "status": "OK" }, "playbackTracking": { "videostatsPlaybackUrl": { "baseUrl": "https://s.youtube.com/api/stats/playback" } } })).unwrap();
        assert_eq!(t.schedule, FlushSchedule::default());
    }

    #[test]
    fn unplayable_or_foreign_urls_are_not_reportable() {
        assert!(PlaybackTracking::from_player(&json!({ "playabilityStatus": { "status": "UNPLAYABLE" } })).is_none());
        assert!(PlaybackTracking::from_player(&json!({ "playabilityStatus": { "status": "OK" } })).is_none());
        let evil = json!({ "playabilityStatus": { "status": "OK" }, "playbackTracking": { "videostatsPlaybackUrl": { "baseUrl": "https://evil.example/api/stats/playback?docid=v" } } });
        assert!(PlaybackTracking::from_player(&evil).is_none());
    }

    #[test]
    fn cpn_shape() {
        let a = cpn();
        assert_eq!(a.len(), 16);
        assert!(a.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
        assert_ne!(a, cpn());
    }

    #[test]
    fn playback_ping_keeps_service_params_and_sets_ours() {
        let t = tracking();
        let u = ping_url(&t, &Ping::Playback { cmt: 2.04, rt: 2.5 }, "NONCE", "1.2026", Some(140)).unwrap();
        for k in ["cl", "docid", "ei", "fexp", "ns", "plid", "el", "len", "of", "vm"] {
            assert!(param(&u, k).is_some(), "kept {k}");
        }
        assert_eq!(param(&u, "el").as_deref(), Some("detailpage"));
        assert_eq!(param(&u, "ver").as_deref(), Some("2"));
        assert_eq!(param(&u, "cpn").as_deref(), Some("NONCE"));
        assert_eq!(param(&u, "c").as_deref(), Some("WEB_REMIX"));
        assert_eq!(param(&u, "cver").as_deref(), Some("1.2026"));
        assert_eq!(param(&u, "cmt").as_deref(), Some("2.040"));
        assert_eq!(param(&u, "rt").as_deref(), Some("2.500"));
        assert_eq!(param(&u, "fmt").as_deref(), Some("140"));
    }

    #[test]
    fn watchtime_ping_lists_heard_segments_without_duplicate_keys() {
        let t = tracking();
        let ping = Ping::Watchtime { st: vec![0.0, 60.0], et: vec![4.0, 63.0], cmt: 63.0, rt: 70.0, playing: false, last: true };
        let u = ping_url(&t, &ping, "NONCE", "1.2026", None).unwrap();
        assert_eq!(param(&u, "st").as_deref(), Some("0.000,60.000"));
        assert_eq!(param(&u, "et").as_deref(), Some("4.000,63.000"));
        assert_eq!(param(&u, "state").as_deref(), Some("paused"));
        assert_eq!(param(&u, "final").as_deref(), Some("1"));
        assert_eq!(u.query_pairs().filter(|(k, _)| k == "cmt").count(), 1, "the base cmt is replaced");
        assert!(param(&u, "fmt").is_none());
        let empty = Ping::Watchtime { st: vec![], et: vec![], cmt: 0.0, rt: 1.0, playing: true, last: false };
        assert!(ping_url(&t, &empty, "N", "v", None).is_none());
    }
}
