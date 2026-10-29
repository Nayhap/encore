//! Playlist writes (D20): request bodies and how to read the answers. Pure
//! functions, tested on synthetic fixtures; `lib.rs` sends them.
//!
//! Edits address a playlist by its bare id (no `VL` browse prefix) and a row
//! by its `setVideoId` (D10). A 200 is not proof: each endpoint's own success
//! shape decides between `Done` and `Rejected`, and a lost response is
//! `OutcomeUnknown`, never retried automatically.

use encore_core::model::*;
use serde_json::{Value, json};

/// Titles YouTube Music refuses (it rejects `<` and `>`), checked before
/// anything is sent.
pub fn check_title(title: &str) -> Result<String, &'static str> {
    let t = title.trim();
    if t.is_empty() {
        return Err("Give the playlist a name.");
    }
    if t.contains('<') || t.contains('>') {
        return Err("Playlist names can't contain < or >.");
    }
    if t.chars().count() > 150 {
        return Err("Playlist names can be up to 150 characters.");
    }
    Ok(t.to_string())
}

pub fn check_description(d: &str) -> Result<String, &'static str> {
    if d.contains('<') || d.contains('>') {
        return Err("Descriptions can't contain < or >.");
    }
    if d.chars().count() > 5000 {
        return Err("Descriptions can be up to 5000 characters.");
    }
    Ok(d.trim().to_string())
}

/// `VLPL…` → `PL…`. Only the browse prefix is removed.
pub fn bare(playlist_id: &str) -> &str {
    playlist_id.strip_prefix("VL").unwrap_or(playlist_id)
}

/// `playlist/create`. Songs given here are added in the same request, so
/// "New playlist…" from "Add to playlist" is one write.
pub fn create_body(title: &str, description: &str, privacy: Privacy, video_ids: &[&str]) -> Value {
    let mut b = json!({ "title": title, "description": description, "privacyStatus": privacy.wire() });
    if !video_ids.is_empty() {
        b["videoIds"] = json!(video_ids);
    }
    b
}

