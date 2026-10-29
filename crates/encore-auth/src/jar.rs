//! A structured cookie jar (not a header string): keeps domain, path, expiry
//! and flags so `Set-Cookie` rotation and tombstones apply correctly.

use encore_core::service::CookieRecord;
use serde::{Deserialize, Serialize};

/// Domains Encore keeps from an imported browser (D8).
pub const KEEP_DOMAINS: &[&str] = &[".youtube.com", "youtube.com", "music.youtube.com", ".music.youtube.com", ".google.com", "google.com", "accounts.google.com"];

pub fn keep_domain(domain: &str) -> bool {
    let d = domain.to_ascii_lowercase();
    KEEP_DOMAINS.contains(&d.as_str())
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CookieJar {
    pub cookies: Vec<CookieRecord>,
}

impl std::fmt::Debug for CookieJar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CookieJar({} cookies)", self.cookies.len())
    }
}

fn now() -> i64 {
    encore_core::store::now()
}

fn domain_match(cookie_domain: &str, host: &str) -> bool {
    let cd = cookie_domain.trim_start_matches('.').to_ascii_lowercase();
    let h = host.to_ascii_lowercase();
    if cookie_domain.starts_with('.') {
        h == cd || h.ends_with(&format!(".{cd}"))
    } else {
        h == cd
    }
}

fn path_match(cookie_path: &str, path: &str) -> bool {
    let cp = if cookie_path.is_empty() { "/" } else { cookie_path };
    path == cp || (path.starts_with(cp) && (cp.ends_with('/') || path[cp.len()..].starts_with('/')))
}

impl CookieJar {
    pub fn new(cookies: Vec<CookieRecord>) -> Self {
        let mut j = CookieJar::default();
        for c in cookies {
            j.upsert(c);
        }
        j
    }

    pub fn len(&self) -> usize {
        self.cookies.len()
    }
    pub fn is_empty(&self) -> bool {
        self.cookies.is_empty()
    }

    fn upsert(&mut self, c: CookieRecord) {
        self.cookies.retain(|x| !(x.name == c.name && x.domain.eq_ignore_ascii_case(&c.domain) && x.path == c.path));
        if c.expires.is_none_or(|e| e > now()) {
            self.cookies.push(c);
        }
    }

    /// Value of a cookie by name for a host (most specific domain wins).
    pub fn get(&self, name: &str, host: &str) -> Option<&str> {
        self.cookies
            .iter()
            .filter(|c| c.name == name && domain_match(&c.domain, host) && c.expires.is_none_or(|e| e > now()))
            .max_by_key(|c| c.domain.len())
            .map(|c| c.value.as_str())
    }

    /// `Cookie` header for a request URL.
    pub fn header_for(&self, url: &url::Url) -> String {
        let host = url.host_str().unwrap_or_default();
        let path = url.path();
        let https = url.scheme() == "https";
        let t = now();
        let mut v: Vec<&CookieRecord> = self
            .cookies
            .iter()
            .filter(|c| domain_match(&c.domain, host) && path_match(&c.path, path) && (!c.secure || https) && c.expires.is_none_or(|e| e > t))
            .filter(|c| !c.value.contains(';') && !c.value.contains('\n') && !c.value.contains('\r'))
            .collect();
        // Longer paths first (RFC 6265 §5.4).
        v.sort_by(|a, b| b.path.len().cmp(&a.path.len()));
        let mut seen = std::collections::HashSet::new();
        v.into_iter().filter(|c| seen.insert(c.name.clone())).map(|c| format!("{}={}", c.name, c.value)).collect::<Vec<_>>().join("; ")
    }

