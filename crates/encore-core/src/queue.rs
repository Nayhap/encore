//! The play queue (D6), owned by Rust.
//!
//! Entries have UUIDs, so the same track can appear twice. Two orders are
//! kept: `entries` is the effective (playing) order, `canon` the canonical
//! order as the user built it. Shuffle reorders only the unplayed suffix;
//! turning shuffle off keeps the played prefix and current entry fixed and
//! restores the surviving suffix by canonical rank. Deleted entries never
//! come back. Explicit `next` bypasses repeat-one.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::{AppError, Result};
use crate::model::Track;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "camelCase")]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Origin {
    Manual,
    Collection,
    Radio,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QueueEntry {
    pub uid: String,
    pub track: Track,
    pub origin: Origin,
}

/// "Playing from …".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QueueSource {
    pub label: String,
    pub collection_id: Option<String>,
    /// Playlist id for radio continuation when the queue came from a radio.
    pub radio_playlist: Option<String>,
}

/// What the player should do after a queue operation.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Play this entry from the start.
    Play(usize),
    /// Restart the current entry.
    Restart,
    /// The queue is exhausted but autoplay wants more: fetch radio, then retry.
    NeedRadio,
    /// Nothing more to play.
    End,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Queue {
    entries: Vec<QueueEntry>,
    canon: Vec<String>,
    index: Option<usize>,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub autoplay: bool,
    pub source: Option<QueueSource>,
    revision: u64,
    rng: u64,
    /// Track ids of the last 50 plays, for radio de-duplication.
    #[serde(default)]
    recent: Vec<String>,
}

impl Default for Queue {
    fn default() -> Self {
        Self::new(0x5eed_cafe_f00d_1234)
    }
}

fn uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

impl Queue {
    pub fn new(seed: u64) -> Self {
        Queue {
            entries: Vec::new(),
            canon: Vec::new(),
            index: None,
            shuffle: false,
            repeat: Repeat::Off,
            autoplay: true,
            source: None,
            revision: 0,
            rng: seed | 1,
            recent: Vec::new(),
        }
    }

