//! yt-dlp-ejs on QuickJS-ng, compiled into Encore through `rquickjs` (D37).
//!
//! Upstream: yt-dlp `jsc/_builtin/quickjs.py` (`QuickJSJCP`) runs the same
//! script with a `qjs` binary; here the engine is in-process, on one
//! thread of its own:
//!
//! * a fresh runtime and context for every solve, so nothing one player
//!   leaves behind reaches the next;
//! * a memory limit, a stack limit and an interrupt handler that stops the
//!   script at its deadline or when Encore quits;
//! * only what the ECMAScript standard library has: no console, no
//!   modules, no file or network access, no Encore objects, no cookies;
//! * the input is handed to `jsc` as a parsed JSON value, never pasted
//!   into source.
//!
//! The preprocessed player (the player reduced to what the solver needs)
//! is kept in memory for the last few players, so the songs after the
//! first one under a player skip the slow parse.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex;
use rquickjs::{Context, Ctx, Function, Runtime, Value};

use super::ejs::{self, Input, Output};
use super::{ChallengeRequest, ChallengeResults, PlayerScript, SolveError, Solver};

/// The engine's version, for caches and Settings (`quickjs.h`).
pub const ENGINE: &str = "quickjs-ng 0.16.2";

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Heap limit of one solve.
    pub memory: usize,
    /// JS stack limit (the thread's own stack is larger).
    pub stack: usize,
    /// A solve from the preprocessed player.
    pub deadline: Duration,
    /// A cold solve: parsing and preprocessing a 2.5 MB player takes about
    /// 2 s on an M4 and several times that on a slow laptop.
    pub cold_deadline: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { memory: 256 << 20, stack: 4 << 20, deadline: Duration::from_secs(10), cold_deadline: Duration::from_secs(30) }
    }
}

/// Native stack of the solver thread: meriyah recurses deeply on a 2 MB
/// player, and QuickJS checks its own limit against this.
const THREAD_STACK: usize = 16 << 20;
/// Preprocessed players kept (one per player variant in use).
const PREPROCESSED_KEPT: usize = 3;

struct Job {
    input: String,
    limits: Limits,
    /// Cold (`Limits::cold_deadline`) or from the preprocessed player.
    cold: bool,
    cancel: Arc<AtomicBool>,
    reply: tokio::sync::oneshot::Sender<Result<Output, SolveError>>,
}

/// Timings of the last solve, for `encore-cli resolve --diagnostics`.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct SolveTiming {
    pub millis: u64,
    pub preprocessed_hit: bool,
}

pub struct QuickJsSolver {
    limits: Limits,
    jobs: Mutex<Option<std::sync::mpsc::Sender<Job>>>,
    /// Set by `close`: a running script is interrupted.
    closed: Arc<AtomicBool>,
    /// (player id, content hash) → preprocessed player, newest last.
    preprocessed: Mutex<VecDeque<(String, Arc<str>)>>,
    last: Mutex<SolveTiming>,
}

impl QuickJsSolver {
    pub fn new(limits: Limits) -> Arc<Self> {
        Arc::new(QuickJsSolver { limits, jobs: Mutex::new(None), closed: Arc::new(AtomicBool::new(false)), preprocessed: Mutex::new(VecDeque::new()), last: Mutex::new(SolveTiming::default()) })
    }

