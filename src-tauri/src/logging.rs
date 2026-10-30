//! Logging to `~/Library/Logs/app.encore.desktop/encore.log` through the
//! redacting writer (D9): cookies, tokens, SAPISID hashes and signed URLs are
//! scrubbed before anything reaches the file.

use std::fs::OpenOptions;
use std::path::Path;
use std::sync::Mutex;

use encore_core::redact::RedactingWriter;

struct FileWriter(Mutex<std::fs::File>);

impl std::io::Write for &FileWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).flush()
    }
}

pub fn init(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    // Keep one previous log; the file is appended to forever otherwise.
    let path = dir.join("encore.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
        let _ = std::fs::rename(&path, dir.join("encore.log.1"));
    }
    let Ok(f) = OpenOptions::new().create(true).append(true).open(dir.join("encore.log")) else { return };
    let fw: &'static FileWriter = Box::leak(Box::new(FileWriter(Mutex::new(f))));
    let filter = tracing_subscriber::EnvFilter::try_from_env("ENCORE_LOG").unwrap_or_else(|_| "info".into());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(move || RedactingWriter::new(fw))
        .try_init();
    // Release builds abort on panic, which would leave no trace of why
    // Encore closed. Record the panic (redacted, like everything else) first.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        tracing::error!(thread = %thread, "Encore crashed: {info}");
        default_hook(info);
    }));
    tracing::info!("Encore {} starting on {} {}", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH);
}

const MAX_LOG_BYTES: u64 = 8 * 1024 * 1024;
