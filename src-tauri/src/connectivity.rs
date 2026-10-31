//! Online or offline (D23). Encore goes offline only after a request failed
//! without an answer (connection, DNS, timeout) and a probe of
//! music.youtube.com confirms it; it is back online as soon as a probe gets
//! any answer. An HTTP error is an answer: YouTube is reachable, it just said
//! no. Probes are coalesced, and while offline one runs every 20 seconds or
//! when the system says the network changed.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::sync::Notify;

const PROBE_URL: &str = "https://music.youtube.com/generate_204";
/// A failed request triggers at most one probe this often.
const MIN_GAP: Duration = Duration::from_secs(15);

type Hook = Arc<dyn Fn(bool) + Send + Sync>;

pub struct Connectivity {
    online: AtomicBool,
    probing: AtomicBool,
    last: Mutex<Option<Instant>>,
    wake: Notify,
    http: reqwest::Client,
    hooks: Mutex<Vec<Hook>>,
}

impl Connectivity {
    pub fn new() -> Arc<Self> {
        let http = reqwest::Client::builder().connect_timeout(Duration::from_secs(5)).timeout(Duration::from_secs(8)).build().expect("http");
        Arc::new(Connectivity { online: AtomicBool::new(true), probing: AtomicBool::new(false), last: Mutex::new(None), wake: Notify::new(), http, hooks: Mutex::new(Vec::new()) })
    }

    pub fn online(&self) -> bool {
        self.online.load(Ordering::SeqCst)
    }

    pub fn on_change(&self, f: impl Fn(bool) + Send + Sync + 'static) {
        self.hooks.lock().push(Arc::new(f));
    }

    /// A request got no answer: find out whether the connection is gone.
    pub fn transport_failed(self: &Arc<Self>) {
        if !self.online() {
            return;
        }
        let recent = self.last.lock().is_some_and(|t| t.elapsed() < MIN_GAP);
        if !recent {
            self.check();
        }
    }

    /// Probe now (the system's network changed, or the user asked).
    pub fn check(self: &Arc<Self>) {
        *self.last.lock() = Some(Instant::now());
        if self.probing.swap(true, Ordering::SeqCst) {
            // Already watching: look again now instead of after the wait.
            self.wake.notify_one();
            return;
        }
        let me = self.clone();
        tauri::async_runtime::spawn(async move { me.watch().await });
    }

    /// Probe once now and say whether YouTube answered. Going offline
    /// still takes the confirmation `check` gives.
    pub async fn check_now(self: &Arc<Self>) -> bool {
        *self.last.lock() = Some(Instant::now());
        let up = self.reachable().await;
        if up {
            self.set(true);
        } else if self.online() {
            self.check();
        }
        up
    }

    async fn reachable(&self) -> bool {
        self.http.head(PROBE_URL).send().await.is_ok()
    }

    async fn watch(self: Arc<Self>) {
        let mut up = self.reachable().await;
        if !up && self.online() {
            // Once more before saying offline: one lost request is not a lost connection.
            tokio::time::sleep(Duration::from_secs(2)).await;
            up = self.reachable().await;
        }
        self.set(up);
        while !self.online() {
            let jitter = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().subsec_millis() % 5000) as u64;
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(20_000 + jitter)) => {}
                _ = self.wake.notified() => {}
            }
            if self.reachable().await {
                self.set(true);
            }
        }
        self.probing.store(false, Ordering::SeqCst);
    }

    fn set(&self, on: bool) {
        if self.online.swap(on, Ordering::SeqCst) == on {
            return;
        }
        tracing::info!(online = on, "connection");
        let hooks = self.hooks.lock().clone();
        for h in hooks {
            h(on);
        }
    }
}
