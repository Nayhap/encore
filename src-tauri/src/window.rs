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
        if window.is_focused().unwrap_or(false) && !window.is_maximized().unwrap_or(false) {
            send_win_z();
        }
    }
    #[cfg(not(windows))]
    let _ = window;
    Ok(())
}

#[cfg(windows)]
fn send_win_z() {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_LWIN};
    let key = |vk: u16, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { 0 }, time: 0, dwExtraInfo: 0 } },
    };
    let z = b'Z' as u16;
    let inputs = [key(VK_LWIN, false), key(z, false), key(z, true), key(VK_LWIN, true)];
    unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
}
