//! LRCLIB candidate matching (D12): NFKC + case folding,
//! "(Official Video)"-style decorations removed, featured artists dropped
//! from titles, version qualifiers (live, remix, acoustic, …) must agree,
//! title similarity ≥ 0.90, artist ≥ 0.85, duration within ±2 s.

use encore_core::model::Track;
use unicode_normalization::UnicodeNormalization;

use crate::LrclibRecord;

const VERSIONS: &[&str] = &["live", "remix", "acoustic", "instrumental", "karaoke", "sped up", "slowed", "nightcore", "demo", "unplugged", "edit", "extended", "remaster", "reprise", "cover", "a cappella", "acapella"];

pub fn norm(s: &str) -> String {
    let s: String = s.nfkc().collect::<String>().to_lowercase();
    let s = s.replace(['’', '‘', '`'], "'").replace(['“', '”'], "\"");
    s.chars().map(|c| if c.is_alphanumeric() || c == '\'' { c } else { ' ' }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Bracketed parts of a title: "(feat. X)", "[Live]", "- Remastered 2011".
fn brackets(title: &str) -> (String, Vec<String>) {
    let mut base = String::new();
    let mut parts = Vec::new();
    let mut depth = 0;
    let mut cur = String::new();
    for c in title.chars() {
        match c {
            '(' | '[' | '{' => {
                if depth == 0 && !cur.is_empty() {
                    base.push_str(&cur);
                    cur.clear();
                }
                depth += 1;
                if depth > 1 {
                    cur.push(c);
                }
            }
            ')' | ']' | '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    parts.push(std::mem::take(&mut cur));
                } else {
                    cur.push(c);
                }
            }
            _ => cur.push(c),
        }
    }
    if depth == 0 {
        base.push_str(&cur);
    } else {
        parts.push(cur);
    }
    // " - Live at X" / " - Remastered" suffixes.
    if let Some(i) = base.find(" - ") {
        parts.push(base[i + 3..].to_string());
        base.truncate(i);
    }
    (base, parts)
}

fn versions_of(parts: &[String], extra: &str) -> Vec<&'static str> {
    let mut v: Vec<&'static str> = Vec::new();
    let hay: Vec<String> = parts.iter().map(|p| norm(p)).chain(std::iter::once(norm(extra))).collect();
    for q in VERSIONS {
        let qq = norm(q);
        if hay.iter().any(|h| h.split(' ').collect::<Vec<_>>().windows(qq.split(' ').count()).any(|w| w.join(" ") == qq) || h.contains(&format!("{qq}ed")) || (qq == "remaster" && h.contains("remastered"))) {
            v.push(q);
        }
    }
    v.sort();
    v.dedup();
    v
}

/// Title without bracketed parts or " - …" suffixes (featured artists,
/// "(Official Video)", versions are compared separately).
pub fn clean_title(t: &str) -> String {
    brackets(t).0.trim().to_string()
}

pub struct Query {
    pub title_raw: String,
    pub title_clean: String,
    pub artist_raw: String,
    pub artists: Vec<String>,
    pub album_raw: Option<String>,
    pub duration: Option<f64>,
    versions: Vec<&'static str>,
}

pub struct Candidate {
    title: String,
    artist: String,
    album: Option<String>,
    duration: Option<f64>,
    versions: Vec<&'static str>,
}

impl From<&LrclibRecord> for Candidate {
    fn from(r: &LrclibRecord) -> Self {
        let t = r.track_name.clone().unwrap_or_default();
        let (_, parts) = brackets(&t);
        Candidate { title: norm(&clean_title(&t)), artist: norm(r.artist_name.as_deref().unwrap_or("")), album: r.album_name.as_deref().map(norm), duration: r.duration, versions: versions_of(&parts, "") }
    }
}

impl Query {
    pub fn from_track(t: &Track) -> Self {
        let mut title = t.title.clone();
        let mut artists: Vec<String> = t.artists.iter().map(|a| a.name.clone()).collect();
        // Music videos are often titled "Artist - Song (Official Video)".
        if let Some((a, rest)) = title.split_once(" - ") {
            if artists.iter().any(|x| norm(x) == norm(a)) || artists.is_empty() {
                if artists.is_empty() {
                    artists.push(a.trim().to_string());
                }
                title = rest.trim().to_string();
            }
        }
        let (_, parts) = brackets(&title);
        let title_clean = clean_title(&title);
        Query {
            title_raw: title_clean.clone(),
            title_clean,
            artist_raw: artists.first().cloned().unwrap_or_default(),
            artists: artists.iter().map(|a| norm(a)).collect(),
            album_raw: t.album.as_ref().map(|a| a.name.clone()).filter(|a| !a.is_empty()),
            duration: t.duration,
            versions: versions_of(&parts, ""),
        }
    }

    /// Hard acceptance rules.
    pub fn accepts(&self, c: &Candidate) -> bool {
        self.score(c).is_some()
    }