    fn bump(&mut self) {
        self.revision += 1;
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn entries(&self) -> &[QueueEntry] {
        &self.entries
    }
    pub fn index(&self) -> Option<usize> {
        self.index
    }
    pub fn current(&self) -> Option<&QueueEntry> {
        self.index.and_then(|i| self.entries.get(i))
    }
    pub fn get(&self, i: usize) -> Option<&QueueEntry> {
        self.entries.get(i)
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn position_of(&self, uid: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.uid == uid)
    }
    /// Entry that would play after the current one without user action
    /// (used for preload). Repeat-one preloads nothing.
    pub fn upcoming(&self) -> Option<&QueueEntry> {
        let i = self.index?;
        match self.repeat {
            Repeat::One => None,
            _ if i + 1 < self.entries.len() => self.entries.get(i + 1),
            Repeat::All => self.entries.first(),
            Repeat::Off => None,
        }
    }
    /// Remaining entries after the current one.
    pub fn remaining(&self) -> usize {
        match self.index {
            Some(i) => self.entries.len().saturating_sub(i + 1),
            None => self.entries.len(),
        }
    }

    fn next_rand(&mut self) -> u64 {
        // splitmix64
        self.rng = self.rng.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn shuffle_from(&mut self, start: usize) {
        let n = self.entries.len();
        if n <= start + 1 {
            return;
        }
        for i in (start + 1..n).rev() {
            // j in start+1..=i: the entry at `start` (current) never moves.
            let j = start + 1 + (self.next_rand() % ((i - start) as u64)) as usize;
            self.entries.swap(i, j);
        }
    }

    fn mark_played(&mut self) {
        if let Some(e) = self.current() {
            // Remember the counterpart too, so radio does not bring back the
            // video of the song that just played (or the other way round).
            let ids: Vec<String> = e.track.recording_ids().map(|i| i.0.clone()).collect();
            self.recent.retain(|x| !ids.contains(x));
            self.recent.extend(ids);
            while self.recent.len() > 50 {
                self.recent.remove(0);
            }
        }
    }

    /// Replace the queue. Empty input is rejected without touching the queue.
    pub fn replace(&mut self, tracks: Vec<Track>, start: usize, source: Option<QueueSource>, shuffle: bool, origin: Origin) -> Result<usize> {
        if tracks.is_empty() {
            return Err(AppError::invalid("Nothing to play."));
        }
        if start >= tracks.len() {
            return Err(AppError::invalid("Start position is outside the list."));
        }
        let entries: Vec<QueueEntry> = tracks.into_iter().map(|t| QueueEntry { uid: uuid(), track: t, origin }).collect();
        self.canon = entries.iter().map(|e| e.uid.clone()).collect();
        self.entries = entries;
        self.source = source;
        self.shuffle = shuffle;
        if shuffle {
            let first = self.entries.remove(start);
            self.entries.insert(0, first);
            self.shuffle_from(0);
            self.index = Some(0);
        } else {
            self.index = Some(start);
        }
        self.mark_played();
        self.bump();
        Ok(self.index.unwrap())
    }

    /// Explicit Next: bypasses repeat-one.
    pub fn next(&mut self) -> Step {
        let Some(i) = self.index else {
            return if self.entries.is_empty() { Step::End } else { self.go(0) };
        };
        if i + 1 < self.entries.len() {
            return self.go(i + 1);
        }
        match self.repeat {
            Repeat::All => self.go(0),
            _ if self.autoplay => Step::NeedRadio,
            _ => Step::End,
        }
    }

    /// Natural end of the current track.
    pub fn on_ended(&mut self) -> Step {
        let Some(i) = self.index else { return Step::End };
        if self.repeat == Repeat::One {
            return Step::Restart;
        }
        if i + 1 < self.entries.len() {
            return self.go(i + 1);
        }
        match self.repeat {
            Repeat::All => self.go(0),
            // Autoplay runs only when repeat is off (repeat-all wraps instead).
            Repeat::Off if self.autoplay => Step::NeedRadio,
            _ => Step::End,
        }
    }

    /// Previous: restart when more than 3 s in or at the first entry.
    pub fn prev(&mut self, position: f64) -> Step {
        match self.index {
            None => Step::End,
            Some(i) if position > 3.0 || i == 0 => Step::Restart,
            Some(i) => self.go(i - 1),
        }
    }

    fn go(&mut self, i: usize) -> Step {
        self.index = Some(i);
        self.mark_played();
        self.bump();
        Step::Play(i)
    }

    pub fn jump(&mut self, uid: &str) -> Result<Step> {
        let i = self.position_of(uid).ok_or_else(|| AppError::new(crate::error::ErrorCode::Stale, "That song is no longer in the queue."))?;
        Ok(self.go(i))
    }

    fn canon_pos(&self, uid: &str) -> Option<usize> {
        self.canon.iter().position(|u| u == uid)
    }

    /// Insert after the current entry, keeping the given order. Returns the
    /// index to start if the queue was empty.
    pub fn play_next(&mut self, tracks: Vec<Track>) -> Option<usize> {
        if tracks.is_empty() {
            return None;
        }
        let new: Vec<QueueEntry> = tracks.into_iter().map(|t| QueueEntry { uid: uuid(), track: t, origin: Origin::Manual }).collect();
        let was_empty = self.entries.is_empty();
        let at = self.index.map(|i| i + 1).unwrap_or(0);
        let cat = self.current().and_then(|c| self.canon_pos(&c.uid)).map(|p| p + 1).unwrap_or(self.canon.len().min(at));
        for (k, e) in new.iter().enumerate() {
            self.canon.insert(cat + k, e.uid.clone());
        }
        for (k, e) in new.into_iter().enumerate() {
            self.entries.insert(at + k, e);
        }
        self.bump();
        if was_empty || self.index.is_none() {
            self.index = Some(0);
            self.mark_played();
            return Some(0);
        }
        None
    }

    pub fn append(&mut self, tracks: Vec<Track>, origin: Origin) -> Option<usize> {
        if tracks.is_empty() {
            return None;
        }
        let was_empty = self.entries.is_empty();
        for t in tracks {
            let e = QueueEntry { uid: uuid(), track: t, origin };
            self.canon.push(e.uid.clone());
            self.entries.push(e);
        }
        self.bump();
        if was_empty || self.index.is_none() {
            self.index = Some(0);
            self.mark_played();
            return Some(0);
        }
        None
    }

    /// Append radio results, dropping tracks that are already queued ahead or
    /// were among the last 50 plays. Returns how many were added.
    pub fn extend_radio(&mut self, tracks: Vec<Track>) -> usize {
        // A recording can arrive as its song or its video: compare both IDs.
        let mut known: HashSet<String> = match self.index {
            Some(i) => self.entries[i..].iter().flat_map(|e| e.track.recording_ids()).map(|id| id.0.clone()).collect(),
            None => HashSet::new(),
        };
        known.extend(self.recent.iter().cloned());
        let fresh: Vec<Track> = tracks
            .into_iter()
            .filter(|t| {
                if !t.playable || t.recording_ids().any(|id| known.contains(&id.0)) {
                    return false;
                }
                known.extend(t.recording_ids().map(|id| id.0.clone()));
                true
            })
            .collect();
        let n = fresh.len();
        if n > 0 {
            self.append(fresh, Origin::Radio);
        }
        n
    }

    /// Remove an entry. The current entry cannot be removed.
    pub fn remove(&mut self, uid: &str) -> Result<()> {
        let i = self.position_of(uid).ok_or_else(|| AppError::new(crate::error::ErrorCode::Stale, "That song is no longer in the queue."))?;
        if Some(i) == self.index {
            return Err(AppError::invalid("The song that is playing cannot be removed."));
        }
        self.entries.remove(i);
        self.canon.retain(|u| u != uid);
        if let Some(c) = self.index {
            if i < c {
                self.index = Some(c - 1);
            }
        }
        self.bump();
        Ok(())
    }

    /// Move an entry to sit before `before` (or to the end).
    pub fn move_entry(&mut self, uid: &str, before: Option<&str>) -> Result<()> {
        let from = self.position_of(uid).ok_or_else(|| AppError::new(crate::error::ErrorCode::Stale, "That song is no longer in the queue."))?;
        if Some(from) == self.index {
            return Err(AppError::invalid("The song that is playing cannot be moved."));
        }
        let cur_uid = self.current().map(|c| c.uid.clone());
        let e = self.entries.remove(from);
        let to = match before {
            Some(b) => self.position_of(b).ok_or_else(|| AppError::new(crate::error::ErrorCode::Stale, "Queue changed; try again."))?,
            None => self.entries.len(),
        };
        self.entries.insert(to, e);
        if let Some(cu) = cur_uid {
            self.index = self.position_of(&cu);
        }
        if !self.shuffle {
            self.canon = self.entries.iter().map(|e| e.uid.clone()).collect();
        }
        self.bump();
        Ok(())
    }

    pub fn clear_up_next(&mut self) {
        let Some(i) = self.index else {
            self.entries.clear();
            self.canon.clear();
            self.bump();
            return;
        };
        let dropped: HashSet<String> = self.entries.drain(i + 1..).map(|e| e.uid).collect();
        self.canon.retain(|u| !dropped.contains(u));
        self.bump();
    }

    pub fn set_shuffle(&mut self, on: bool) {
        if on == self.shuffle {
            return;
        }
        let start = self.index.unwrap_or(0);
        if on {
            if self.index.is_some() {
                self.shuffle_from(start);
            } else if !self.entries.is_empty() {
                self.shuffle_from(0);
            }
        } else {
            // Keep history and current; restore the suffix by canonical rank.
            let split = self.index.map(|i| i + 1).unwrap_or(0);
            let rank: HashMap<&String, usize> = self.canon.iter().enumerate().map(|(i, u)| (u, i)).collect();
            let mut tail: Vec<QueueEntry> = self.entries.drain(split..).collect();
            tail.sort_by_key(|e| rank.get(&e.uid).copied().unwrap_or(usize::MAX));
            self.entries.extend(tail);
        }
        self.shuffle = on;
        self.bump();
    }

    pub fn cycle_repeat(&mut self) -> Repeat {
        self.repeat = match self.repeat {
            Repeat::Off => Repeat::All,
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::Off,
        };
        self.bump();
        self.repeat
    }

    pub fn set_repeat(&mut self, r: Repeat) {
        self.repeat = r;
        self.bump();
    }

    pub fn set_autoplay(&mut self, on: bool) {
        self.autoplay = on;
        self.bump();
    }

    /// Put another version of the recording into one entry (the song/video
    /// switch). The entry keeps its uid, place, origin and the queue its
    /// source; other entries of the same track are left alone.
    pub fn replace_entry_track(&mut self, uid: &str, track: Track) -> Result<()> {
        let i = self.position_of(uid).ok_or_else(|| AppError::new(crate::error::ErrorCode::Stale, "That song is no longer in the queue."))?;
        self.entries[i].track = track;
        self.bump();
        Ok(())
    }

    /// Update metadata for every entry of a track (e.g. like state).
    pub fn update_track(&mut self, id: &str, f: impl Fn(&mut Track)) {
        let mut changed = false;
        for e in self.entries.iter_mut().filter(|e| e.track.id.0 == id) {
            f(&mut e.track);
            changed = true;
        }
        if changed {
            self.bump();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ItemId, TrackKind};

    fn t(n: &str) -> Track {
        Track {
            id: ItemId::ytm(n),
            title: n.into(),
            artists: vec![],
            album: None,
            duration: Some(100.0),
            explicit: false,
            art: None,
            kind: TrackKind::Song,
            plays: None,
            liked: None,
            playable: true,
            counterpart: None,
        }
    }
    fn ids(q: &Queue) -> Vec<String> {
        q.entries().iter().map(|e| e.track.title.clone()).collect()
    }
    fn q5() -> Queue {
        let mut q = Queue::new(42);
        q.replace(["a", "b", "c", "d", "e"].iter().map(|n| t(n)).collect(), 0, None, false, Origin::Collection).unwrap();
        q
    }

    #[test]
    fn version_switch_replaces_one_entry_in_place() {
        let mut q = Queue::new(1);
        q.replace(vec![t("a"), t("b"), t("a")], 0, Some(QueueSource { label: "L".into(), collection_id: None, radio_playlist: None }), false, Origin::Collection).unwrap();
        let uids: Vec<String> = q.entries().iter().map(|e| e.uid.clone()).collect();
        let mut v = t("a-video");
        v.kind = TrackKind::Video;
        v.counterpart = Some(ItemId::ytm("a"));
        q.replace_entry_track(&uids[0], v).unwrap();
        assert_eq!(ids(&q), vec!["a-video", "b", "a"], "the duplicate entry is not touched");
        assert_eq!(q.entries().iter().map(|e| e.uid.clone()).collect::<Vec<_>>(), uids);
        assert_eq!(q.index(), Some(0));
        assert_eq!(q.source.as_ref().map(|s| s.label.as_str()), Some("L"));
        assert!(q.replace_entry_track("gone", t("x")).is_err());
    }

    #[test]
    fn empty_replace_keeps_queue() {
        let mut q = q5();
        assert!(q.replace(vec![], 0, None, false, Origin::Manual).is_err());
        assert_eq!(q.len(), 5);
    }

    #[test]
    fn duplicates_are_distinct() {
        let mut q = Queue::new(1);
        q.replace(vec![t("a"), t("a"), t("b")], 0, None, false, Origin::Manual).unwrap();
        let u1 = q.entries()[0].uid.clone();
        let u2 = q.entries()[1].uid.clone();
        assert_ne!(u1, u2);
        q.remove(&u2).unwrap();
        assert_eq!(ids(&q), vec!["a", "b"]);
        assert_eq!(q.entries()[0].uid, u1);
    }

    #[test]
    fn next_bypasses_repeat_one_but_eof_does_not() {
        let mut q = q5();
        q.set_repeat(Repeat::One);
        assert_eq!(q.on_ended(), Step::Restart);
        assert_eq!(q.next(), Step::Play(1));
    }

    #[test]
    fn repeat_all_wraps_and_suppresses_autoplay() {
        let mut q = q5();
        q.set_repeat(Repeat::All);
        q.jump(&q.entries()[4].uid.clone()).unwrap();
        assert_eq!(q.on_ended(), Step::Play(0));
        q.set_repeat(Repeat::Off);
        q.jump(&q.entries()[4].uid.clone()).unwrap();
        assert_eq!(q.on_ended(), Step::NeedRadio);
        q.set_autoplay(false);
        assert_eq!(q.on_ended(), Step::End);
        assert_eq!(q.next(), Step::End);
    }

    #[test]
    fn prev_restarts_after_three_seconds() {
        let mut q = q5();
        q.next();
        assert_eq!(q.prev(5.0), Step::Restart);
        assert_eq!(q.prev(1.0), Step::Play(0));
        assert_eq!(q.prev(1.0), Step::Restart);
    }

    #[test]
    fn shuffle_keeps_history_and_current() {
        let mut q = q5();
        q.next(); // b current, a played
        q.set_shuffle(true);
        assert_eq!(&ids(&q)[..2], &["a", "b"]);
        let mut rest = ids(&q)[2..].to_vec();
        rest.sort();
        assert_eq!(rest, vec!["c", "d", "e"]);
        q.next();
        q.next(); // two more played in shuffled order
        let played: Vec<String> = ids(&q)[..4].to_vec();
        q.set_shuffle(false);
        // history and current stay put; remaining restored in canonical order
        assert_eq!(&ids(&q)[..4], &played[..]);
        assert_eq!(q.index(), Some(3));
        assert_eq!(q.len(), 5);
    }

    #[test]
    fn shuffle_off_never_resurrects_deleted() {
        let mut q = q5();
        q.set_shuffle(true);
        let victim = q.entries().iter().find(|e| e.track.title == "d").unwrap().uid.clone();
        q.remove(&victim).unwrap();
        q.append(vec![t("z")], Origin::Manual);
        q.set_shuffle(false);
        assert_eq!(ids(&q), vec!["a", "b", "c", "e", "z"]);
    }

    #[test]
    fn shuffle_off_includes_play_next_once() {
        let mut q = q5();
        q.set_shuffle(true);
        q.play_next(vec![t("x")]);
        assert_eq!(ids(&q)[1], "x");
        q.set_shuffle(false);
        assert_eq!(ids(&q), vec!["a", "x", "b", "c", "d", "e"]);
    }

    #[test]
    fn replace_with_shuffle_starts_with_chosen() {
        let mut q = Queue::new(7);
        q.replace(["a", "b", "c", "d"].iter().map(|n| t(n)).collect(), 2, None, true, Origin::Collection).unwrap();
        assert_eq!(q.current().unwrap().track.title, "c");
        assert_eq!(q.index(), Some(0));
        assert!(q.shuffle);
    }

    #[test]
    fn remove_and_move_keep_current() {
        let mut q = q5();
        q.next();
        q.next(); // c current (index 2)
        let a = q.entries()[0].uid.clone();
        q.remove(&a).unwrap();
        assert_eq!(q.current().unwrap().track.title, "c");
        let cur = q.current().unwrap().uid.clone();
        assert!(q.remove(&cur).is_err());
        let e = q.entries().iter().find(|e| e.track.title == "e").unwrap().uid.clone();
        let b = q.entries().iter().find(|e| e.track.title == "b").unwrap().uid.clone();
        q.move_entry(&e, Some(&b)).unwrap();
        assert_eq!(ids(&q), vec!["e", "b", "c", "d"]);
        assert_eq!(q.current().unwrap().track.title, "c");
    }

    #[test]
    fn clear_up_next_drops_future_only() {
        let mut q = q5();
        q.next();
        q.clear_up_next();
        assert_eq!(ids(&q), vec!["a", "b"]);
        assert_eq!(q.next(), Step::NeedRadio);
    }

    #[test]
    fn radio_skips_duplicates_and_recent() {
        let mut q = q5();
        q.next();
        let added = q.extend_radio(vec![t("a"), t("c"), t("x"), t("x"), t("y")]);
        // a was played recently, c is ahead, x deduplicated
        assert_eq!(added, 2);
        assert_eq!(ids(&q), vec!["a", "b", "c", "d", "e", "x", "y"]);
        assert_eq!(q.entries()[5].origin, Origin::Radio);
    }

    #[test]
    fn radio_skips_counterparts_of_queued_and_played_songs() {
        let with = |n: &str, c: &str| Track { counterpart: Some(ItemId::ytm(c)), ..t(n) };
        let mut q = Queue::new(42);
        // An album row resolved to its song "a", with music video "va".
        q.replace(vec![with("a", "va"), with("b", "vb")], 0, None, false, Origin::Collection).unwrap();
        q.next();
        // "va" is the video of a played song, "vb" of a queued one; "c"/"vc"
        // are one recording offered twice; "vb2" is a different video.
        let added = q.extend_radio(vec![t("va"), t("vb"), with("c", "vc"), t("vc"), with("vb2", "b"), t("d")]);
        assert_eq!(added, 2);
        assert_eq!(ids(&q), vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn play_next_on_empty_starts() {
        let mut q = Queue::new(3);
        assert_eq!(q.play_next(vec![t("a"), t("b")]), Some(0));
        assert_eq!(q.current().unwrap().track.title, "a");
        q.play_next(vec![t("x"), t("y")]);
        assert_eq!(ids(&q), vec!["a", "x", "y", "b"]);
    }

    #[test]
    fn upcoming_respects_repeat() {
        let mut q = q5();
        assert_eq!(q.upcoming().unwrap().track.title, "b");
        q.set_repeat(Repeat::One);
        assert!(q.upcoming().is_none());
        q.set_repeat(Repeat::All);
        q.jump(&q.entries()[4].uid.clone()).unwrap();
        assert_eq!(q.upcoming().unwrap().track.title, "a");
    }

    #[test]
    fn cycle_repeat_order() {
        let mut q = q5();
        assert_eq!(q.cycle_repeat(), Repeat::All);
        assert_eq!(q.cycle_repeat(), Repeat::One);
        assert_eq!(q.cycle_repeat(), Repeat::Off);
    }

    #[test]
    fn serde_roundtrip_restores_state() {
        let mut q = q5();
        q.next();
        q.set_shuffle(true);
        let s = serde_json::to_string(&q).unwrap();
        let r: Queue = serde_json::from_str(&s).unwrap();
        assert_eq!(ids(&r), ids(&q));
        assert_eq!(r.index(), q.index());
        assert!(r.shuffle);
    }
}
