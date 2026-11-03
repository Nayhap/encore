//! Tempo and key, estimated from a song's own audio file (D28).
//!
//! Only files Encore already has (a download, or a complete file in the
//! audio cache) are read; nothing is fetched and nothing taps playback. The
//! file is decoded once, folded to mono at about 11 kHz, and then:
//!
//! * **tempo**: a spectral-flux onset envelope (86 frames a second), its
//!   autocorrelation between 60 and 200 BPM weighted towards 120 BPM, the
//!   peak refined between frames;
//! * **key**: a chroma profile over the whole song (55 Hz to 1.76 kHz, each
//!   frame normalised so loud passages don't outvote quiet ones),
//!   correlated with the Krumhansl–Kessler major and minor profiles in all
//!   twelve keys.
//!
//! Each comes with a confidence; below its threshold the value is `None`, and
//! the UI shows "—" rather than a guess. These are estimates and are always
//! labelled so.

use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use rustfft::FftPlanner;
use rustfft::num_complex::Complex;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::registry::CodecRegistry;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// Bump when the estimator changes: older results are worked out again.
pub const VERSION: u32 = 1;

/// The rate the analysis runs at, roughly: the file's rate divided down.
const TARGET_RATE: u32 = 11_025;
/// At most this much of a song is read (a long mix says enough by then).
const MAX_SECS: f64 = 600.0;
/// Below these, the value is not shown.
pub const TEMPO_MIN: f64 = 0.25;
pub const KEY_MIN: f64 = 0.6;

#[derive(Clone, Debug, PartialEq)]
pub struct Estimate {
    /// Beats per minute, rounded to a tenth.
    pub bpm: Option<f64>,
    /// "F♯ minor".
    pub key: Option<String>,
    pub tempo_confidence: f64,
    pub key_confidence: f64,
}

#[derive(Debug, PartialEq)]
pub enum Error {
    Cancelled,
    /// The file couldn't be opened or decoded; the text says why.
    Unreadable(String),
    /// Too little audio to say anything (under 10 seconds).
    TooShort,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Cancelled => f.write_str("cancelled"),
            Error::Unreadable(m) => write!(f, "couldn't read the file: {m}"),
            Error::TooShort => f.write_str("too short to estimate"),
        }
    }
}

fn codecs() -> &'static CodecRegistry {
    static REG: OnceLock<CodecRegistry> = OnceLock::new();
    REG.get_or_init(|| {
        let mut r = CodecRegistry::new();
        symphonia::default::register_enabled_codecs(&mut r);
        // Opus is not part of Symphonia (D5).
        r.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();
        r
    })
}

/// Decode a file to mono near [`TARGET_RATE`]. Checks `cancel` between packets.
pub fn decode_mono(path: &Path, cancel: &AtomicBool) -> Result<(Vec<f32>, u32), Error> {
    let bad = |e: String| Error::Unreadable(e);
    let file = std::fs::File::open(path).map_err(|e| bad(format!("{}", e.kind())))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut format = symphonia::default::get_probe()
        .probe(&Hint::new(), mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| bad(format!("unsupported or damaged audio: {e}")))?;
    let track = format.default_track(TrackType::Audio).ok_or_else(|| bad("no audio track".into()))?.clone();
    let params = track.codec_params.as_ref().and_then(|p| p.audio()).ok_or_else(|| bad("no audio codec parameters".into()))?.clone();
    let rate = params.sample_rate.ok_or_else(|| bad("unknown sample rate".into()))?;
    let mut decoder = codecs().make_audio_decoder(&params, &AudioDecoderOptions::default()).map_err(|e| bad(format!("no decoder for this codec: {e}")))?;
    let factor = (rate / TARGET_RATE).max(1) as usize;
    let out_rate = rate / factor as u32;
    let max = (MAX_SECS * out_rate as f64) as usize;
    let mut out = Vec::with_capacity((out_rate as usize) * 300);
    let mut tmp = Vec::new();
    let (mut acc, mut n) = (0f32, 0usize);
    let mut errors = 0;
    while out.len() < max {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let p = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymError::IoError(_)) => break,
            Err(e) => return Err(bad(e.to_string())),
        };
        if p.track_id != track.id {
            continue;
        }
        let buf = match decoder.decode(&p) {
            Ok(b) => b,
            Err(SymError::DecodeError(_)) => {
                errors += 1;
                if errors > 64 {
                    return Err(bad("too many decode errors".into()));
                }
                continue;
            }
            Err(e) => return Err(bad(e.to_string())),
        };
        let ch = buf.spec().channels().count().max(1);
        let frames = buf.frames();
        tmp.resize(frames * ch, 0.0f32);
        buf.copy_to_slice_interleaved(&mut tmp[..]);
        // Mono, then a box average over `factor` samples on the way down.
        for f in 0..frames {
            let s: f32 = tmp[f * ch..(f + 1) * ch].iter().sum::<f32>() / ch as f32;
            acc += s;
            n += 1;
            if n == factor {
                out.push(acc / factor as f32);
                acc = 0.0;
                n = 0;
            }
        }
    }
    Ok((out, out_rate))
}

