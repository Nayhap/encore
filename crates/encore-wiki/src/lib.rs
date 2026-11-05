//! An artist's Wikipedia summary, for artist pages YouTube gives no
//! description (D29). Through Wikipedia's REST API (`page/summary`, no key).
//!
//! Names are ambiguous ("Genesis", "Nirvana"), so a page is accepted only
//! when it is clearly the musician:
//!
//! * its title is the artist's name, or the name with " (band)",
//!   " (musician)" or " (singer)" (Wikipedia's own way of telling namesakes
//!   apart), compared without case or accents;
//! * it is an article, not a disambiguation page;
//! * its short description says it is a musician, band, singer, …
//!
//! The plain name is tried first, then each variant; nothing else (no
//! search, no "closest match"). No answer is better than another person's
//! life story under an artist's name.

use encore_core::local::integrations::WikiExtract;
use serde_json::Value;
use unicode_normalization::UnicodeNormalization;

/// The one language Encore asks YouTube in, so the one Wikipedia it reads.
pub const LANG: &str = "en";

const VARIANTS: [&str; 3] = ["band", "musician", "singer"];

#[derive(Debug, PartialEq)]
pub enum Error {
    /// No answer from Wikipedia; try again later.
    Network,
}

/// The pages to try, in order.
pub fn candidates(name: &str) -> Vec<String> {
    let name = name.trim();
    std::iter::once(name.to_string()).chain(VARIANTS.iter().map(|v| format!("{name} ({v})"))).collect()
}

/// Lower case, accents dropped, spaces collapsed.
fn fold(s: &str) -> String {
    s.nfkd().filter(|c| !('\u{300}'..='\u{36f}').contains(c)).collect::<String>().to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A short description that names a musician or a band.
pub fn is_musical(description: &str) -> bool {
    const STRONG: &[&str] = &[
        "band", "singer", "musician", "rapper", "dj", "duo", "trio", "composer", "songwriter", "guitarist", "pianist", "drummer", "bassist", "vocalist",
        "orchestra", "choir", "idol", "boyband", "supergroup", "disc jockey", "beatmaker", "violinist", "cellist", "saxophonist", "trumpeter",
    ];
    const WEAK: &[&str] = &["group", "producer", "ensemble", "quartet", "quintet", "artist", "project", "collective", "act"];
    const MUSIC: &[&str] = &[
        "music", "musical", "record", "rock", "pop", "hip hop", "hip-hop", "rap", "jazz", "metal", "punk", "folk", "soul", "r&b", "electronic", "indie", "country",
        "vocal", "girl", "boy", "k-pop", "j-pop", "techno", "house", "edm", "reggae", "blues", "gospel", "classical", "ambient", "dance",
    ];
    let d = fold(description);
    let words: Vec<&str> = d.split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '&')).filter(|w| !w.is_empty()).collect();
    let has = |w: &str| if w.contains(' ') { d.contains(w) } else { words.iter().any(|x| *x == w || x.split('-').any(|p| p == w)) };
    STRONG.iter().any(|w| has(w)) || (WEAK.iter().any(|w| has(w)) && MUSIC.iter().any(|w| has(w)))
}

/// The extract, if `summary` is the artist's page under the rules above.
pub fn accept(name: &str, summary: &Value) -> Option<WikiExtract> {
    if summary.get("type").and_then(Value::as_str) != Some("standard") {
        return None;
    }
    let title = summary.get("title").and_then(Value::as_str)?;
    let description = summary.get("description").and_then(Value::as_str)?;
    let bare = VARIANTS.iter().find_map(|v| title.strip_suffix(&format!(" ({v})"))).unwrap_or(title);
    if fold(bare) != fold(name) || !is_musical(description) {
        return None;
    }
    let extract = summary.get("extract").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())?;
    let url = summary.pointer("/content_urls/desktop/page").and_then(Value::as_str)?;
    if !url.starts_with(&format!("https://{LANG}.wikipedia.org/")) {
        return None;
    }
    Some(WikiExtract { title: title.to_string(), description: Some(description.to_string()), extract: extract.to_string(), url: url.to_string(), lang: LANG.into() })
}

