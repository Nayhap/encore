//! Decoder worker: one thread per loaded track.
//!
//! Probes the container (MP4/AAC, WebM/Opus, Ogg, WAV), decodes packets,
//! converts to interleaved stereo f32 at the device rate (rubato FFT
//! resampler when rates differ) and pushes into the track's PCM ring. A seek
//! arrives with a fresh ring; the worker seeks the demuxer, resets the decoder
//! and discards samples before the target.
//!
//! An A–B loop (D26) is kept here, not by the player's clock: the audio
//! pushed into the ring stops at B's frame and goes on with A's, so the wrap
//! is exact and gapless. Positions follow a `Timeline` of marks.
//!
//! An MP4's edit list says how many frames the encoder put before the song
//! (its delay) and how long the song is; the rest of the last packet is
//! padding. Both are cut (`Trim`), so a song that runs into the next one on
//! the album does so without the 50 ms or so of silence they add up to (D5).
//! Song time 0 is the first frame after the delay.

use std::fs::File;
use std::io::SeekFrom;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use parking_lot::Mutex;
use rtrb::Producer;
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};
use symphonia::core::codecs::audio::well_known::{CODEC_ID_AAC, CODEC_ID_OPUS};
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::codecs::registry::CodecRegistry;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::Time;

use crate::cache::AudioCache;
use crate::source::{HttpSource, SourceSpec};

pub(crate) enum WorkerCmd {
    Seek { secs: f64, prod: Producer<f32>, done: Arc<AtomicBool>, timeline: Arc<Mutex<Timeline>>, frames: Arc<AtomicU64> },
    /// Loop between two song times from now on, or stop looping (D26).
    Loop { range: Option<(f64, f64)>, spare: Option<Spare> },
    Stop,
}

#[derive(Debug, Clone)]
pub enum WorkerEvent {
    Ready { id: u64, duration: Option<f64>, codec: String, source_rate: u32, from_cache: bool },
    Failed { id: u64, error: String },
    Seeked { id: u64, secs: f64 },
    /// The decoder had passed B: the loop goes on in the spare ring from
    /// frame `at` of ring generation `ring`.
    Spliced { id: u64, ring: u64, at: u64 },
}

fn codecs() -> &'static CodecRegistry {
    static REG: OnceLock<CodecRegistry> = OnceLock::new();
    REG.get_or_init(|| {
        let mut r = CodecRegistry::new();
        symphonia::default::register_enabled_codecs(&mut r);
        // Opus is not part of symphonia; register the libopus adapter (D5).
        r.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();
        r
    })
}

pub fn codec_name(id: symphonia::core::codecs::audio::AudioCodecId) -> &'static str {
    if id == CODEC_ID_OPUS {
        "opus"
    } else if id == CODEC_ID_AAC {
        "aac"
    } else {
        "pcm/other"
    }
}

/// What of a decoded stream is the song (gapless playback): `lead` frames
/// of encoder delay come first, then `len` frames of the song (unknown when
/// the file doesn't say), then padding. In frames at the source rate.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Trim {
    lead: u64,
    len: Option<u64>,
}

impl Trim {
    /// Of a decoded buffer of `n` frames whose first frame is stream frame
    /// `first`, the range that is the song.
    fn keep(&self, first: u64, n: usize) -> std::ops::Range<usize> {
        let from = self.lead.saturating_sub(first).min(n as u64) as usize;
        let to = match self.len {
            Some(len) => (self.lead + len).saturating_sub(first).min(n as u64) as usize,
            None => n,
        };
        from..to.max(from)
    }
}

/// The body of the `moov` box of an MP4 that starts with `ftyp`, reading
/// only the top-level boxes before it (a few hundred bytes in YouTube's
/// files, so the first sound doesn't wait for more of the download).
fn read_moov(src: &mut dyn MediaSource) -> Option<Vec<u8>> {
    let mut seen = 0usize;
    for i in 0..4 {
        let mut h = [0u8; 8];
        src.read_exact(&mut h).ok()?;
        let size = u32::from_be_bytes(h[..4].try_into().unwrap()) as usize;
        let kind = &h[4..];
        if (i == 0 && kind != b"ftyp") || size < 8 {
            return None;
        }
        seen += size;
        if seen > 64 * 1024 {
            return None;
        }
        let mut body = vec![0u8; size - 8];
        src.read_exact(&mut body).ok()?;
        if kind == b"moov" {
            return Some(body);
        }
        if kind == b"mdat" || kind == b"moof" {
            return None;
        }
    }
    None
}