    /// Before Encore quits or installs an update (D24): the running script
    /// is interrupted and no other starts. The thread ends with its queue.
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.jobs.lock().take();
    }

    /// Quitting didn't happen after all.
    pub fn reopen(&self) {
        self.closed.store(false, Ordering::SeqCst);
    }

    /// Forget the preprocessed players (Settings → Reset player cache).
    pub fn clear(&self) {
        self.preprocessed.lock().clear();
    }

    pub fn last_timing(&self) -> SolveTiming {
        *self.last.lock()
    }

    /// The solver thread's queue, starting the thread when there is none
    /// or the last one ended (`fresh`: it did, so start another).
    fn sender(&self, fresh: bool) -> Result<std::sync::mpsc::Sender<Job>, SolveError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(SolveError::Closed);
        }
        let mut g = self.jobs.lock();
        if fresh {
            g.take();
        }
        if let Some(tx) = g.as_ref() {
            return Ok(tx.clone());
        }
        let (tx, rx) = std::sync::mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("encore-jsc".into())
            .stack_size(THREAD_STACK)
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    // A panic is this solve's failure, not the thread's
                    // (release builds abort on panic: there the process ends,
                    // as it would on a crash inside the engine).
                    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&job.input, job.limits, job.cold, &job.cancel)))
                        .unwrap_or_else(|_| Err(SolveError::Engine("the solver failed".into())));
                    let _ = job.reply.send(out);
                }
            })
            .map_err(|e| SolveError::Engine(format!("could not start: {}", e.kind())))?;
        *g = Some(tx.clone());
        Ok(tx)
    }

    async fn run_job(&self, input: String, cold: bool) -> Result<Output, SolveError> {
        let (reply, rx) = tokio::sync::oneshot::channel();
        // The job's own flag follows `closed`; a dropped future cancels it too.
        let cancel = Arc::new(AtomicBool::new(false));
        struct CancelOnDrop(Arc<AtomicBool>);
        impl Drop for CancelOnDrop {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let _guard = CancelOnDrop(cancel.clone());
        let closed = self.closed.clone();
        let watch = cancel.clone();
        let job = Job { input, limits: self.limits, cold, cancel: watch, reply };
        // A thread that ended (it panicked) takes no job: start another.
        if let Err(std::sync::mpsc::SendError(job)) = self.sender(false)?.send(job) {
            self.sender(true)?.send(job).map_err(|_| SolveError::Engine("the solver didn't start".into()))?;
        }
        let poll = async {
            loop {
                tokio::time::sleep(Duration::from_millis(50)).await;
                if closed.load(Ordering::SeqCst) {
                    cancel.store(true, Ordering::SeqCst);
                }
            }
        };
        tokio::select! {
            r = rx => r.map_err(|_| SolveError::Engine("the solver thread stopped".into()))?,
            _ = poll => unreachable!(),
        }
    }
}

fn key(player: &PlayerScript) -> String {
    format!("{}:{:016x}", player.id, crate::fxhash(&player.code))
}

