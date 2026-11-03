//! The local play log's observer (D30). It follows the player's own events,
//! independently of history reporting (D18): every play of a queue entry
//! that became audible gets a row (`encore_core::local::plays`), online or
//! offline, signed in or not, with the heard seconds checkpointed as it goes
//! and the counting rule applied once. Others follow its [`PlayEvent`]s
//! (Last.fm scrobbles, lane E) instead of watching the player themselves.
//!
//! Heard time is the [`Listen`] accounting of `history.rs`: pauses, loading
//! and seek jumps add nothing. A seek back inside the same entry (an A–B
//! loop wrapping, D26) stays the same play. The owner is the account the
//! play started under.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use encore_core::local::Owner;
use encore_core::local::plays::{PlaySink, PlayStart, counts};
use encore_core::model::Track;
use tokio::sync::broadcast;

use crate::history::{Listen, MIN_HEARD};
use crate::{Phase, PlayerEvent};

/// Heard seconds between checkpoints of a play in progress.
const CHECKPOINT: f64 = 15.0;

#[derive(Clone, Debug)]
pub enum PlayEvent {
    /// A play became audible; its row exists.
    Started(Arc<PlayStart>),
    /// It met the counting rule (once per play).
    Counted { play: Arc<PlayStart>, heard: f64 },
    /// It is over: another song, a stop, the end of the queue.
    Ended { play: Arc<PlayStart>, heard: f64, counted: bool },
}

pub type OwnerFn = Arc<dyn Fn() -> String + Send + Sync>;

pub struct PlayObserver {
    tx: broadcast::Sender<PlayEvent>,
}

struct Current {
    track_gen: String,
    uid: String,
    seek_gen: String,
    track: Track,
    duration: Option<f64>,
    source: Option<String>,
    owner: Owner,
    listen: Listen,
    play: Option<Arc<PlayStart>>,
    counted: bool,
    saved: f64,
    /// Audio was playing at the last observation.
    playing: bool,
}

impl PlayObserver {
    /// Follow `events` and write plays to `sink`. `owner` names the current
    /// account; it is read once per play, when the play begins.
    pub fn spawn(events: broadcast::Receiver<PlayerEvent>, sink: Arc<dyn PlaySink>, owner: OwnerFn, online: Arc<AtomicBool>, private: Arc<AtomicBool>) -> Arc<Self> {
        let (tx, _) = broadcast::channel(128);
        let me = Arc::new(PlayObserver { tx: tx.clone() });
        tokio::spawn(run(events, sink, owner, online, private, tx));
        me
    }

    pub fn subscribe(&self) -> broadcast::Receiver<PlayEvent> {
        self.tx.subscribe()
    }
}

