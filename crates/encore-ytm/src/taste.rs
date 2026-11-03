//! "Not interested" (D36): Home's song rows and cards carry a feedback
//! token for it in their menu (`NOT_INTERESTED`), sent through `feedback`
//! like history removals (D19). Pure functions over a response; `lib.rs`
//! keeps the tokens in memory, bound to the account epoch, never on disk.

use serde_json::Value;

use crate::parse::find;

/// The `NOT_INTERESTED` token of one row's menu.
fn token(row: &Value) -> Option<String> {
    let items = row.get("menu")?.get("menuRenderer")?.get("items")?.as_array()?;
    items.iter().find_map(|i| {
        let m = i.get("menuServiceItemRenderer")?;
        if m.get("icon")?.get("iconType")?.as_str()? != "NOT_INTERESTED" {
            return None;
        }
        m.get("serviceEndpoint")?.get("feedbackEndpoint")?.get("feedbackToken")?.as_str().map(str::to_string)
    })
}

/// The video a row plays: its own id, else where its title or play button
/// goes. Albums and playlists have none and are left out.
fn video_id(row: &Value) -> Option<String> {
    let watch = |v: &Value| find(v, "watchEndpoint").and_then(|w| w.get("videoId")).and_then(Value::as_str).map(str::to_string);
    row.get("playlistItemData")
        .and_then(|p| p.get("videoId"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| row.get("navigationEndpoint").and_then(watch))
        .or_else(|| row.get("flexColumns").and_then(watch))
        .or_else(|| row.get("overlay").and_then(watch))
}

/// Every (video id, token) pair in a response.
pub fn not_interested_tokens(resp: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    walk(resp, &mut out);
    out
}

fn walk(v: &Value, out: &mut Vec<(String, String)>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                if k == "musicResponsiveListItemRenderer" || k == "musicTwoRowItemRenderer" {
                    if let (Some(id), Some(t)) = (video_id(x), token(x)) {
                        out.push((id, t));
                    }
                } else {
                    walk(x, out);
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn home_rows_carry_not_interested_tokens() {
        let v: Value = serde_json::from_str(include_str!("../fixtures/home.json")).unwrap();
        let t = not_interested_tokens(&v);
        assert!(t.len() >= 20, "{}", t.len());
        assert!(t.iter().all(|(id, tok)| id.len() == 11 && !tok.is_empty()));
    }

    #[test]
    fn only_the_not_interested_item_counts() {
        let row = json!({ "musicResponsiveListItemRenderer": {
            "playlistItemData": { "videoId": "abcdefghijk" },
            "menu": { "menuRenderer": { "items": [
                { "menuServiceItemRenderer": { "icon": { "iconType": "REMOVE_FROM_HISTORY" }, "serviceEndpoint": { "feedbackEndpoint": { "feedbackToken": "remove" } } } },
                { "menuServiceItemRenderer": { "icon": { "iconType": "NOT_INTERESTED" }, "serviceEndpoint": { "feedbackEndpoint": { "feedbackToken": "nope" } } } }
            ] } }
        } });
        assert_eq!(not_interested_tokens(&json!([row])), vec![("abcdefghijk".to_string(), "nope".to_string())]);
        let album = json!({ "musicTwoRowItemRenderer": { "navigationEndpoint": { "browseEndpoint": { "browseId": "MPREb" } }, "menu": row["musicResponsiveListItemRenderer"]["menu"].clone() } });
        assert!(not_interested_tokens(&album).is_empty());
    }
}
