// The native shell: Tauri commands and events in front of the Encore crates.

mod art;
mod commands;
mod connectivity;
mod core;
mod events;
mod logging;
mod media;
mod selftest;
mod signin;
mod updates;
mod window;

use std::sync::Arc;
use std::time::Duration;

use encore_core::service::PlayFailureKind;
use encore_downloads::DownloadsUpdate;
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
            runtimes_status,
            runtimes_install,
            runtimes_cancel,
            runtimes_remove,
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
            downloads_snapshot,
            download_tracks,
            download_remove,
            download_keep,
            download_unkeep,
            downloads_sync,
            downloads_pause,
            download_pause_collection,
            downloads_retry,
            downloads_remove_all,
            downloads_storage,
            connectivity_check,
            update_status,
            update_check,
            update_download,
            update_install,
            update_skip,
            update_unskip,
            set_auto_update,
            window::snap_layouts,
            window::set_system_titlebar,
            window::get_system_titlebar,
        ])
        .events(collect_events![
            PlayerStateEvent,
            PlayerPositionEvent,
            QueueEvent,
            AuthEvent,
            LyricsEvent,
            ArtHueEvent,
            NoticeEvent,
            SigninEvent,
            PlaylistsEvent,
            HistoryEvent,
            RuntimesEvent,
            SetupNeededEvent,
            DownloadsEvent,
            DownloadItemEvent,
            ConnectivityEvent,
            UpdateEvent
        ])
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
                    // A song that couldn't be reached: maybe the connection is gone.
                    if s.error.as_ref().is_some_and(|e| e.kind == PlayFailureKind::Network) {
                        core.connectivity.transport_failed();
                    }
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
                PlayerEvent::SetupNeeded => {
                    let track_id = core.player.snapshot().current.map(|c| c.track.id.0);
                    let _ = SetupNeededEvent { track_id }.emit(&app);
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

/// Forward downloads to the webview (D23).
fn forward_downloads(app: tauri::AppHandle, core: Arc<Core>) {
    let Some(d) = core.downloads.clone() else { return };
    let mut rx = d.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(DownloadsUpdate::Summary(s)) => {
                    let _ = DownloadsEvent(s).emit(&app);
                }
                Ok(DownloadsUpdate::Item { revision, item }) => {
                    let _ = DownloadItemEvent { revision: revision.to_string(), item }.emit(&app);
                }
                // Missed some: a newer revision makes the UI take a snapshot.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    let _ = DownloadsEvent(d.summary()).emit(&app);
                }
                Err(_) => break,
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
    if selftest::wanted() {
        selftest::run();
    }
    let builder = specta_builder();
    let app = tauri::Builder::default();
    // One Encore at a time: a second launch shows the running one. macOS
    // usually prevents a second copy itself, but not for a copy started from
    // another place (the disk image, Downloads) or with `open -n`; two
    // instances would share one database and play over each other.
    let app = app.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
        }
    }));
    app.plugin(tauri_plugin_opener::init())
        // Updates (D24): used from Rust only; the UI has no updater permission.
        .plugin(tauri_plugin_updater::Builder::new().build())
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
                match r {
                    Ok(Ok(st)) => {
                        if !matches!(st, encore_core::model::AuthState::Anonymous) {
                            c3.player.stop();
                            c3.switch_queue();
                        }
                        let _ = AuthEvent(st).emit(&h3);
                        // The account is known: check its kept collections.
                        if let Some(d) = &c3.downloads {
                            d.account_changed();
                        }
                    }
                    Ok(Err(e)) => {
                        // Access denied at the Keychain prompt, a locked keyring:
                        // say why the account is gone instead of silently
                        // starting signed out. (No Secret Service at all just
                        // means there is nothing saved.)
                        tracing::warn!(error = %e.message, "could not restore the session");
                        if !e.message.starts_with("No Secret Service") {
                            let text = format!("Encore couldn't read your saved sign-in: {} You're signed out for now. Restart Encore and allow access, or sign in again from Settings.", e.message);
                            if let Some(text) = c3.startup_notice(text) {
                                let _ = NoticeEvent { text, level: "error".into(), action: None }.emit(&h3);
                            }
                        }
                    }
                    Err(_) => {}
                }
            });
            forward_player_events(handle.clone(), core.clone());
            forward_downloads(handle.clone(), core.clone());
            // Online or offline (D23): playback, lyrics and downloads follow.
            let (c5, h5) = (core.clone(), handle.clone());
            core.connectivity.on_change(move |on| {
                c5.player.set_online(on);
                c5.lyrics.set_online(on);
                c5.art.set_online(on);
                if let Some(d) = &c5.downloads {
                    d.set_online(on);
                }
                let _ = ConnectivityEvent { online: on }.emit(&h5);
            });
            if let Some(d) = &core.downloads {
                d.set_setup_ready(core.resolver.runtimes().status().usable);
                d.start();
                // The index was lost and rebuilt from the song files (D23).
                let n = d.recovered();
                if n > 0 {
                    let text = format!("Encore's database was damaged and replaced. It found your {n} downloaded songs again; download playlists and albums again to keep them up to date.");
                    if let Some(text) = core.startup_notice(text) {
                        let _ = NoticeEvent { text, level: "warn".into(), action: None }.emit(&handle);
                    }
                }
            }
            // Plays go to YouTube Music history while the setting is on (D18).
            let h4 = handle.clone();
            core.player.report_history(core.ytm.clone(), Arc::new(move |id: &encore_core::model::ItemId| {
                let _ = HistoryEvent { track_id: id.0.clone() }.emit(&h4);
            }));
            media::install(&handle, core.clone());
            keepalive(handle.clone(), core.clone());
            // Updates (D24): checks start 20 s from now.
            let updates = updates::Updates::new(handle.clone(), core.clone());
            app.manage(updates.clone());
            updates.start();
            // Player components (D22): progress goes to the UI; a finished
            // install starts the song that was waiting for it. Components the
            // user already approved are updated (or finished) on launch
            // without asking again; otherwise the UI offers the setup.
            let (c2, h2) = (core.clone(), handle.clone());
            let was_installing = std::sync::atomic::AtomicBool::new(false);
            core.resolver.runtimes().on_change(move |st| {
                let _ = RuntimesEvent(st.clone()).emit(&h2);
                let before = was_installing.swap(st.installing, std::sync::atomic::Ordering::SeqCst);
                if before && !st.installing && st.usable {
                    c2.player.resume_after_setup();
                }
                if let Some(d) = &c2.downloads {
                    d.set_setup_ready(st.usable);
                }
            });
            let rt = core.resolver.runtimes().clone();
            tauri::async_runtime::spawn(async move {
                let st = rt.status();
                tracing::info!(ready = st.ready, usable = st.usable, approved = st.approved, "player components");
                if st.supported && st.approved && !st.ready {
                    rt.start();
                }
            });
            // macOS: closing the window hides it and playback continues; the
            // Dock icon brings it back (D15). Windows and Linux have no such
            // way back without a tray icon, so closing quits there.
            #[cfg(windows)]
            if let Some(w) = app.get_webview_window("main") {
                window::round_corners(&w);
            }

            // Closing with an update ready installs it first (D24): the
            // window stays up for that moment, saying so.
            #[cfg(not(target_os = "macos"))]
            if let Some(w) = app.get_webview_window("main") {
                let h = app.handle().clone();
                let (w2, u2) = (w.clone(), updates.clone());
                w.on_window_event(move |e| match e {
                    WindowEvent::CloseRequested { api, .. } if u2.install_on_close() => {
                        api.prevent_close();
                        let (w3, u3) = (w2.clone(), u2.clone());
                        tauri::async_runtime::spawn(async move {
                            // On Windows a successful install exits here.
                            let _ = u3.install(false).await;
                            // Installed or not, the window was closed: quit.
                            u3.begin_quit();
                            let _ = w3.destroy();
                        });
                    }
                    WindowEvent::Destroyed => {
                        signin::close(&h);
                        h.exit(0);
                    }
                    _ => {}
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
            // Every quit settles playback, history, downloads and the
            // helpers first, and installs an update that is ready (D24).
            // The exit is held for that, then let through.
            RunEvent::ExitRequested { api, code, .. } => {
                let Some(u) = app.try_state::<Arc<updates::Updates>>().map(|u| u.inner().clone()) else {
                    if let Some(core) = app.try_state::<Arc<Core>>() {
                        core.player.pause();
                    }
                    return;
                };
                if !u.begin_quit() {
                    return;
                }
                api.prevent_exit();
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    u.quit().await;
                    if code == Some(tauri::RESTART_EXIT_CODE) {
                        app.restart();
                    }
                    app.exit(code.unwrap_or(0));
                });
            }
            // macOS Quit (the menu, Cmd+Q, the Dock, logout) skips
            // ExitRequested: AppKit is already terminating and the process
            // ends when this returns, so the same work runs here, blocking.
            // Every step has its bound.
            RunEvent::Exit => {
                if let Some(u) = app.try_state::<Arc<updates::Updates>>().map(|u| u.inner().clone()) {
                    if u.begin_quit() {
                        tauri::async_runtime::block_on(u.quit());
                    }
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