#[async_trait]
impl Solver for QuickJsSolver {
    async fn solve(&self, player: &PlayerScript, requests: Vec<ChallengeRequest>) -> Result<Vec<ChallengeResults>, SolveError> {
        let started = Instant::now();
        let k = key(player);
        let cached = self.preprocessed.lock().iter().find(|(id, _)| *id == k).map(|(_, p)| p.clone());
        let hit = cached.is_some();
        let input = match &cached {
            Some(pre) => Input::Preprocessed { preprocessed_player: pre, requests: &requests },
            None => Input::Player { player: &player.code, requests: &requests, output_preprocessed: true },
        };
        let input = serde_json::to_string(&input).map_err(|e| SolveError::Engine(e.to_string()))?;
        let out = self.run_job(input, !hit).await?;
        *self.last.lock() = SolveTiming { millis: started.elapsed().as_millis() as u64, preprocessed_hit: hit };
        match out {
            Output::Error { error } => Err(SolveError::Player(first_line(&error))),
            Output::Result { preprocessed_player, responses } => {
                if responses.len() != requests.len() {
                    return Err(SolveError::Engine("the solver answered a different number of requests".into()));
                }
                if let Some(pre) = preprocessed_player {
                    let mut q = self.preprocessed.lock();
                    q.retain(|(id, _)| *id != k);
                    q.push_back((k, pre.into()));
                    while q.len() > PREPROCESSED_KEPT {
                        q.pop_front();
                    }
                }
                Ok(requests.iter().zip(responses).map(|(req, r)| super::validate(req, r.into_results())).collect())
            }
        }
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").chars().take(300).collect()
}

/// One solve on a fresh engine. Runs on the solver thread.
fn run(input: &str, limits: Limits, cold: bool, cancel: &Arc<AtomicBool>) -> Result<Output, SolveError> {
    let rt = Runtime::new().map_err(|e| SolveError::Engine(e.to_string()))?;
    rt.set_memory_limit(limits.memory);
    rt.set_max_stack_size(limits.stack);
    let deadline = Instant::now() + if cold { limits.cold_deadline } else { limits.deadline };
    let stop = cancel.clone();
    let interrupted = Arc::new(AtomicBool::new(false));
    let flag = interrupted.clone();
    rt.set_interrupt_handler(Some(Box::new(move || {
        let s = stop.load(Ordering::Relaxed) || Instant::now() > deadline;
        if s {
            flag.store(true, Ordering::Relaxed);
        }
        s
    })));
    let ctx = Context::full(&rt).map_err(|e| SolveError::Engine(e.to_string()))?;
    let res = ctx.with(|ctx| -> Result<String, SolveError> {
        let js = |e: rquickjs::Error, ctx: &Ctx<'_>| -> SolveError {
            let what = if matches!(e, rquickjs::Error::Exception) { describe(ctx, ctx.catch()) } else { e.to_string() };
            if interrupted.load(Ordering::Relaxed) {
                SolveError::Engine(if cancel.load(Ordering::Relaxed) { "cancelled".into() } else { "out of time".into() })
            } else if what.contains("out of memory") || what.contains("stack overflow") {
                SolveError::Engine(what)
            } else {
                SolveError::Player(what)
            }
        };
        ctx.eval::<(), _>(ejs::LIB).map_err(|e| js(e, &ctx))?;
        ctx.eval::<(), _>(ejs::GLUE).map_err(|e| js(e, &ctx))?;
        ctx.eval::<(), _>(ejs::CORE).map_err(|e| js(e, &ctx))?;
        let jsc: Function = ctx.globals().get("jsc").map_err(|e| js(e, &ctx))?;
        let value = ctx.json_parse(input).map_err(|e| js(e, &ctx))?;
        let out: Value = jsc.call((value,)).map_err(|e| js(e, &ctx))?;
        let s = ctx.json_stringify(out).map_err(|e| js(e, &ctx))?.ok_or_else(|| SolveError::Engine("the solver returned nothing".into()))?;
        s.to_string().map_err(|e| js(e, &ctx))
    });
    drop(ctx);
    let json = res?;
    serde_json::from_str(&json).map_err(|e| SolveError::Engine(format!("unreadable output: {e}")))
}