    /// Apply one `Set-Cookie` header received from `url`. Returns true if
    /// the jar changed.
    pub fn apply_set_cookie(&mut self, url: &url::Url, header: &str) -> bool {
        let host = url.host_str().unwrap_or_default().to_string();
        let mut parts = header.split(';');
        let Some((name, value)) = parts.next().and_then(|nv| nv.split_once('=')) else { return false };
        let (name, value) = (name.trim().to_string(), value.trim().to_string());
        if name.is_empty() {
            return false;
        }
        let mut domain = host.clone();
        let mut host_only = true;
        let mut path = "/".to_string();
        let mut expires: Option<i64> = None;
        let mut max_age: Option<i64> = None;
        let mut secure = false;
        let mut http_only = false;
        for a in parts {
            let a = a.trim();
            let (k, v) = a.split_once('=').map(|(k, v)| (k.trim(), v.trim())).unwrap_or((a, ""));
            match k.to_ascii_lowercase().as_str() {
                "domain" => {
                    let d = v.trim_start_matches('.').to_ascii_lowercase();
                    if !d.is_empty() && (host == d || host.ends_with(&format!(".{d}"))) {
                        domain = format!(".{d}");
                        host_only = false;
                    } else {
                        return false; // cookie for a foreign domain: ignore
                    }
                }
                "path" if v.starts_with('/') => path = v.to_string(),
                "expires" => expires = httpdate::parse_http_date(v).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).or(Some(0)),
                "max-age" => max_age = v.parse::<i64>().ok(),
                "secure" => secure = true,
                "httponly" => http_only = true,
                _ => {}
            }
        }
        let _ = host_only;
        let exp = match (max_age, expires) {
            (Some(m), _) => Some(now() + m),
            (None, e) => e,
        };
        let before = self.cookies.clone();
        let rec = CookieRecord { name, value, domain, path, secure, http_only, expires: exp };
        if exp.is_some_and(|e| e <= now()) {
            // Tombstone: delete.
            self.cookies.retain(|x| !(x.name == rec.name && x.domain.eq_ignore_ascii_case(&rec.domain) && x.path == rec.path));
        } else {
            self.upsert(rec);
        }
        self.cookies != before
    }

    /// Drop anything outside the YouTube/Google domains Encore needs.
    pub fn retain_needed(&mut self) {
        self.cookies.retain(|c| keep_domain(&c.domain));
    }

    pub fn has_login(&self) -> bool {
        self.get("SAPISID", "music.youtube.com").is_some() || self.get("__Secure-3PAPISID", "music.youtube.com").is_some()
    }

    pub fn records(&self) -> &[CookieRecord] {
        &self.cookies
    }

    /// Short non-secret fingerprint of the login, for cache scoping: changes
    /// when the user logs into a different Google account.
    pub fn login_fingerprint(&self) -> Option<String> {
        use sha2::{Digest, Sha256};
        let sid = self.get("SAPISID", "music.youtube.com").or_else(|| self.get("__Secure-3PAPISID", "music.youtube.com"))?;
        let d = Sha256::digest(format!("encore-account:{sid}").as_bytes());
        Some(hex::encode(&d[..8]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(name: &str, value: &str, domain: &str) -> CookieRecord {
        CookieRecord { name: name.into(), value: value.into(), domain: domain.into(), path: "/".into(), secure: true, http_only: false, expires: None }
    }

    #[test]
    fn header_matches_domains() {
        let j = CookieJar::new(vec![c("SAPISID", "a", ".youtube.com"), c("X", "b", "music.youtube.com"), c("G", "z", ".google.com")]);
        let u = url::Url::parse("https://music.youtube.com/youtubei/v1/browse").unwrap();
        let h = j.header_for(&u);
        assert!(h.contains("SAPISID=a"));
        assert!(h.contains("X=b"));
        assert!(!h.contains("G=z"));
        let w = url::Url::parse("https://www.youtube.com/").unwrap();
        assert!(!j.header_for(&w).contains("X=b"));
    }

    #[test]
    fn set_cookie_rotation_and_tombstone() {
        let mut j = CookieJar::new(vec![c("SIDCC", "old", ".youtube.com"), c("GONE", "x", ".youtube.com")]);
        let u = url::Url::parse("https://music.youtube.com/verify_session").unwrap();
        assert!(j.apply_set_cookie(&u, "SIDCC=new; Domain=.youtube.com; Path=/; Max-Age=3600; Secure; HttpOnly"));
        assert_eq!(j.get("SIDCC", "music.youtube.com"), Some("new"));
        assert!(j.apply_set_cookie(&u, "GONE=; Domain=.youtube.com; Path=/; Expires=Thu, 01 Jan 1970 00:00:01 GMT"));
        assert_eq!(j.get("GONE", "music.youtube.com"), None);
        // foreign domain ignored
        assert!(!j.apply_set_cookie(&u, "EVIL=1; Domain=.example.com"));
        assert_eq!(j.len(), 1);
    }

    #[test]
    fn filters_domains() {
        let mut j = CookieJar::new(vec![c("A", "1", ".youtube.com"), c("B", "2", ".doubleclick.net"), c("C", "3", "accounts.google.com"), c("D", "4", "mail.google.com")]);
        j.retain_needed();
        let names: Vec<&str> = j.records().iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["A", "C"]);
    }
}