async fn run(mut events: broadcast::Receiver<PlayerEvent>, sink: Arc<dyn PlaySink>, owner: OwnerFn, online: Arc<AtomicBool>, private: Arc<AtomicBool>, tx: broadcast::Sender<PlayEvent>) {
    let t0 = Instant::now();
    let clock = move || t0.elapsed().as_secs_f64();
    let mut cur: Option<Current> = None;
    let end = |c: Option<Current>| {
        let Some(c) = c else { return };
        let Some(play) = c.play else { return };
        let heard = c.listen.heard();
        sink.update(&play.id, heard, c.counted, true);
        let _ = tx.send(PlayEvent::Ended { play, heard, counted: c.counted });
    };
    loop {
        let ev = match events.recv().await {
            Ok(e) => e,
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(_) => break,
        };
        let now = clock();
        match ev {
            PlayerEvent::State(s) => {
                let Some(entry) = s.current.clone() else {
                    end(cur.take());
                    continue;
                };
                let same = cur.as_ref().is_some_and(|c| c.track_gen == s.track_gen && c.uid == entry.uid);
                if !same {
                    end(cur.take());
                    cur = Some(Current {
                        track_gen: s.track_gen.clone(),
                        uid: entry.uid.clone(),
                        seek_gen: s.seek_gen.clone(),
                        duration: s.duration.or(entry.track.duration),
                        track: entry.track,
                        source: s.source.as_ref().and_then(|x| x.collection_id.clone()),
                        owner: Owner::new(owner()),
                        listen: Listen::new(),
                        play: None,
                        counted: false,
                        saved: 0.0,
                        playing: false,
                    });
                }
                if matches!(s.phase, Phase::Ended | Phase::Error | Phase::Idle) {
                    end(cur.take());
                    continue;
                }
                let c = cur.as_mut().unwrap();
                if s.duration.is_some() {
                    c.duration = s.duration;
                }
                let seeked = std::mem::replace(&mut c.seek_gen, s.seek_gen.clone()) != s.seek_gen;
                c.playing = s.phase == Phase::Playing && s.playing;
                c.listen.observe(now, s.position, c.playing, seeked);
            }
            PlayerEvent::Position(p) => {
                let Some(c) = cur.as_mut().filter(|c| c.track_gen == p.track_gen) else { continue };
                if p.duration.is_some() {
                    c.duration = p.duration;
                }
                let seeked = std::mem::replace(&mut c.seek_gen, p.seek_gen.clone()) != p.seek_gen;
                c.playing = p.playing;
                c.listen.observe(now, p.position, p.playing, seeked);
            }
            _ => continue,
        }
        let Some(c) = cur.as_mut() else { continue };
        let heard = c.listen.heard();
        match &c.play {
            None if heard >= MIN_HEARD => {
                let play = Arc::new(PlayStart {
                    id: uuid::Uuid::new_v4().to_string(),
                    owner: c.owner.clone(),
                    track: Track { duration: c.duration.or(c.track.duration), ..c.track.clone() },
                    started: encore_core::store::now() - heard.round() as i64,
                    offline: !online.load(Ordering::SeqCst),
                    private: private.load(Ordering::SeqCst),
                    source: c.source.clone(),
                });
                sink.begin(&play);
                sink.update(&play.id, heard, false, false);
                c.saved = heard;
                c.play = Some(play.clone());
                let _ = tx.send(PlayEvent::Started(play));
            }
            Some(play) => {
                let play = play.clone();
                if !c.counted && counts(heard, c.duration) {
                    c.counted = true;
                    c.saved = heard;
                    sink.update(&play.id, heard, true, false);
                    let _ = tx.send(PlayEvent::Counted { play, heard });
                } else if heard - c.saved >= CHECKPOINT || (!c.playing && heard > c.saved) {
                    // Every 15 heard seconds, and on pause: Encore may be
                    // quit while paused.
                    c.saved = heard;
                    sink.update(&play.id, heard, c.counted, false);
                }
            }
            None => {}
        }
    }
    end(cur.take());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{PlayerSnapshot, PositionAnchor};
    use encore_core::model::{ItemId, TrackKind};
    use encore_core::queue::{Origin, QueueEntry, Repeat};
    use parking_lot::Mutex;

    #[derive(Default)]
    struct Sink {
        begun: Mutex<Vec<PlayStart>>,
        updates: Mutex<Vec<(String, f64, bool, bool)>>,
    }
    impl PlaySink for Sink {
        fn begin(&self, p: &PlayStart) {
            self.begun.lock().push(p.clone());
        }
        fn update(&self, id: &str, heard: f64, counted: bool, ended: bool) {
            self.updates.lock().push((id.into(), heard, counted, ended));
        }
    }

    fn snap(track_gen: &str, uid: &str, phase: Phase, pos: f64) -> PlayerSnapshot {
        let track = Track { id: ItemId::ytm(uid), title: uid.into(), artists: vec![], album: None, duration: Some(200.0), explicit: false, art: None, kind: TrackKind::Song, plays: None, liked: None, playable: true, counterpart: None };
        PlayerSnapshot {
            revision: "1".into(),
            track_gen: track_gen.into(),
            seek_gen: "0".into(),
            phase,
            playing: phase == Phase::Playing,
            current: Some(QueueEntry { uid: uid.into(), track, origin: Origin::Manual }),
            index: Some(0),
            position: pos,
            duration: Some(200.0),
            shuffle: false,
            repeat: Repeat::Off,
            autoplay: false,
            volume: 1.0,
            muted: false,
            source: None,
            stream: None,
            error: None,
            skipped_in_a_row: 0,
            queue_loading: false,
            no_output: false,
            ab_loop: None,
            sleep: None,
            private: None,
        }
    }

    fn at(track_gen: &str, pos: f64) -> PlayerEvent {
        PlayerEvent::Position(PositionAnchor { track_gen: track_gen.into(), seek_gen: "0".into(), position: pos, duration: Some(200.0), playing: true })
    }

    #[tokio::test]
    async fn a_play_is_written_counted_and_ended() {
        let (etx, erx) = broadcast::channel(1024);
        let sink = Arc::new(Sink::default());
        let obs = PlayObserver::spawn(erx, sink.clone(), Arc::new(|| "acc".to_string()), Arc::new(AtomicBool::new(true)), Arc::new(AtomicBool::new(false)));
        let mut seen = obs.subscribe();
        etx.send(PlayerEvent::State(snap("1", "a", Phase::Playing, 0.0))).unwrap();
        // Second by second: one heard second each.
        for s in 1..=120 {
            etx.send(at("1", s as f64)).unwrap();
        }
        etx.send(PlayerEvent::State(snap("2", "b", Phase::Playing, 0.0))).unwrap();
        let mut kinds = vec![];
        while kinds.len() < 3 {
            match tokio::time::timeout(std::time::Duration::from_secs(5), seen.recv()).await.unwrap().unwrap() {
                PlayEvent::Started(p) => kinds.push(format!("started {} {}", p.track.id.remote(), p.owner)),
                PlayEvent::Counted { heard, .. } => kinds.push(format!("counted {}", heard.round())),
                PlayEvent::Ended { heard, counted, .. } => kinds.push(format!("ended {} {counted}", heard.round())),
            }
        }
        assert_eq!(kinds, vec!["started a acc", "counted 100", "ended 120 true"]);
        let begun = sink.begun.lock();
        assert_eq!(begun.len(), 1, "b isn't audible yet");
        assert!(!begun[0].offline);
        let last = sink.updates.lock().last().cloned().unwrap();
        assert_eq!((last.1.round(), last.2, last.3), (120.0, true, true));
    }

    #[tokio::test]
    async fn a_pause_saves_what_was_heard() {
        let (etx, erx) = broadcast::channel(64);
        let sink = Arc::new(Sink::default());
        let _obs = PlayObserver::spawn(erx, sink.clone(), Arc::new(|| "acc".to_string()), Arc::new(AtomicBool::new(true)), Arc::new(AtomicBool::new(false)));
        etx.send(PlayerEvent::State(snap("1", "a", Phase::Playing, 0.0))).unwrap();
        for s in 1..=9 {
            etx.send(at("1", s as f64)).unwrap();
        }
        etx.send(PlayerEvent::State(snap("1", "a", Phase::Paused, 9.0))).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let last = sink.updates.lock().last().cloned().unwrap();
        assert_eq!((last.1.round(), last.3), (9.0, false), "saved, not ended");
    }

    #[tokio::test]
    async fn a_song_skipped_at_once_leaves_no_row() {
        let (etx, erx) = broadcast::channel(64);
        let sink = Arc::new(Sink::default());
        let _obs = PlayObserver::spawn(erx, sink.clone(), Arc::new(|| "acc".to_string()), Arc::new(AtomicBool::new(true)), Arc::new(AtomicBool::new(false)));
        etx.send(PlayerEvent::State(snap("1", "a", Phase::Playing, 0.0))).unwrap();
        etx.send(at("1", 1.0)).unwrap();
        etx.send(PlayerEvent::State(snap("2", "b", Phase::Paused, 0.0))).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(sink.begun.lock().is_empty());
    }
}
