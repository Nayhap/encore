//! JavaScript challenges: the `n` parameter and the signature (D37).
//!
//! Upstream: yt-dlp `yt_dlp/extractor/youtube/jsc/provider.py` (the request
//! and response model) and `jsc/_director.py` (`validate_response`), at the
//! commit in `upstream.toml`. The solver itself is yt-dlp-ejs, vendored
//! verbatim in `vendor/ejs/` and run by [`quickjs::QuickJsSolver`].

pub mod cache;
pub mod ejs;
pub mod quickjs;

use std::collections::HashMap;

use async_trait::async_trait;

/// `JsChallengeType` in `jsc/provider.py`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChallengeType {
    N,
    Sig,
}

/// One kind of challenge with every value of it a video needs, solved in
/// one go (`JsChallengeRequest`; `_video.py` `solve_js_challenges`).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ChallengeRequest {
    #[serde(rename = "type")]
    pub kind: ChallengeType,
    pub challenges: Vec<String>,
}

/// A player script to solve against. `id` names it in caches and logs: the
/// player id and variant (`9fcf08e8/main`), never a URL with a query.
#[derive(Clone, Debug)]
pub struct PlayerScript {
    pub id: String,
    pub code: std::sync::Arc<str>,
}

/// What one request came back with: challenge → answer, or why not.
pub type ChallengeResults = Result<HashMap<String, String>, String>;

/// How a whole solve failed. Per-request failures are in
/// [`ChallengeResults`]; this is for the engine or the script.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SolveError {
    /// The player could not be preprocessed (yt-dlp-ejs threw: usually a
    /// player layout it doesn't know yet).
    #[error("the challenge solver could not read this player: {0}")]
    Player(String),
    /// Out of time or memory, or the engine failed.
    #[error("the challenge solver stopped: {0}")]
    Engine(String),
    /// The solver is closed (Encore is quitting).
    #[error("the challenge solver is closed")]
    Closed,
}

/// Something that answers n and signature challenges for a player.
/// [`quickjs::QuickJsSolver`] is the one Encore ships; tests use fakes.
#[async_trait]
pub trait Solver: Send + Sync {
    async fn solve(&self, player: &PlayerScript, requests: Vec<ChallengeRequest>) -> Result<Vec<ChallengeResults>, SolveError>;
}

/// `validate_response` (`jsc/_director.py:255-290`): every challenge must
/// have a string answer, and an `n` answer that ends with its input means
/// the player's function threw and returned its fallback.
pub fn validate(request: &ChallengeRequest, results: ChallengeResults) -> ChallengeResults {
    let data = results?;
    if data.len() != request.challenges.len() || !request.challenges.iter().all(|c| data.contains_key(c)) {
        return Err(format!("invalid {:?} output", request.kind));
    }
    if request.kind == ChallengeType::N
        && let Some((c, _)) = data.iter().find(|(c, r)| r.ends_with(c.as_str())) {
            return Err(format!("n result is invalid for a challenge of {} characters", c.len()));
        }
    Ok(data)
}

/// The synthetic signature challenge for a signature length
/// (`_video.py:3323`: `''.join(map(chr, range(spec_id)))`).
pub fn sig_spec_challenge(len: usize) -> String {
    (0..len as u32).filter_map(char::from_u32).collect()
}

/// The permutation a solved spec challenge describes: output position →
/// input position (`_video.py:3330`: `[ord(c) for c in result]`).
pub fn sig_permutation(result: &str) -> Vec<usize> {
    result.chars().map(|c| c as usize).collect()
}

/// Apply a permutation to a real signature (`_video.py` `_decrypt_signature`
/// with a cached `sigfuncs` entry). `None` if it doesn't fit.
pub fn apply_permutation(perm: &[usize], s: &str) -> Option<String> {
    let chars: Vec<char> = s.chars().collect();
    perm.iter().map(|&i| chars.get(i).copied()).collect()
}

/// A tiny player written for Encore (not YouTube's code) in the shape
/// yt-dlp-ejs looks for: one IIFE whose URL-builder function calls
/// `.set("alr", "yes")` and returns an object whose first own method
/// applies the `n` transform. It lets the self-test and the unit tests run
/// the whole solver offline, with nothing of YouTube's in the binary.
pub const SYNTHETIC_PLAYER: &str = r#"(function(g){
var Q = function(base){ this.base = base; this.q = {}; };
Q.prototype.set = function(k, v){ this.q[k] = v; };
Q.prototype.get = function(k){ return this.q[k]; };
Q.prototype.clone = function(){ var c = new Q(this.base); c.q = Object.assign({}, this.q); return c; };
Q.prototype.finish = function(){ if (this.q.n) this.q.n = nt(this.q.n); };
var nt = function(a){ /*N*/return a.split("").reverse().join(""); };
var st = function(a){ return a.split("").reverse().slice(1).join(""); };
var build = function(url, sp, sig){ var u = new Q(url); u.set("alr", "yes"); if (sig) u.set(sp, encodeURIComponent(st(decodeURIComponent(sig)))); return u; };
}).call(this);"#;
/// What the synthetic player makes of `n=abcdef` and signature `abcdef`.
pub const SYNTHETIC_N_ABCDEF: &str = "fedcba";
pub const SYNTHETIC_SIG_ABCDEF: &str = "edcba";

/// The synthetic player with another `n` function body (adversarial tests).
pub fn synthetic_with_n_body(body: &str) -> String {
    SYNTHETIC_PLAYER.replace("/*N*/", &format!("{body};"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn n_answers_that_echo_the_input_are_errors() {
        let req = ChallengeRequest { kind: ChallengeType::N, challenges: vec!["abc".into()] };
        let ok: HashMap<_, _> = [("abc".to_string(), "xyz".to_string())].into();
        assert!(validate(&req, Ok(ok)).is_ok());
        let echo: HashMap<_, _> = [("abc".to_string(), "enhanced_except_abc".to_string())].into();
        assert!(validate(&req, Ok(echo)).is_err());
        assert!(validate(&req, Ok(HashMap::new())).is_err());
    }

    #[test]
    fn a_signature_is_permuted_like_its_spec_challenge() {
        let spec = sig_spec_challenge(5);
        assert_eq!(spec.chars().count(), 5);
        // Reversed, then the last character dropped: what a player does.
        let perm = sig_permutation("\u{4}\u{3}\u{2}\u{1}");
        assert_eq!(apply_permutation(&perm, "abcde").as_deref(), Some("edcb"));
        assert_eq!(apply_permutation(&[9], "abc"), None);
    }
}
