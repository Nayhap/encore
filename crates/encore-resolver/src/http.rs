//! Reading YouTube's answers with a size cap, and telling a refusal from
//! a rate limit (D4, D37). Every request of a resolve goes through here:
//! the pages, the player script and `/player`.

use reqwest::header::HeaderMap;

/// How a fetch failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fetch {
    /// No answer, a timeout, an interrupted body or a 5xx.
    Network(String),
    /// 429, or YouTube's "Sorry" page: a bot check for this address.
    RateLimited,
    /// Any other status.
    Status(u16),
    /// Larger than this kind of answer ever is.
    TooLarge,
}

/// The answer, read to the end but never past `max` bytes (decompressed),
/// whatever `Content-Length` says.
pub async fn read_capped(mut r: reqwest::Response, max: usize) -> Result<Vec<u8>, Fetch> {
    if r.content_length().is_some_and(|l| l as usize > max) {
        return Err(Fetch::TooLarge);
    }
    let mut out = Vec::new();
    while let Some(c) = r.chunk().await.map_err(|e| Fetch::Network(if e.is_timeout() { "timed out".into() } else { "interrupted".into() }))? {
        if out.len() + c.len() > max {
            return Err(Fetch::TooLarge);
        }
        out.extend_from_slice(&c);
    }
    Ok(out)
}

/// YouTube's "unusual traffic" page (what yt-dlp reports as a 429).
pub fn sorry_page(body: &[u8]) -> bool {
    let head = &body[..body.len().min(4096)];
    let s = String::from_utf8_lossy(head);
    s.trim_start().starts_with('<') && (s.contains("<title>Sorry") || s.contains("google.com/sorry"))
}

/// Send a request and read a 2xx answer of at most `max` bytes.
pub async fn get(rb: reqwest::RequestBuilder, max: usize) -> Result<(u16, HeaderMap, Vec<u8>), Fetch> {
    let r = rb.send().await.map_err(|e| Fetch::Network(if e.is_timeout() { "timed out".into() } else { "unreachable".into() }))?;
    let status = r.status().as_u16();
    let headers = r.headers().clone();
    if status == 429 {
        return Err(Fetch::RateLimited);
    }
    if status >= 500 {
        return Err(Fetch::Network(format!("answered {status}")));
    }
    let body = read_capped(r, max).await?;
    if sorry_page(&body) {
        return Err(Fetch::RateLimited);
    }
    Ok((status, headers, body))
}

/// The `Set-Cookie` headers of an answer.
pub fn set_cookies(h: &HeaderMap) -> Vec<String> {
    h.get_all("set-cookie").iter().filter_map(|v| v.to_str().ok()).map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sorry_page_is_a_rate_limit() {
        assert!(sorry_page(b"<html><head><title>Sorry...</title>"));
        assert!(sorry_page(b"  <!DOCTYPE html><a href=\"https://www.google.com/sorry/index\">"));
        assert!(!sorry_page(b"{\"playabilityStatus\":{}}"));
        assert!(!sorry_page(b"<html><title>YouTube</title>"));
    }
}
