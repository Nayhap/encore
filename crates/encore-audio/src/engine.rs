//! The engine actor.
//!
//! One actor thread owns the cpal output stream and all decoder workers. The
//! realtime callback only drains PCM rings and reads atomics: no locks, no IO,
//! no allocation in the steady state. Retired rings are sent back to the actor
//! to be dropped there.
//!
//! Each loaded track ("deck") has an id chosen by the caller, a PCM ring, a
//! `done` flag set by its decoder, and a frame counter advanced by the
//! callback. A track has ended when its decoder is done **and** its ring is
//! drained (D5). If a next deck is queued (preload) the callback switches to
//! it on the very next frame, which gives near-gapless transitions.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use parking_lot::Mutex;
use rtrb::{Consumer, Producer, RingBuffer};

use crate::cache::AudioCache;
use crate::decode::{WorkerCmd, WorkerEvent, run_worker};
use crate::source::SourceSpec;

/// Seconds of PCM buffered per deck.
const RING_SECS: f64 = 2.0;

#[derive(Debug, Clone)]
pub enum EngineEvent {
    /// Decoder opened the stream.
    Ready { id: u64, duration: Option<f64>, codec: String, from_cache: bool },
    /// Audio for `id` is reaching the device.
    Started { id: u64 },
    /// The current deck finished and a preloaded deck took over.
    Advanced { from: u64, to: u64 },
    /// The current deck finished and nothing was queued.
    Ended { id: u64 },
    Failed { id: u64, error: String },
    /// Output starved while the decoder was still working.
    Buffering { id: u64, buffering: bool },
    DeviceChanged { name: String },
    /// No audio output could be opened; playback waits until one appears.
    NoOutput { error: String },
}

#[derive(Debug, Clone, Default)]
pub struct EngineStatus {
    pub id: u64,
    pub position: f64,
    pub duration: Option<f64>,
    pub paused: bool,
    pub buffering: bool,
    pub next_id: Option<u64>,
    /// Frames of decoded audio delivered to the device since the engine started.
    pub frames_out: u64,
    /// Number of output callbacks served (proves the device is pulling audio).
    pub callbacks: u64,
    pub underruns: u64,
    pub device: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub volume: f32,
}

struct Ring {
    id: u64,
    cons: Consumer<f32>,
    done: Arc<AtomicBool>,
    frames: Arc<AtomicU64>,
    gain: f32,
}

enum CbMsg {
    SetCurrent(Ring),
    SetNext(Ring),
    ClearNext,
    PromoteNext,
    Clear,
}

struct Atoms {
    paused: AtomicBool,
    volume: AtomicU32,
    frames_out: AtomicU64,
    callbacks: AtomicU64,
    underruns: AtomicU64,
    /// id of the deck that is currently audible (0 = none)
    playing_id: AtomicU64,
    /// set by the callback when the current deck ended with nothing queued
    ended_id: AtomicU64,
    /// set when the callback hit an empty ring that was not done
    starving: AtomicBool,
    device_error: AtomicBool,
}

struct DeckInfo {
    id: u64,
    base: f64,
    frames: Arc<AtomicU64>,
    duration: Option<f64>,
    tx: Sender<WorkerCmd>,
    spec: SourceSpec,
    gain: f32,
    started: bool,
}

enum Cmd {
    Load { id: u64, spec: SourceSpec, start: f64, gain_db: f32, play: bool },
    Preload { id: u64, spec: SourceSpec, gain_db: f32 },
    ClearPreload,
    Seek { secs: f64 },
    Pause,
    Resume,
    Stop,
    Volume,
    Shutdown,
}

pub struct Engine {
    tx: Sender<Cmd>,
    atoms: Arc<Atoms>,
    shared: Arc<Mutex<SharedStatus>>,
    events: Mutex<Option<Receiver<EngineEvent>>>,
}