/// `playlist/create` answers with the new id at the top level. Anything else
/// (a dialog asking for something first) is a refusal.
pub fn created(resp: &Value) -> Option<String> {
    resp.get("playlistId").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

/// `browse/edit_playlist` actions for the details that change.
pub fn edit_actions(edit: &PlaylistEdit) -> Vec<Value> {
    let mut a = Vec::new();
    if let Some(t) = &edit.title {
        a.push(json!({ "action": "ACTION_SET_PLAYLIST_NAME", "playlistName": t }));
    }
    if let Some(d) = &edit.description {
        a.push(json!({ "action": "ACTION_SET_PLAYLIST_DESCRIPTION", "playlistDescription": d }));
    }
    if let Some(p) = edit.privacy {
        a.push(json!({ "action": "ACTION_SET_PLAYLIST_PRIVACY", "playlistPrivacy": p.wire() }));
    }
    a
}

/// Add actions. `DEDUPE_OPTION_CHECK` makes the service refuse a song that is
/// already there; `DEDUPE_OPTION_SKIP` skips that check ("Add anyway").
pub fn add_actions(video_ids: &[&str], allow_duplicates: bool) -> Vec<Value> {
    let dedupe = if allow_duplicates { "DEDUPE_OPTION_SKIP" } else { "DEDUPE_OPTION_CHECK" };
    video_ids.iter().map(|v| json!({ "action": "ACTION_ADD_VIDEO", "addedVideoId": v, "dedupeOption": dedupe })).collect()
}

/// Move one row before another (by `setVideoId`), or to the end.
pub fn move_action(set_video_id: &str, before: Option<&str>) -> Value {
    let mut a = json!({ "action": "ACTION_MOVE_VIDEO_BEFORE", "setVideoId": set_video_id });
    if let Some(b) = before {
        a["movedSetVideoIdSuccessor"] = json!(b);
    }
    a
}

pub fn edit_body(playlist_id: &str, actions: Vec<Value>) -> Value {
    json!({ "playlistId": bare(playlist_id), "actions": actions })
}

/// The answer to `browse/edit_playlist`.
///
/// `STATUS_SUCCEEDED` is done. A refused add of a song that is already in
/// the playlist comes back as `STATUS_FAILED` with a confirm dialog whose
/// button re-sends the add with `DEDUPE_OPTION_SKIP`: that is `Duplicate`.
/// Any other failure is `Rejected`.
pub fn edit_outcome(resp: &Value) -> MutationOutcome {
    match resp.get("status").and_then(Value::as_str) {
        Some("STATUS_SUCCEEDED") => MutationOutcome::Done,
        Some(_) if offers_add_anyway(resp) => MutationOutcome::Duplicate,
        Some(_) => MutationOutcome::Rejected,
        // Without a status the write can't be confirmed.
        None => MutationOutcome::OutcomeUnknown,
    }
}

fn offers_add_anyway(resp: &Value) -> bool {
    let mut eps = Vec::new();
    crate::parse::find_all(resp.get("actions").unwrap_or(&Value::Null), "playlistEditEndpoint", &mut eps);
    eps.iter().any(|ep| {
        ep.get("actions").and_then(Value::as_array).is_some_and(|a| {
            a.iter().any(|x| x.get("action").and_then(Value::as_str) == Some("ACTION_ADD_VIDEO") && x.get("dedupeOption").and_then(Value::as_str) == Some("DEDUPE_OPTION_SKIP"))
        })
    })
}

/// `playlist/delete` answers with an (almost) empty object. An `actions`
/// list means the service showed something instead (a refusal).
pub fn delete_outcome(resp: &Value) -> MutationOutcome {
    if resp.get("actions").and_then(Value::as_array).is_some_and(|a| !a.is_empty()) {
        return MutationOutcome::Rejected;
    }
    match resp.get("status").and_then(Value::as_str) {
        Some("STATUS_SUCCEEDED") | None => MutationOutcome::Done,
        Some(_) => MutationOutcome::Rejected,
    }
}

/// Privacy of an editable playlist, from its edit header.
pub fn privacy_of(resp: &Value) -> Option<Privacy> {
    let h = crate::parse::find(resp.get("contents")?, "musicEditablePlaylistDetailHeaderRenderer")?;
    let p = crate::parse::find(h, "musicPlaylistEditHeaderRenderer")?.get("privacy")?.as_str()?;
    Privacy::from_wire(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_are_checked_before_sending() {
        assert_eq!(check_title("  Road trip "), Ok("Road trip".into()));
        assert!(check_title("   ").is_err());
        assert!(check_title("a <b>").is_err());
        assert!(check_title(&"x".repeat(151)).is_err());
        assert_eq!(check_description(" notes "), Ok("notes".into()));
        assert_eq!(check_description(""), Ok(String::new()));
    }

    #[test]
    fn create_body_seeds_songs_and_defaults_nothing() {
        let b = create_body("Mix", "", Privacy::Private, &["aaaaaaaaaaa"]);
        assert_eq!(b, json!({ "title": "Mix", "description": "", "privacyStatus": "PRIVATE", "videoIds": ["aaaaaaaaaaa"] }));
        let b = create_body("Mix", "d", Privacy::Unlisted, &[]);
        assert!(b.get("videoIds").is_none());
        assert_eq!(b["privacyStatus"], "UNLISTED");
    }

    #[test]
    fn edit_actions_only_for_changed_fields() {
        assert!(edit_actions(&PlaylistEdit::default()).is_empty());
        let a = edit_actions(&PlaylistEdit { title: Some("New".into()), description: Some(String::new()), privacy: Some(Privacy::Public) });
        assert_eq!(a[0], json!({ "action": "ACTION_SET_PLAYLIST_NAME", "playlistName": "New" }));
        // An empty description is sent: it clears the old one.
        assert_eq!(a[1], json!({ "action": "ACTION_SET_PLAYLIST_DESCRIPTION", "playlistDescription": "" }));
        assert_eq!(a[2], json!({ "action": "ACTION_SET_PLAYLIST_PRIVACY", "playlistPrivacy": "PUBLIC" }));
    }

    #[test]
    fn add_and_move_actions() {
        assert_eq!(add_actions(&["v1"], false)[0]["dedupeOption"], "DEDUPE_OPTION_CHECK");
        assert_eq!(add_actions(&["v1"], true)[0]["dedupeOption"], "DEDUPE_OPTION_SKIP");
        assert_eq!(move_action("s1", Some("s2")), json!({ "action": "ACTION_MOVE_VIDEO_BEFORE", "setVideoId": "s1", "movedSetVideoIdSuccessor": "s2" }));
        assert!(move_action("s1", None).get("movedSetVideoIdSuccessor").is_none());
        assert_eq!(edit_body("VLPLabc", vec![])["playlistId"], "PLabc");
        assert_eq!(bare("OLAK5uy_x"), "OLAK5uy_x");
    }

    #[test]
    fn created_needs_an_id() {
        assert_eq!(created(&json!({ "playlistId": "PLnew" })).as_deref(), Some("PLnew"));
        assert_eq!(created(&json!({ "actions": [{ "openPopupAction": {} }] })), None);
        assert_eq!(created(&json!({ "playlistId": "" })), None);
    }

    #[test]
    fn delete_outcomes() {
        assert_eq!(delete_outcome(&json!({})), MutationOutcome::Done);
        assert_eq!(delete_outcome(&json!({ "status": "STATUS_SUCCEEDED" })), MutationOutcome::Done);
        assert_eq!(delete_outcome(&json!({ "actions": [{ "openPopupAction": {} }] })), MutationOutcome::Rejected);
        assert_eq!(delete_outcome(&json!({ "status": "STATUS_FAILED" })), MutationOutcome::Rejected);
    }
}