/// Estimate a file's tempo and key.
pub fn analyse_file(path: &Path, cancel: &AtomicBool) -> Result<Estimate, Error> {
    let (mono, rate) = decode_mono(path, cancel)?;
    estimate(&mono, rate, cancel)
}

/// Estimate from mono samples at `rate`.
pub fn estimate(mono: &[f32], rate: u32, cancel: &AtomicBool) -> Result<Estimate, Error> {
    if mono.len() < rate as usize * 10 {
        return Err(Error::TooShort);
    }
    let (bpm, tc) = tempo(mono, rate, cancel)?;
    let (key, kc) = key(mono, rate, cancel)?;
    Ok(Estimate {
        bpm: (tc >= TEMPO_MIN).then(|| (bpm * 10.0).round() / 10.0),
        key: (kc >= KEY_MIN).then_some(key),
        tempo_confidence: tc,
        key_confidence: kc,
    })
}

fn hann(n: usize) -> Vec<f32> {
    (0..n).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / n as f32).cos()).collect()
}

/// Magnitude spectra of `x`, frame by frame. `f` sees each frame's bins
/// (0 to n/2); returning `false` stops early.
fn spectra(x: &[f32], n: usize, hop: usize, cancel: &AtomicBool, mut f: impl FnMut(&[f32])) -> Result<(), Error> {
    let fft = FftPlanner::<f32>::new().plan_fft_forward(n);
    let win = hann(n);
    let mut buf = vec![Complex::new(0.0f32, 0.0); n];
    let mut scratch = vec![Complex::new(0.0f32, 0.0); fft.get_inplace_scratch_len()];
    let mut mag = vec![0f32; n / 2 + 1];
    let mut start = 0;
    let mut i = 0usize;
    while start + n <= x.len() {
        i += 1;
        if i.is_multiple_of(256) && cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        for (k, b) in buf.iter_mut().enumerate() {
            *b = Complex::new(x[start + k] * win[k], 0.0);
        }
        fft.process_with_scratch(&mut buf, &mut scratch);
        for (k, m) in mag.iter_mut().enumerate() {
            *m = buf[k].norm();
        }
        f(&mag);
        start += hop;
    }
    Ok(())
}

/// (BPM, confidence 0..1).
fn tempo(x: &[f32], rate: u32, cancel: &AtomicBool) -> Result<(f64, f64), Error> {
    const N: usize = 1024;
    const HOP: usize = 128;
    let fps = rate as f64 / HOP as f64;
    // Spectral flux of log magnitudes, half-wave rectified.
    let mut prev: Vec<f32> = Vec::new();
    let mut flux = Vec::new();
    spectra(x, N, HOP, cancel, |mag| {
        let cur: Vec<f32> = mag.iter().map(|m| (1.0 + 100.0 * m).ln()).collect();
        if !prev.is_empty() {
            flux.push(cur.iter().zip(&prev).map(|(c, p)| (c - p).max(0.0)).sum::<f32>() as f64);
        }
        prev = cur;
    })?;
    // Minus a moving average (one second), so slow swells aren't beats.
    let w = (fps / 2.0) as usize;
    let mut prefix = vec![0f64; flux.len() + 1];
    for (i, v) in flux.iter().enumerate() {
        prefix[i + 1] = prefix[i] + v;
    }
    let env: Vec<f64> = (0..flux.len())
        .map(|i| {
            let (a, b) = (i.saturating_sub(w), (i + w + 1).min(flux.len()));
            (flux[i] - (prefix[b] - prefix[a]) / (b - a) as f64).max(0.0)
        })
        .collect();
    let n = env.len();
    let acf = |lag: usize| -> f64 {
        if lag >= n {
            return 0.0;
        }
        env[..n - lag].iter().zip(&env[lag..]).map(|(a, b)| a * b).sum::<f64>() / (n - lag) as f64
    };
    let a0 = acf(0);
    if a0 <= 1e-12 {
        return Ok((0.0, 0.0));
    }
    let lo = (fps * 60.0 / 200.0).floor() as usize;
    let hi = (fps * 60.0 / 60.0).ceil() as usize;
    let lags: Vec<f64> = (0..=hi * 2 + 1).map(acf).collect();
    let mean = lags[lo..=hi].iter().sum::<f64>() / (hi - lo + 1) as f64;
    let mut best = (lo, f64::MIN);
    for lag in lo..=hi {
        let bpm = 60.0 * fps / lag as f64;
        // Towards 120 BPM, an octave either side is about half as likely.
        let prior = (-0.5 * (bpm / 120.0).log2().powi(2)).exp();
        // A beat's period also shows at twice its lag.
        let score = (lags[lag] + 0.5 * lags[lag * 2]) * prior;
        if score > best.1 {
            best = (lag, score);
        }
    }
    let l = best.0;
    // Between frames: the vertex of a parabola through the peak.
    let (y0, y1, y2) = (lags[l - 1], lags[l], lags[l + 1]);
    let d = y0 - 2.0 * y1 + y2;
    let off = if d.abs() > 1e-12 { (0.5 * (y0 - y2) / d).clamp(-0.5, 0.5) } else { 0.0 };
    let bpm = 60.0 * fps / (l as f64 + off);
    let confidence = ((y1 - mean) / (a0 - mean).max(1e-12)).clamp(0.0, 1.0);
    Ok((bpm, confidence))
}