#[derive(Default)]
struct SharedStatus {
    current: Option<(u64, f64, Arc<AtomicU64>, Option<f64>)>,
    next: Option<u64>,
    device: String,
    rate: u32,
    channels: u16,
}

fn db_to_gain(db: f32) -> f32 {
    // Normalisation only ever turns loud tracks down (never above 0 dB).
    10f32.powf(db.min(0.0).max(-24.0) / 20.0)
}

impl Engine {
    /// Start the engine on the default output device.
    pub fn new(cache: Option<AudioCache>) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel();
        let (etx, erx) = mpsc::channel();
        let atoms = Arc::new(Atoms {
            paused: AtomicBool::new(true),
            volume: AtomicU32::new(0.8f32.to_bits()),
            frames_out: AtomicU64::new(0),
            callbacks: AtomicU64::new(0),
            underruns: AtomicU64::new(0),
            playing_id: AtomicU64::new(0),
            ended_id: AtomicU64::new(0),
            starving: AtomicBool::new(false),
            device_error: AtomicBool::new(false),
        });
        let shared = Arc::new(Mutex::new(SharedStatus::default()));
        let (ready_tx, ready_rx) = mpsc::channel();
        let a2 = atoms.clone();
        let s2 = shared.clone();
        std::thread::Builder::new()
            .name("encore-audio".into())
            .spawn(move || actor(rx, etx, a2, s2, cache, ready_tx))
            .map_err(|e| e.to_string())?;
        ready_rx.recv().map_err(|_| "audio thread died".to_string())??;
        Ok(Self { tx, atoms, shared, events: Mutex::new(Some(erx)) })
    }

    /// Take the event receiver (one consumer).
    pub fn take_events(&self) -> Option<Receiver<EngineEvent>> {
        self.events.lock().take()
    }

    pub fn load(&self, id: u64, spec: SourceSpec, start: f64, gain_db: f32, play: bool) {
        let _ = self.tx.send(Cmd::Load { id, spec, start, gain_db, play });
    }
    pub fn preload(&self, id: u64, spec: SourceSpec, gain_db: f32) {
        let _ = self.tx.send(Cmd::Preload { id, spec, gain_db });
    }
    pub fn clear_preload(&self) {
        let _ = self.tx.send(Cmd::ClearPreload);
    }
    pub fn seek(&self, secs: f64) {
        let _ = self.tx.send(Cmd::Seek { secs });
    }
    pub fn pause(&self) {
        self.atoms.paused.store(true, Ordering::Relaxed);
        let _ = self.tx.send(Cmd::Pause);
    }
    pub fn resume(&self) {
        self.atoms.paused.store(false, Ordering::Relaxed);
        let _ = self.tx.send(Cmd::Resume);
    }
    pub fn stop(&self) {
        let _ = self.tx.send(Cmd::Stop);
    }
    pub fn set_volume(&self, v: f32) {
        let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.8 };
        self.atoms.volume.store(v.to_bits(), Ordering::Relaxed);
        let _ = self.tx.send(Cmd::Volume);
    }

    pub fn status(&self) -> EngineStatus {
        let s = self.shared.lock();
        let rate = s.rate.max(1) as f64;
        let (id, position, duration) = match &s.current {
            Some((id, base, frames, dur)) => {
                let p = base + frames.load(Ordering::Relaxed) as f64 / rate;
                (*id, dur.map(|d| p.min(d)).unwrap_or(p), *dur)
            }
            None => (0, 0.0, None),
        };
        EngineStatus {
            id,
            position,
            duration,
            paused: self.atoms.paused.load(Ordering::Relaxed),
            buffering: self.atoms.starving.load(Ordering::Relaxed),
            next_id: s.next,
            frames_out: self.atoms.frames_out.load(Ordering::Relaxed),
            callbacks: self.atoms.callbacks.load(Ordering::Relaxed),
            underruns: self.atoms.underruns.load(Ordering::Relaxed),
            device: s.device.clone(),
            sample_rate: s.rate,
            channels: s.channels,
            volume: f32::from_bits(self.atoms.volume.load(Ordering::Relaxed)),
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Shutdown);
    }
}