/// Try each candidate through `fetch` (a page's summary, `None` when there
/// is no such page) and return the first that passes.
pub async fn lookup_with<F, Fut>(name: &str, mut fetch: F) -> Result<Option<WikiExtract>, Error>
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = Result<Option<Value>, Error>>,
{
    if name.trim().is_empty() {
        return Ok(None);
    }
    for title in candidates(name) {
        if let Some(s) = fetch(title).await?
            && let Some(found) = accept(name, &s)
        {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

/// A JSON body read to its end but never past `max` bytes; `None` when
/// it was interrupted, larger or not JSON.
async fn json_capped(mut r: reqwest::Response, max: usize) -> Option<Value> {
    if r.content_length().is_some_and(|l| l as usize > max) {
        return None;
    }
    let mut out = Vec::new();
    while let Some(c) = r.chunk().await.ok()? {
        if out.len() + c.len() > max {
            return None;
        }
        out.extend_from_slice(&c);
    }
    serde_json::from_slice(&out).ok()
}


/// [`lookup_with`] over HTTPS.
pub async fn lookup(http: &reqwest::Client, name: &str) -> Result<Option<WikiExtract>, Error> {
    lookup_with(name, |title| async move {
        let mut u = url::Url::parse(&format!("https://{LANG}.wikipedia.org/api/rest_v1/page/summary/")).expect("static URL");
        u.path_segments_mut().expect("base").pop_if_empty().push(&title.replace(' ', "_"));
        u.set_query(Some("redirect=true"));
        let r = http.get(u).send().await.map_err(|_| Error::Network)?;
        match r.status().as_u16() {
            // A summary is a few KB.
            200 => json_capped(r, 1024 * 1024).await.map(Some).ok_or(Error::Network),
            404 => Ok(None),
            // Busy or down: no answer now, so nothing is remembered.
            _ => Err(Error::Network),
        }
    })
    .await
}

/// The client Wikipedia asks for: one that says who it is.
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("Encore/", env!("CARGO_PKG_VERSION"), " (https://github.com/Nayhap/encore)"))
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn fixture(name: &str) -> Value {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(name);
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    #[test]
    fn music_descriptions() {
        for d in ["French electronic music duo", "American rock band", "Japanese singer-songwriter", "South Korean girl group", "British record producer", "American rapper and singer", "Icelandic post-rock band"] {
            assert!(is_musical(d), "{d}");
        }
        for d in ["Book of the Bible", "American media group", "Film producer", "Genus of plants", "Japanese manga artist"] {
            assert!(!is_musical(d), "{d}");
        }
    }

    #[test]
    fn accepts_only_the_musician() {
        assert_eq!(accept("Daft Punk", &fixture("daft_punk.json")).map(|w| w.title), Some("Daft Punk".into()));
        assert_eq!(accept("daft punk", &fixture("daft_punk.json")).map(|w| w.url), Some("https://en.wikipedia.org/wiki/Daft_Punk".into()));
        assert!(accept("Beyonce", &fixture("beyonce.json")).is_some(), "accents don't matter");
        assert!(accept("Nirvana", &fixture("nirvana.json")).is_none(), "a disambiguation page");
        assert!(accept("Nirvana", &fixture("nirvana_band.json")).is_some());
        assert!(accept("Genesis", &fixture("genesis.json")).is_none(), "not a musician");
        assert!(accept("Daft Punk Tribute", &fixture("daft_punk.json")).is_none(), "another name");
    }

    #[tokio::test]
    async fn tries_the_variants_in_order() {
        let pages: HashMap<&str, Value> = [("Nirvana", fixture("nirvana.json")), ("Nirvana (band)", fixture("nirvana_band.json"))].into();
        let mut asked = vec![];
        let r = lookup_with("Nirvana", |t| {
            asked.push(t.clone());
            let v = pages.get(t.as_str()).cloned();
            async move { Ok(v) }
        })
        .await
        .unwrap();
        assert_eq!(r.map(|w| w.title), Some("Nirvana (band)".into()));
        assert_eq!(asked, ["Nirvana", "Nirvana (band)"]);
        let none = lookup_with("Genesis", |_| async { Ok(Some(fixture("genesis.json"))) }).await.unwrap();
        assert_eq!(none, None);
        let off = lookup_with("Daft Punk", |_| async { Err(Error::Network) }).await;
        assert_eq!(off, Err(Error::Network));
    }
}