/// The edit list of an MP4 (ISO BMFF) at the start of `src`, as a `Trim` in
/// the media's timescale (converted to frames once the rate is known), and
/// `src` back at its start. Only the plain case is read: `ftyp`, then a
/// `moov` within the first 64 KiB whose one track has one edit that plays
/// at normal speed, as YouTube's audio files have. Anything else is no trim,
/// and the file plays as it did.
fn mp4_trim(src: &mut dyn MediaSource) -> Option<(Trim, u32)> {
    let moov = read_moov(src);
    // The demuxer reads from the start, whatever was found.
    if src.seek(SeekFrom::Start(0)).is_err() {
        return None;
    }
    let moov = moov?;
    let boxes = |b: &[u8]| -> Vec<([u8; 4], std::ops::Range<usize>)> {
        let mut out = Vec::new();
        let mut o = 0usize;
        while o + 8 <= b.len() {
            let size = u32::from_be_bytes(b[o..o + 4].try_into().unwrap()) as usize;
            let kind: [u8; 4] = b[o + 4..o + 8].try_into().unwrap();
            if size < 8 || o + size > b.len() {
                break;
            }
            out.push((kind, o + 8..o + size));
            o += size;
        }
        out
    };
    let find = |b: &[u8], k: &[u8; 4]| boxes(b).into_iter().find(|(t, _)| t == k).map(|(_, r)| b[r].to_vec());
    let traks: Vec<_> = boxes(&moov).into_iter().filter(|(t, _)| t == b"trak").collect();
    if traks.len() != 1 {
        return None;
    }
    let trak = &moov[traks[0].1.clone()];
    // (version, timescale) of a full box with the mvhd/mdhd layout.
    let timescale = |b: &[u8]| -> Option<u32> {
        let at = if *b.first()? == 1 { 20 } else { 12 };
        Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?)).filter(|t| *t > 0)
    };
    let movie_ts = timescale(&find(&moov, b"mvhd")?)?;
    let media_ts = timescale(&find(&find(trak, b"mdia")?, b"mdhd")?)?;
    let elst = find(&find(trak, b"edts")?, b"elst")?;
    let v1 = *elst.first()? == 1;
    if u32::from_be_bytes(elst.get(4..8)?.try_into().ok()?) != 1 {
        return None;
    }
    let e = elst.get(8..)?;
    let (dur, time, rate) = if v1 {
        (u64::from_be_bytes(e.get(0..8)?.try_into().ok()?), i64::from_be_bytes(e.get(8..16)?.try_into().ok()?), e.get(16..18)?)
    } else {
        (u32::from_be_bytes(e.get(0..4)?.try_into().ok()?) as u64, i32::from_be_bytes(e.get(4..8)?.try_into().ok()?) as i64, e.get(8..10)?)
    };
    if time < 0 || rate != [0, 1] {
        return None;
    }
    // The edit's length is in the movie's timescale; the delay in the media's.
    let len = (dur > 0).then(|| (dur as u128 * media_ts as u128 / movie_ts as u128) as u64);
    Some((Trim { lead: time as u64, len }, media_ts))
}

struct Opened {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    tb: Option<symphonia::core::units::TimeBase>,
    duration: Option<f64>,
    rate: u32,
    codec: &'static str,
    from_cache: bool,
    /// Encoder delay and padding to cut, in frames at `rate`.
    trim: Option<Trim>,
}

impl Opened {
    /// The song's time as stream time: the delay comes first.
    fn lead_secs(&self) -> f64 {
        self.trim.map_or(0.0, |t| t.lead as f64 / self.rate as f64)
    }

    /// The stream frame `pts` stands for, at the source rate.
    fn frame_of(&self, pts: Option<f64>) -> Option<u64> {
        pts.map(|t| (t.max(0.0) * self.rate as f64).round() as u64)
    }

    /// The part of a decoded buffer of `n` frames starting at `pts` (stream
    /// time) that is the song, and the stream time of its first frame.
    fn song_part(&self, pts: Option<f64>, n: usize) -> (std::ops::Range<usize>, Option<f64>) {
        match (self.trim, self.frame_of(pts)) {
            (Some(t), Some(first)) => {
                let r = t.keep(first, n);
                let at = pts.map(|p| p + r.start as f64 / self.rate as f64);
                (r, at)
            }
            _ => (0..n, pts),
        }
    }
}

fn open(spec: &SourceSpec, cache: Option<&AudioCache>) -> Result<Opened, String> {
    let mut from_cache = false;
    let src: Box<dyn MediaSource> = match spec {
        SourceSpec::File(p) => Box::new(File::open(p).map_err(|e| format!("cannot open file: {}", e.kind()))?),
        SourceSpec::Http(h) => match cache.and_then(|c| c.lookup(&h.cache_key)) {
            Some(p) => {
                from_cache = true;
                Box::new(File::open(p).map_err(|e| format!("cannot open cached file: {}", e.kind()))?)
            }
            None => Box::new(HttpSource::open(h.clone(), cache.cloned()).map_err(|e| e.to_string())?),
        },
    };
    let mut src = src;
    let edit = mp4_trim(&mut *src);
    let mss = MediaSourceStream::new(src, Default::default());
    let format = symphonia::default::get_probe()
        .probe(&Hint::new(), mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| format!("unsupported or damaged audio: {e}"))?;
    let track = format.default_track(TrackType::Audio).ok_or("no audio track")?.clone();
    let params = track.codec_params.as_ref().and_then(|p| p.audio()).ok_or("no audio codec parameters")?.clone();
    let rate = params.sample_rate.ok_or("unknown sample rate")?;
    let decoder = codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| format!("no decoder for this codec: {e}"))?;
    let trim = edit.map(|(t, ts)| {
        let at_rate = |v: u64| (v as u128 * rate as u128 / ts as u128) as u64;
        Trim { lead: at_rate(t.lead), len: t.len.map(at_rate) }
    });
    let duration = match (trim.and_then(|t| t.len), track.time_base, track.duration) {
        (Some(len), _, _) => Some(len as f64 / rate as f64),
        (None, Some(tb), Some(d)) => tb.calc_duration(d).map(|t| t.as_nanos() as f64 / 1e9),
        _ => track.num_frames.map(|n| n as f64 / rate as f64),
    };
    Ok(Opened {
        format,
        decoder,
        track_id: track.id,
        tb: track.time_base,
        duration,
        rate,
        codec: codec_name(params.codec),
        from_cache,
        trim,
    })
}