/// Name of the placeholder used while no device can be opened.
const NO_OUTPUT: &str = "No audio output";

struct Output {
    /// `None` for the placeholder used while no device can be opened.
    _stream: Option<cpal::Stream>,
    msgs: Producer<CbMsg>,
    retired: Consumer<Ring>,
    rate: u32,
    channels: u16,
    name: String,
}

fn build_output(atoms: &Arc<Atoms>) -> Result<Output, String> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("no audio output device")?;
    let name = device.description().map(|d| d.to_string()).unwrap_or_else(|_| "default output".into());
    let cfg = device.default_output_config().map_err(|e| format!("output config: {e}"))?;
    let fmt = cfg.sample_format();
    let config: cpal::StreamConfig = cfg.into();
    let rate = config.sample_rate;
    let channels = config.channels;
    let (msg_p, msg_c) = RingBuffer::<CbMsg>::new(64);
    let (ret_p, ret_c) = RingBuffer::<Ring>::new(64);
    let stream = match fmt {
        cpal::SampleFormat::F32 => make_stream::<f32>(&device, config, atoms.clone(), msg_c, ret_p)?,
        cpal::SampleFormat::I16 => make_stream::<i16>(&device, config, atoms.clone(), msg_c, ret_p)?,
        cpal::SampleFormat::I32 => make_stream::<i32>(&device, config, atoms.clone(), msg_c, ret_p)?,
        other => return Err(format!("unsupported output sample format {other}")),
    };
    stream.play().map_err(|e| format!("cannot start output: {e}"))?;
    Ok(Output { _stream: Some(stream), msgs: msg_p, retired: ret_c, rate, channels, name })
}

/// A stand-in output with no device behind it, so Encore can start on a
/// machine with no working sound output and pick one up when it appears.
fn silent_output() -> Output {
    let (msg_p, _msg_c) = RingBuffer::<CbMsg>::new(64);
    let (_ret_p, ret_c) = RingBuffer::<Ring>::new(64);
    Output { _stream: None, msgs: msg_p, retired: ret_c, rate: 48_000, channels: 2, name: NO_OUTPUT.into() }
}

struct Cb {
    current: Option<Ring>,
    next: Option<Ring>,
    vol: f32,
    scratch: Vec<f32>,
}

