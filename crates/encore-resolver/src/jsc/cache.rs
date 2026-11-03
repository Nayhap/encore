//! Solved challenges, per player (D37).
//!
//! Upstream: yt-dlp `_video.py` `_load_player_data_from_cache` /
//! `_store_player_data_to_cache` (2208-2227): `n` answers per challenge,
//! signature permutations (`sigfuncs`) per signature length, keyed by the
//! player. Here the key also holds the player's content hash and the
//! solver's and engine's versions, so a new ejs or QuickJS never reuses an
//! old answer. Kept in memory only, for the last few players, bounded.

use std::collections::{HashMap, VecDeque};

use parking_lot::Mutex;

const PLAYERS_KEPT: usize = 3;
const N_PER_PLAYER: usize = 512;

#[derive(Default)]
struct Entry {
    n: HashMap<String, String>,
    n_order: VecDeque<String>,
    sig: HashMap<usize, Vec<usize>>,
}

#[derive(Default)]
pub struct SolvedCache {
    players: Mutex<VecDeque<(String, Entry)>>,
}

pub fn key(player_id: &str, code: &str) -> String {
    format!("{player_id}:{:016x}:ejs-{}:{}", crate::fxhash(code), super::ejs::EJS_VERSION, super::quickjs::ENGINE)
}

impl SolvedCache {
    fn with<R>(&self, key: &str, f: impl FnOnce(&mut Entry) -> R) -> R {
        let mut q = self.players.lock();
        if let Some(i) = q.iter().position(|(k, _)| k == key) {
            let e = q.remove(i).expect("entry");
            q.push_back(e);
        } else {
            q.push_back((key.to_string(), Entry::default()));
            while q.len() > PLAYERS_KEPT {
                q.pop_front();
            }
        }
        f(&mut q.back_mut().expect("entry").1)
    }

    pub fn n(&self, key: &str, challenge: &str) -> Option<String> {
        self.with(key, |e| e.n.get(challenge).cloned())
    }

    pub fn put_n(&self, key: &str, challenge: &str, answer: &str) {
        self.with(key, |e| {
            if e.n.insert(challenge.to_string(), answer.to_string()).is_none() {
                e.n_order.push_back(challenge.to_string());
                while e.n_order.len() > N_PER_PLAYER {
                    if let Some(old) = e.n_order.pop_front() {
                        e.n.remove(&old);
                    }
                }
            }
        })
    }

    pub fn sig(&self, key: &str, len: usize) -> Option<Vec<usize>> {
        self.with(key, |e| e.sig.get(&len).cloned())
    }

    pub fn put_sig(&self, key: &str, len: usize, perm: Vec<usize>) {
        self.with(key, |e| {
            e.sig.insert(len, perm);
        })
    }

    pub fn clear(&self) {
        self.players.lock().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_kept_per_player_and_bounded() {
        let c = SolvedCache::default();
        c.put_n("a", "x", "y");
        c.put_sig("a", 3, vec![2, 1, 0]);
        assert_eq!(c.n("a", "x").as_deref(), Some("y"));
        assert_eq!(c.sig("a", 3), Some(vec![2, 1, 0]));
        assert_eq!(c.n("b", "x"), None);
        for i in 0..N_PER_PLAYER + 10 {
            c.put_n("a", &i.to_string(), "z");
        }
        assert_eq!(c.n("a", "x"), None, "the oldest answer went");
        for p in ["c", "d", "e"] {
            c.n(p, "x");
        }
        assert_eq!(c.sig("a", 3), None, "the oldest player went");
        assert!(key("9fcf08e8/main", "code").contains("ejs-0.8.0:quickjs-ng"));
    }
}
