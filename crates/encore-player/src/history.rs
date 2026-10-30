//! Plays reported to the service's history (D18).
//!
//! A [`Listen`] follows one play of one queue entry and turns what was
//! actually heard into pings: nothing until 2 s of audible playback, then
//! one `Playback` ping (that is what creates the history entry), `Watchtime`
//! pings with the heard stretches on the service's flush schedule, on pause,
//! and a last one when the play ends. Pauses, loading and seeks add nothing.
//! It is pure (the clock is passed in) so the rules are tested without
//! network or audio.
//!
//! The [`HistoryReporter`] task feeds it from the player's own events and
//! sends the pings through a [`PlayReporter`], one at a time. Failures are
//! logged once and never touch playback.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use encore_core::model::ItemId;
use encore_core::service::{FlushSchedule, Ping, PlayReport, PlayReporter};
use tokio::sync::{broadcast, mpsc};

use crate::{Phase, PlayerEvent};

/// Audible seconds before a play counts.
pub const MIN_HEARD: f64 = 2.0;

#[derive(Debug, Clone)]
pub struct Listen {
    /// Clock when audio was first heard.
    started: Option<f64>,
    last: Option<(f64, f64, bool)>,
    /// Heard stretch in progress (start, end) in media seconds.
    open: Option<(f64, f64)>,
    /// Closed stretches not reported yet.
    pending: Vec<(f64, f64)>,
    heard: f64,
    schedule: Option<FlushSchedule>,
    flushes: usize,
}

impl Default for Listen {
    fn default() -> Self {
        Self::new()
    }
}

impl Listen {
    pub fn new() -> Self {
        Listen { started: None, last: None, open: None, pending: vec![], heard: 0.0, schedule: None, flushes: 0 }
    }

    pub fn heard(&self) -> f64 {
        self.heard
    }

    /// Long enough to count as a play.
    pub fn counts(&self) -> bool {
        self.heard >= MIN_HEARD
    }

    pub fn reporting(&self) -> bool {
        self.schedule.is_some()
    }

    fn close(&mut self) {
        if let Some((s, e)) = self.open.take() {
            if e > s + 0.001 {
                self.pending.push((s, e));
            }
        }
    }

    fn rt(&self, now: f64) -> f64 {
        self.started.map(|s| now - s).unwrap_or(0.0)
    }

    fn watchtime(&mut self, now: f64, cmt: f64, playing: bool, last: bool) -> Option<Ping> {
        // Report the stretch in progress too, and keep it open from here.
        if let Some((s, e)) = self.open {
            if e > s + 0.001 {
                self.pending.push((s, e));
            }
            self.open = playing.then_some((e, e));
        }
        if self.pending.is_empty() {
            return None;
        }
        let (st, et) = self.pending.drain(..).unzip();
        Some(Ping::Watchtime { st, et, cmt, rt: self.rt(now), playing, last })
    }

    /// Feed one observation: the position, whether audio is playing, and
    /// whether a seek happened since the last one.
    pub fn observe(&mut self, now: f64, pos: f64, playing: bool, seeked: bool) -> Vec<Ping> {
        let mut out = Vec::new();
        if !pos.is_finite() || pos < 0.0 {
            return out;
        }
        let was_playing = self.last.is_some_and(|l| l.2);
        if seeked {
            // A seek: the jump is not listening.
            self.close();
        } else if let (true, Some((at, lp, _))) = (was_playing, self.last) {
            let d = pos - lp;
            let wall = (now - at).max(0.0);
            if d >= -0.05 && d <= wall * 1.5 + 1.0 {
                let d = d.max(0.0);
                self.heard += d;
                let s = self.open.map(|o| o.0).unwrap_or(lp);
                self.open = Some((s, pos.max(s)));
            } else {
                self.close();
            }
        }
        if playing {
            if self.started.is_none() {
                self.started = Some(now);
            }
            if self.open.is_none() {
                self.open = Some((pos, pos));
            }
        } else {
            self.close();
        }
        self.last = Some((now, pos, playing));

        if let Some(sched) = self.schedule.clone() {
            if was_playing && !playing {
                // Paused: report what was heard so far.
                out.extend(self.watchtime(now, pos, false, false));
            } else if self.rt(now) >= sched.due(self.flushes) {
                // Due (once, however late: no catch-up bursts).
                while self.rt(now) >= sched.due(self.flushes) {
                    self.flushes += 1;
                }
                out.extend(self.watchtime(now, pos, playing, false));
            }
        }
        out
    }

