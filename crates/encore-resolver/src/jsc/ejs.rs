//! yt-dlp-ejs, the challenge solver yt-dlp runs (D37).
//!
//! Upstream: yt-dlp `yt_dlp/extractor/youtube/jsc/_builtin/ejs.py`
//! (`_construct_stdin`, the output handling in `_real_bulk_solve`) and
//! `jsc/_builtin/vendor/_info.py` (the version and hashes), at the commit in
//! `upstream.toml`. The two bundles are the yt-dlp-ejs release files,
//! unmodified; `scripts/sync-upstream.sh` replaces them and the pins below.

use serde::{Deserialize, Serialize};

use super::{ChallengeRequest, ChallengeResults};

/// The yt-dlp-ejs release the bundles come from (`vendor/_info.py` VERSION).
pub const EJS_VERSION: &str = "0.8.0";

/// meriyah and astring, bundled as `lib` (ISC and MIT; their notices are in
/// the file's banner and in `NOTICE`).
pub const LIB: &str = include_str!("../../vendor/ejs/yt.solver.lib.min.js");
/// The solver itself, defining `jsc` (Unlicense).
pub const CORE: &str = include_str!("../../vendor/ejs/yt.solver.core.min.js");

/// SHA3-512 of the two bundles, as pinned by yt-dlp's `vendor/_info.py`
/// (`yt.solver.lib.min.js`, `yt.solver.core.min.js`).
pub const LIB_SHA3_512: &str = "8420c259ad16e99ce004e4651ac1bcabb53b4457bf5668a97a9359be9a998a789fee8ab124ee17f91a2ea8fd84e0f2b2fc8eabcaf0b16a186ba734cf422ad053";
pub const CORE_SHA3_512: &str = "ee5b307d07f55e91e4723edf5ac205cc877a474187849d757dc1322e38427b157a9d706d510c1723d3670f98e5a3f8cbcde77874a80406bd7204bc9fea30f283";

/// The glue between the two bundles (`_construct_stdin`), minus the input:
/// that is handed to `jsc` as a value, never pasted into source.
pub const GLUE: &str = "Object.assign(globalThis, lib);";

/// Whether the compiled-in bundles are the pinned ones.
pub fn bundles_verified() -> bool {
    sha3_hex(LIB) == LIB_SHA3_512 && sha3_hex(CORE) == CORE_SHA3_512
}

pub fn sha3_hex(s: &str) -> String {
    use sha3::Digest;
    hex::encode(sha3::Sha3_512::digest(s.as_bytes()))
}

/// The solver's input (`src/yt/solver/main.ts` `Input`).
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Input<'a> {
    Player { player: &'a str, requests: &'a [ChallengeRequest], output_preprocessed: bool },
    Preprocessed { preprocessed_player: &'a str, requests: &'a [ChallengeRequest] },
}

/// The solver's output (`main.ts` `Output`).
#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Output {
    Result {
        #[serde(default)]
        preprocessed_player: Option<String>,
        responses: Vec<Response>,
    },
    Error {
        error: String,
    },
}

#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Result { data: std::collections::HashMap<String, String> },
    Error { error: String },
}

impl Response {
    pub fn into_results(self) -> ChallengeResults {
        match self {
            Response::Result { data } => Ok(data),
            // The error may carry a JS stack: keep its first line.
            Response::Error { error } => Err(error.lines().next().unwrap_or("").chars().take(300).collect()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vendored_bundles_are_the_pinned_release() {
        assert!(bundles_verified(), "vendor/ejs does not match the SHA3-512 in upstream.toml; run scripts/sync-upstream.sh");
        let pins = include_str!("../../upstream.toml");
        assert!(pins.contains(LIB_SHA3_512) && pins.contains(CORE_SHA3_512) && pins.contains(&format!("version = \"{EJS_VERSION}\"")));
    }

    #[test]
    fn input_is_the_shape_ejs_reads() {
        let reqs = [ChallengeRequest { kind: super::super::ChallengeType::N, challenges: vec!["a".into()] }];
        let v = serde_json::to_value(Input::Player { player: "p", requests: &reqs, output_preprocessed: true }).unwrap();
        assert_eq!(v, serde_json::json!({"type": "player", "player": "p", "requests": [{"type": "n", "challenges": ["a"]}], "output_preprocessed": true}));
    }
}
