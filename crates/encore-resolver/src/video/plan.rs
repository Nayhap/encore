//! Which clients to ask, in which order (D37).
//!
//! Upstream: yt-dlp `yt_dlp/extractor/youtube/_video.py` at the commit in
//! `upstream.toml`: `_DEFAULT_CLIENTS` and friends (143-147),
//! `_get_requested_clients` (2969-3017) and the fallbacks of
//! `_extract_player_responses` (3047-3174: `append_client`, made for kids,
//! age gate, age verification).
//!
//! Intentional divergences (docs/upstream.md lists them all):
//! * Clients are asked one at a time; the next one only when this one gave
//!   no D5-preferred AAC (yt-dlp asks every client and merges).
//! * `web` is never asked (it is SABR-only now, and needs a GVS PO token),
//!   nor `web_music` signed in (a PO token without Premium, and Encore
//!   doesn't detect Premium); `web_creator` is planned as upstream but
//!   skipped by the PO token policy.
//! * `web_embedded` and `tv` close both lists, as later fallbacks.
//! * The made-for-kids fallback doesn't look for "made for kids" in a
//!   watch page (Encore fetches none): any visionOS refusal moves on.

use std::collections::{HashSet, VecDeque};

/// `_DEFAULT_CLIENTS` without `web`, then Encore's fallbacks.
pub const ANONYMOUS: &[&str] = &["tv_samsung", "visionos", "web_embedded", "tv"];
/// `_DEFAULT_AUTHED_CLIENTS` without `web`, then Encore's fallback. Every
/// one supports cookies, so `_get_requested_clients` drops none.
pub const AUTHENTICATED: &[&str] = &["web_embedded", "tv_samsung", "tv"];

pub struct Plan {
    queue: VecDeque<String>,
    used: HashSet<String>,
    authenticated: bool,
}

impl Plan {
    /// The default plan, or just `only` (`encore-cli resolve --client`).
    pub fn new(authenticated: bool, only: Option<&str>) -> Self {
        let list: Vec<String> = match only {
            Some(c) => vec![c.to_string()],
            None => (if authenticated { AUTHENTICATED } else { ANONYMOUS }).iter().map(|s| s.to_string()).collect(),
        };
        let mut used: HashSet<String> = list.iter().cloned().collect();
        // An explicit client is the only one asked: no fallbacks join it.
        if only.is_some() {
            used.extend(crate::base::clients::INNERTUBE_CLIENTS.keys().cloned());
        }
        Plan { queue: list.into(), used, authenticated }
    }

    pub fn next(&mut self) -> Option<String> {
        self.queue.pop_front()
    }

    /// `append_client`: the first of `names` not asked yet runs next
    /// (upstream pushes it on top of its stack). One still waiting in
    /// Encore's longer list moves up instead.
    fn append(&mut self, names: &[&str]) {
        for n in names {
            if let Some(i) = self.queue.iter().position(|q| q == n) {
                let c = self.queue.remove(i).expect("queued");
                self.queue.push_front(c);
                return;
            }
            if crate::base::client(n).is_some() && !self.used.contains(*n) {
                self.used.insert(n.to_string());
                self.queue.push_front(n.to_string());
                return;
            }
        }
    }

    /// What upstream schedules after a client's answer.
    pub fn after(&mut self, client: &str, agegated: bool, unplayable: bool) {
        // Made for kids on visionOS: embedded web may play it.
        if matches!(client, "android_vr" | "visionos") && unplayable {
            self.append(&["web_embedded"]);
        }
        // `web_embedded.<base>` can work around the age gate.
        if agegated && client != "web_embedded" {
            self.append(&["web_embedded"]);
        }
        // Age verification of a signed-in account: web_creator, which
        // needs a PO token (so it is skipped, and says so).
        let embedding_is_disabled = client == "web_embedded" && unplayable;
        if self.authenticated && (agegated || embedding_is_disabled) {
            self.append(&["web_creator"]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(p: &mut Plan) -> Vec<String> {
        std::iter::from_fn(|| p.next()).collect()
    }

    #[test]
    fn default_orders() {
        assert_eq!(drain(&mut Plan::new(false, None)), ["tv_samsung", "visionos", "web_embedded", "tv"]);
        assert_eq!(drain(&mut Plan::new(true, None)), ["web_embedded", "tv_samsung", "tv"]);
        assert_eq!(drain(&mut Plan::new(true, Some("tv"))), ["tv"]);
        for c in ANONYMOUS.iter().chain(AUTHENTICATED) {
            assert!(crate::base::client(c).is_some(), "{c}");
        }
        assert!(AUTHENTICATED.iter().all(|c| crate::base::client(c).unwrap().supports_cookies));
    }

    #[test]
    fn an_age_gate_moves_embedded_up_and_asks_for_verification_signed_in() {
        let mut p = Plan::new(false, None);
        assert_eq!(p.next().as_deref(), Some("tv_samsung"));
        p.after("tv_samsung", true, false);
        assert_eq!(drain(&mut p), ["web_embedded", "visionos", "tv"]);

        let mut p = Plan::new(true, None);
        assert_eq!(p.next().as_deref(), Some("web_embedded"));
        p.after("web_embedded", true, false);
        assert_eq!(p.next().as_deref(), Some("web_creator"));
        p.after("tv_samsung", true, false);
        assert_eq!(drain(&mut p), ["tv_samsung", "tv"]);

        let mut p = Plan::new(true, Some("tv"));
        p.next();
        p.after("tv", true, true);
        assert!(p.next().is_none(), "an explicit client stays alone");
    }
}