fn make_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    atoms: Arc<Atoms>,
    mut msgs: Consumer<CbMsg>,
    mut retired: Producer<Ring>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    let channels = config.channels as usize;
    let mut cb = Cb { current: None, next: None, vol: f32::from_bits(atoms.volume.load(Ordering::Relaxed)), scratch: vec![0.0; 16384] };
    let a_err = atoms.clone();
    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                atoms.callbacks.fetch_add(1, Ordering::Relaxed);
                // Apply queued control messages.
                while let Ok(m) = msgs.pop() {
                    match m {
                        CbMsg::SetCurrent(r) => {
                            if let Some(old) = cb.current.replace(r) {
                                let _ = retired.push(old);
                            }
                        }
                        CbMsg::SetNext(r) => {
                            if let Some(old) = cb.next.replace(r) {
                                let _ = retired.push(old);
                            }
                        }
                        CbMsg::ClearNext => {
                            if let Some(old) = cb.next.take() {
                                let _ = retired.push(old);
                            }
                        }
                        CbMsg::PromoteNext => {
                            if let Some(nx) = cb.next.take() {
                                if let Some(old) = cb.current.replace(nx) {
                                    let _ = retired.push(old);
                                }
                            }
                        }
                        CbMsg::Clear => {
                            if let Some(old) = cb.current.take() {
                                let _ = retired.push(old);
                            }
                            if let Some(old) = cb.next.take() {
                                let _ = retired.push(old);
                            }
                        }
                    }
                }
                let frames = data.len() / channels;
                let target = f32::from_bits(atoms.volume.load(Ordering::Relaxed));
                let paused = atoms.paused.load(Ordering::Relaxed);
                if cb.scratch.len() < frames * 2 {
                    // Only happens if the device asks for an unusually large buffer.
                    cb.scratch.resize(frames * 2, 0.0);
                }
                let mut filled = 0usize;
                let mut starving = false;
                if !paused {
                    while filled < frames {
                        let Some(cur) = cb.current.as_mut() else { break };
                        let want = (frames - filled) * 2;
                        let avail = cur.cons.slots() & !1;
                        let n = want.min(avail);
                        if n > 0 {
                            let chunk = cur.cons.read_chunk(n).unwrap();
                            let (a, b) = chunk.as_slices();
                            let dst = &mut cb.scratch[filled * 2..filled * 2 + n];
                            dst[..a.len()].copy_from_slice(a);
                            dst[a.len()..].copy_from_slice(b);
                            chunk.commit_all();
                            if cur.gain != 1.0 {
                                for s in dst.iter_mut() {
                                    *s *= cur.gain;
                                }
                            }
                            filled += n / 2;
                            cur.frames.fetch_add((n / 2) as u64, Ordering::Relaxed);
                            atoms.playing_id.store(cur.id, Ordering::Relaxed);
                            continue;
                        }
                        if cur.done.load(Ordering::Acquire) && cur.cons.slots() < 2 {
                            // Track finished: hand over to the preloaded deck, if any.
                            let old = cb.current.take().unwrap();
                            let old_id = old.id;
                            let _ = retired.push(old);
                            if let Some(nx) = cb.next.take() {
                                atoms.playing_id.store(nx.id, Ordering::Relaxed);
                                cb.current = Some(nx);
                            } else {
                                atoms.ended_id.store(old_id, Ordering::Relaxed);
                            }
                            continue;
                        }
                        starving = true;
                        break;
                    }
                }
                atoms.starving.store(starving, Ordering::Relaxed);
                if starving {
                    atoms.underruns.fetch_add(1, Ordering::Relaxed);
                }
                atoms.frames_out.fetch_add(filled as u64, Ordering::Relaxed);
                // Volume ramp across this buffer, then write out.
                let start = cb.vol;
                let step = (target - start) / frames.max(1) as f32;
                for f in 0..frames {
                    let v = start + step * f as f32;
                    let (l, r) = if f < filled { (cb.scratch[f * 2] * v, cb.scratch[f * 2 + 1] * v) } else { (0.0, 0.0) };
                    let base = f * channels;
                    for c in 0..channels {
                        let s = match c {
                            0 => l,
                            1 => r,
                            _ => 0.0,
                        };
                        data[base + c] = T::from_sample(s.clamp(-1.0, 1.0));
                    }
                }
                if channels == 1 {
                    for f in 0..frames.min(filled) {
                        data[f] = T::from_sample(((cb.scratch[f * 2] + cb.scratch[f * 2 + 1]) * 0.5 * target).clamp(-1.0, 1.0));
                    }
                }
                cb.vol = target;
            },
            move |_err| {
                a_err.device_error.store(true, Ordering::Relaxed);
            },
            None,
        )
        .map_err(|e| format!("cannot open output stream: {e}"))?;
    Ok(stream)
}

fn new_ring(rate: u32) -> (Producer<f32>, Consumer<f32>) {
    RingBuffer::<f32>::new(((rate as f64 * RING_SECS) as usize) * 2)
}

