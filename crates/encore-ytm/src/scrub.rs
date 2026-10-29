//! Fixture scrubbing: removes identifying or session-bound fields from a
//! recorded response before it is written to `fixtures/` (D10). Tokens that
//! parsers need structurally (continuations) are replaced with fake values.

use serde_json::Value;

const DROP: &[&str] = &[
    "responseContext",
    "trackingParams",
    "clickTrackingParams",
    "visitorData",
    "loggingContext",
    "loggingDirectives",
    "serializedContextData",
    "serializedShareEntity",
    "frameworkUpdates",
    "adPlacements",
    "playerAds",
    "attestation",
    "playbackTracking",
    "streamingData",
    "heartbeatParams",
    "accountName",
    "accountPhoto",
    "email",
];

const FAKE: &[&str] = &["continuation", "token", "ctoken", "feedbackToken", "multiSelectParams", "multiSelectItem", "datasyncIdToken", "pageId"];

pub fn scrub(v: &mut Value) {
    match v {
        Value::Object(m) => {
            for k in DROP {
                m.remove(*k);
            }
            for (k, x) in m.iter_mut() {
                if FAKE.contains(&k.as_str()) && x.is_string() {
                    *x = Value::String(format!("SCRUBBED_{}", k.to_ascii_uppercase()));
                    continue;
                }
                if let Value::String(s) = x {
                    if s.contains("googlevideo.com") || s.contains("videoplayback") {
                        *s = "https://example.invalid/scrubbed".into();
                    }
                    continue;
                }
                scrub(x);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(scrub),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn drops_and_fakes() {
        let mut v = json!({"responseContext": {"visitorData": "x"}, "a": {"trackingParams": "t", "continuation": "real", "u": "https://r1.googlevideo.com/videoplayback?sig=1"}});
        scrub(&mut v);
        let s = v.to_string();
        assert!(!s.contains("visitorData") && !s.contains("trackingParams") && !s.contains("real") && !s.contains("sig=1"));
        assert!(s.contains("SCRUBBED_CONTINUATION"));
    }
}
