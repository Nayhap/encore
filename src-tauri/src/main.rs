// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Before any thread starts or GTK/WebKit loads (D17).
    #[cfg(target_os = "linux")]
    appimage_webkit();
    encore_lib::run()
}

/// The AppImage carries the WebKitGTK of the oldest Ubuntu we support
/// (22.04) and draws with the host's much newer Mesa. On some of those hosts
/// (Fedora 44, Mesa 26) WebKit's DMA-BUF renderer aborts its web process and
/// the window goes blank or closes, so the AppImage turns that renderer off
/// unless the user chose otherwise. A setting someone already exported wins.
#[cfg(target_os = "linux")]
fn appimage_webkit() {
    if std::env::var_os("APPIMAGE").is_none() || std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_some() {
        return;
    }
    // SAFETY: first thing in main, while the process has one thread.
    unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
}
