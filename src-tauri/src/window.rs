//! Window chrome off macOS. Windows and Linux use Encore's own title-bar
//! buttons (the window has no native decorations); the UI calls the window
//! API for minimise, maximise and close.

use encore_core::error::Result;

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
