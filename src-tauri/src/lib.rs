// The native shell: Tauri commands and events in front of the Encore crates.

mod art;
mod commands;
mod core;
mod events;
mod logging;
mod media;
mod signin;
mod window;

use std::sync::Arc;
use std::time::Duration;

use encore_player::PlayerEvent;
use tauri::{Manager, RunEvent, WindowEvent};
use tauri_specta::{Builder, Event, collect_commands, collect_events};

use crate::core::Core;
use crate::events::*;

pub fn specta_builder() -> Builder<tauri::Wry> {
    use commands::*;
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            bootstrap,
            resolver_setup,
            cache_info,
            cache_clear,
            home,
            explore,
            browse_page,
            more_shelves,
            search,
            search_more,
            suggestions,
            collection,
            collection_more,
            artist,
            library,
            library_more,
            lyrics,
            player_snapshot,
            queue_snapshot,
            play_tracks,
            play_collection,
            play_track,
            start_radio,
            toggle,
            next,
            prev,
            seek,
            jump_to,
            play_next,
            add_to_queue,
            queue_remove,
            queue_move,
            clear_up_next,
            set_shuffle,
            set_repeat,
            set_autoplay,
            set_volume,
            set_muted,
            set_like,
            set_saved,
            set_subscribed,
            playlist_add,
            playlist_remove,
            playlist_create,
            playlist_edit,
            playlist_delete,
            playlist_move,
            history,
            history_more,
            history_remove,
            counterpart,
            switch_version,
            play_version,
            set_report_history,
            auth_state,
            browser_profiles,
            import_browser,
            select_channel,
            reload_channels,
            sign_out,
            begin_signin,
            cancel_signin,
            window::snap_layouts,
            window::set_system_titlebar,
            window::get_system_titlebar,
        ])
        .events(collect_events![PlayerStateEvent, PlayerPositionEvent, QueueEvent, AuthEvent, LyricsEvent, ArtHueEvent, NoticeEvent, SigninEvent, PlaylistsEvent, HistoryEvent])
}

/// Forward player events to the webview and drive lyrics.
fn forward_player_events(app: tauri::AppHandle, core: Arc<Core>) {
    let mut rx = core.player.subscribe();
    tauri::async_runtime::spawn(async move {
        let mut last_seek = String::new();
        let mut lyrics_gen = 0u64;
        loop {
            let ev = match rx.recv().await {
                Ok(e) => e,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // Missed events: the UI resyncs from a snapshot.
                    let _ = PlayerStateEvent(core.player.snapshot()).emit(&app);
                    continue;
                }
                Err(_) => break,
            };
            match ev {
                PlayerEvent::State(s) => {
                    if s.seek_gen != last_seek {
                        last_seek = s.seek_gen.clone();
                        media::position_changed(&app, s.clone());
                    }
                    let _ = PlayerStateEvent(s).emit(&app);
                }
                PlayerEvent::Queue(q) => {
                    let _ = QueueEvent(q).emit(&app);
                }
                PlayerEvent::Position(p) => {
                    let _ = PlayerPositionEvent(p).emit(&app);
                }
                PlayerEvent::TrackChanged(entry) => {
                    lyrics_gen += 1;
                    let my = lyrics_gen;
                    let Some(e) = entry else { continue };
                    let core2 = core.clone();
                    let app2 = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let l = core2.lyrics.get(&e.track).await;
                        let _ = my;
                        let _ = LyricsEvent { track_id: e.track.id.0.clone(), lyrics: l }.emit(&app2);
                    });
                }
                PlayerEvent::Notice(text) => {
                    let _ = NoticeEvent { text, level: "warn".into(), action: None }.emit(&app);
                }
                PlayerEvent::SessionExpired => {
                    core.accounts.mark_expired("YouTube refused the signed-in stream.");
                    let _ = AuthEvent(core.accounts.state()).emit(&app);
                    let _ = NoticeEvent { text: "Your YouTube Music session expired. Import it again from Settings.".into(), level: "error".into(), action: Some("reimport".into()) }.emit(&app);
                }
            }
        }
    });
}