const MAJOR: [f64; 12] = [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88];
const MINOR: [f64; 12] = [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17];
const MAJOR_NAMES: [&str; 12] = ["C", "D♭", "D", "E♭", "E", "F", "F♯", "G", "A♭", "A", "B♭", "B"];
const MINOR_NAMES: [&str; 12] = ["C", "C♯", "D", "E♭", "E", "F", "F♯", "G", "G♯", "A", "B♭", "B"];

fn pearson(a: &[f64; 12], b: &[f64; 12]) -> f64 {
    let ma = a.iter().sum::<f64>() / 12.0;
    let mb = b.iter().sum::<f64>() / 12.0;
    let (mut num, mut da, mut db) = (0.0, 0.0, 0.0);
    for i in 0..12 {
        num += (a[i] - ma) * (b[i] - mb);
        da += (a[i] - ma).powi(2);
        db += (b[i] - mb).powi(2);
    }
    if da <= 1e-12 || db <= 1e-12 { 0.0 } else { num / (da * db).sqrt() }
}

/// The song's chroma: energy per pitch class, C first.
fn chroma(x: &[f32], rate: u32, cancel: &AtomicBool) -> Result<[f64; 12], Error> {
    const N: usize = 8192;
    const HOP: usize = 4096;
    let pcs: Vec<Option<usize>> = (0..=N / 2)
        .map(|k| {
            let f = k as f64 * rate as f64 / N as f64;
            (55.0..=1760.0).contains(&f).then(|| ((12.0 * (f / 440.0).log2() + 69.0).round() as i64).rem_euclid(12) as usize)
        })
        .collect();
    let mut total = [0f64; 12];
    let mut loudest = 0f64;
    let mut frames: Vec<[f64; 12]> = Vec::new();
    spectra(x, N, HOP, cancel, |mag| {
        let mut c = [0f64; 12];
        for (k, m) in mag.iter().enumerate() {
            if let Some(pc) = pcs[k] {
                c[pc] += *m as f64;
            }
        }
        let sum: f64 = c.iter().sum();
        loudest = loudest.max(sum);
        frames.push(c);
    })?;
    // Frames near silence say nothing; the rest count equally.
    for c in frames {
        let sum: f64 = c.iter().sum();
        let peak = c.iter().cloned().fold(0.0, f64::max);
        if sum > loudest * 0.05 && peak > 0.0 {
            for i in 0..12 {
                total[i] += c[i] / peak;
            }
        }
    }
    Ok(total)
}