/// Probe a source and return (codec, sample rate, duration) without playing.
pub fn probe_info(spec: &SourceSpec) -> Result<(String, u32, Option<f64>), String> {
    let o = open(spec, None)?;
    Ok((o.codec.to_string(), o.rate, o.duration))
}

struct Conv {
    rs: Option<Fft<f32>>,
    ratio: f64,
    pending: Vec<f32>,
    out: Vec<f32>,
    /// output frames still to drop for the resampler's startup delay
    skip: usize,
    in_total: u64,
    out_total: u64,
}

impl Conv {
    fn new(src: u32, dst: u32) -> Result<Self, String> {
        let rs = if src == dst {
            None
        } else {
            Some(Fft::<f32>::new(src as usize, dst as usize, 1024, 2, FixedSync::Input).map_err(|e| format!("resampler: {e}"))?)
        };
        let skip = rs.as_ref().map(|r| r.output_delay()).unwrap_or(0);
        Ok(Self { rs, ratio: dst as f64 / src as f64, pending: Vec::new(), out: Vec::new(), skip, in_total: 0, out_total: 0 })
    }
    fn reset(&mut self) {
        if let Some(r) = &mut self.rs {
            r.reset();
            self.skip = r.output_delay();
        }
        self.pending.clear();
        self.in_total = 0;
        self.out_total = 0;
    }
    /// Feed interleaved stereo; returns resampled interleaved stereo ready to
    /// push. With `flush`, drains the resampler so the tail is not lost.
    fn feed(&mut self, stereo: &[f32], flush: bool) -> &[f32] {
        self.out.clear();
        let Some(rs) = &mut self.rs else {
            self.out.extend_from_slice(stereo);
            return &self.out;
        };
        self.pending.extend_from_slice(stereo);
        self.in_total += (stereo.len() / 2) as u64;
        let target = (self.in_total as f64 * self.ratio).round() as u64;
        loop {
            let need = rs.input_frames_next();
            let have = self.pending.len() / 2;
            if have < need {
                if !flush || self.out_total >= target {
                    break;
                }
                // Pad with silence to push the remaining signal out.
                self.pending.resize(need * 2, 0.0);
            }
            let out_max = rs.output_frames_max();
            let start = self.out.len();
            self.out.resize(start + out_max * 2, 0.0);
            let inp = InterleavedSlice::new(&self.pending[..need * 2], 2, need).unwrap();
            let mut outp = InterleavedSlice::new_mut(&mut self.out[start..], 2, out_max).unwrap();
            let wrote = rs.process_into_buffer(&inp, &mut outp, None).map(|r| r.1).unwrap_or(0);
            self.out.truncate(start + wrote * 2);
            self.pending.drain(..need * 2);
            // Drop the startup delay.
            if self.skip > 0 {
                let drop = self.skip.min(wrote);
                self.out.drain(start..start + drop * 2);
                self.skip -= drop;
            }
            let produced = (self.out.len() - start) / 2;
            self.out_total += produced as u64;
        }
        if flush && self.out_total > target {
            let extra = (self.out_total - target) as usize;
            let keep = self.out.len().saturating_sub(extra * 2);
            self.out.truncate(keep);
            self.out_total = target;
        }
        &self.out
    }
}

/// Push as much of `data` as fits, waiting while the ring is full. Returns
/// how many samples went in: fewer than `data.len()` when a command arrived
/// (it is left in `pending`) or the ring was abandoned.
fn push_some(prod: &mut Producer<f32>, data: &[f32], rx: &Receiver<WorkerCmd>, pending: &mut Option<WorkerCmd>) -> usize {
    let mut done = 0;
    while done < data.len() {
        if prod.is_abandoned() {
            return done;
        }
        let rest = {
            let (_, rest) = prod.push_partial_slice(&data[done..]);
            rest.len()
        };
        done = data.len() - rest;
        if done < data.len() {
            match rx.try_recv() {
                Ok(cmd) => {
                    *pending = Some(cmd);
                    return done;
                }
                Err(TryRecvError::Disconnected) => {
                    *pending = Some(WorkerCmd::Stop);
                    return done;
                }
                Err(TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(8)),
            }
        }
    }
    done
}

/// Where a deck's frames are in the song (D26). Each mark says: from this
/// frame of the deck's ring (as the callback counts them), the song is at
/// this time. A seek starts a new timeline at its target; an A–B loop adds
/// a mark at every wrap, so positions stay right while the ring holds audio
/// from both sides of a wrap.
#[derive(Debug)]
pub(crate) struct Timeline {
    marks: Vec<(u64, f64)>,
}