fn keepalive(app: tauri::AppHandle, core: Arc<Core>) {
    tauri::async_runtime::spawn(async move {
        let mut first = true;
        loop {
            if !first {
                // 5 minutes ± 30 s
                let jitter = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().subsec_millis() as u64 % 60_000) as i64 - 30_000;
                tokio::time::sleep(Duration::from_millis((300_000i64 + jitter) as u64)).await;
            }
            first = false;
            let before = core.accounts.state();
            let r = core.accounts.keepalive().await;
            if r.is_err() && core.accounts.state() != before {
                let _ = AuthEvent(core.accounts.state()).emit(&app);
            }
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let paths = core::Paths::new();
    logging::init(&paths.logs);
    let builder = specta_builder();
    let app = tauri::Builder::default();
    // One Encore at a time: a second launch shows the running one.
    #[cfg(not(target_os = "macos"))]
    let app = app.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
        }
    }));
    app.plugin(tauri_plugin_opener::init())
        .invoke_handler(builder.invoke_handler())
        .register_asynchronous_uri_scheme_protocol("art", |ctx, req, responder| {
            let app = ctx.app_handle().clone();
            let uri = req.uri().to_string();
            tauri::async_runtime::spawn(async move {
                let resp = match art::parse_request(&uri) {
                    Some((key, size)) => {
                        let Some(core) = app.try_state::<Arc<Core>>().map(|c| c.inner().clone()) else {
                            responder.respond(tauri::http::Response::new(Vec::new()));
                            return;
                        };
                        match core.art.get(&key, size).await {
                            Ok((bytes, mime)) => tauri::http::Response::builder().status(200).header("Content-Type", mime).header("Cache-Control", "max-age=604800").body(bytes),
                            Err(e) => {
                                tracing::warn!(key = %key, size, error = %e, "artwork failed");
                                tauri::http::Response::builder().status(404).body(Vec::new())
                            }
                        }
                    }
                    None => tauri::http::Response::builder().status(400).body(Vec::new()),
                };
                responder.respond(resp.unwrap_or_else(|_| tauri::http::Response::new(Vec::new())));
            });
        })
        .setup(move |app| {
            // Off macOS the window is built here, with the chosen title bar.
            #[cfg(not(target_os = "macos"))]
            window::create_main(app.handle())?;
            builder.mount_events(app);
            let t0 = std::time::Instant::now();
            let core = match tauri::async_runtime::block_on(async { Core::new() }) {
                Ok(c) => c,
                Err(e) => {
                    // Failing here used to close Encore without a word. Keep
                    // the window up and let the UI say what went wrong.
                    tracing::error!(error = %e.message, "Encore could not start");
                    app.manage(core::StartupFailure(core::startup_failure_message(&e.message)));
                    return Ok(());
                }
            };
            tracing::info!(ms = t0.elapsed().as_millis() as u64, "core ready");
            app.manage(core.clone());
            let handle = app.handle().clone();
            // Artwork hues → UI.
            let h2 = handle.clone();
            core.art.on_hue(move |key, h1, h2v| {
                let _ = ArtHueEvent { key: key.to_string(), h1, h2: h2v }.emit(&h2);
            });
            // Session (one read of the OS credential store, which may show a
            // macOS Keychain prompt), then the queue for that account. Off the
            // main thread so the window never waits on it.
            core.restore_queue();
            core.install_persistence();
            let (c3, h3) = (core.clone(), handle.clone());
            tauri::async_runtime::spawn(async move {
                let acc = c3.accounts.clone();
                let r = tokio::task::spawn_blocking(move || acc.restore()).await;
                if let Ok(Ok(st)) = r {
                    if !matches!(st, encore_core::model::AuthState::Anonymous) {
                        c3.player.stop();
                        c3.switch_queue();
                    }
                    let _ = AuthEvent(st).emit(&h3);
                }
            });
            forward_player_events(handle.clone(), core.clone());
            // Plays go to YouTube Music history while the setting is on (D18).
            let h4 = handle.clone();
            core.player.report_history(core.ytm.clone(), Arc::new(move |id: &encore_core::model::ItemId| {
                let _ = HistoryEvent { track_id: id.0.clone() }.emit(&h4);
            }));
            media::install(&handle, core.clone());
            keepalive(handle.clone(), core.clone());
            // Bootstrap the stream resolver in the background (first run creates the venv).
            let c2 = core.clone();
            tauri::async_runtime::spawn(async move {
                let _ = c2.resolver.ensure_ready().await;
            });
            // macOS: closing the window hides it and playback continues; the
            // Dock icon brings it back (D15). Windows and Linux have no such
            // way back without a tray icon, so closing quits there.
            #[cfg(windows)]
            if let Some(w) = app.get_webview_window("main") {
                window::round_corners(&w);
            }

            #[cfg(not(target_os = "macos"))]
            if let Some(w) = app.get_webview_window("main") {
                let h = app.handle().clone();
                w.on_window_event(move |e| {
                    if let WindowEvent::Destroyed = e {
                        signin::close(&h);
                        h.exit(0);
                    }
                });
            }
            #[cfg(target_os = "macos")]
            if let Some(w) = app.get_webview_window("main") {
                let w2 = w.clone();
                w.on_window_event(move |e| {
                    if let WindowEvent::CloseRequested { api, .. } = e {
                        api.prevent_close();
                        let _ = w2.hide();
                    }
                });
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, ev| match ev {
            #[cfg(target_os = "macos")]
            RunEvent::Reopen { .. } => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            RunEvent::ExitRequested { .. } => {
                if let Some(core) = app.try_state::<Arc<Core>>() {
                    core.player.pause();
                }
            }
            _ => {}
        });
}

#[cfg(test)]
mod tests {
    /// Regenerates `src/lib/backend/bindings.ts` (run as part of `cargo test`).
    #[test]
    fn export_bindings() {
        super::specta_builder()
            .export(
                specta_typescript::Typescript::default().header("// @ts-nocheck\n/* eslint-disable */"),
                "../src/lib/backend/bindings.ts",
            )
            .expect("export bindings");
    }
}
