//! The signed-in session: cookie jar plus the selected channel routing.
//! Stored only in the Keychain (D9).

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

use crate::jar::CookieJar;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChannelSelection {
    /// `X-Goog-AuthUser`: which signed-in Google login.
    pub auth_user: u32,
    /// `X-Goog-PageId` for a brand channel.
    pub page_id: Option<String>,
    /// Manager's user session id (from `datasyncIdToken`), for brand SAPISIDHASH.
    pub user_session_id: Option<String>,
    pub name: String,
    pub handle: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub version: u32,
    pub jar: CookieJar,
    /// `None` until the user has picked a channel.
    pub channel: Option<ChannelSelection>,
    /// "edge:Default", "webview", ...
    pub source: String,
    pub created_at: i64,
    pub verified_at: Option<i64>,
    /// Visitor data string bound to this identity (not a login secret, but kept with it).
    pub visitor_data: Option<String>,
}

impl Session {
    pub fn new(jar: CookieJar, source: &str) -> Self {
        Session { version: 1, jar, channel: None, source: source.into(), created_at: encore_core::store::now(), verified_at: None, visitor_data: None }
    }

    /// Opaque account key for cache scoping: login fingerprint + channel.
    pub fn account_key(&self) -> String {
        let login = self.jar.login_fingerprint().unwrap_or_else(|| "nologin".into());
        match &self.channel {
            Some(c) => format!("{login}-{}-{}", c.auth_user, c.page_id.as_deref().unwrap_or("primary")),
            None => format!("{login}-pending"),
        }
    }
}

/// `Authorization` header value for InnerTube (SAPISIDHASH and the 1P/3P
/// variants). With `user` (a brand channel's manager session id) the hash
/// covers it and carries the `_u` suffix.
pub fn sapisid_authorization(jar: &CookieJar, origin: &str, ts: u64, user: Option<&str>) -> Option<String> {
    let host = url::Url::parse(origin).ok()?.host_str()?.to_string();
    let third = jar.get("__Secure-3PAPISID", &host).map(str::to_string);
    let sapisid = jar.get("SAPISID", &host).map(str::to_string).or_else(|| third.clone())?;
    let schemes = [("SAPISIDHASH", Some(sapisid)), ("SAPISID1PHASH", jar.get("__Secure-1PAPISID", &host).map(str::to_string)), ("SAPISID3PHASH", third)];
    let parts: Vec<String> = schemes
        .into_iter()
        .filter_map(|(scheme, sid)| {
            let sid = sid?;
            let input = match user {
                Some(u) => format!("{u} {ts} {sid} {origin}"),
                None => format!("{ts} {sid} {origin}"),
            };
            let hash = hex::encode(Sha1::digest(input.as_bytes()));
            Some(match user {
                Some(_) => format!("{scheme} {ts}_{hash}_u"),
                None => format!("{scheme} {ts}_{hash}"),
            })
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use encore_core::service::CookieRecord;

    fn jar() -> CookieJar {
        let c = |n: &str, v: &str| CookieRecord { name: n.into(), value: v.into(), domain: ".youtube.com".into(), path: "/".into(), secure: true, http_only: false, expires: None };
        CookieJar::new(vec![c("SAPISID", "testsapisid"), c("__Secure-1PAPISID", "one"), c("__Secure-3PAPISID", "three")])
    }

    #[test]
    fn sapisidhash_matches_independent_sha1() {
        // Expected values computed independently with Python:
        // hashlib.sha1(b"1700000000 testsapisid https://music.youtube.com").hexdigest()
        let a = sapisid_authorization(&jar(), "https://music.youtube.com", 1_700_000_000, None).unwrap();
        let first = a.split(' ').take(2).collect::<Vec<_>>().join(" ");
        assert_eq!(first, "SAPISIDHASH 1700000000_f4115ca88d67a648ff621771c417aacbbce2e6b3");
        assert!(a.contains("SAPISID1PHASH 1700000000_"));
        assert!(a.contains("SAPISID3PHASH 1700000000_"));
    }

    #[test]
    fn brand_hash_has_user_and_suffix() {
        let a = sapisid_authorization(&jar(), "https://music.youtube.com", 1_700_000_000, Some("1000")).unwrap();
        assert!(a.starts_with("SAPISIDHASH 1700000000_28aeaf32d2bf7d28196071879ab70500a597e610_u"));
    }

    #[test]
    fn account_key_is_not_secret() {
        let mut s = Session::new(jar(), "test");
        s.channel = Some(ChannelSelection { auth_user: 0, page_id: Some("123".into()), ..Default::default() });
        let k = s.account_key();
        assert!(!k.contains("testsapisid"));
        assert!(k.ends_with("-0-123"));
    }
}