impl Timeline {
    pub(crate) fn new(start: f64) -> Arc<Mutex<Timeline>> {
        Arc::new(Mutex::new(Timeline { marks: vec![(0, start)] }))
    }

    /// The song time of `frame`.
    pub(crate) fn at(&self, frame: u64, rate: f64) -> f64 {
        let (f, t) = self.marks.iter().rev().find(|(f, _)| *f <= frame).copied().unwrap_or(self.marks[0]);
        t + frame.saturating_sub(f) as f64 / rate
    }

    /// The first frame at or after `from` that plays song time `t`, if the
    /// frames up to `end` hold it.
    fn frame_of(&self, t: f64, from: u64, end: u64, rate: f64) -> Option<u64> {
        for (i, &(f0, t0)) in self.marks.iter().enumerate() {
            let f1 = self.marks.get(i + 1).map(|m| m.0).unwrap_or(end);
            if t < t0 {
                continue;
            }
            let f = f0 + ((t - t0) * rate).round() as u64;
            if f >= from && f < f1.max(f0 + 1) && f <= end {
                return Some(f);
            }
        }
        None
    }

    /// From `frame` on, the song is at `t` (marks after it are dropped).
    fn mark(&mut self, frame: u64, t: f64) {
        self.marks.retain(|m| m.0 < frame);
        self.marks.push((frame, t));
        // Marks the callback has long passed are of no use; a ring holds a
        // couple of seconds and a loop is at least one, so a few suffice.
        if self.marks.len() > 8 {
            self.marks.remove(0);
        }
    }
}

