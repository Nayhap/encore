//! What a player response says about playing the video (D4, D37).
//!
//! Upstream: yt-dlp `yt_dlp/extractor/youtube/_video.py` at the commit in
//! `upstream.toml`: `_is_agegated`, `_is_unplayable`, `_is_error_response`
//! (2894-2913), `_invalid_player_response` (3022), and the "no formats"
//! reasons (4045-4083: `playerErrorMessageRenderer`, the interstitial view
//! model, the geo subreason, "sign in", the captcha view model, the rate
//! limit). yt-dlp turns these into one error text; Encore keeps them as
//! kinds, so a bot check is never read as a sign-out or a removed song.

use serde_json::Value;

/// The reasons YouTube gives, kept as kinds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Playability {
    Ok,
    /// `_is_agegated`: another client (web_embedded) may still play it.
    AgeGated,
    /// A bot check, a captcha or the hour-long rate limit.
    Challenged(Challenge),
    /// The same for every client: never worth asking another one.
    Restricted(Restriction),
    /// Sign-in required for some other reason.
    LoginRequired,
    /// This client can't play it (made for kids, embedding disabled, …);
    /// another client may.
    ClientRefused,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Challenge {
    BotCheck,
    Captcha,
    RateLimit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Restriction {
    Geo,
    Private,
    Removed,
    Premium,
    Members,
    Live,
}

/// `_get_text`: `simpleText` or the joined `runs`.
fn text(v: &Value) -> Option<String> {
    if let Some(s) = v.as_str() {
        return Some(s.to_string());
    }
    if let Some(s) = v.get("simpleText").and_then(Value::as_str) {
        return Some(s.to_string());
    }
    let runs = v.get("runs")?.as_array()?;
    let s: String = runs.iter().filter_map(|r| r.get("text").and_then(Value::as_str)).collect();
    (!s.is_empty()).then_some(s)
}

/// `reason` and `subreason` as yt-dlp reads them (`_video.py:4052-4062`).
pub fn reasons(pr: &Value) -> (String, Option<String>) {
    let ps = &pr["playabilityStatus"];
    let pemr = &ps["errorScreen"]["playerErrorMessageRenderer"];
    let ivm = &ps["errorScreen"]["playerInterstitialRenderer"]["content"]["interstitialViewModel"];
    let reason = text(&pemr["reason"]).or_else(|| ps["reason"].as_str().map(str::to_string)).or_else(|| ivm["title"]["content"].as_str().map(str::to_string)).unwrap_or_default();
    let sub = text(&pemr["subreason"]).or_else(|| ivm["description"]["content"].as_str().map(str::to_string)).filter(|s| !s.is_empty());
    (reason, sub)
}

pub fn status(pr: &Value) -> &str {
    pr["playabilityStatus"]["status"].as_str().unwrap_or("")
}

/// `_is_agegated`.
pub fn is_agegated(pr: &Value) -> bool {
    let ps = &pr["playabilityStatus"];
    // Python truthiness of the field.
    let truthy = |v: &Value| !(v.is_null() || v == &Value::Bool(false) || v.as_f64() == Some(0.0) || v.as_str() == Some("") || v.as_object().is_some_and(|o| o.is_empty()));
    if ps.get("desktopLegacyAgeGateReason").is_some_and(truthy) {
        return true;
    }
    const AGE_GATE_REASONS: [&str; 5] = ["confirm your age", "age-restricted", "inappropriate", "age_verification_required", "age_check_required"];
    let status = status(pr).to_ascii_lowercase();
    let reason = ps["reason"].as_str().unwrap_or("").to_ascii_lowercase();
    AGE_GATE_REASONS.iter().any(|r| status.contains(r) || reason.contains(r))
}

/// `_invalid_player_response`: the id of another video, if YouTube
/// answered with one ("your IP is likely being blocked").
pub fn other_video(pr: &Value, video_id: &str) -> Option<String> {
    match pr["videoDetails"]["videoId"].as_str() {
        Some(id) if id != video_id => Some(id.to_string()),
        _ => None,
    }
}

pub fn classify(pr: &Value) -> Playability {
    let st = status(pr);
    let (reason, sub) = reasons(pr);
    let r = format!("{reason} {}", sub.as_deref().unwrap_or("")).to_ascii_lowercase();
    if st == "OK" {
        if pr["videoDetails"]["isLive"].as_bool() == Some(true) || pr["videoDetails"]["isUpcoming"].as_bool() == Some(true) {
            return Playability::Restricted(Restriction::Live);
        }
        return Playability::Ok;
    }
    if pr["playabilityStatus"]["errorScreen"].get("playerCaptchaViewModel").is_some() {
        return Playability::Challenged(Challenge::Captcha);
    }
    if r.contains("not a bot") || r.contains("confirm you\u{2019}re not") || r.contains("confirm you're not") {
        return Playability::Challenged(Challenge::BotCheck);
    }
    if reason.contains("This content isn't available, try again later") || reason.contains("This content isn\u{2019}t available, try again later") {
        return Playability::Challenged(Challenge::RateLimit);
    }
    if is_agegated(pr) {
        return Playability::AgeGated;
    }
    if sub.as_deref().is_some_and(|s| s.starts_with("The uploader has not made this video available in your country")) || r.contains("not available in your country") {
        return Playability::Restricted(Restriction::Geo);
    }
    if st == "LIVE_STREAM_OFFLINE" || r.contains("premieres in") || r.contains("live event will begin") || r.contains("live stream recording is not available") {
        return Playability::Restricted(Restriction::Live);
    }
    if r.contains("private video") {
        return Playability::Restricted(Restriction::Private);
    }
    if r.contains("music premium") || r.contains("youtube premium") || r.contains("premium members") {
        return Playability::Restricted(Restriction::Premium);
    }
    if r.contains("members-only") || r.contains("members only") || r.contains("join this channel") {
        return Playability::Restricted(Restriction::Members);
    }
    match st {
        // "Video unavailable", removed, terminated, copyright.
        "ERROR" if !r.contains("made for kids") => Playability::Restricted(Restriction::Removed),
        "LOGIN_REQUIRED" => Playability::LoginRequired,
        _ => Playability::ClientRefused,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pr(status: &str, reason: &str) -> Value {
        json!({ "playabilityStatus": { "status": status, "reason": reason }, "videoDetails": { "videoId": "abcdefghijk" } })
    }

    #[test]
    fn statuses_become_kinds() {
        assert_eq!(classify(&json!({ "playabilityStatus": { "status": "OK" } })), Playability::Ok);
        assert_eq!(classify(&pr("LOGIN_REQUIRED", "Sign in to confirm you’re not a bot")), Playability::Challenged(Challenge::BotCheck));
        assert_eq!(classify(&pr("LOGIN_REQUIRED", "Sign in to confirm your age")), Playability::AgeGated);
        assert_eq!(classify(&pr("AGE_VERIFICATION_REQUIRED", "")), Playability::AgeGated);
        assert_eq!(classify(&pr("UNPLAYABLE", "This content isn't available, try again later.")), Playability::Challenged(Challenge::RateLimit));
        assert_eq!(classify(&pr("ERROR", "Video unavailable")), Playability::Restricted(Restriction::Removed));
        assert_eq!(classify(&pr("LOGIN_REQUIRED", "This video is private")), Playability::LoginRequired);
        assert_eq!(classify(&pr("LOGIN_REQUIRED", "Private video")), Playability::Restricted(Restriction::Private));
        assert_eq!(classify(&pr("UNPLAYABLE", "Playback on other websites has been disabled by the video owner")), Playability::ClientRefused);
        assert_eq!(classify(&pr("LIVE_STREAM_OFFLINE", "Premieres in 3 hours")), Playability::Restricted(Restriction::Live));
        assert_eq!(classify(&pr("UNPLAYABLE", "This live stream recording is not available.")), Playability::Restricted(Restriction::Live));
        assert_eq!(classify(&pr("UNPLAYABLE", "The page needs to be reloaded.")), Playability::ClientRefused);
        assert_eq!(classify(&pr("UNPLAYABLE", "This video is only available to Music Premium members")), Playability::Restricted(Restriction::Premium));
        assert_eq!(classify(&pr("UNPLAYABLE", "Join this channel to get access to members-only content")), Playability::Restricted(Restriction::Members));
        let live = json!({ "playabilityStatus": { "status": "OK" }, "videoDetails": { "isLive": true } });
        assert_eq!(classify(&live), Playability::Restricted(Restriction::Live));
    }

    #[test]
    fn error_screens_are_read_like_yt_dlp() {
        let geo = json!({ "playabilityStatus": { "status": "UNPLAYABLE", "reason": "Video unavailable", "errorScreen": { "playerErrorMessageRenderer": {
            "reason": { "simpleText": "Video unavailable" },
            "subreason": { "runs": [{ "text": "The uploader has not made this video available in your country" }] } } } } });
        assert_eq!(classify(&geo), Playability::Restricted(Restriction::Geo));
        assert_eq!(reasons(&geo).1.as_deref(), Some("The uploader has not made this video available in your country"));
        let captcha = json!({ "playabilityStatus": { "status": "LOGIN_REQUIRED", "errorScreen": { "playerCaptchaViewModel": {} } } });
        assert_eq!(classify(&captcha), Playability::Challenged(Challenge::Captcha));
        let ivm = json!({ "playabilityStatus": { "status": "LOGIN_REQUIRED", "errorScreen": { "playerInterstitialRenderer": { "content": { "interstitialViewModel": {
            "title": { "content": "Sign in to confirm you're not a bot" } } } } } } });
        assert_eq!(classify(&ivm), Playability::Challenged(Challenge::BotCheck));
    }

    #[test]
    fn another_videos_answer_is_noticed() {
        assert_eq!(other_video(&pr("OK", ""), "abcdefghijk"), None);
        assert_eq!(other_video(&pr("OK", ""), "zzzzzzzzzzz").as_deref(), Some("abcdefghijk"));
        assert_eq!(other_video(&json!({}), "zzzzzzzzzzz"), None);
    }
}