/// A thrown value as text: an Error's message, or the value itself (ejs
/// throws plain strings such as "unexpected structure").
fn describe<'js>(ctx: &Ctx<'js>, v: Value<'js>) -> String {
    if let Some(ex) = v.as_exception() {
        return ex.message().unwrap_or_else(|| "exception".into());
    }
    if let Some(s) = v.as_string() {
        return s.to_string().unwrap_or_default();
    }
    ctx.json_stringify(v).ok().flatten().and_then(|s| s.to_string().ok()).unwrap_or_else(|| "exception".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script(code: &str) -> PlayerScript {
        PlayerScript { id: "test/main".into(), code: Arc::from(code) }
    }

    fn n(c: &str) -> Vec<ChallengeRequest> {
        vec![ChallengeRequest { kind: super::super::ChallengeType::N, challenges: vec![c.into()] }]
    }

    #[tokio::test]
    async fn the_synthetic_player_solves_cold_and_preprocessed() {
        let s = QuickJsSolver::new(Limits::default());
        let p = script(crate::jsc::SYNTHETIC_PLAYER);
        for round in 0..2 {
            let reqs = vec![
                ChallengeRequest { kind: super::super::ChallengeType::N, challenges: vec!["abcdef".into()] },
                ChallengeRequest { kind: super::super::ChallengeType::Sig, challenges: vec![super::super::sig_spec_challenge(6)] },
            ];
            let out = s.solve(&p, reqs).await.unwrap();
            assert_eq!(out[0].as_ref().unwrap()["abcdef"], crate::jsc::SYNTHETIC_N_ABCDEF);
            let perm = super::super::sig_permutation(&out[1].as_ref().unwrap()[&super::super::sig_spec_challenge(6)]);
            assert_eq!(super::super::apply_permutation(&perm, "abcdef").as_deref(), Some(crate::jsc::SYNTHETIC_SIG_ABCDEF));
            assert_eq!(s.last_timing().preprocessed_hit, round == 1);
        }
    }

    #[tokio::test]
    async fn a_player_ejs_cant_read_is_a_player_error() {
        let s = QuickJsSolver::new(Limits::default());
        let e = s.solve(&script("var a = 1;"), n("x")).await.unwrap_err();
        assert!(matches!(e, SolveError::Player(_)), "{e:?}");
        let e = s.solve(&script("this is not javascript"), n("x")).await.unwrap_err();
        assert!(matches!(e, SolveError::Player(_)), "{e:?}");
    }

    /// The adversarial cases: a script that never ends, one that eats
    /// memory, one that recurses forever. Each stops with an engine error
    /// and the solver goes on working afterwards.
    #[tokio::test]
    async fn runaway_players_are_stopped() {
        let s = QuickJsSolver::new(Limits { deadline: Duration::from_millis(800), cold_deadline: Duration::from_millis(800), ..Limits::default() });
        let looping = crate::jsc::synthetic_with_n_body("for(;;){}");
        let started = Instant::now();
        let e = s.solve(&script(&looping), n("abcdef")).await.unwrap_err();
        assert!(matches!(e, SolveError::Engine(ref m) if m == "out of time"), "{e:?}");
        assert!(started.elapsed() < Duration::from_secs(5));

        // Out of memory and stack overflow are errors the player's code
        // can catch, and ejs does (its `multiTry`): they come back as a
        // failed request or as an engine error, never as an answer.
        let failed = |r: Result<Vec<ChallengeResults>, SolveError>| match r {
            Err(SolveError::Engine(_)) => true,
            Ok(v) => v.iter().all(|r| r.is_err()),
            Err(_) => false,
        };
        let hog = crate::jsc::synthetic_with_n_body("var a=[]; for(;;){ a.push(new Array(1e6).fill(1)); }");
        assert!(failed(s.solve(&script(&hog), n("abcdef")).await));

        let deep = crate::jsc::synthetic_with_n_body("function f(x){ return f(x+1)+1; } return f(0);");
        assert!(failed(s.solve(&script(&deep), n("abcdef")).await));

        // A player too deeply nested to parse.
        let nested = format!("(function(){{ var a = {}1{}; }}).call(this);", "[".repeat(200_000), "]".repeat(200_000));
        assert!(s.solve(&script(&nested), n("abcdef")).await.is_err());

        let ok = s.solve(&script(crate::jsc::SYNTHETIC_PLAYER), n("abcdef")).await.unwrap();
        assert_eq!(ok[0].as_ref().unwrap()["abcdef"], crate::jsc::SYNTHETIC_N_ABCDEF);
    }

    #[tokio::test]
    async fn a_closed_solver_refuses_and_interrupts() {
        let s = QuickJsSolver::new(Limits { deadline: Duration::from_secs(30), cold_deadline: Duration::from_secs(30), ..Limits::default() });
        let looping = script(&crate::jsc::synthetic_with_n_body("for(;;){}"));
        let s2 = s.clone();
        let running = tokio::spawn(async move { s2.solve(&looping, n("abcdef")).await });
        tokio::time::sleep(Duration::from_millis(300)).await;
        let started = Instant::now();
        s.close();
        let e = running.await.unwrap().unwrap_err();
        assert!(matches!(e, SolveError::Engine(ref m) if m == "cancelled"), "{e:?}");
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(s.solve(&script(crate::jsc::SYNTHETIC_PLAYER), n("a")).await.unwrap_err(), SolveError::Closed);
        s.reopen();
        assert!(s.solve(&script(crate::jsc::SYNTHETIC_PLAYER), n("abcdef")).await.is_ok());
    }
}
