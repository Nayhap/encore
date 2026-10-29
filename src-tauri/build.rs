fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let mut attrs = tauri_build::Attributes::new();
    if target_os == "windows" && target_env == "msvc" {
        // Embed the Common Controls v6 manifest through the linker so test
        // binaries get it too; without it they fail to start with
        // STATUS_ENTRYPOINT_NOT_FOUND (tauri-build only adds it to the app).
        attrs = attrs.windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
    tauri_build::try_build(attrs).expect("tauri-build failed");
}
