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
        self.0.lock().unwrap().write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.lock().unwrap().flush()
    }
}

pub fn init(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    let Ok(f) = OpenOptions::new().create(true).append(true).open(dir.join("encore.log")) else { return };
    let fw: &'static FileWriter = Box::leak(Box::new(FileWriter(Mutex::new(f))));
    let filter = tracing_subscriber::EnvFilter::try_from_env("ENCORE_LOG").unwrap_or_else(|_| "info".into());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(move || RedactingWriter::new(fw))
        .try_init();
    tracing::info!("Encore {} starting", env!("CARGO_PKG_VERSION"));
}
