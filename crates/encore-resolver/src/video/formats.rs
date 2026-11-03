//! Audio formats of a player response, the one Encore plays, its URL.
//!
//! Upstream: yt-dlp `yt_dlp/extractor/youtube/_video.py` at the commit in
//! `upstream.toml`: `_extract_formats_and_subtitles` (3227-3620:
//! `get_language_code_and_preference`, `get_stream_id`,
//! `process_format_stream`, `process_https_formats` with its live, OTF,
//! DRM, PO token and SABR checks, the signature and `n` URL rebuild), and
//! `_get_available_at_timestamp` (3822-3859). The choice among formats is
//! Encore's (D5), moved here from the old helper's `pick()`.

use serde_json::Value;

use crate::base::clients::{InnertubeClient, StreamingProtocol, gvs_pot_required};

/// D5: AAC 141 (Premium) and 140 first, then Opus, then low AAC.
pub const PREFERENCE: [u32; 7] = [141, 140, 774, 251, 250, 249, 139];

/// The itags that end the search through clients (D5's AAC).
pub fn preferred(itag: u32) -> bool {
    matches!(itag, 141 | 140)
}

/// One audio-only https format a client offered.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub itag: u32,
    pub mime: String,
    /// `aac`, `opus` or the codec string.
    pub codec: String,
    pub bitrate: Option<u32>,
    pub content_length: Option<u64>,
    pub duration: Option<f64>,
    /// The URL without its signature (with `n` still to transform).
    pub url: String,
    /// `signatureCipher`'s `s` and `sp`.
    pub s: Option<String>,
    pub sp: String,
    pub n: Option<String>,
    pub is_drc: bool,
    pub language_preference: i32,
}

impl Candidate {
    pub fn decodable(&self) -> bool {
        (self.mime == "audio/mp4" && self.codec == "aac") || (self.mime == "audio/webm" && self.codec == "opus")
    }

    /// Lower is better: the D5 order, then anything else decodable.
    pub fn rank(&self) -> (usize, i64) {
        let pos = PREFERENCE.iter().position(|&i| i == self.itag).unwrap_or(PREFERENCE.len());
        (pos, -(self.bitrate.unwrap_or(0) as i64))
    }
}

/// Why formats of a response were left out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Skipped {
    /// No `url` and no `signatureCipher`: SABR only.
    pub sabr: usize,
    pub drm: usize,
    /// A GVS PO token would be needed and Encore has none.
    pub po_token: usize,
    /// Live or OTF formats.
    pub live: usize,
}

/// `get_language_code_and_preference`.
fn language_preference(f: &Value) -> i32 {
    let track = &f["audioTrack"];
    let name = track["displayName"].as_str().unwrap_or("").to_ascii_lowercase();
    if name.contains("descriptive") {
        -10
    } else if name.contains("original") {
        10
    } else if track["audioIsDefault"].as_bool() == Some(true) {
        5
    } else {
        -1
    }
}

/// `mimeType` → (`audio/mp4`, `aac`).
fn mime_codec(m: &str) -> (String, String) {
    let (mime, rest) = m.split_once(';').unwrap_or((m, ""));
    let codecs = rest.split_once("codecs=").map(|(_, c)| c.trim().trim_matches('"')).unwrap_or("");
    let codec = if codecs.starts_with("mp4a") {
        "aac".to_string()
    } else if codecs.starts_with("opus") {
        "opus".to_string()
    } else {
        codecs.to_string()
    };
    (mime.trim().to_string(), codec)
}