/// A second ring for an A–B loop whose end the decoder had already passed:
/// the decoder fills it from A, and the callback switches to it at B.
pub(crate) struct Spare {
    pub prod: Producer<f32>,
    pub done: Arc<AtomicBool>,
    /// The ring generation it belongs to (a seek starts a new one).
    pub ring: u64,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_worker(
    id: u64,
    spec: SourceSpec,
    cache: Option<AudioCache>,
    out_rate: u32,
    start_secs: f64,
    mut prod: Producer<f32>,
    mut done: Arc<AtomicBool>,
    mut timeline: Arc<Mutex<Timeline>>,
    frames: Arc<AtomicU64>,
    rx: Receiver<WorkerCmd>,
    events: Sender<WorkerEvent>,
) {
    let mut frames = frames;
    let mut o = match open(&spec, cache.as_ref()) {
        Ok(o) => o,
        Err(e) => {
            done.store(true, Ordering::Release);
            let _ = events.send(WorkerEvent::Failed { id, error: e });
            return;
        }
    };
    let _ = events.send(WorkerEvent::Ready {
        id,
        duration: o.duration,
        codec: o.codec.to_string(),
        source_rate: o.rate,
        from_cache: o.from_cache,
    });
    let mut conv = match Conv::new(o.rate, out_rate) {
        Ok(c) => c,
        Err(e) => {
            done.store(true, Ordering::Release);
            let _ = events.send(WorkerEvent::Failed { id, error: e });
            return;
        }
    };
    let rate = out_rate as f64;
    let mut discard_until: Option<f64> = None;
    let mut pending: Option<WorkerCmd> = None;
    if start_secs > 0.0 {
        discard_until = seek(&mut o, start_secs);
        conv.reset();
    }
    let mut interleaved: Vec<f32> = Vec::new();
    let mut stereo: Vec<f32> = Vec::new();
    // Decoded audio not yet in the ring.
    let mut out: Vec<f32> = Vec::new();
    // Frames pushed into the current ring since it started.
    let mut pushed: u64 = 0;
    // The A–B loop, in song seconds (D26).
    let mut lp: Option<(f64, f64)> = None;
    // The decoder reached the end; what's left in `out` is the tail.
    let mut ending = false;
    let mut eof = false;
    // The demuxer reached the end: it has to be opened again to seek.
    let mut exhausted = false;
    let mut errors = 0u32;
    loop {
        // Commands first.
        let cmd = pending.take().or_else(|| match rx.try_recv() {
            Ok(c) => Some(c),
            Err(TryRecvError::Disconnected) => Some(WorkerCmd::Stop),
            Err(TryRecvError::Empty) => None,
        });
        match cmd {
            Some(WorkerCmd::Stop) => return,
            Some(WorkerCmd::Seek { secs, prod: p, done: d, timeline: tl, frames: fr }) => {
                prod = p;
                done = d;
                timeline = tl;
                frames = fr;
                pushed = 0;
                out.clear();
                ending = false;
                discard_until = reseek(&mut o, &spec, cache.as_ref(), secs, &mut exhausted);
                conv.reset();
                eof = false;
                let _ = events.send(WorkerEvent::Seeked { id, secs });
                continue;
            }
            Some(WorkerCmd::Loop { range, spare }) => {
                lp = range;
                let Some((a, b)) = range else { continue };
                // The ring may already hold audio past B: then the loop
                // starts over in the spare ring, which the callback takes
                // up at B's frame (or at once, if it is past B already).
                let heard = frames.load(Ordering::Relaxed);
                let splice = {
                    let tl = timeline.lock();
                    let past = tl.at(pushed, rate) > b + 0.5 / rate;
                    past.then(|| tl.frame_of(b, heard, pushed, rate).unwrap_or(heard))
                };
                let Some(at) = splice else {
                    if eof {
                        // The whole song was decoded and B is its end: go
                        // round instead of finishing.
                        done.store(false, Ordering::Release);
                        eof = false;
                        timeline.lock().mark(pushed, a);
                        discard_until = reseek(&mut o, &spec, cache.as_ref(), a, &mut exhausted);
                        conv.reset();
                    }
                    continue;
                };
                let Some(sp) = spare else { continue };
                prod = sp.prod;
                done = sp.done;
                pushed = at;
                timeline.lock().mark(at, a);
                out.clear();
                ending = false;
                eof = false;
                discard_until = reseek(&mut o, &spec, cache.as_ref(), a, &mut exhausted);
                conv.reset();
                let _ = events.send(WorkerEvent::Spliced { id, ring: sp.ring, at });
                continue;
            }
            None => {}
        }
        if out.is_empty() {
            if ending {
                ending = false;
                if let Some((a, _)) = lp {
                    // The loop ends at the end of the song.
                    timeline.lock().mark(pushed, a);
                    discard_until = reseek(&mut o, &spec, cache.as_ref(), a, &mut exhausted);
                    conv.reset();
                } else {
                    done.store(true, Ordering::Release);
                    eof = true;
                }
                continue;
            }
            if eof {
                match rx.recv_timeout(Duration::from_millis(200)) {
                    Ok(c) => pending = Some(c),
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                    Err(_) => {}
                }
                continue;
            }
            let packet = match o.format.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) => {
                    exhausted = true;
                    out.extend_from_slice(conv.feed(&[], true));
                    ending = true;
                    continue;
                }
                Err(SymError::ResetRequired) => {
                    done.store(true, Ordering::Release);
                    eof = true;
                    continue;
                }
                Err(e) => {
                    // An IO error here is a network failure that the source
                    // already retried; end this track with the reason.
                    done.store(true, Ordering::Release);
                    let _ = events.send(WorkerEvent::Failed { id, error: format!("{e}") });
                    eof = true;
                    continue;
                }
            };
            if packet.track_id != o.track_id {
                continue;
            }
            let pts = o.tb.and_then(|tb| tb.calc_time(packet.pts)).map(|t| t.as_nanos() as f64 / 1e9);
            let buf = match o.decoder.decode(&packet) {
                Ok(b) => b,
                Err(SymError::DecodeError(_)) | Err(SymError::IoError(_)) => {
                    errors += 1;
                    if errors > 50 {
                        done.store(true, Ordering::Release);
                        let _ = events.send(WorkerEvent::Failed { id, error: "too many decode errors".into() });
                        eof = true;
                    }
                    continue;
                }
                Err(e) => {
                    done.store(true, Ordering::Release);
                    let _ = events.send(WorkerEvent::Failed { id, error: format!("decoder failed: {e}") });
                    eof = true;
                    continue;
                }
            };
            let ch = buf.spec().channels().count().max(1);
            let n = buf.frames();
            interleaved.resize(n * ch, 0.0);
            buf.copy_to_slice_interleaved(&mut interleaved[..]);
            stereo.clear();
            stereo.reserve(n * 2);
            for f in 0..n {
                let l = interleaved[f * ch];
                let r = if ch > 1 { interleaved[f * ch + 1] } else { l };
                stereo.push(l);
                stereo.push(r);
            }
            // Encoder delay and padding are not the song (gapless).
            let (keep, pts) = o.song_part(pts, n);
            let n = keep.len();
            let mut slice: &[f32] = &stereo[keep.start * 2..keep.end * 2];
            if n == 0 {
                continue;
            }
            if let (Some(target), Some(start)) = (discard_until, pts) {
                let end = start + n as f64 / o.rate as f64;
                if end <= target {
                    continue;
                }
                let skip = (((target - start).max(0.0)) * o.rate as f64) as usize;
                slice = &slice[(skip * 2).min(slice.len())..];
                discard_until = None;
            } else if discard_until.is_some() && pts.is_none() {
                discard_until = None;
            }
            out.extend_from_slice(conv.feed(slice, false));
            continue;
        }
        // Into the ring, cut at B while looping: the frame that would play B
        // is followed by A's, so the wrap is exact to the frame.
        let n = out.len() / 2;
        let mut take = n;
        let mut wrap = None;
        if let Some((a, b)) = lp {
            let t0 = timeline.lock().at(pushed, rate);
            if t0 + n as f64 / rate >= b {
                take = (((b - t0) * rate).round().max(0.0) as usize).min(n);
                wrap = Some(a);
            }
        }
        let went = push_some(&mut prod, &out[..take * 2], &rx, &mut pending);
        pushed += (went / 2) as u64;
        out.drain(..went);
        if went < take * 2 {
            continue; // a command came first
        }
        if let Some(a) = wrap {
            out.clear();
            ending = false;
            timeline.lock().mark(pushed, a);
            discard_until = reseek(&mut o, &spec, cache.as_ref(), a, &mut exhausted);
            conv.reset();
        }
    }
}