    /// The report for this play is ready: the `Playback` ping that registers
    /// it. What was heard meanwhile goes out with the next flush.
    pub fn opened(&mut self, schedule: FlushSchedule, now: f64) -> Ping {
        self.schedule = Some(schedule);
        let cmt = self.last.map(|l| l.1).unwrap_or(0.0);
        Ping::Playback { cmt, rt: self.rt(now) }
    }

    /// The play ended (track change, stop, end of queue): the last ping, if
    /// anything was heard since the previous one.
    pub fn finish(&mut self, now: f64) -> Option<Ping> {
        self.schedule.as_ref()?;
        let cmt = self.last.map(|l| l.1).unwrap_or(0.0);
        self.close();
        let p = self.watchtime(now, cmt, false, true);
        self.schedule = None;
        p
    }
}

// ---------- the reporting task ----------

struct Active {
    track_gen: String,
    uid: String,
    id: ItemId,
    seek_gen: String,
    itag: Option<u32>,
    listen: Listen,
    report: ReportState,
}

enum ReportState {
    Waiting,
    Opening,
    Open(Arc<dyn PlayReport>),
    Off,
}

type Opened = (String, String, Option<Arc<dyn PlayReport>>);

pub struct HistoryReporter;

impl HistoryReporter {
    /// Follow the player's events and report plays while `enabled`.
    /// `on_registered` runs after a play's first ping was accepted.
    pub fn spawn(events: broadcast::Receiver<PlayerEvent>, reporter: Arc<dyn PlayReporter>, enabled: Arc<AtomicBool>, on_registered: Arc<dyn Fn(&ItemId) + Send + Sync>) {
        let (send_tx, mut send_rx) = mpsc::unbounded_channel::<(Arc<dyn PlayReport>, Ping, Option<u32>, ItemId)>();
        // One ping at a time, in order.
        tokio::spawn(async move {
            let mut failed_for: Option<ItemId> = None;
            while let Some((r, ping, itag, id)) = send_rx.recv().await {
                let first = matches!(ping, Ping::Playback { .. });
                match r.send(&ping, itag).await {
                    Ok(()) => {
                        if first {
                            tracing::info!(video = %id, "play reported");
                            on_registered(&id);
                        }
                    }
                    Err(e) if failed_for.as_ref() != Some(&id) => {
                        failed_for = Some(id.clone());
                        tracing::warn!(video = %id, error = %e.message, "play report failed");
                    }
                    Err(_) => {}
                }
            }
        });
        tokio::spawn(run(events, reporter, enabled, send_tx));
    }
}

