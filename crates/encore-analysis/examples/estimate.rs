//! `cargo run --release -p encore-analysis --example estimate -- FILE...`:
//! the estimate and both confidences for each file, to check the thresholds.
fn main() {
    let go = std::sync::atomic::AtomicBool::new(false);
    for p in std::env::args().skip(1) {
        let t = std::time::Instant::now();
        match encore_analysis::analyse_file(std::path::Path::new(&p), &go) {
            Ok(e) => println!("{p}: {:?} ({:.2}) · {:?} ({:.2}) · {} ms", e.bpm, e.tempo_confidence, e.key, e.key_confidence, t.elapsed().as_millis()),
            Err(e) => println!("{p}: {e}"),
        }
    }
}