/// `seek`, opening the source again first when its demuxer has reached the
/// end (neither the MP4 nor the WebM reader seeks back from there) or when
/// the seek fails. A loop that ends at the end of a song needs this, and so
/// does seeking back in a song's last seconds.
fn reseek(o: &mut Opened, spec: &SourceSpec, cache: Option<&AudioCache>, secs: f64, exhausted: &mut bool) -> Option<f64> {
    if !*exhausted && let Some(t) = seek(o, secs) {
        return Some(t);
    }
    *exhausted = false;
    match open(spec, cache) {
        Ok(n) => {
            *o = n;
            seek(o, secs)
        }
        Err(e) => {
            tracing::warn!(error = %e, "couldn't open the source again to seek");
            None
        }
    }
}

/// Seek to song time `secs`. Returns the stream time to discard up to.
fn seek(o: &mut Opened, secs: f64) -> Option<f64> {
    let secs = secs.max(0.0) + o.lead_secs();
    let time = Time::try_from_secs_f64(secs)?;
    match o.format.seek(SeekMode::Accurate, SeekTo::Time { time, track_id: Some(o.track_id) }) {
        Ok(_) => {
            o.decoder.reset();
            Some(secs)
        }
        Err(_) => {
            o.decoder.reset();
            None
        }
    }
}

/// Decode a whole source offline to interleaved stereo at its own rate.
/// Used by tests and `encore-cli probe-file`.
pub fn decode_all(spec: &SourceSpec, start: f64) -> Result<(Vec<f32>, u32, &'static str), String> {
    let mut o = open(spec, None)?;
    let mut discard = if start > 0.0 { seek(&mut o, start) } else { None };
    let mut out = Vec::new();
    let mut tmp = Vec::new();
    while let Some(p) = o.format.next_packet().map_err(|e| e.to_string())? {
        if p.track_id != o.track_id {
            continue;
        }
        let pts = o.tb.and_then(|tb| tb.calc_time(p.pts)).map(|t| t.as_nanos() as f64 / 1e9);
        let buf = match o.decoder.decode(&p) {
            Ok(b) => b,
            Err(SymError::DecodeError(_)) => continue,
            Err(e) => return Err(e.to_string()),
        };
        let ch = buf.spec().channels().count().max(1);
        let frames = buf.frames();
        tmp.resize(frames * ch, 0.0f32);
        buf.copy_to_slice_interleaved(&mut tmp[..]);
        let (keep, pts) = o.song_part(pts, frames);
        let mut skip = keep.start;
        if let (Some(t), Some(s)) = (discard, pts) {
            if s + keep.len() as f64 / o.rate as f64 <= t {
                continue;
            }
            skip += ((t - s).max(0.0) * o.rate as f64) as usize;
            discard = None;
        }
        for f in skip.min(keep.end)..keep.end {
            let l = tmp[f * ch];
            out.push(l);
            out.push(if ch > 1 { tmp[f * ch + 1] } else { l });
        }
    }
    Ok((out, o.rate, o.codec))
}

