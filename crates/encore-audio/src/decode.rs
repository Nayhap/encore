//! Decoder worker: one thread per loaded track.
//!
//! Probes the container (MP4/AAC, WebM/Opus, Ogg, WAV), decodes packets,
//! converts to interleaved stereo f32 at the device rate (rubato FFT
//! resampler when rates differ) and pushes into the track's PCM ring. A seek
//! arrives with a fresh ring; the worker seeks the demuxer, resets the decoder
//! and discards samples before the target.

use std::fs::File;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

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
    Seek { secs: f64, prod: Producer<f32>, done: Arc<AtomicBool> },
    Stop,
}

#[derive(Debug, Clone)]
pub enum WorkerEvent {
    Ready { id: u64, duration: Option<f64>, codec: String, source_rate: u32, from_cache: bool },
    Failed { id: u64, error: String },
    Seeked { id: u64, secs: f64 },
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

struct Opened {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    tb: Option<symphonia::core::units::TimeBase>,
    duration: Option<f64>,
    rate: u32,
    codec: &'static str,
    from_cache: bool,
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
    let duration = match (track.time_base, track.duration) {
        (Some(tb), Some(d)) => tb.calc_duration(d).map(|t| t.as_nanos() as f64 / 1e9),
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

/// Push all samples, waiting while the ring is full. Returns false if a
/// command arrived that interrupts this ring (seek/stop).
fn push_all(prod: &mut Producer<f32>, mut data: &[f32], rx: &Receiver<WorkerCmd>, pending: &mut Option<WorkerCmd>) -> bool {
    while !data.is_empty() {
        if prod.is_abandoned() {
            return false;
        }
        let (pushed, rest) = {
            let (a, b) = prod.push_partial_slice(data);
            (a.len(), b)
        };
        let _ = pushed;
        data = rest;
        if !data.is_empty() {
            match rx.try_recv() {
                Ok(cmd) => {
                    *pending = Some(cmd);
                    return false;
                }
                Err(TryRecvError::Disconnected) => {
                    *pending = Some(WorkerCmd::Stop);
                    return false;
                }
                Err(TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(8)),
            }
        }
    }
    true
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
    rx: Receiver<WorkerCmd>,
    events: Sender<WorkerEvent>,
) {
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
    let mut discard_until: Option<f64> = None;
    let mut pending: Option<WorkerCmd> = None;
    if start_secs > 0.0 {
        discard_until = seek(&mut o, start_secs);
        conv.reset();
    }
    let mut interleaved: Vec<f32> = Vec::new();
    let mut stereo: Vec<f32> = Vec::new();
    let mut eof = false;
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
            Some(WorkerCmd::Seek { secs, prod: p, done: d }) => {
                prod = p;
                done = d;
                discard_until = seek(&mut o, secs);
                conv.reset();
                eof = false;
                let _ = events.send(WorkerEvent::Seeked { id, secs });
                continue;
            }
            None => {}
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
                let tail = conv.feed(&[], true).to_vec();
                push_all(&mut prod, &tail, &rx, &mut pending);
                done.store(true, Ordering::Release);
                eof = true;
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
        let frames = buf.frames();
        interleaved.resize(frames * ch, 0.0);
        buf.copy_to_slice_interleaved(&mut interleaved[..]);
        stereo.clear();
        stereo.reserve(frames * 2);
        for f in 0..frames {
            let l = interleaved[f * ch];
            let r = if ch > 1 { interleaved[f * ch + 1] } else { l };
            stereo.push(l);
            stereo.push(r);
        }
        let mut slice: &[f32] = &stereo;
        if let (Some(target), Some(start)) = (discard_until, pts) {
            let end = start + frames as f64 / o.rate as f64;
            if end <= target {
                continue;
            }
            let skip = (((target - start).max(0.0)) * o.rate as f64) as usize;
            slice = &stereo[(skip * 2).min(stereo.len())..];
            discard_until = None;
        } else if discard_until.is_some() && pts.is_none() {
            discard_until = None;
        }
        let out = conv.feed(slice, false).to_vec();
        if !push_all(&mut prod, &out, &rx, &mut pending) {
            continue;
        }
    }
}

fn seek(o: &mut Opened, secs: f64) -> Option<f64> {
    let time = Time::try_from_secs_f64(secs.max(0.0))?;
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
        let mut skip = 0;
        if let (Some(t), Some(s)) = (discard, pts) {
            if s + frames as f64 / o.rate as f64 <= t {
                continue;
            }
            skip = ((t - s).max(0.0) * o.rate as f64) as usize;
            discard = None;
        }
        for f in skip..frames {
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