/// The audio-only https formats of a response, with what was left out.
pub fn candidates(pr: &Value, client: &InnertubeClient, is_premium_subscriber: bool) -> (Vec<Candidate>, Skipped) {
    let mut out = Vec::new();
    let mut skipped = Skipped::default();
    let policy = client.gvs_policy(StreamingProtocol::Https);
    let video_secs = pr["videoDetails"]["lengthSeconds"].as_str().and_then(|s| s.parse::<f64>().ok());
    let Some(list) = pr["streamingData"]["adaptiveFormats"].as_array() else { return (out, skipped) };
    for f in list {
        let (mime, codec) = mime_codec(f["mimeType"].as_str().unwrap_or(""));
        if !mime.starts_with("audio/") {
            continue;
        }
        if f.get("targetDurationSec").is_some() || f["type"].as_str() == Some("FORMAT_STREAM_TYPE_OTF") {
            skipped.live += 1;
            continue;
        }
        if f["drmFamilies"].as_array().is_some_and(|a| !a.is_empty()) {
            skipped.drm += 1;
            continue;
        }
        let itag = f["itag"].as_u64().unwrap_or(0) as u32;
        if itag != 18 && gvs_pot_required(policy, is_premium_subscriber, false) {
            skipped.po_token += 1;
            continue;
        }
        let (url, s, sp) = match f["url"].as_str() {
            Some(u) => (u.to_string(), None, "signature".to_string()),
            None => {
                let sc = f["signatureCipher"].as_str().unwrap_or("");
                let q: Vec<(String, String)> = url::form_urlencoded::parse(sc.as_bytes()).into_owned().collect();
                let get = |k: &str| q.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
                match (get("url"), get("s")) {
                    (Some(u), Some(s)) => (u, Some(s), get("sp").unwrap_or_else(|| "signature".into())),
                    _ => {
                        skipped.sabr += 1;
                        continue;
                    }
                }
            }
        };
        let n = url::Url::parse(&url).ok().and_then(|u| u.query_pairs().find(|(k, _)| k == "n").map(|(_, v)| v.into_owned()));
        let duration = f["approxDurationMs"].as_str().and_then(|s| s.parse::<f64>().ok()).map(|ms| ms / 1000.0).or(video_secs);
        out.push(Candidate {
            itag,
            mime,
            codec,
            bitrate: f["averageBitrate"].as_u64().or_else(|| f["bitrate"].as_u64()).map(|b| (b / 1000) as u32),
            content_length: f["contentLength"].as_str().and_then(|s| s.parse().ok()),
            duration,
            url,
            s,
            sp,
            n,
            is_drc: f["isDrc"].as_bool() == Some(true),
            language_preference: language_preference(f),
        });
    }
    (out, skipped)
}

/// D5's choice: no dynamic-range-compressed copies while plain ones exist,
/// the original (else default) audio track, then the itag order, else the
/// highest bitrate Encore decodes.
pub fn pick(cands: &[Candidate]) -> Option<&Candidate> {
    let plain: Vec<&Candidate> = cands.iter().filter(|c| !c.is_drc).collect();
    let pool: Vec<&Candidate> = if plain.is_empty() { cands.iter().collect() } else { plain };
    let best_lang = pool.iter().map(|c| c.language_preference).max()?;
    let pool: Vec<&Candidate> = pool.into_iter().filter(|c| c.language_preference == best_lang).collect();
    for itag in PREFERENCE {
        if let Some(c) = pool.iter().find(|c| c.itag == itag) {
            return Some(c);
        }
    }
    pool.into_iter().filter(|c| c.decodable()).max_by_key(|c| c.bitrate.unwrap_or(0))
}

/// The playable URL: the signature appended under `sp`, `n` replaced
/// (`_video.py:3560-3580`). Only https URLs on googlevideo.com come out.
pub fn build_url(c: &Candidate, sig: Option<&str>, n: Option<&str>) -> Option<String> {
    let mut u = url::Url::parse(&c.url).ok()?;
    if u.scheme() != "https" || !u.host_str().is_some_and(|h| h.ends_with(".googlevideo.com")) || u.port().is_some() {
        return None;
    }
    let pairs: Vec<(String, String)> = u.query_pairs().into_owned().collect();
    {
        let mut q = u.query_pairs_mut();
        q.clear();
        for (k, v) in &pairs {
            if k == "n" {
                if let Some(n) = n {
                    q.append_pair(k, n);
                    continue;
                }
            }
            q.append_pair(k, v);
        }
        if let Some(sig) = sig {
            q.append_pair(&c.sp, sig);
        }
    }
    Some(u.into())
}

/// The URL's `expire` (unix seconds).
pub fn expires(url: &str) -> Option<i64> {
    url::Url::parse(url).ok()?.query_pairs().find(|(k, _)| k == "expire").and_then(|(_, v)| v.parse().ok())
}