/// Check that a finished download will play (D23): the container opens with
/// the decoders Encore registers, and its first `packets` audio packets
/// decode. Returns the codec and the duration, when the container gives one.
pub fn validate_file(path: &std::path::Path, packets: usize) -> Result<(&'static str, Option<f64>), String> {
    let mut o = open(&SourceSpec::File(path.to_path_buf()), None)?;
    let mut decoded = 0usize;
    let mut errors = 0usize;
    while decoded < packets {
        let p = match o.format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(e) => return Err(format!("damaged audio: {e}")),
        };
        if p.track_id != o.track_id {
            continue;
        }
        match o.decoder.decode(&p) {
            Ok(_) => decoded += 1,
            Err(SymError::DecodeError(_)) => {
                errors += 1;
                if errors > 8 {
                    return Err("too many decode errors".into());
                }
            }
            Err(e) => return Err(format!("decoder failed: {e}")),
        }
    }
    if decoded == 0 {
        return Err("no audio could be decoded".into());
    }
    Ok((o.codec, o.duration))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(n: &str) -> SourceSpec {
        SourceSpec::File(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(n))
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    /// Count positive-going zero crossings of the left channel per second.
    fn freq(x: &[f32], rate: u32) -> f32 {
        let l: Vec<f32> = x.iter().step_by(2).copied().collect();
        let n = l.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        n as f32 / (l.len() as f32 / rate as f32)
    }

    /// A test worker's ring, timeline, counter and channels.
    type TestWorker = (rtrb::Consumer<f32>, Arc<Mutex<Timeline>>, Arc<AtomicU64>, Sender<WorkerCmd>, Receiver<WorkerEvent>);

    /// Start a worker on a fixture with an output ring of `secs` seconds.
    fn worker(n: &str, secs: f64) -> TestWorker {
        let (p, c) = rtrb::RingBuffer::<f32>::new((48000.0 * secs) as usize * 2);
        let (tx, rx) = std::sync::mpsc::channel();
        let (etx, erx) = std::sync::mpsc::channel();
        let tl = Timeline::new(0.0);
        let frames = Arc::new(AtomicU64::new(0));
        let (t2, f2, spec) = (tl.clone(), frames.clone(), fixture(n));
        std::thread::spawn(move || run_worker(1, spec, None, 48000, 0.0, p, Arc::new(AtomicBool::new(false)), t2, f2, rx, etx));
        (c, tl, frames, tx, erx)
    }

    /// Read `n` frames from the ring as the callback would, counting them.
    fn drain(c: &mut rtrb::Consumer<f32>, frames: &AtomicU64, n: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(n * 2);
        let t0 = std::time::Instant::now();
        while out.len() < n * 2 && t0.elapsed() < Duration::from_secs(10) {
            let k = (c.slots() & !1).min(n * 2 - out.len());
            if k == 0 {
                std::thread::sleep(Duration::from_millis(2));
                continue;
            }
            let chunk = c.read_chunk(k).unwrap();
            let (a, b) = chunk.as_slices();
            out.extend_from_slice(a);
            out.extend_from_slice(b);
            chunk.commit_all();
            frames.fetch_add((k / 2) as u64, Ordering::Relaxed);
        }
        out
    }

    #[test]
    fn a_loop_wraps_at_b_to_the_frame() {
        let (mut c, tl, frames, tx, _ev) = worker("tone-opus.webm", 0.25);
        tx.send(WorkerCmd::Loop { range: Some((0.5, 1.0)), spare: None }).unwrap();
        // Three times round the half-second loop, after the first second.
        let pcm = drain(&mut c, &frames, 48000 * 5 / 2);
        assert_eq!(pcm.len(), 48000 * 5, "nothing is lost at a wrap");
        assert!(rms(&pcm[48000 * 2..]) > 0.05, "the loop plays sound");
        let tl = tl.lock();
        let wraps: Vec<(u64, f64)> = tl.marks.iter().copied().filter(|m| m.0 > 0).collect();
        assert!(wraps.len() >= 3, "{wraps:?}");
        assert_eq!(wraps[0], (48000, 0.5), "the first wrap is at B's frame");
        for w in wraps.windows(2) {
            assert_eq!(w[1].0 - w[0].0, 24000, "each time round is exactly A to B");
        }
        let at = |f: u64| tl.at(f, 48000.0);
        assert!((0.5..=1.0).contains(&at(48000 * 2)), "positions stay inside the loop");
        tx.send(WorkerCmd::Stop).unwrap();
    }

    #[test]
    fn a_loop_to_the_end_of_the_song_goes_round() {
        // AAC at 44.1 kHz, resampled: the wrap comes at the end of the file.
        let (mut c, tl, frames, tx, _ev) = worker("tone-aac.m4a", 0.25);
        tx.send(WorkerCmd::Loop { range: Some((1.0, 60.0)), spare: None }).unwrap();
        let pcm = drain(&mut c, &frames, 48000 * 4);
        assert_eq!(pcm.len(), 48000 * 8, "the song never ends while it loops");
        assert!(rms(&pcm[48000 * 5..]) > 0.05, "and keeps sounding after the wrap");
        assert!(tl.lock().marks.iter().filter(|m| m.1 == 1.0).count() >= 2);
        tx.send(WorkerCmd::Stop).unwrap();
    }

    #[test]
    fn a_loop_set_behind_the_decoder_goes_on_in_the_spare_ring() {
        // A ring longer than the song: the decoder is done before the loop.
        let (mut c, tl, frames, tx, ev) = worker("tone-opus.webm", 3.0);
        drain(&mut c, &frames, 4800); // 0.1 s heard
        let t0 = std::time::Instant::now();
        while c.slots() < 48000 * 2 * 18 / 10 && t0.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(5));
        }
        let (p, mut spare) = rtrb::RingBuffer::<f32>::new(48000 * 2);
        tx.send(WorkerCmd::Loop { range: Some((0.2, 0.6)), spare: Some(Spare { prod: p, done: Arc::new(AtomicBool::new(false)), ring: 0 }) }).unwrap();
        let at = loop {
            match ev.recv_timeout(Duration::from_secs(5)).unwrap() {
                WorkerEvent::Spliced { at, ring, .. } => {
                    assert_eq!(ring, 0);
                    break at;
                }
                _ => continue,
            }
        };
        assert_eq!(at, (0.6 * 48000.0) as u64, "the switch is at B's frame");
        assert_eq!(tl.lock().at(at, 48000.0), 0.2, "and plays A from there");
        let more = drain(&mut spare, &AtomicU64::new(0), 48000);
        assert!(rms(&more) > 0.05);
        assert!(tl.lock().marks.iter().filter(|m| m.1 == 0.2).count() >= 3, "it wraps in the spare ring too");
        tx.send(WorkerCmd::Stop).unwrap();
    }

    #[test]
    fn decodes_aac_m4a() {
        let (pcm, rate, codec) = decode_all(&fixture("tone-aac.m4a"), 0.0).unwrap();
        assert_eq!(codec, "aac");
        assert_eq!(rate, 44100);
        let secs = pcm.len() as f32 / 2.0 / rate as f32;
        assert!((secs - 2.0).abs() < 0.05, "duration {secs}");
        assert!(rms(&pcm) > 0.05);
        let f = freq(&pcm[8820..pcm.len() - 8820], rate);
        assert!((f - 440.0).abs() < 3.0, "freq {f}");
    }

    #[test]
    fn trim_keeps_only_the_song() {
        let t = Trim { lead: 1600, len: Some(10_000) };
        assert_eq!(t.keep(0, 1024), 1024..1024, "all delay");
        assert_eq!(t.keep(1024, 1024), 576..1024, "delay ends inside");
        assert_eq!(t.keep(4096, 1024), 0..1024, "all song");
        assert_eq!(t.keep(11_264, 1024), 0..336, "padding starts at frame 11,600");
        assert_eq!(t.keep(12_288, 1024), 0..0, "all padding");
        assert_eq!(Trim { lead: 0, len: None }.keep(0, 1024), 0..1024);
    }

    #[test]
    fn an_mp4_plays_exactly_its_edit() {
        // The fixture's edit list: 1,024 frames of delay, then 2 s (2,000
        // in a timescale of 1,000) of a 440 Hz tone at 44.1 kHz.
        let mut f = std::fs::File::open(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/tone-aac.m4a")).unwrap();
        let (t, ts) = mp4_trim(&mut f).unwrap();
        assert_eq!((t, ts), (Trim { lead: 1024, len: Some(88_200) }, 44_100));
        assert_eq!(std::io::Seek::stream_position(&mut f).unwrap(), 0, "rewound for the demuxer");
        let (pcm, _, _) = decode_all(&fixture("tone-aac.m4a"), 0.0).unwrap();
        assert_eq!(pcm.len() / 2, 88_200, "neither delay nor padding");
        let level = rms(&pcm);
        assert!(rms(&pcm[..2 * 512]) > level * 0.8, "the tone from the first frame: {} of {level}", rms(&pcm[..2 * 512]));
        assert!(rms(&pcm[pcm.len() - 2 * 512..]) > level * 0.8, "and to the last: {} of {level}", rms(&pcm[pcm.len() - 2 * 512..]));
        // A seek counts song time, after the delay.
        let (tail, _, _) = decode_all(&fixture("tone-aac.m4a"), 1.5).unwrap();
        assert!((tail.len() as i64 / 2 - 22_050).abs() <= 2, "{} frames after 1.5 s", tail.len() / 2);
        // Not an MP4: no trim.
        let mut w = std::fs::File::open(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/tone-opus.webm")).unwrap();
        assert!(mp4_trim(&mut w).is_none());
    }

    #[test]
    fn validates_whole_files_and_refuses_cut_ones() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures");
        for (n, c) in [("tone-aac.m4a", "aac"), ("tone-opus.webm", "opus")] {
            assert_eq!(validate_file(&dir.join(n), 16).unwrap().0, c);
        }
        let cut = std::env::temp_dir().join(format!("encore-cut-{}.m4a", std::process::id()));
        std::fs::write(&cut, &std::fs::read(dir.join("tone-aac.m4a")).unwrap()[..200]).unwrap();
        assert!(validate_file(&cut, 16).is_err());
        let _ = std::fs::remove_file(&cut);
    }

    #[test]
    fn decodes_opus_webm_with_registered_libopus() {
        let (pcm, rate, codec) = decode_all(&fixture("tone-opus.webm"), 0.0).unwrap();
        assert_eq!(codec, "opus");
        assert_eq!(rate, 48000);
        let secs = pcm.len() as f32 / 2.0 / rate as f32;
        assert!((secs - 2.0).abs() < 0.05, "duration {secs}");
        let f = freq(&pcm[9600..pcm.len() - 9600], rate);
        assert!((f - 660.0).abs() < 3.0, "freq {f}");
    }

    #[test]
    fn seek_discards_to_target() {
        let (all, rate, _) = decode_all(&fixture("tone-opus.webm"), 0.0).unwrap();
        let (tail, _, _) = decode_all(&fixture("tone-opus.webm"), 1.5).unwrap();
        let want = all.len() as f64 / 2.0 / rate as f64 - 1.5;
        let got = tail.len() as f64 / 2.0 / rate as f64;
        assert!((want - got).abs() < 0.03, "want {want} got {got}");
    }

    #[test]
    fn resampler_converts_rate() {
        let (pcm, rate, _) = decode_all(&fixture("tone-aac.m4a"), 0.0).unwrap();
        let mut c = Conv::new(rate, 48000).unwrap();
        let mut out = c.feed(&pcm, false).to_vec();
        out.extend_from_slice(c.feed(&[], true));
        let secs = out.len() as f64 / 2.0 / 48000.0;
        let want = pcm.len() as f64 / 2.0 / rate as f64;
        assert!((secs - want).abs() < 0.002, "resampled length {secs} vs {want}");
        // No leading silence from the resampler delay (synthetic sine).
        let sine: Vec<f32> = (0..44100).flat_map(|i| {
            let v = (i as f32 * 440.0 * std::f32::consts::TAU / 44100.0).sin() * 0.5;
            [v, v]
        }).collect();
        let mut c2 = Conv::new(44100, 48000).unwrap();
        let o2 = c2.feed(&sine, false).to_vec();
        assert!(o2[..200].iter().any(|s| s.abs() > 0.05), "leading silence");
        let f = freq(&out[4800..out.len() - 4800], 48000);
        assert!((f - 440.0).abs() < 3.0, "freq {f}");
    }
}

