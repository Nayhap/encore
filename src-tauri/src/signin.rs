//! In-app sign-in (D7): a `WebviewWindow` with a non-persistent data store
//! (`incognito`: WebKit's ephemeral store, WebView2's InPrivate). WebKit
//! engines (macOS, WebKitGTK) present a Safari UA; WebView2 keeps its own,
//! which is current Edge's. Cookies are read with
//! `cookies_for_url`; the window closes only after the session validates
//! (the account list loads). Channel selection happens in Encore afterwards.

use std::sync::Arc;
use std::time::{Duration, Instant};

use encore_auth::{CookieJar, Session};
use encore_core::error::{AppError, Result};
use encore_core::service::CookieRecord;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_specta::Event;

use crate::core::Core;
use crate::events::{AuthEvent, SigninEvent};

const LABEL: &str = "signin";
const SIGNIN_URL: &str = "https://accounts.google.com/ServiceLogin?service=youtube&passive=true&continue=https%3A%2F%2Fmusic.youtube.com%2F";
const SAFARI_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";

fn stage(app: &AppHandle, s: &str, m: Option<String>) {
    let _ = SigninEvent { stage: s.into(), message: m }.emit(app);
}

pub fn close(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.close();
    }
}

pub fn open(app: &AppHandle) -> Result<()> {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.set_focus();
        return Ok(());
    }
    let url = SIGNIN_URL.parse().map_err(|_| AppError::internal("bad sign-in URL"))?;
    let app2 = app.clone();
    let b = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::External(url)).title("Sign in to YouTube Music").inner_size(480.0, 720.0).incognito(true);
    let b = if cfg!(windows) { b } else { b.user_agent(SAFARI_UA) };
    let w = b
        .on_navigation(move |u| {
            if u.host_str() == Some("accounts.google.com") && u.path().contains("/signin/rejected") {
                stage(&app2, "rejected", Some("Google refused to sign in inside Encore. Use “Import from browser” instead.".into()));
            }
            // Only Google and YouTube pages may load here.
            let h = u.host_str().unwrap_or("");
            u.scheme() == "https" && (h.ends_with("google.com") || h.ends_with("youtube.com") || h.ends_with("gstatic.com") || h.ends_with("googleusercontent.com") || h.starts_with("accounts.google."))
        })
        .build()
        .map_err(|e| AppError::internal(format!("could not open the sign-in window: {e}")))?;
    stage(app, "opened", None);
    let app = app.clone();
    std::thread::Builder::new()
        .name("encore-signin".into())
        .spawn(move || watch(app, w))
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(())
}

fn to_records(cs: Vec<tauri::webview::Cookie<'static>>) -> Vec<CookieRecord> {
    cs.into_iter()
        .map(|c| CookieRecord {
            name: c.name().to_string(),
            value: c.value().to_string(),
            domain: c.domain().map(|d| d.to_string()).unwrap_or_else(|| ".youtube.com".into()),
            path: c.path().unwrap_or("/").to_string(),
            secure: c.secure().unwrap_or(true),
            http_only: c.http_only().unwrap_or(false),
            expires: c.expires_datetime().map(|t| t.unix_timestamp()),
        })
        .collect()
}

fn watch(app: AppHandle, w: tauri::WebviewWindow) {
    let t0 = Instant::now();
    let music: url::Url = "https://music.youtube.com/".parse().unwrap();
    let google: url::Url = "https://accounts.google.com/".parse().unwrap();
    loop {
        std::thread::sleep(Duration::from_millis(1000));
        if app.get_webview_window(LABEL).is_none() {
            stage(&app, "cancelled", None);
            return;
        }
        if t0.elapsed() > Duration::from_secs(15 * 60) {
            stage(&app, "failed", Some("Sign-in timed out.".into()));
            let _ = w.close();
            return;
        }
        // Wait until the page is back on music.youtube.com.
        let on_music = w.url().ok().is_some_and(|u| u.host_str() == Some("music.youtube.com"));
        if !on_music {
            continue;
        }
        let Ok(mut cs) = w.cookies_for_url(music.clone()) else { continue };
        if let Ok(g) = w.cookies_for_url(google.clone()) {
            cs.extend(g);
        }
        let mut jar = CookieJar::new(to_records(cs));
        jar.retain_needed();
        if !jar.has_login() {
            continue;
        }
        stage(&app, "validating", None);
        let core = app.state::<Arc<Core>>().inner().clone();
        let res = tauri::async_runtime::block_on(async { core.accounts.adopt(Session::new(jar, "webview")).await });
        match res {
            Ok(st) => {
                let _ = AuthEvent(st).emit(&app);
                stage(&app, "chooseChannel", None);
                let _ = w.close();
                return;
            }
            Err(e) => {
                // Keep the window open so the user can finish or retry.
                stage(&app, "error", Some(e.message));
                std::thread::sleep(Duration::from_secs(3));
            }
        }
    }
}