/// `_get_available_at_timestamp`: seconds of pre-roll ads (or until they
/// may be skipped) before a URL from this response answers.
pub fn preroll_seconds(pr: &Value) -> u64 {
    let mut renderers: Vec<&Value> = Vec::new();
    for p in pr["adPlacements"].as_array().into_iter().flatten() {
        if p["adPlacementRenderer"]["config"]["adPlacementConfig"]["kind"].as_str() == Some("AD_PLACEMENT_KIND_START") {
            renderers.push(&p["adPlacementRenderer"]["renderer"]);
        }
    }
    for s in pr["adSlots"].as_array().into_iter().flatten() {
        let slot = &s["adSlotRenderer"];
        if slot["adSlotMetadata"]["triggerEvent"].as_str() != Some("SLOT_TRIGGER_EVENT_BEFORE_CONTENT") {
            continue;
        }
        let rc = &slot["fulfillmentContent"]["fulfilledLayout"]["playerBytesAdLayoutRenderer"]["renderingContent"];
        renderers.push(rc);
        for l in rc["playerBytesSequentialLayoutRenderer"]["sequentialLayouts"].as_array().into_iter().flatten() {
            renderers.push(&l["playerBytesAdLayoutRenderer"]["renderingContent"]);
        }
    }
    let mut wait = 0.0f64;
    for r in renderers {
        let ad = &r["instreamVideoAdRenderer"];
        if !ad.is_object() {
            continue;
        }
        let length = ad["playerVars"].as_str().and_then(|pv| url::form_urlencoded::parse(pv.as_bytes()).filter(|(k, _)| k == "length_seconds").last().and_then(|(_, v)| v.parse::<f64>().ok()));
        let skip = ad["skipOffsetMilliseconds"].as_f64().or_else(|| ad["skipOffsetMilliseconds"].as_str().and_then(|s| s.parse().ok())).map(|ms| ms / 1000.0);
        if let Some(d) = skip.or(length) {
            wait += d;
        }
    }
    wait.ceil() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fmt(itag: u32, mime: &str, extra: Value) -> Value {
        let mut f = json!({ "itag": itag, "mimeType": mime, "bitrate": 130000, "contentLength": "3000000", "approxDurationMs": "180000",
            "url": format!("https://rr1---sn-x.googlevideo.com/videoplayback?expire=1800000000&itag={itag}&n=abcdef&sparams=expire%2Citag") });
        for (k, v) in extra.as_object().unwrap() {
            f[k] = v.clone();
        }
        f
    }

    fn pr(formats: Vec<Value>) -> Value {
        json!({ "streamingData": { "adaptiveFormats": formats }, "videoDetails": { "lengthSeconds": "180" } })
    }

    const AAC: &str = "audio/mp4; codecs=\"mp4a.40.2\"";
    const OPUS: &str = "audio/webm; codecs=\"opus\"";

    #[test]
    fn aac_140_is_picked_over_opus_and_drc() {
        let r = pr(vec![
            fmt(251, OPUS, json!({})),
            fmt(140, AAC, json!({ "isDrc": true })),
            fmt(140, AAC, json!({})),
            fmt(137, "video/mp4; codecs=\"avc1\"", json!({})),
        ]);
        let (c, _) = candidates(&r, crate::base::client("tv_samsung").unwrap(), false);
        assert_eq!(c.len(), 3);
        let p = pick(&c).unwrap();
        assert_eq!((p.itag, p.is_drc, p.codec.as_str(), p.mime.as_str()), (140, false, "aac", "audio/mp4"));
        assert_eq!(p.n.as_deref(), Some("abcdef"));
        assert_eq!(p.content_length, Some(3_000_000));
    }

    #[test]
    fn the_original_track_wins_over_a_dub() {
        let r = pr(vec![
            fmt(140, AAC, json!({ "audioTrack": { "id": "de.3", "displayName": "German" } })),
            fmt(251, OPUS, json!({ "audioTrack": { "id": "en.4", "displayName": "English original", "audioIsDefault": true } })),
        ]);
        let (c, _) = candidates(&r, crate::base::client("tv_samsung").unwrap(), false);
        assert_eq!(pick(&c).unwrap().itag, 251);
    }

    #[test]
    fn sabr_drm_live_and_po_token_formats_are_left_out() {
        let r = pr(vec![
            json!({ "itag": 140, "mimeType": AAC }),
            fmt(141, AAC, json!({ "drmFamilies": ["WIDEVINE"] })),
            fmt(251, OPUS, json!({ "targetDurationSec": 5 })),
        ]);
        let (c, s) = candidates(&r, crate::base::client("tv_samsung").unwrap(), false);
        assert!(c.is_empty());
        assert_eq!(s, Skipped { sabr: 1, drm: 1, po_token: 0, live: 1 });
        let (c, s) = candidates(&pr(vec![fmt(140, AAC, json!({}))]), crate::base::client("web_music").unwrap(), false);
        assert!(c.is_empty() && s.po_token == 1);
        let (c, _) = candidates(&pr(vec![fmt(140, AAC, json!({}))]), crate::base::client("web_music").unwrap(), true);
        assert_eq!(c.len(), 1, "Premium is exempt from web_music's GVS token");
    }

    #[test]
    fn a_ciphered_url_gets_its_signature_and_n() {
        let sc = "s=ABCDEF%3D&sp=sig&url=https%3A%2F%2Frr1---sn-x.googlevideo.com%2Fvideoplayback%3Fexpire%3D1800000000%26n%3Dnnnn%26lsig%3DAB%253D%253D";
        let r = pr(vec![json!({ "itag": 140, "mimeType": AAC, "signatureCipher": sc })]);
        let (c, _) = candidates(&r, crate::base::client("tv_samsung").unwrap(), false);
        let c = &c[0];
        assert_eq!((c.s.as_deref(), c.sp.as_str(), c.n.as_deref()), (Some("ABCDEF="), "sig", Some("nnnn")));
        let u = build_url(c, Some("FEDCBA="), Some("solved")).unwrap();
        let q: Vec<(String, String)> = url::Url::parse(&u).unwrap().query_pairs().into_owned().collect();
        assert!(q.contains(&("n".into(), "solved".into())));
        assert!(q.contains(&("sig".into(), "FEDCBA=".into())));
        assert!(q.contains(&("lsig".into(), "AB==".into())), "other parameters keep their value: {q:?}");
        assert_eq!(expires(&u), Some(1_800_000_000));
    }

    #[test]
    fn media_urls_must_be_googlevideo_https() {
        let mut c = candidates(&pr(vec![fmt(140, AAC, json!({}))]), crate::base::client("tv_samsung").unwrap(), false).0.remove(0);
        assert!(build_url(&c, None, None).is_some());
        c.url = "http://rr1---sn-x.googlevideo.com/videoplayback".into();
        assert!(build_url(&c, None, None).is_none());
        c.url = "https://evil.example/videoplayback".into();
        assert!(build_url(&c, None, None).is_none());
    }

    #[test]
    fn preroll_ads_delay_the_url() {
        let r = json!({ "adPlacements": [
            { "adPlacementRenderer": { "config": { "adPlacementConfig": { "kind": "AD_PLACEMENT_KIND_START" } },
              "renderer": { "instreamVideoAdRenderer": { "playerVars": "length_seconds=31&x=1", "skipOffsetMilliseconds": 5000 } } } },
            { "adPlacementRenderer": { "config": { "adPlacementConfig": { "kind": "AD_PLACEMENT_KIND_END" } },
              "renderer": { "instreamVideoAdRenderer": { "playerVars": "length_seconds=99" } } } } ],
            "adSlots": [{ "adSlotRenderer": { "adSlotMetadata": { "triggerEvent": "SLOT_TRIGGER_EVENT_BEFORE_CONTENT" },
              "fulfillmentContent": { "fulfilledLayout": { "playerBytesAdLayoutRenderer": { "renderingContent": {
                "instreamVideoAdRenderer": { "playerVars": "length_seconds=20" } } } } } } }] });
        assert_eq!(preroll_seconds(&r), 25);
        assert_eq!(preroll_seconds(&json!({})), 0);
    }
}
