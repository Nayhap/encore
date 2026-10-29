//! Secret redaction (D9). Everything that might reach a log, an error message
//! or the UI goes through [`redact`]: cookie values, SAPISID hashes, OAuth/PO
//! tokens and the query string of signed googlevideo URLs are replaced.

use std::fmt;
use std::io::{self, Write};
use std::sync::OnceLock;

use regex::Regex;

/// Wrapper whose `Debug`/`Display` never print the value.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret<T>(T);

impl<T> Secret<T> {
    pub fn new(v: T) -> Self {
        Secret(v)
    }
    pub fn expose(&self) -> &T {
        &self.0
    }
    pub fn into_inner(self) -> T {
        self.0
    }
}
impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}
impl<T> fmt::Display for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

fn patterns() -> &'static [(Regex, &'static str)] {
    static P: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    P.get_or_init(|| {
        let r = |s: &str| Regex::new(s).expect("redaction regex");
        vec![
            // Signed media URLs: keep scheme/host/path, drop the query.
            (r(r"(https?://[A-Za-z0-9.\-]*googlevideo\.com/[^\s?#]*)\?[^\s]*"), "$1?<redacted>"),
            (r(r"(https?://[^\s?#]+/videoplayback)\?[^\s]*"), "$1?<redacted>"),
            // Authorization header values.
            (r(r"(?i)(SAPISID(?:1P|3P)?HASH)\s+[0-9]+_[0-9a-f]+(?:_u)?"), "$1 <redacted>"),
            (r(r"(?i)(authorization\s*[:=]\s*)(Bearer\s+)?[^\s,;]+"), "$1<redacted>"),
            // Cookie name=value pairs for Google session cookies.
            (
                r(r"(?i)\b((?:__Secure-[0-9A-Za-z\-]+|__Host-[0-9A-Za-z\-]+|SAPISID|APISID|HSID|SSID|SID|SIDCC|LOGIN_INFO|NID|AEC|SOCS|VISITOR_INFO1_LIVE|VISITOR_PRIVACY_METADATA|YSC|PREF)\s*=\s*)[^;\s,]+"),
                "$1<redacted>",
            ),
            // Token-ish query/body parameters.
            // Query parameters that carry tokens/signatures (must follow ? or &).
            (r(r"(?i)([?&](?:pot|po_token|poToken|sig|lsig|signature|n|token|access_token|refresh_token|id_token)=)[^&\s\x22]+"), "$1<redacted>"),
            (r(r"(?i)\b((?:po_token|access_token|refresh_token|id_token)=)[^&\s\x22]+"), "$1<redacted>"),
            (r(r#"(?i)("(?:poToken|visitorData|cookie|authorization|sapisid|accessToken|refreshToken)"\s*:\s*")[^"]*""#), "$1<redacted>\""),
            // Long opaque blobs that look like tokens.
            (r(r"\bya29\.[0-9A-Za-z_\-]+"), "<redacted-oauth>"),
        ]
    })
}

pub fn redact(s: &str) -> String {
    let mut out = s.to_string();
    for (re, rep) in patterns() {
        if re.is_match(&out) {
            out = re.replace_all(&out, *rep).into_owned();
        }
    }
    out
}

/// Strip the query string of any URL (for logging hosts/paths only).
pub fn url_for_log(u: &str) -> String {
    match u.split_once('?') {
        Some((a, _)) => format!("{a}?<redacted>"),
        None => u.to_string(),
    }
}

/// A writer that redacts complete lines before passing them on. Partial
/// lines are buffered so a secret split across `write` calls is still caught.
pub struct RedactingWriter<W: Write>(pub W, Vec<u8>);

impl<W: Write> RedactingWriter<W> {
    pub fn new(w: W) -> Self {
        RedactingWriter(w, Vec::new())
    }
    pub fn into_inner(mut self) -> W {
        let _ = self.flush();
        let RedactingWriter(w, _) = self;
        w
    }
}

impl<W: Write> Write for RedactingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.1.extend_from_slice(buf);
        while let Some(i) = self.1.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.1.drain(..=i).collect();
            let s = String::from_utf8_lossy(&line);
            self.0.write_all(redact(&s).as_bytes())?;
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        if !self.1.is_empty() {
            let line = std::mem::take(&mut self.1);
            let s = String::from_utf8_lossy(&line);
            self.0.write_all(redact(&s).as_bytes())?;
        }
        self.0.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Canary secrets: none of these may survive redaction (D9 canary test).
    const CANARY: &str = "CANARYcanary0123456789";

    #[test]
    fn cookies_are_redacted() {
        let s = format!("Cookie: SAPISID={CANARY}; __Secure-3PAPISID={CANARY}; SID={CANARY}; PREF=f6=40000000");
        let r = redact(&s);
        assert!(!r.contains(CANARY), "{r}");
        assert!(!r.contains("f6=4"), "{r}");
    }

    #[test]
    fn auth_headers_are_redacted() {
        let s = format!("Authorization: SAPISIDHASH 1730000000_{}abcdef SAPISID1PHASH 1730000000_0123abcd_u", "0123456789");
        let r = redact(&s);
        assert!(!r.contains("0123456789abcdef"), "{r}");
        assert!(!r.contains("0123abcd"), "{r}");
        let r2 = redact(&format!("authorization=Bearer ya29.{CANARY}"));
        assert!(!r2.contains(CANARY), "{r2}");
    }

    #[test]
    fn signed_urls_lose_query() {
        let u = format!("https://rr3---sn-abc.googlevideo.com/videoplayback?expire=1&sig={CANARY}&n={CANARY}&pot={CANARY}");
        let r = redact(&format!("fetching {u} failed"));
        assert!(!r.contains(CANARY), "{r}");
        assert!(r.contains("googlevideo.com/videoplayback?<redacted>"), "{r}");
    }

    #[test]
    fn plain_text_is_left_alone() {
        let s = r#"Live on ESPN.\n\n= WATCH LIVE, a=b, sign=1"#;
        assert_eq!(redact(s), s);
    }

    #[test]
    fn json_tokens_redacted() {
        let s = format!(r#"{{"poToken":"{CANARY}","visitorData":"{CANARY}","ok":1}}"#);
        assert!(!redact(&s).contains(CANARY));
    }

    #[test]
    fn secret_debug_hides_value() {
        let s = Secret::new(CANARY.to_string());
        assert!(!format!("{s:?} {s}").contains(CANARY));
    }

    #[test]
    fn writer_redacts() {
        let mut w = RedactingWriter::new(Vec::new());
        write!(w, "SAPISID={CANARY}").unwrap();
        writeln!(w, " and more").unwrap();
        let out = String::from_utf8(w.into_inner()).unwrap();
        assert!(out.contains("SAPISID=<redacted>"), "{out}");
        assert!(!out.contains(CANARY));
    }
}
