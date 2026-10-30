//! `encore --self-test [VIDEO_ID] [--out FILE] [--strict]`: checks, without a
//! window, the parts of Encore that depend on the machine it runs on: the
//! data folder and database, the audio output, the yt-dlp setup (Python, the
//! venv, a JS runtime) and, given a video id, resolving and decoding a stream.
//!
//! It runs the installed artifact the way users get it (inside the AppImage,
//! from the .app, from the installed .exe), which is where the AppImage
//! playback bug and the no-audio-device crash hid. CI runs it on every build.
//!
//! Prints one JSON object (and writes it to `--out` if given, since Windows
//! release builds have no console). Exit code 0 when everything needed for
//! playback works. YouTube's bot checks and network failures while resolving
//! are reported but don't fail the test unless `--strict` is given: CI
//! runners are often refused by YouTube.

use std::sync::Arc;
use std::time::{Duration, Instant};

use encore_audio::{Engine, EngineEvent, HttpSpec, SourceSpec};
use encore_core::service::{PlayFailureKind, StreamResolver};
use encore_core::store::Store;
use encore_resolver::{ResolverConfig, YtDlpResolver};
use serde_json::{Value, json};

use crate::core::Paths;

pub fn wanted() -> bool {
    std::env::args().skip(1).any(|a| a == "--self-test")
}

pub fn run() -> ! {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = args.iter().position(|a| a == "--out").and_then(|i| args.get(i + 1)).cloned();
    let strict = args.iter().any(|a| a == "--strict");
    let video = args.iter().filter(|a| !a.starts_with("--")).find(|a| Some(*a) != out.as_ref()).cloned();
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => finish(json!({ "ok": false, "error": format!("no async runtime: {e}") }), out.as_deref()),
    };
    let report = rt.block_on(check(video, strict));
    finish(report, out.as_deref())
}

fn finish(report: Value, out: Option<&str>) -> ! {
    let text = serde_json::to_string_pretty(&report).unwrap_or_default();
    println!("{text}");
    if let Some(p) = out {
        let _ = std::fs::write(p, &text);
    }
    tracing::info!(ok = report["ok"].as_bool().unwrap_or(false), "self-test finished");
    std::process::exit(if report["ok"].as_bool() == Some(true) { 0 } else { 1 })
}

async fn check(video: Option<String>, strict: bool) -> Value {
    let mut r = json!({ "version": env!("CARGO_PKG_VERSION"), "os": std::env::consts::OS, "arch": std::env::consts::ARCH, "appimage": std::env::var_os("APPIMAGE").is_some() });
    let mut ok = true;
    let paths = Paths::new();

    // Data folder and database.
    match Store::open(&paths.data.join("encore.sqlite")) {
        Ok(_) => r["store"] = json!({ "ok": true }),
        Err(e) => {
            ok = false;
            r["store"] = json!({ "ok": false, "error": e.message });
        }
    }

    // Audio output: must never stop Encore from starting.
    let t0 = Instant::now();
    let engine = match Engine::new(None) {
        Ok(e) => {
            let st = e.status();
            r["audio"] = json!({ "ok": true, "device": st.device, "rate": st.sample_rate, "channels": st.channels, "ms": t0.elapsed().as_millis() as u64 });
            Some(e)
        }
        Err(e) => {
            ok = false;
            r["audio"] = json!({ "ok": false, "error": e });
            None
        }
    };

    // yt-dlp: Python, the venv, the pinned packages, a JS runtime.
    let resolver = YtDlpResolver::new(ResolverConfig { root: paths.data.join("resolver"), cache_dir: paths.cache.join("yt-dlp") });
    let t0 = Instant::now();
    let setup = resolver.ensure_ready().await;
    let st = resolver.status();
    r["resolver"] = json!({ "ok": setup.is_ok(), "python": st.python, "jsRuntime": st.js_runtime, "ytDlp": st.yt_dlp, "error": setup.as_ref().err(), "ms": t0.elapsed().as_millis() as u64 });
    if setup.is_err() {
        ok = false;
    } else {
        match resolver.health().await {
            Ok(v) => r["resolver"]["helper"] = json!(v),
            Err(e) => {
                ok = false;
                r["resolver"]["ok"] = json!(false);
                r["resolver"]["error"] = json!(e.message);
            }
        }
    }

    // A real stream: resolve, probe, decode.
    if let (Some(id), true) = (video, ok) {
        r["stream"] = stream(&resolver, engine.as_ref(), &id).await;
        let s = &r["stream"];
        let soft = !strict && matches!(s["kind"].as_str(), Some("challenged") | Some("network"));
        if s["ok"].as_bool() != Some(true) && !soft {
            ok = false;
        }
    }
    r["ok"] = json!(ok);
    r
}

async fn stream(resolver: &Arc<YtDlpResolver>, engine: Option<&Engine>, id: &str) -> Value {
    let t0 = Instant::now();
    let s = match resolver.resolve(id, None).await {
        Ok(s) => s,
        Err(f) => {
            let kind = match f.kind {
                PlayFailureKind::Challenged => "challenged",
                PlayFailureKind::Network => "network",
                PlayFailureKind::Unavailable => "unavailable",
                PlayFailureKind::SessionExpired => "sessionExpired",
                PlayFailureKind::Setup => "setup",
                PlayFailureKind::Unknown => "unknown",
            };
            return json!({ "ok": false, "kind": kind, "error": f.message });
        }
    };
    // Never the URL: it's signed.
    let mut v = json!({ "ok": true, "itag": s.itag, "codec": s.codec, "resolveMs": t0.elapsed().as_millis() as u64 });
    let Some(engine) = engine else { return v };
    let Some(events) = engine.take_events() else { return v };
    engine.set_volume(0.0);
    let spec = SourceSpec::Http(HttpSpec { url: s.url.expose().clone(), headers: s.headers.clone(), content_length: s.content_length, cache_key: format!("selftest-{id}"), range_safe: s.range_safe, refresher: None });
    engine.load(1, spec, 0.0, 0.0, true);
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut decoded = false;
    let mut error = None;
    while Instant::now() < deadline {
        match events.recv_timeout(Duration::from_millis(200)) {
            Ok(EngineEvent::Ready { .. }) => decoded = true,
            Ok(EngineEvent::Failed { error: e, .. }) => {
                error = Some(e);
                break;
            }
            _ => {}
        }
        // Decoded, and either audio is reaching a device or there is none.
        let st = engine.status();
        if decoded && (st.frames_out > st.sample_rate as u64 || st.device == encore_audio::engine::NO_OUTPUT) {
            break;
        }
    }
    let st = engine.status();
    engine.stop();
    v["decoded"] = json!(decoded);
    v["framesOut"] = json!(st.frames_out);
    v["device"] = json!(st.device);
    if let Some(e) = error {
        v["error"] = json!(e);
    }
    v["ok"] = json!(decoded);
    v
}
