//! `cargo run -p encore-resolver --example sha3 -- <file>...`: the SHA3-512
//! of files, for scripts/sync-upstream.sh where OpenSSL lacks it.

fn main() {
    use sha3::Digest;
    for f in std::env::args().skip(1) {
        let bytes = std::fs::read(&f).unwrap_or_else(|e| panic!("{f}: {e}"));
        println!("{}", hex::encode(sha3::Sha3_512::digest(&bytes)));
    }
}