/// ("A minor", confidence 0..1).
fn key(x: &[f32], rate: u32, cancel: &AtomicBool) -> Result<(String, f64), Error> {
    let c = chroma(x, rate, cancel)?;
    let mut best = (String::new(), f64::MIN);
    for tonic in 0..12 {
        let rot = |p: &[f64; 12]| std::array::from_fn::<f64, 12, _>(|i| p[(i + 12 - tonic) % 12]);
        let maj = pearson(&c, &rot(&MAJOR));
        let min = pearson(&c, &rot(&MINOR));
        if maj > best.1 {
            best = (format!("{} major", MAJOR_NAMES[tonic]), maj);
        }
        if min > best.1 {
            best = (format!("{} minor", MINOR_NAMES[tonic]), min);
        }
    }
    Ok((best.0, best.1.max(0.0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    const RATE: u32 = 11_025;
    fn go() -> AtomicBool {
        AtomicBool::new(false)
    }

    /// A tiny deterministic noise source.
    fn noise(seed: &mut u32) -> f32 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 17;
        *seed ^= *seed << 5;
        (*seed as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// A kick-and-hat pattern at `bpm` over a quiet pad.
    fn beat(bpm: f64, secs: f64) -> Vec<f32> {
        let n = (secs * RATE as f64) as usize;
        let period = 60.0 / bpm * RATE as f64;
        let mut seed = 7;
        (0..n)
            .map(|i| {
                let t = i as f64 % period;
                let decay = (-t / (0.03 * RATE as f64)).exp() as f32;
                let kick = (TAU * 60.0 * t as f32 / RATE as f32).sin() * decay;
                let hat = noise(&mut seed) * decay * 0.4;
                let pad = 0.05 * (TAU * 220.0 * i as f32 / RATE as f32).sin();
                kick + hat + pad
            })
            .collect()
    }

    /// Chords, two seconds each, with a few overtones.
    fn chords(prog: &[&[f32]], secs: f64) -> Vec<f32> {
        let n = (secs * RATE as f64) as usize;
        let per = 2 * RATE as usize;
        (0..n)
            .map(|i| {
                let notes = prog[(i / per) % prog.len()];
                let t = i as f32 / RATE as f32;
                notes.iter().map(|&m| {
                    let f = 440.0 * 2f32.powf((m - 69.0) / 12.0);
                    (1..=3).map(|h| (TAU * f * h as f32 * t).sin() / h as f32).sum::<f32>()
                }).sum::<f32>() * 0.1
            })
            .collect()
    }

    #[test]
    fn finds_the_tempo_of_a_beat() {
        for bpm in [92.0, 120.0, 128.0, 174.0 / 2.0 * 2.0] {
            let e = estimate(&beat(bpm, 40.0), RATE, &go()).unwrap();
            let got = e.bpm.unwrap_or_else(|| panic!("no tempo for {bpm} (confidence {})", e.tempo_confidence));
            assert!((got - bpm).abs() <= 1.0, "{bpm} BPM read as {got}");
        }
    }

    #[test]
    fn finds_major_and_minor_keys() {
        // C major: C–F–G–C. A minor: Am–Dm–E–Am.
        let c = chords(&[&[60.0, 64.0, 67.0], &[65.0, 69.0, 72.0], &[67.0, 71.0, 74.0], &[60.0, 64.0, 67.0]], 32.0);
        assert_eq!(estimate(&c, RATE, &go()).unwrap().key.as_deref(), Some("C major"));
        let a = chords(&[&[57.0, 60.0, 64.0], &[62.0, 65.0, 69.0], &[64.0, 68.0, 71.0], &[57.0, 60.0, 64.0]], 32.0);
        assert_eq!(estimate(&a, RATE, &go()).unwrap().key.as_deref(), Some("A minor"));
        // F♯ minor, the same progression up a minor third.
        let fs = chords(&[&[66.0, 69.0, 73.0], &[71.0, 74.0, 78.0], &[73.0, 77.0, 80.0], &[66.0, 69.0, 73.0]], 32.0);
        assert_eq!(estimate(&fs, RATE, &go()).unwrap().key.as_deref(), Some("F♯ minor"));
    }

    #[test]
    fn noise_gives_no_answer() {
        let mut seed = 99;
        let x: Vec<f32> = (0..RATE as usize * 30).map(|_| noise(&mut seed) * 0.3).collect();
        let e = estimate(&x, RATE, &go()).unwrap();
        assert_eq!(e.bpm, None, "tempo confidence {}", e.tempo_confidence);
        assert_eq!(e.key, None, "key confidence {}", e.key_confidence);
    }

    #[test]
    fn short_and_cancelled() {
        assert_eq!(estimate(&[0.0; 1000], RATE, &go()), Err(Error::TooShort));
        assert_eq!(estimate(&beat(120.0, 30.0), RATE, &AtomicBool::new(true)), Err(Error::Cancelled));
    }

    #[test]
    fn decodes_files_encore_plays() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../encore-audio/fixtures");
        for f in ["tone-aac.m4a", "tone-opus.webm"] {
            let (x, rate) = decode_mono(&dir.join(f), &go()).unwrap();
            assert!((11_000..=12_100).contains(&rate), "{f}: {rate} Hz");
            assert!(x.len() > rate as usize, "{f}: {} samples", x.len());
        }
        assert!(matches!(decode_mono(Path::new("/nonexistent"), &go()), Err(Error::Unreadable(_))));
    }
}
