// The native shell: Tauri commands and events in front of the Encore crates.

mod art;
mod commands;
mod connectivity;
mod core;
mod events;
mod features;
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
            player_engine,
            player_engine_reset,
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
            // One block per area, each appended to only by its lane
            // (foundation.md): `features::<area>::<command>,` lines.
            // features: playback (lane A)
            features::playback::loop_set,
            features::playback::loop_clear,
            features::playback::sleep_set,
            features::playback::sleep_cancel,

            // features: library tools (lane B)
            features::library_tools::library_sort_facts,
            features::library_tools::library_years,
            features::library_tools::library_years_stop,
            features::library_tools::library_like,
            features::library_tools::library_like_stop,
            features::library_tools::library_discography,

            // features: saved (lane C)
            features::saved::bookmarks_list,
            features::saved::bookmarks_add,
            features::saved::bookmarks_remove,
            features::saved::saved_queues_list,
            features::saved::saved_queue_save,
            features::saved::saved_queue_restore,
            features::saved::saved_queue_remove,
            features::saved::folders_list,
            features::saved::folders_create,
            features::saved::folders_rename,
            features::saved::folders_remove,
            features::saved::folders_set_collapsed,
            features::saved::folders_add,
            features::saved::folders_remove_item,
            features::saved::folders_move_item,
            features::saved::folders_choose_cover,
            features::saved::folders_clear_cover,
            features::saved::folders_export,
            features::saved::folders_import,

            // features: taste and stats (lane D)
            features::taste::taste_state,
            features::taste::taste_block,
            features::taste::taste_unblock,
            features::taste::taste_set_hide_podcasts,
            features::taste::taste_private,
            features::stats::stats_overview,
            features::stats::stats_period,
            features::stats::stats_play_counts,
            features::stats::stats_clear,

            // features: integrations (lane E)
            features::lastfm::lastfm_status,
            features::lastfm::lastfm_begin,
            features::lastfm::lastfm_finish,
            features::lastfm::lastfm_cancel,
            features::lastfm::lastfm_disconnect,
            features::lastfm::lastfm_plays,
            features::presence::presence_status,
            features::presence::presence_set,
            features::wiki::wiki_artist,
            features::details::details_track,
            features::details::details_analyse,
            features::details::details_cancel,

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
            DownloadsEvent,
            DownloadItemEvent,
            ConnectivityEvent,
            UpdateEvent,
            // features: playback (lane A)

            // features: library tools (lane B)
            features::library_tools::LibraryYearEvent,
            features::library_tools::LibraryLikeEvent,

            // features: saved (lane C)

            // features: taste and stats (lane D)
            features::taste::TasteEvent,
            features::stats::StatsEvent,

            // features: integrations (lane E)
            features::lastfm::LastfmEvent,
            features::presence::PresenceEvent,

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
                PlayerEvent::SessionExpired => {
                    // Refresh the session and check it before saying it
                    // expired; when it works again, play the song again (D38).
                    let (core, app) = (core.clone(), app.clone());
                    tauri::async_runtime::spawn(async move {
                        match core.accounts.recover("YouTube refused the signed-in stream.").await {
                            encore_ytm::account::Recovery::Working => {
                                let _ = AuthEvent(core.accounts.state()).emit(&app);
                                core.player.retry_failed();
                            }
                            encore_ytm::account::Recovery::Expired => {
                                let _ = AuthEvent(core.accounts.state()).emit(&app);
                                let _ = NoticeEvent { text: "Your YouTube Music session expired. Import it again from Settings.".into(), level: "error".into(), action: Some("reimport".into()) }.emit(&app);
                            }
                            encore_ytm::account::Recovery::Unknown => {}
                        }
                    });
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
            let _ = core.accounts.keepalive().await;
            // Expired, or working again (D38).
            if core.accounts.state() != before {
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
    let app = app.plugin(tauri_plugin_opener::init());
    // Updates (D24): used from Rust only; the UI has no updater permission.
    let app = app.plugin(tauri_plugin_updater::Builder::new().build());
    // features: plugins, one `let app = app.plugin(...);` line per lane.
    let app = app.plugin(tauri_plugin_dialog::init()); // lane C: folder covers and files

    app.invoke_handler(builder.invoke_handler())
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
                d.start();
            }
            // A damaged database was replaced (D11), and downloaded songs
            // the index no longer knew were found from their files (D23).
            let found = core.downloads.as_ref().map(|d| d.recovered()).unwrap_or(0);
            if let Some(text) = core.recovery_notice(found).and_then(|t| core.startup_notice(t)) {
                let _ = NoticeEvent { text, level: "warn".into(), action: None }.emit(&handle);
            }
            // Plays go to YouTube Music history while the setting is on (D18).
            let h4 = handle.clone();
            core.player.report_history(core.ytm.clone(), Arc::new(move |id: &encore_core::model::ItemId| {
                let _ = HistoryEvent { track_id: id.0.clone() }.emit(&h4);
            }));
            media::install(&handle, core.clone());
            features::install(&handle, &core);
            keepalive(handle.clone(), core.clone());
            // Updates (D24): checks start 20 s from now.
            let updates = updates::Updates::new(handle.clone(), core.clone());
            app.manage(updates.clone());
            updates.start();
            // D22's player components are no longer used (D37): remove them
            // once, in the background, without touching anything else.
            let paths = (core.paths.data.clone(), core.paths.cache.clone());
            tauri::async_runtime::spawn_blocking(move || encore_resolver::cleanup::run(&paths.0, &paths.1));
            // macOS: closing the window hides it and playback continues; the
            // Dock icon brings it back (D15). Windows and Linux have no such
            // way back without a tray icon, so closing quits there.
            #[cfg(windows)]
            if let Some(w) = app.get_webview_window("main") {
                window::round_corners(&w);
            }

            // Closing with an update ready installs it first (D24): the
            // window stays up for that moment, saying so. A second close
            // while that runs waits for it.
            #[cfg(not(target_os = "macos"))]
            if let Some(w) = app.get_webview_window("main") {
                let h = app.handle().clone();
                let (h2, u2) = (app.handle().clone(), updates.clone());
                w.on_window_event(move |e| match e {
                    WindowEvent::CloseRequested { api, .. } if u2.exit_state() == updates::Exit::Exiting => api.prevent_close(),
                    WindowEvent::CloseRequested { api, .. } if u2.exit_state() == updates::Exit::Running && u2.install_on_close() && u2.begin_exit() => {
                        api.prevent_close();
                        let (h3, u3) = (h2.clone(), u2.clone());
                        tauri::async_runtime::spawn(async move {
                            // On Windows a successful install exits in here.
                            u3.finish_exit().await;
                            h3.exit(0);
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
            // One way out (D24): the first exit request settles playback,
            // history, downloads and the helpers and installs a ready
            // update; every request until then is held (a second close, the
            // runtime's own request when the last window goes); after it,
            // they go through.
            RunEvent::ExitRequested { api, code, .. } => {
                let Some(u) = app.try_state::<Arc<updates::Updates>>().map(|u| u.inner().clone()) else {
                    if let Some(core) = app.try_state::<Arc<Core>>() {
                        core.player.pause();
                    }
                    return;
                };
                if u.exit_state() == updates::Exit::Released {
                    return;
                }
                api.prevent_exit();
                if !u.begin_exit() {
                    return;
                }
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    u.finish_exit().await;
                    if code == Some(tauri::RESTART_EXIT_CODE) {
                        app.restart();
                    }
                    app.exit(code.unwrap_or(0));
                });
            }
            // macOS Quit (the menu, Cmd+Q, the Dock, logout) skips
            // ExitRequested: AppKit is already terminating and the process
            // ends when this returns, so the same work runs here, blocking
            // and bounded. A quit while Restart to update installs waits for
            // that install instead of cutting it off.
            RunEvent::Exit => {
                let Some(u) = app.try_state::<Arc<updates::Updates>>().map(|u| u.inner().clone()) else { return };
                tauri::async_runtime::block_on(async {
                    u.exit_settled(Duration::from_secs(30)).await;
                    if u.begin_exit() {
                        let _ = tokio::time::timeout(Duration::from_secs(30), u.finish_exit()).await;
                    }
                });
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
