// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Before any thread starts or GTK/WebKit loads (D17).
    #[cfg(target_os = "linux")]
    encore_lib::webkit::choose();
    encore_lib::run()
}
