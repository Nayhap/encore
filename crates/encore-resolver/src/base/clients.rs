//! The InnerTube clients (D37).
//!
//! Upstream: yt-dlp `yt_dlp/extractor/youtube/_base.py` at the commit in
//! `upstream.toml`: the PO token policy dataclasses (lines 36-69),
//! `INNERTUBE_CLIENTS` (97-405, as data in `innertube_clients.json`) and
//! `build_innertube_clients` (430-458, the defaults below). Keep the
//! upstream names, so a diff of `_base.py` reads straight across.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use serde::Deserialize;
use serde_json::Value;

/// `StreamingProtocol`. Encore only plays https formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamingProtocol {
    Https,
    Dash,
    Hls,
}

/// `GvsPoTokenPolicy` (and the fields of `BasePoTokenPolicy`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct GvsPoTokenPolicy {
    pub required: bool,
    pub recommended: bool,
    pub not_required_for_premium: bool,
    pub not_required_with_player_token: bool,
}

/// `PlayerPoTokenPolicy`, `SubsPoTokenPolicy`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct PoTokenPolicy {
    pub required: bool,
    pub recommended: bool,
    pub not_required_for_premium: bool,
}

/// One `INNERTUBE_CLIENTS` entry after `build_innertube_clients`.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub struct InnertubeClient {
    #[serde(skip)]
    pub key: String,
    #[serde(default = "default_host")]
    pub innertube_host: String,
    /// `{"client": {...}}`, sent as `context` with hl and the time zone added.
    pub innertube_context: Value,
    pub innertube_context_client_name: u32,
    #[serde(default = "yes")]
    pub require_js_player: bool,
    #[serde(default)]
    pub gvs_po_token_policy: BTreeMap<StreamingProtocol, GvsPoTokenPolicy>,
    #[serde(default)]
    pub player_po_token_policy: PoTokenPolicy,
    #[serde(default)]
    pub subs_po_token_policy: PoTokenPolicy,
    #[serde(default)]
    pub require_auth: bool,
    #[serde(default)]
    pub supports_cookies: bool,
    #[serde(default)]
    pub supports_ad_playback_context: bool,
    #[serde(default)]
    pub player_params: Option<String>,
}

fn default_host() -> String {
    "www.youtube.com".into()
}

fn yes() -> bool {
    true
}

impl InnertubeClient {
    pub fn client(&self) -> &Value {
        &self.innertube_context["client"]
    }

    pub fn client_name(&self) -> &str {
        self.client()["clientName"].as_str().unwrap_or("")
    }

    pub fn client_version(&self) -> &str {
        self.client()["clientVersion"].as_str().unwrap_or("")
    }

    pub fn user_agent(&self) -> Option<&str> {
        self.client()["userAgent"].as_str()
    }

    /// The `GVS_PO_TOKEN_POLICY` of a protocol (an empty policy when the
    /// entry has none, as `build_innertube_clients` fills in).
    pub fn gvs_policy(&self, p: StreamingProtocol) -> GvsPoTokenPolicy {
        self.gvs_po_token_policy.get(&p).copied().unwrap_or_default()
    }

    /// `_split_innertube_client(...)[2] == 'embedded'`.
    pub fn is_embedded(&self) -> bool {
        self.key.ends_with("_embedded")
    }
}

/// `gvs_pot_required` (`_video.py:3267`).
pub fn gvs_pot_required(policy: GvsPoTokenPolicy, is_premium_subscriber: bool, has_player_token: bool) -> bool {
    policy.required && !(policy.not_required_with_player_token && has_player_token) && !(policy.not_required_for_premium && is_premium_subscriber)
}

const DATA: &str = include_str!("innertube_clients.json");

/// `INNERTUBE_CLIENTS` with upstream's defaults filled in.
pub static INNERTUBE_CLIENTS: LazyLock<BTreeMap<String, InnertubeClient>> = LazyLock::new(|| {
    let raw: BTreeMap<String, Value> = serde_json::from_str(DATA).expect("innertube_clients.json");
    raw.into_iter()
        .filter(|(k, _)| !k.starts_with('_'))
        .map(|(k, v)| {
            let mut c: InnertubeClient = serde_json::from_value(v).unwrap_or_else(|e| panic!("innertube_clients.json {k}: {e}"));
            // `ytcfg['INNERTUBE_CONTEXT']['client'].setdefault('hl', 'en')`
            if c.innertube_context["client"].get("hl").is_none() {
                c.innertube_context["client"]["hl"] = "en".into();
            }
            // `_fix_embedded_ytcfg`: any valid non-YouTube URL.
            if k.ends_with("_embedded") {
                c.innertube_context["thirdParty"] = serde_json::json!({ "embedUrl": "https://www.reddit.com/" });
            }
            c.key = k.clone();
            (k, c)
        })
        .collect()
});

pub fn client(key: &str) -> Option<&'static InnertubeClient> {
    INNERTUBE_CLIENTS.get(key)
}

/// The commit `innertube_clients.json` was ported at (its `_upstream`).
pub fn data_commit() -> String {
    let raw: Value = serde_json::from_str(DATA).expect("innertube_clients.json");
    raw["_upstream"].as_str().unwrap_or("").rsplit(' ').next().unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_loads_with_upstream_defaults() {
        let tv = client("tv_samsung").unwrap();
        assert_eq!(tv.client_name(), "TVHTML5");
        assert_eq!(tv.innertube_context_client_name, 7);
        assert!(tv.require_js_player && tv.supports_cookies && !tv.require_auth);
        assert_eq!(tv.innertube_host, "www.youtube.com");
        assert_eq!(tv.client()["hl"], "en");
        assert!(!gvs_pot_required(tv.gvs_policy(StreamingProtocol::Https), false, false));

        let vision = client("visionos").unwrap();
        assert!(!vision.require_js_player && !vision.supports_cookies);

        let emb = client("web_embedded").unwrap();
        assert!(emb.is_embedded());
        assert_eq!(emb.innertube_context["thirdParty"]["embedUrl"], "https://www.reddit.com/");

        let music = client("web_music").unwrap();
        assert_eq!(music.innertube_host, "music.youtube.com");
        let https = music.gvs_policy(StreamingProtocol::Https);
        assert!(gvs_pot_required(https, false, false), "web_music needs a GVS token signed in without Premium");
        assert!(!gvs_pot_required(https, true, false), "Premium is exempt");
        assert!(client("web_creator").unwrap().require_auth);
        assert!(gvs_pot_required(client("android").unwrap().gvs_policy(StreamingProtocol::Https), true, false));
        assert!(!gvs_pot_required(client("android").unwrap().gvs_policy(StreamingProtocol::Https), false, true));
    }

    #[test]
    fn the_table_is_from_the_pinned_commit() {
        let pins = include_str!("../../upstream.toml");
        assert!(pins.contains(&format!("commit = \"{}\"", data_commit())), "innertube_clients.json was ported at another yt-dlp commit than upstream.toml pins");
        assert_eq!(INNERTUBE_CLIENTS.len(), 14);
    }
}
