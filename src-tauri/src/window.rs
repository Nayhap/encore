//! Window chrome off macOS. Windows and Linux use Encore's own title-bar
//! buttons (the window has no native decorations); the UI calls the window
//! API for minimise, maximise and close.

use encore_core::error::Result;

/// Marker file for "Use the system title bar". Read before the window is
/// first shown: on Linux, GTK under Wayland ignores decorations switched on
/// after the window is mapped, so the choice has to be made up front.
fn titlebar_marker() -> std::path::PathBuf {
    crate::core::Paths::new().data.join("system-titlebar")
}

pub fn system_titlebar() -> bool {
    titlebar_marker().exists()
}

/// Off macOS the main window isn't created from the config (`create: false`
/// in tauri.*.conf.json) so it can be built with the chosen chrome. GTK
/// can't add a title bar to a window that already exists.
#[cfg(not(target_os = "macos"))]
pub fn create_main(app: &tauri::AppHandle) -> tauri::Result<tauri::WebviewWindow> {
    let cfg = app.config().app.windows.first().cloned().unwrap_or_default();
    let w = tauri::WebviewWindowBuilder::from_config(app, &cfg)?.decorations(system_titlebar()).build()?;
    fit_to_screen(&w);
    Ok(w)
}

/// The default 1360×860 window is taller than a 1366×768 laptop screen and,
/// on a 1080p screen, its player bar ended up behind the taskbar. Shrink it
/// to the screen's work area (not below the minimum size) and center it.
#[cfg(not(target_os = "macos"))]
fn fit_to_screen(w: &tauri::WebviewWindow) {
    let Ok(Some(m)) = w.current_monitor() else { return };
    let area = m.work_area();
    let Ok(size) = w.outer_size() else { return };
    let margin = (24.0 * m.scale_factor()) as u32;
    let (aw, ah) = (area.size.width.saturating_sub(margin), area.size.height.saturating_sub(margin));
    if size.width <= aw && size.height <= ah {
        let _ = w.center();
        return;
    }
    let (min_w, min_h) = ((880.0 * m.scale_factor()) as u32, (600.0 * m.scale_factor()) as u32);
    let inner = w.inner_size().unwrap_or(size);
    let (dw, dh) = (size.width.saturating_sub(inner.width), size.height.saturating_sub(inner.height));
    let width = size.width.min(aw).max(min_w).saturating_sub(dw);
    let height = size.height.min(ah).max(min_h).saturating_sub(dh);
    let _ = w.set_size(tauri::PhysicalSize::new(width, height));
    let _ = w.center();
}

/// Whether the system title bar is on. The window's chrome follows this, so
/// the UI reads it here rather than keeping its own copy.
#[tauri::command]
#[specta::specta]
pub async fn get_system_titlebar() -> Result<bool> {
    Ok(system_titlebar())
}

/// Saves the title bar choice. Windows applies it at once; Linux restarts
/// Encore so the window is created with it.
#[tauri::command]
#[specta::specta]
pub async fn set_system_titlebar(app: tauri::AppHandle, window: tauri::WebviewWindow, on: bool, apply: bool) -> Result<()> {
    let m = titlebar_marker();
    if on {
        if let Some(d) = m.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        std::fs::write(&m, b"1").map_err(|e| encore_core::error::AppError::new(encore_core::ErrorCode::Unsupported, format!("could not save the setting: {}", e.kind())))?;
    } else if m.exists() {
        std::fs::remove_file(&m).map_err(|e| encore_core::error::AppError::new(encore_core::ErrorCode::Unsupported, format!("could not save the setting: {}", e.kind())))?;
    }
    if !apply {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        let _ = window;
        app.restart();
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = app;
        let _ = window.set_decorations(on);
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Windows 11 snap layouts: the flyout normally appears when the pointer
/// rests on the native maximise button, which Encore doesn't have. Pressing
/// Win+Z opens the same flyout for the active window. The UI calls this after
/// the pointer has rested on its maximise button while the window is focused.
#[tauri::command]
#[specta::specta]
pub async fn snap_layouts(window: tauri::WebviewWindow) -> Result<()> {
    #[cfg(windows)]
    {
        let focused = window.is_focused().unwrap_or(false);
        let maximized = window.is_maximized().unwrap_or(false);
        let sent = if focused && !maximized { send_win_z() } else { 0 };
        tracing::debug!(focused, maximized, sent, "snap layouts");
    }
    #[cfg(not(windows))]
    let _ = window;
    Ok(())
}

/// Windows 11 rounds undecorated windows only when asked.
#[cfg(windows)]
pub fn round_corners(window: &tauri::WebviewWindow) {
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute};
    if let Ok(h) = window.hwnd() {
        let pref = DWMWCP_ROUND;
        // Fails harmlessly on Windows 10, which has no rounded corners.
        unsafe { DwmSetWindowAttribute(h.0 as _, DWMWA_WINDOW_CORNER_PREFERENCE as u32, &pref as *const _ as *const _, std::mem::size_of_val(&pref) as u32) };
    }
}

#[cfg(windows)]
fn send_win_z() -> u32 {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_LWIN};
    let key = |vk: u16, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { 0 }, time: 0, dwExtraInfo: 0 } },
    };
    let z = b'Z' as u16;
    let inputs = [key(VK_LWIN, false), key(z, false), key(z, true), key(VK_LWIN, true)];
    unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) }
}
