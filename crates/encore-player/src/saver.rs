//! Saving the queue (D6). The whole queue is written only when its content
//! changed (`Queue::content`); a move to another entry or along the song
//! is a small checkpoint naming that write (`stamp`); nothing is written
//! while neither changed.
//!
//! The slot (where the queue goes and what is there now) is held through
//! every write, from planning it to the hook's answer, and replacing it
//! waits for that: a write planned for one account never lands after the
//! queue moved to another, or after its place was purged, and a switch
//! back reads what the last write left.

use encore_core::queue::{Checkpoint, Queue};
use parking_lot::Mutex;

/// One write of the queue, for the app's `on_persist`.
pub enum SaveQueue<'a> {
    /// The whole queue and the position in its song; checkpoints after it
    /// carry its `stamp`.
    Full { scope: &'a str, stamp: &'a str, queue: &'a Queue, position: f64 },
    /// Only the entry playing and the position, for the whole write named
    /// `stamp`.
    Checkpoint { scope: &'a str, stamp: &'a str, at: &'a Checkpoint, position: f64 },
}

type Hook = Box<dyn Fn(SaveQueue<'_>) -> bool + Send + Sync>;

/// Where the app saves the queue (an opaque scope: the account) and what
/// is there now. The whole queue is written only when its content changed
/// (`Queue::content`); a move to another entry or along the song is a
/// small checkpoint naming that write (`stamp`); nothing at all is written
/// while neither changed (paused, or stopped).
pub(crate) struct Slot {
    pub scope: String,
    /// The last whole write; `None` until there is one to name.
    pub stamp: Option<String>,
    pub content: u64,
    pub revision: u64,
    pub position: f64,
}

/// A position that moved less than this since the last write isn't
/// written again.
const CHECKPOINT_MOVE: f64 = 0.5;

#[derive(Debug, PartialEq)]
pub(crate) enum Write {
    Nothing,
    Full,
    Checkpoint,
}

impl Slot {
    /// What a queue now at `content`, `revision` and `position` needs
    /// written; `checkpoint`: it has an entry playing to name.
    pub fn write(&self, content: u64, revision: u64, position: f64, checkpoint: bool) -> Write {
        if content != self.content {
            return Write::Full;
        }
        if revision == self.revision && (position - self.position).abs() < CHECKPOINT_MOVE {
            return Write::Nothing;
        }
        // A checkpoint names a whole write: without one, write that first.
        if self.stamp.is_some() && checkpoint { Write::Checkpoint } else { Write::Full }
    }
}

pub(crate) enum Job {
    Full(Queue),
    Checkpoint(Checkpoint),
}

/// A write to make, and the queue it leaves saved.
pub(crate) struct Planned {
    pub job: Job,
    pub content: u64,
    pub revision: u64,
    pub position: f64,
}

#[derive(Default)]
pub(crate) struct Saver {
    slot: Mutex<Option<Slot>>,
    hook: Mutex<Option<Hook>>,
}

impl Saver {
    pub fn set_hook(&self, f: Hook) {
        *self.hook.lock() = Some(f);
    }

    /// Plan a write against the slot, make it, and note what is saved now;
    /// a write that wasn't made is planned again next time.
    pub fn persist(&self, plan: impl FnOnce(&Slot) -> Option<Planned>) {
        let mut g = self.slot.lock();
        let Some(slot) = g.as_ref() else { return };
        let Some(Planned { job, content, revision, position }) = plan(slot) else { return };
        let stamp = match (&job, &slot.stamp) {
            (Job::Checkpoint(_), Some(s)) => s.clone(),
            _ => uuid::Uuid::new_v4().to_string(),
        };
        let scope = slot.scope.clone();
        let saved = match self.hook.lock().as_ref() {
            Some(f) => match &job {
                Job::Full(queue) => f(SaveQueue::Full { scope: &scope, stamp: &stamp, queue, position }),
                Job::Checkpoint(at) => f(SaveQueue::Checkpoint { scope: &scope, stamp: &stamp, at, position }),
            },
            None => false,
        };
        if saved {
            *g = Some(Slot { scope, stamp: Some(stamp), content, revision, position });
        }
    }

    /// Put the slot `f` makes in place, once any write on its way has
    /// landed and before another starts; `f` may read the place it saves
    /// to and install the queue found there.
    pub fn replace<R>(&self, f: impl FnOnce() -> (Option<Slot>, R)) -> R {
        let mut g = self.slot.lock();
        let (slot, r) = f();
        *g = slot;
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::mpsc;
    use std::time::Duration;


    fn slot(stamp: Option<&str>) -> Slot {
        Slot { scope: "account:a".into(), stamp: stamp.map(str::to_string), content: 7, revision: 3, position: 40.0 }
    }

    #[test]
    fn nothing_is_written_while_nothing_changed() {
        assert_eq!(slot(Some("s")).write(7, 3, 40.0, true), Write::Nothing);
        assert_eq!(slot(Some("s")).write(7, 3, 40.3, true), Write::Nothing, "less than half a second");
        assert_eq!(slot(None).write(7, 3, 40.0, true), Write::Nothing, "a queue restored from 0.6 isn't written again for nothing");
    }

    #[test]
    fn a_move_is_a_checkpoint_and_a_change_the_whole_queue() {
        assert_eq!(slot(Some("s")).write(7, 3, 45.0, true), Write::Checkpoint, "playing on");
        assert_eq!(slot(Some("s")).write(7, 4, 0.0, true), Write::Checkpoint, "next");
        assert_eq!(slot(Some("s")).write(8, 4, 40.0, true), Write::Full, "an edit");
        assert_eq!(slot(None).write(7, 3, 45.0, true), Write::Full, "no whole write to point at yet");
        assert_eq!(slot(Some("s")).write(7, 4, 0.0, false), Write::Full, "no entry to name");
    }

    type Db = Arc<Mutex<HashMap<String, String>>>;

    /// A hook that writes stamps to `db` by scope, and waits for `go`
    /// before the first write lands (a write held up on its way).
    fn saver(db: &Db, go: mpsc::Receiver<()>, started: mpsc::Sender<()>) -> Arc<Saver> {
        let s = Arc::new(Saver::default());
        let (db, go) = (db.clone(), Mutex::new(Some(go)));
        s.set_hook(Box::new(move |w| {
            if let Some(go) = go.lock().take() {
                let _ = started.send(());
                let _ = go.recv();
            }
            let (SaveQueue::Full { scope, stamp, .. } | SaveQueue::Checkpoint { scope, stamp, .. }) = w;
            db.lock().insert(scope.to_string(), stamp.to_string());
            true
        }));
        s
    }

    fn full() -> Planned {
        Planned { job: Job::Full(Queue::default()), content: 1, revision: 1, position: 0.0 }
    }

    fn at(scope: &str, stamp: Option<&str>) -> Slot {
        Slot { scope: scope.into(), stamp: stamp.map(str::to_string), content: 0, revision: 0, position: 0.0 }
    }

    #[test]
    fn a_switch_back_reads_what_a_held_up_write_left() {
        let db: Db = Default::default();
        db.lock().insert("a".into(), "old".into());
        let (go_tx, go) = mpsc::channel();
        let (started, on_way) = mpsc::channel();
        let s = saver(&db, go, started);
        s.replace(|| (Some(at("a", Some("old"))), ()));
        let w = { let s = s.clone(); std::thread::spawn(move || s.persist(|_| Some(full()))) };
        on_way.recv().unwrap();
        // A → B → A while A's write is held up: the switch waits for it.
        let back = {
            let (s, db) = (s.clone(), db.clone());
            std::thread::spawn(move || {
                s.replace(|| (Some(at("b", None)), ()));
                s.replace(|| {
                    let stamp = db.lock().get("a").cloned();
                    (Some(at("a", stamp.as_deref())), stamp)
                })
            })
        };
        std::thread::sleep(Duration::from_millis(50));
        assert!(!back.is_finished(), "the switch waits for the write on its way");
        go_tx.send(()).unwrap();
        w.join().unwrap();
        let read = back.join().unwrap();
        let landed = db.lock().get("a").cloned();
        assert_ne!(landed.as_deref(), Some("old"));
        assert_eq!(read, landed, "the switch back read the write that landed");
        // A checkpoint after it names that write.
        s.persist(|_| Some(Planned { job: Job::Checkpoint(Checkpoint { uid: "u".into(), recent: vec![] }), content: 0, revision: 1, position: 3.0 }));
        assert_eq!(db.lock().get("a"), read.as_ref());
    }

    #[test]
    fn nothing_lands_after_saving_stops_for_a_purge() {
        let db: Db = Default::default();
        let (go_tx, go) = mpsc::channel();
        let (started, on_way) = mpsc::channel();
        let s = saver(&db, go, started);
        s.replace(|| (Some(at("a", None)), ()));
        let w = { let s = s.clone(); std::thread::spawn(move || s.persist(|_| Some(full()))) };
        on_way.recv().unwrap();
        let stop = { let s = s.clone(); std::thread::spawn(move || s.replace(|| (None, ()))) };
        std::thread::sleep(Duration::from_millis(50));
        assert!(!stop.is_finished(), "stopping waits for the write on its way");
        go_tx.send(()).unwrap();
        w.join().unwrap();
        stop.join().unwrap();
        // Sign-out purges now; nothing is written after it.
        db.lock().clear();
        s.persist(|_| Some(full()));
        assert!(db.lock().is_empty());
    }
}