    /// Score in [0, 1] if the candidate passes all thresholds.
    pub fn score(&self, c: &Candidate) -> Option<f64> {
        let qt = norm(&self.title_clean);
        let ts = strsim::normalized_levenshtein(&qt, &c.title).max(if c.title.contains(&qt) || qt.contains(&c.title) { 0.92 } else { 0.0 });
        if ts < 0.90 {
            return None;
        }
        // Best match over all credited artists; also accept "A & B" candidates.
        let as_ = self.artists.iter().map(|a| strsim::normalized_levenshtein(a, &c.artist).max(if c.artist.contains(a.as_str()) && !a.is_empty() { 0.9 } else { 0.0 })).fold(0.0, f64::max);
        if as_ < 0.85 {
            return None;
        }
        if self.versions != c.versions {
            return None;
        }
        let dur_score = match (self.duration, c.duration) {
            (Some(a), Some(b)) => {
                let d = (a - b).abs();
                if d > 2.0f64.max(a * 0.01) {
                    return None;
                }
                1.0 - d / 2.0f64.max(a * 0.01)
            }
            // Unknown duration: demand an exact title and artist.
            _ => {
                if ts < 0.999 || as_ < 0.999 {
                    return None;
                }
                0.5
            }
        };
        let album = match (&self.album_raw, &c.album) {
            (Some(a), Some(b)) => strsim::normalized_levenshtein(&norm(a), b),
            _ => 0.5,
        };
        let s = 0.5 * ts + 0.3 * as_ + 0.15 * dur_score + 0.05 * album;
        (s >= 0.90).then_some(s)
    }
}

fn lyric_text(r: &LrclibRecord) -> String {
    let t = r.plain_lyrics.clone().or_else(|| r.synced_lyrics.as_deref().map(|s| crate::lrc::parse(s).into_iter().map(|l| l.text).collect::<Vec<_>>().join("\n"))).unwrap_or_default();
    norm(&t).chars().take(600).collect()
}

/// Highest score wins. If the runner-up is within 0.02 but carries clearly
/// different lyrics, refuse to guess (no automatic match).
pub fn pick(mut v: Vec<(f64, &LrclibRecord)>) -> Option<&LrclibRecord> {
    v.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let first = v.first()?;
    let ft = lyric_text(first.1);
    for other in v.iter().skip(1).take_while(|o| first.0 - o.0 < 0.02) {
        if other.1.id != first.1.id && strsim::normalized_levenshtein(&ft, &lyric_text(other.1)) < 0.8 {
            return None;
        }
    }
    Some(first.1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use encore_core::model::*;

    fn track(title: &str, artist: &str, dur: f64) -> Track {
        Track {
            id: ItemId::ytm("x"),
            title: title.into(),
            artists: vec![Credit { name: artist.into(), id: None }],
            album: None,
            duration: Some(dur),
            explicit: false,
            art: None,
            kind: TrackKind::Song,
            plays: None,
            liked: None,
            playable: true,
            counterpart: None,
        }
    }
    fn rec(title: &str, artist: &str, dur: f64) -> LrclibRecord {
        LrclibRecord { id: Some(1), track_name: Some(title.into()), artist_name: Some(artist.into()), album_name: None, duration: Some(dur), instrumental: false, plain_lyrics: Some("x".into()), synced_lyrics: Some("[00:01.00]x".into()) }
    }

    #[test]
    fn accepts_same_song_with_decorations() {
        let q = Query::from_track(&track("Get Lucky (feat. Pharrell Williams and Nile Rodgers)", "Daft Punk", 369.0));
        assert!(q.accepts(&Candidate::from(&rec("Get Lucky", "Daft Punk", 367.5))));
        let v = Query::from_track(&track("Daft Punk - Get Lucky (Official Video)", "Daft Punk", 368.0));
        assert!(v.accepts(&Candidate::from(&rec("Get Lucky", "Daft Punk", 369.0))));
    }

    #[test]
    fn rejects_wrong_duration_artist_or_version() {
        let q = Query::from_track(&track("Get Lucky", "Daft Punk", 369.0));
        assert!(!q.accepts(&Candidate::from(&rec("Get Lucky", "Daft Punk", 248.0))), "radio edit length");
        assert!(!q.accepts(&Candidate::from(&rec("Get Lucky", "Some Cover Band", 369.0))));
        assert!(!q.accepts(&Candidate::from(&rec("Get Lucky (Live)", "Daft Punk", 369.0))));
        assert!(!q.accepts(&Candidate::from(&rec("Get Lucky - Remix", "Daft Punk", 369.0))));
        let live = Query::from_track(&track("Get Lucky (Live)", "Daft Punk", 369.0));
        assert!(live.accepts(&Candidate::from(&rec("Get Lucky (Live)", "Daft Punk", 369.0))));
    }

    #[test]
    fn normalises_unicode() {
        assert_eq!(norm("Ｈｅｌｌｏ， World!"), "hello world");
        assert_eq!(norm("Don’t Stop"), "don't stop");
    }

    #[test]
    fn ambiguous_tie_gives_no_match() {
        let a = rec("Song", "Artist", 200.0);
        let mut b = rec("Song", "Artist", 200.0);
        b.id = Some(2);
        b.synced_lyrics = Some("[00:01.00]other".into());
        b.plain_lyrics = Some("other".into());
        assert!(pick(vec![(0.95, &a), (0.945, &b)]).is_none());
        assert!(pick(vec![(0.99, &a), (0.91, &b)]).is_some());
    }
}