#[allow(clippy::too_many_arguments)]
fn spawn_deck(
    id: u64,
    spec: SourceSpec,
    start: f64,
    gain: f32,
    cache: &Option<AudioCache>,
    rate: u32,
    wev: &Sender<WorkerEvent>,
) -> (DeckInfo, Ring) {
    let (p, c) = new_ring(rate);
    let done = Arc::new(AtomicBool::new(false));
    let frames = Arc::new(AtomicU64::new(0));
    let (tx, rx) = mpsc::channel();
    let spec2 = spec.clone();
    let cache2 = cache.clone();
    let d2 = done.clone();
    let wev2 = wev.clone();
    std::thread::Builder::new()
        .name(format!("encore-decode-{id}"))
        .spawn(move || run_worker(id, spec2, cache2, rate, start, p, d2, rx, wev2))
        .expect("spawn decoder");
    let info = DeckInfo { id, base: start, frames: frames.clone(), duration: None, tx, spec, gain, started: false };
    (info, Ring { id, cons: c, done, frames, gain })
}

fn actor(
    rx: Receiver<Cmd>,
    events: Sender<EngineEvent>,
    atoms: Arc<Atoms>,
    shared: Arc<Mutex<SharedStatus>>,
    cache: Option<AudioCache>,
    ready: Sender<Result<(), String>>,
) {
    // No working output (no sound card, audio server down, headset just
    // unplugged) must not stop Encore from starting: run silent and retry.
    let mut no_output: Option<String> = None;
    let mut out = match build_output(&atoms) {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!(error = %e, "no audio output; retrying in the background");
            no_output = Some(e);
            silent_output()
        }
    };
    {
        let mut s = shared.lock();
        s.device = out.name.clone();
        s.rate = out.rate;
        s.channels = out.channels;
    }
    let _ = ready.send(Ok(()));
    let (wev_tx, wev_rx) = mpsc::channel::<WorkerEvent>();
    let mut current: Option<DeckInfo> = None;
    let mut next: Option<DeckInfo> = None;
    let mut durations: HashMap<u64, Option<f64>> = HashMap::new();
    let mut was_starving = false;
    let mut last_dev_check = Instant::now();
    // An output error seen at this time, with the callback count then.
    let mut suspect: Option<(Instant, u64)> = None;

    let publish = |shared: &Arc<Mutex<SharedStatus>>, current: &Option<DeckInfo>, next: &Option<DeckInfo>| {
        let mut s = shared.lock();
        s.current = current.as_ref().map(|d| (d.id, d.base, d.frames.clone(), d.duration));
        s.next = next.as_ref().map(|d| d.id);
    };

    loop {
        let cmd = match rx.recv_timeout(Duration::from_millis(15)) {
            Ok(c) => Some(c),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => Some(Cmd::Shutdown),
        };
        if let Some(cmd) = cmd {
            match cmd {
                Cmd::Shutdown => {
                    let _ = out.msgs.push(CbMsg::Clear);
                    if let Some(d) = current.take() {
                        let _ = d.tx.send(WorkerCmd::Stop);
                    }
                    if let Some(d) = next.take() {
                        let _ = d.tx.send(WorkerCmd::Stop);
                    }
                    std::thread::sleep(Duration::from_millis(30));
                    return;
                }
                Cmd::Load { id, spec, start, gain_db, play } => {
                    if let Some(e) = &no_output {
                        let _ = events.send(EngineEvent::NoOutput { error: e.clone() });
                    }
                    if let Some(d) = current.take() {
                        let _ = d.tx.send(WorkerCmd::Stop);
                    }
                    atoms.ended_id.store(0, Ordering::Relaxed);
                    atoms.paused.store(!play, Ordering::Relaxed);
                    // A load of the preloaded track just promotes its deck.
                    if next.as_ref().is_some_and(|n| n.id == id) && start == 0.0 {
                        let n = next.take().unwrap();
                        let _ = out.msgs.push(CbMsg::PromoteNext);
                        current = Some(n);
                    } else {
                        if let Some(n) = next.take() {
                            let _ = n.tx.send(WorkerCmd::Stop);
                        }
                        let (info, ring) = spawn_deck(id, spec, start.max(0.0), db_to_gain(gain_db), &cache, out.rate, &wev_tx);
                        let _ = out.msgs.push(CbMsg::Clear);
                        let _ = out.msgs.push(CbMsg::SetCurrent(ring));
                        current = Some(info);
                    }
                    publish(&shared, &current, &next);
                }
                Cmd::Preload { id, spec, gain_db } => {
                    if next.as_ref().is_some_and(|n| n.id == id) {
                        continue;
                    }
                    if let Some(n) = next.take() {
                        let _ = n.tx.send(WorkerCmd::Stop);
                    }
                    let (info, ring) = spawn_deck(id, spec, 0.0, db_to_gain(gain_db), &cache, out.rate, &wev_tx);
                    let _ = out.msgs.push(CbMsg::SetNext(ring));
                    next = Some(info);
                    publish(&shared, &current, &next);
                }
                Cmd::ClearPreload => {
                    if let Some(n) = next.take() {
                        let _ = n.tx.send(WorkerCmd::Stop);
                        let _ = out.msgs.push(CbMsg::ClearNext);
                    }
                    publish(&shared, &current, &next);
                }
                Cmd::Seek { secs } => {
                    if let Some(d) = current.as_mut() {
                        let secs = match d.duration {
                            Some(dur) => secs.clamp(0.0, (dur - 0.05).max(0.0)),
                            None => secs.max(0.0),
                        };
                        let (p, c) = new_ring(out.rate);
                        let done = Arc::new(AtomicBool::new(false));
                        let frames = Arc::new(AtomicU64::new(0));
                        let _ = d.tx.send(WorkerCmd::Seek { secs, prod: p, done: done.clone() });
                        d.base = secs;
                        d.frames = frames.clone();
                        let _ = out.msgs.push(CbMsg::SetCurrent(Ring { id: d.id, cons: c, done, frames, gain: d.gain }));
                        atoms.ended_id.store(0, Ordering::Relaxed);
                        publish(&shared, &current, &next);
                    }
                }
                Cmd::Pause | Cmd::Resume | Cmd::Volume => {}
                Cmd::Stop => {
                    let _ = out.msgs.push(CbMsg::Clear);
                    if let Some(d) = current.take() {
                        let _ = d.tx.send(WorkerCmd::Stop);
                    }
                    if let Some(d) = next.take() {
                        let _ = d.tx.send(WorkerCmd::Stop);
                    }
                    atoms.paused.store(true, Ordering::Relaxed);
                    publish(&shared, &current, &next);
                }
            }
        }

        // Retired rings are dropped here, off the audio thread.
        while out.retired.pop().is_ok() {}

        // Worker events.
        while let Ok(ev) = wev_rx.try_recv() {
            match ev {
                WorkerEvent::Ready { id, duration, codec, from_cache, .. } => {
                    durations.insert(id, duration);
                    for d in [current.as_mut(), next.as_mut()].into_iter().flatten() {
                        if d.id == id {
                            d.duration = duration;
                        }
                    }
                    publish(&shared, &current, &next);
                    let _ = events.send(EngineEvent::Ready { id, duration, codec, from_cache });
                }
                WorkerEvent::Failed { id, error } => {
                    let _ = events.send(EngineEvent::Failed { id, error });
                }
                WorkerEvent::Seeked { .. } => {}
            }
        }

        // Track transitions observed from the callback.
        let playing = atoms.playing_id.load(Ordering::Relaxed);
        if let Some(n) = &next {
            if playing == n.id && current.as_ref().is_some_and(|c| c.id != n.id) {
                let old = current.take().unwrap();
                let _ = old.tx.send(WorkerCmd::Stop);
                let mut n = next.take().unwrap();
                n.started = true;
                let from = old.id;
                let to = n.id;
                current = Some(n);
                publish(&shared, &current, &next);
                let _ = events.send(EngineEvent::Advanced { from, to });
            }
        }
        if let Some(c) = current.as_mut() {
            if !c.started && playing == c.id && c.frames.load(Ordering::Relaxed) > 0 {
                c.started = true;
                let _ = events.send(EngineEvent::Started { id: c.id });
            }
        }
        let ended = atoms.ended_id.swap(0, Ordering::Relaxed);
        if ended != 0 {
            if current.as_ref().is_some_and(|c| c.id == ended) {
                let _ = events.send(EngineEvent::Ended { id: ended });
            }
        }
        let starving = atoms.starving.load(Ordering::Relaxed) && !atoms.paused.load(Ordering::Relaxed);
        if starving != was_starving {
            was_starving = starving;
            if let Some(c) = &current {
                if c.started {
                    let _ = events.send(EngineEvent::Buffering { id: c.id, buffering: starving });
                }
            }
        }

        // Device lost or changed: rebuild the stream and re-prime from the
        // last consumed position.
        // (The error flag is read once: checking it again after the swap
        // always saw false, so a disabled or unplugged device that came back
        // under the same name was never reopened and playback stayed silent.)
        // Some backends report recoverable hiccups through the same error
        // callback, so only act on an error if the device then stops asking
        // for audio.
        if atoms.device_error.swap(false, Ordering::Relaxed) && suspect.is_none() {
            suspect = Some((Instant::now(), atoms.callbacks.load(Ordering::Relaxed)));
        }
        let device_error = match suspect {
            Some((at, cbs)) if at.elapsed() > Duration::from_millis(400) => {
                suspect = None;
                atoms.callbacks.load(Ordering::Relaxed) == cbs
            }
            _ => false,
        };
        if device_error || last_dev_check.elapsed() > Duration::from_secs(2) {
            let changed = last_dev_check.elapsed() > Duration::from_secs(2) && (no_output.is_some() || default_device_name().is_some_and(|n| n != out.name));
            last_dev_check = Instant::now();
            if changed || device_error {
                let pos = shared.lock().current.as_ref().map(|(_, base, fr, _)| base + fr.load(Ordering::Relaxed) as f64 / out.rate as f64);
                let built = build_output(&atoms);
                if let Err(e) = &built {
                    // Lost the output and none to replace it: run silent, say
                    // so once, and keep trying (no_output makes every check
                    // try again).
                    if no_output.is_none() {
                        tracing::warn!(error = %e, "audio output lost");
                        no_output = Some(e.clone());
                        out = silent_output();
                        shared.lock().device = out.name.clone();
                        let _ = events.send(EngineEvent::NoOutput { error: e.clone() });
                    }
                }
                if let Ok(o) = built {
                    out = o;
                    no_output = None;
                    {
                        let mut s = shared.lock();
                        s.device = out.name.clone();
                        s.rate = out.rate;
                        s.channels = out.channels;
                    }
                    let _ = events.send(EngineEvent::DeviceChanged { name: out.name.clone() });
                    if let (Some(c), Some(pos)) = (current.take(), pos) {
                        let _ = c.tx.send(WorkerCmd::Stop);
                        let (info, ring) = spawn_deck(c.id, c.spec.clone(), pos, c.gain, &cache, out.rate, &wev_tx);
                        let mut info = info;
                        info.duration = c.duration;
                        info.started = true;
                        let _ = out.msgs.push(CbMsg::SetCurrent(ring));
                        current = Some(info);
                    }
                    if let Some(n) = next.take() {
                        let _ = n.tx.send(WorkerCmd::Stop);
                        let (info, ring) = spawn_deck(n.id, n.spec.clone(), 0.0, n.gain, &cache, out.rate, &wev_tx);
                        let _ = out.msgs.push(CbMsg::SetNext(ring));
                        next = Some(info);
                    }
                    publish(&shared, &current, &next);
                }
            }
        }
    }
}

fn default_device_name() -> Option<String> {
    cpal::default_host().default_output_device().and_then(|d| d.description().ok().map(|x| x.to_string()))
}
