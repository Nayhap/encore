//! Listening history (`FEmusic_history`, D19): day groups in the service's
//! order, rows as they were played, and each row's "Remove from history"
//! token. Pure functions over the response; `lib.rs` keeps the tokens in
//! memory and hands the UI opaque row ids instead.

use encore_core::model::*;
use serde_json::Value;

use crate::pages::Cont;
use crate::parse::*;

pub struct RawGroup {
    /// Empty for rows that continue the previous page's last group.
    pub title: String,
    pub rows: Vec<(Track, Option<String>)>,
}

pub struct RawHistory {
    pub groups: Vec<RawGroup>,
    pub notice: Option<String>,
    pub paused: bool,
    pub cont: Option<Cont>,
}

/// The removal token of one row: only the menu item whose icon says
/// `REMOVE_FROM_HISTORY` (rows also carry other feedback tokens, such as
/// "Not interested").
pub fn removal_token(row: &Value) -> Option<String> {
    let items = row.get("menu")?.get("menuRenderer")?.get("items")?.as_array()?;
    items.iter().find_map(|i| {
        let m = i.get("menuServiceItemRenderer")?;
        if m.get("icon")?.get("iconType")?.as_str()? != "REMOVE_FROM_HISTORY" {
            return None;
        }
        m.get("serviceEndpoint")?.get("feedbackEndpoint")?.get("feedbackToken")?.as_str().map(str::to_string)
    })
}

/// A history row is the version that was played. `list_item` resolves album
/// rows listed by their music video to the song (D4); here the played video
/// stays the track and the song becomes its counterpart.
fn played_track(r: &Value) -> Option<Track> {
    let ShelfItem::Track(mut t) = list_item(r)? else { return None };
    let row_id = r.get("playlistItemData").and_then(|p| p.get("videoId")).and_then(Value::as_str);
    if let (Some(vid), Some(cp)) = (row_id, t.counterpart.clone()) {
        if vid == cp.remote() && vid != t.id.remote() {
            let song = std::mem::replace(&mut t.id, cp);
            t.counterpart = Some(song);
            t.kind = TrackKind::Video;
        }
    }
    Some(t)
}

fn rows_of(list: &Value) -> Vec<(Track, Option<String>)> {
    list.as_array()
        .map(|a| a.iter().filter_map(|w| w.get("musicResponsiveListItemRenderer")).filter_map(|r| Some((played_track(r)?, removal_token(r)))).collect())
        .unwrap_or_default()
}

/// Text of a notifier or message section ("Your watch history is paused").
fn notice_of(sec: &Value) -> Option<String> {
    let r = sec.get("musicNotifierShelfRenderer").or_else(|| sec.get("messageRenderer")).or_else(|| sec.get("itemSectionRenderer").and_then(|i| find(i, "messageRenderer")))?;
    let mut parts = Vec::new();
    for k in ["title", "text", "subtext", "description"] {
        if let Some(t) = r.get(k).and_then(text_opt) {
            parts.push(t);
        }
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn is_paused(notice: &str) -> bool {
    notice.to_ascii_lowercase().contains("paused")
}

pub fn history(resp: &Value) -> Result<RawHistory, String> {
    let sl = section_list(resp).ok_or("no history sections")?;
    let mut groups = Vec::new();
    let mut notice = None;
    let mut cont = next_continuation(sl).map(Cont::Query).or_else(|| sl.get("contents").and_then(item_continuation).map(Cont::Body));
    for sec in sl.get("contents").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]) {
        if let Some(ms) = sec.get("musicShelfRenderer") {
            let list = ms.get("contents").unwrap_or(&Value::Null);
            let title = ms.get("title").and_then(text_opt).unwrap_or_default();
            let rows = rows_of(list);
            if let Some(c) = item_continuation(list).map(Cont::Body).or_else(|| next_continuation(ms).map(Cont::Query)) {
                cont = Some(c);
            }
            if !rows.is_empty() {
                groups.push(RawGroup { title, rows });
            }
        } else if let Some(n) = notice_of(sec) {
            notice.get_or_insert(n);
        }
    }
    let paused = notice.as_deref().is_some_and(is_paused);
    Ok(RawHistory { groups, notice, paused, cont })
}

/// More history: new day groups, or more rows of the last one.
pub fn history_continuation(resp: &Value) -> Result<RawHistory, String> {
    let mut out = RawHistory { groups: vec![], notice: None, paused: false, cont: None };
    if let Some(c) = resp.get("continuationContents") {
        if let Some(sc) = c.get("sectionListContinuation") {
            // Same shape as the first page's section list.
            return history(&serde_json::json!({ "contents": { "sectionListRenderer": sc } }));
        }
        if let Some(ms) = c.get("musicShelfContinuation") {
            out.groups.push(RawGroup { title: String::new(), rows: rows_of(ms.get("contents").unwrap_or(&Value::Null)) });
            out.cont = next_continuation(ms).map(Cont::Query);
            return Ok(out);
        }
    }
    let items = resp
        .get("onResponseReceivedActions")
        .and_then(Value::as_array)
        .and_then(|a| a.iter().find_map(|x| x.get("appendContinuationItemsAction").and_then(|y| y.get("continuationItems"))))
        .ok_or("unexpected history continuation")?;
    let rows = rows_of(items);
    if !rows.is_empty() {
        out.groups.push(RawGroup { title: String::new(), rows });
    }
    for sec in items.as_array().map(Vec::as_slice).unwrap_or(&[]) {
        if let Some(ms) = sec.get("musicShelfRenderer") {
            let rows = rows_of(ms.get("contents").unwrap_or(&Value::Null));
            if !rows.is_empty() {
                out.groups.push(RawGroup { title: ms.get("title").and_then(text_opt).unwrap_or_default(), rows });
            }
        }
    }
    out.cont = item_continuation(items).map(Cont::Body);
    Ok(out)
}

/// The answer to `feedback`: every token must come back processed.
pub fn feedback_outcome(resp: &Value) -> MutationOutcome {
    match resp.get("feedbackResponses").and_then(Value::as_array) {
        Some(a) if !a.is_empty() && a.iter().all(|r| r.get("isProcessed").and_then(Value::as_bool) == Some(true)) => MutationOutcome::Done,
        Some(a) if !a.is_empty() => MutationOutcome::Rejected,
        _ => MutationOutcome::OutcomeUnknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn feedback_needs_every_token_processed() {
        assert_eq!(feedback_outcome(&json!({ "feedbackResponses": [{ "isProcessed": true }] })), MutationOutcome::Done);
        assert_eq!(feedback_outcome(&json!({ "feedbackResponses": [{ "isProcessed": true }, { "isProcessed": false }] })), MutationOutcome::Rejected);
        assert_eq!(feedback_outcome(&json!({})), MutationOutcome::OutcomeUnknown);
    }

    #[test]
    fn only_the_history_token_is_taken() {
        let row = json!({ "menu": { "menuRenderer": { "items": [
            { "menuServiceItemRenderer": { "icon": { "iconType": "DISMISSAL" }, "serviceEndpoint": { "feedbackEndpoint": { "feedbackToken": "not-interested" } } } },
            { "menuServiceItemRenderer": { "icon": { "iconType": "REMOVE_FROM_HISTORY" }, "serviceEndpoint": { "feedbackEndpoint": { "feedbackToken": "remove" } } } }
        ] } } });
        assert_eq!(removal_token(&row).as_deref(), Some("remove"));
        assert_eq!(removal_token(&json!({})), None);
    }
}