async fn run(
    mut events: broadcast::Receiver<PlayerEvent>,
    reporter: Arc<dyn PlayReporter>,
    enabled: Arc<AtomicBool>,
    send: mpsc::UnboundedSender<(Arc<dyn PlayReport>, Ping, Option<u32>, ItemId)>,
) {
    let t0 = Instant::now();
    let clock = move || t0.elapsed().as_secs_f64();
    let (open_tx, mut open_rx) = mpsc::unbounded_channel::<Opened>();
    let mut active: Option<Active> = None;
    let ship = |a: &Active, pings: Vec<Ping>| {
        if let ReportState::Open(r) = &a.report {
            for p in pings {
                let _ = send.send((r.clone(), p, a.itag, a.id.clone()));
            }
        }
    };
    let end = |a: Option<Active>, now: f64| {
        if let Some(mut a) = a {
            if let Some(p) = a.listen.finish(now) {
                ship(&a, vec![p]);
            }
        }
    };
    loop {
        tokio::select! {
            ev = events.recv() => {
                let ev = match ev {
                    Ok(e) => e,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                };
                let now = clock();
                if !enabled.load(Ordering::Relaxed) {
                    // Turned off: stop at once, nothing more is sent.
                    active = None;
                    continue;
                }
                match ev {
                    PlayerEvent::State(s) => {
                        let Some(cur) = s.current.clone() else {
                            end(active.take(), now);
                            continue;
                        };
                        let same = active.as_ref().is_some_and(|a| a.track_gen == s.track_gen && a.uid == cur.uid);
                        if !same {
                            end(active.take(), now);
                            active = Some(Active { track_gen: s.track_gen.clone(), uid: cur.uid.clone(), id: cur.track.id.clone(), seek_gen: s.seek_gen.clone(), itag: None, listen: Listen::new(), report: ReportState::Waiting });
                        }
                        let a = active.as_mut().unwrap();
                        if let Some(st) = &s.stream {
                            a.itag = Some(st.itag);
                        }
                        if matches!(s.phase, Phase::Ended | Phase::Error | Phase::Idle) {
                            end(active.take(), now);
                            continue;
                        }
                        let seeked = std::mem::replace(&mut a.seek_gen, s.seek_gen.clone()) != s.seek_gen;
                        let pings = a.listen.observe(now, s.position, s.phase == Phase::Playing && s.playing, seeked);
                        ship(a, pings);
                    }
                    PlayerEvent::Position(p) => {
                        let Some(a) = active.as_mut().filter(|a| a.track_gen == p.track_gen) else { continue };
                        let seeked = std::mem::replace(&mut a.seek_gen, p.seek_gen.clone()) != p.seek_gen;
                        let pings = a.listen.observe(now, p.position, p.playing, seeked);
                        ship(a, pings);
                    }
                    _ => continue,
                }
                // Long enough heard: ask for this play's report, once.
                if let Some(a) = active.as_mut().filter(|a| matches!(a.report, ReportState::Waiting) && a.listen.counts()) {
                    a.report = ReportState::Opening;
                    let (r, tx, id, tg, uid) = (reporter.clone(), open_tx.clone(), a.id.clone(), a.track_gen.clone(), a.uid.clone());
                    tokio::spawn(async move {
                        let res = match r.open_report(&id).await {
                            Ok(x) => x,
                            Err(e) => {
                                tracing::warn!(video = %id, error = %e.message, "play report unavailable");
                                None
                            }
                        };
                        let _ = tx.send((tg, uid, res));
                    });
                }
            }
            Some((tg, uid, res)) = open_rx.recv() => {
                // A report for a play that has already ended is dropped unsent.
                let Some(a) = active.as_mut().filter(|a| a.track_gen == tg && a.uid == uid) else { continue };
                if !enabled.load(Ordering::Relaxed) {
                    active = None;
                    continue;
                }
                match res {
                    Some(r) => {
                        let first = a.listen.opened(r.schedule(), clock());
                        a.report = ReportState::Open(r);
                        ship(a, vec![first]);
                    }
                    None => a.report = ReportState::Off,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Play from `from` for `secs` seconds, observing every 0.5 s.
    fn play(l: &mut Listen, now: &mut f64, from: f64, secs: f64) -> Vec<Ping> {
        let mut out = Vec::new();
        let steps = (secs / 0.5).round() as usize;
        for i in 0..=steps {
            out.extend(l.observe(*now, from + i as f64 * 0.5, true, false));
            if i < steps {
                *now += 0.5;
            }
        }
        out
    }

    fn spans(p: &Ping) -> (Vec<f64>, Vec<f64>) {
        match p {
            Ping::Watchtime { st, et, .. } => (st.clone(), et.clone()),
            _ => panic!("not watchtime: {p:?}"),
        }
    }

    #[test]
    fn nothing_before_two_audible_seconds() {
        let mut l = Listen::new();
        // Loading: not playing, nothing counts.
        assert!(l.observe(0.0, 0.0, false, false).is_empty());
        assert!(l.observe(3.0, 0.0, false, false).is_empty());
        let mut now = 3.0;
        play(&mut l, &mut now, 0.0, 1.5);
        assert!(!l.counts());
        play(&mut l, &mut now, 1.5, 0.5);
        assert!(l.counts(), "heard {}", l.heard());
    }

    #[test]
    fn playback_then_flushes_on_schedule() {
        let mut l = Listen::new();
        let mut now = 0.0;
        assert!(play(&mut l, &mut now, 0.0, 2.0).is_empty(), "no pings before the report is open");
        let first = l.opened(FlushSchedule::default(), now);
        assert!(matches!(first, Ping::Playback { cmt, rt } if (cmt - 2.0).abs() < 1e-9 && (rt - 2.0).abs() < 1e-9));
        // Up to 10 s of wall time: one flush covering 0..10.
        let p = play(&mut l, &mut now, 2.0, 8.0);
        assert_eq!(p.len(), 1);
        assert_eq!(spans(&p[0]), (vec![0.0], vec![10.0]));
        let p = play(&mut l, &mut now, 10.0, 60.0);
        // 20, 30 and 70 s.
        assert_eq!(p.len(), 3, "{p:?}");
        assert_eq!(spans(&p[2]), (vec![30.0], vec![70.0]));
        // Contiguous: every stretch starts where the previous ended.
        assert!(p.windows(2).all(|w| spans(&w[0]).1[0] == spans(&w[1]).0[0]));
    }

    #[test]
    fn a_pause_adds_nothing_and_flushes() {
        let mut l = Listen::new();
        let mut now = 0.0;
        play(&mut l, &mut now, 0.0, 4.0);
        l.opened(FlushSchedule::default(), now);
        now += 0.5;
        let p = l.observe(now, 4.0, false, false);
        assert_eq!(p.len(), 1);
        assert!(matches!(&p[0], Ping::Watchtime { playing: false, last: false, .. }));
        assert_eq!(spans(&p[0]), (vec![0.0], vec![4.0]));
        // A long pause: no time heard, no empty pings.
        for _ in 0..200 {
            now += 0.5;
            assert!(l.observe(now, 4.0, false, false).is_empty());
        }
        assert!((l.heard() - 4.0).abs() < 1e-9);
    }

    #[test]
    fn a_seek_splits_the_stretches() {
        let mut l = Listen::new();
        let mut now = 0.0;
        play(&mut l, &mut now, 0.0, 4.0);
        l.opened(FlushSchedule { at: vec![100.0], every: 40.0 }, now);
        now += 0.5;
        l.observe(now, 60.0, true, true);
        play(&mut l, &mut now, 60.0, 3.0);
        let last = l.finish(now).unwrap();
        assert_eq!(spans(&last), (vec![0.0, 60.0], vec![4.0, 63.0]));
        assert!(matches!(last, Ping::Watchtime { last: true, playing: false, cmt, .. } if cmt == 63.0));
        assert!((l.heard() - 7.0).abs() < 1e-9, "the jump is not heard: {}", l.heard());
    }

    #[test]
    fn a_jump_without_a_seek_flag_is_not_heard() {
        let mut l = Listen::new();
        let mut now = 0.0;
        play(&mut l, &mut now, 0.0, 2.0);
        now += 0.5;
        l.observe(now, 90.0, true, false);
        assert!((l.heard() - 2.0).abs() < 1e-9);
    }

    #[test]
    fn late_report_holds_back_then_catches_up_once() {
        let mut l = Listen::new();
        let mut now = 0.0;
        play(&mut l, &mut now, 0.0, 45.0);
        l.opened(FlushSchedule::default(), now);
        // One flush with everything so far, not three.
        let p = play(&mut l, &mut now, 45.0, 0.5);
        assert_eq!(p.len(), 1);
        assert_eq!(spans(&p[0]), (vec![0.0], vec![45.0]));
    }

    #[test]
    fn finish_without_report_sends_nothing() {
        let mut l = Listen::new();
        let mut now = 0.0;
        play(&mut l, &mut now, 0.0, 30.0);
        assert!(l.finish(now).is_none());
        // And a finished listen stays finished.
        let mut l = Listen::new();
        play(&mut l, &mut now, 0.0, 3.0);
        l.opened(FlushSchedule::default(), now);
        assert!(l.finish(now).is_some());
        assert!(l.finish(now).is_none());
    }

    // ---------- the task, with a fake reporter ----------

    use crate::{PlayerSnapshot, PositionAnchor};
    use async_trait::async_trait;
    use encore_core::model::{Track, TrackKind};
    use encore_core::queue::{Origin, QueueEntry, Repeat};
    use parking_lot::Mutex;
    use std::time::Duration;

    #[derive(Default)]
    struct Log {
        opened: Mutex<Vec<String>>,
        pings: Mutex<Vec<(String, Ping)>>,
    }
    struct FakeReport(Arc<Log>, String);
    #[async_trait]
    impl PlayReport for FakeReport {
        fn schedule(&self) -> FlushSchedule {
            FlushSchedule::default()
        }
        async fn send(&self, ping: &Ping, _itag: Option<u32>) -> encore_core::error::Result<()> {
            self.0.pings.lock().push((self.1.clone(), ping.clone()));
            Ok(())
        }
    }
    struct FakeReporter(Arc<Log>, Duration);
    #[async_trait]
    impl PlayReporter for FakeReporter {
        async fn open_report(&self, id: &ItemId) -> encore_core::error::Result<Option<Arc<dyn PlayReport>>> {
            self.0.opened.lock().push(id.remote().to_string());
            tokio::time::sleep(self.1).await;
            Ok(Some(Arc::new(FakeReport(self.0.clone(), id.remote().to_string()))))
        }
    }

    fn entry(id: &str) -> QueueEntry {
        QueueEntry {
            uid: format!("u-{id}"),
            track: Track { id: ItemId::ytm(id), title: id.into(), artists: vec![], album: None, duration: Some(200.0), explicit: false, art: None, kind: TrackKind::Song, plays: None, liked: None, playable: true, counterpart: None },
            origin: Origin::Manual,
        }
    }
    fn state(id: &str, tg: u64, phase: Phase, pos: f64) -> PlayerEvent {
        PlayerEvent::State(PlayerSnapshot {
            revision: "1".into(), track_gen: tg.to_string(), seek_gen: "0".into(), phase, playing: phase == Phase::Playing, current: Some(entry(id)), index: Some(0),
            position: pos, duration: Some(200.0), shuffle: false, repeat: Repeat::Off, autoplay: false, volume: 1.0, muted: false, source: None, stream: None,
            error: None, skipped_in_a_row: 0, queue_loading: false, no_output: false,
        })
    }
    fn anchor(tg: u64, pos: f64) -> PlayerEvent {
        PlayerEvent::Position(PositionAnchor { track_gen: tg.to_string(), seek_gen: "0".into(), position: pos, duration: Some(200.0), playing: true })
    }

    async fn harness(open_delay: Duration, on: bool) -> (broadcast::Sender<PlayerEvent>, Arc<Log>, Arc<Mutex<Vec<String>>>) {
        let (tx, rx) = broadcast::channel(64);
        let log = Arc::new(Log::default());
        let reg = Arc::new(Mutex::new(Vec::new()));
        let r2 = reg.clone();
        HistoryReporter::spawn(rx, Arc::new(FakeReporter(log.clone(), open_delay)), Arc::new(AtomicBool::new(on)), Arc::new(move |id: &ItemId| r2.lock().push(id.remote().to_string())));
        (tx, log, reg)
    }
    async fn settle() {
        tokio::time::sleep(Duration::from_millis(60)).await;
    }

    #[tokio::test]
    async fn a_play_is_reported_once_after_it_is_heard() {
        let (tx, log, reg) = harness(Duration::ZERO, true).await;
        // Resolving and loading count for nothing.
        tx.send(state("a", 1, Phase::Loading, 0.0)).unwrap();
        settle().await;
        assert!(log.opened.lock().is_empty());
        tx.send(state("a", 1, Phase::Playing, 0.0)).unwrap();
        for i in 1..=6 {
            tx.send(anchor(1, i as f64 * 0.5)).unwrap();
        }
        settle().await;
        assert_eq!(*log.opened.lock(), vec!["a"]);
        tx.send(anchor(1, 3.5)).unwrap();
        settle().await;
        assert_eq!(*log.opened.lock(), vec!["a"], "opened once");
        assert!(matches!(log.pings.lock().first(), Some((id, Ping::Playback { .. })) if id == "a"));
        assert_eq!(*reg.lock(), vec!["a"]);
        // The next track: the last ping of this play, then a new play.
        tx.send(state("b", 2, Phase::Loading, 0.0)).unwrap();
        settle().await;
        let pings = log.pings.lock().clone();
        assert!(matches!(pings.last(), Some((id, Ping::Watchtime { last: true, .. })) if id == "a"), "{pings:?}");
    }

    #[tokio::test]
    async fn a_play_skipped_while_its_report_opens_sends_nothing() {
        let (tx, log, _) = harness(Duration::from_millis(150), true).await;
        tx.send(state("a", 1, Phase::Playing, 0.0)).unwrap();
        for i in 1..=5 {
            tx.send(anchor(1, i as f64 * 0.5)).unwrap();
        }
        settle().await;
        assert_eq!(*log.opened.lock(), vec!["a"]);
        tx.send(state("b", 2, Phase::Loading, 0.0)).unwrap();
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert!(log.pings.lock().is_empty(), "{:?}", log.pings.lock());
    }

    #[tokio::test]
    async fn short_plays_and_disabled_reporting_send_nothing() {
        let (tx, log, _) = harness(Duration::ZERO, true).await;
        tx.send(state("a", 1, Phase::Playing, 0.0)).unwrap();
        tx.send(anchor(1, 1.0)).unwrap();
        tx.send(state("b", 2, Phase::Loading, 0.0)).unwrap();
        settle().await;
        assert!(log.opened.lock().is_empty(), "a 1 s skip-through is not a play");

        let (tx, log, _) = harness(Duration::ZERO, false).await;
        tx.send(state("a", 1, Phase::Playing, 0.0)).unwrap();
        for i in 1..=10 {
            tx.send(anchor(1, i as f64 * 0.5)).unwrap();
        }
        settle().await;
        assert!(log.opened.lock().is_empty());
    }

}
