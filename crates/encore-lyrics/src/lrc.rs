//! LRC parser: multiple timestamps per line, fractional precision, `[offset:]`
//! tags, metadata lines, and enhanced word tags `<mm:ss.xx>`.

use encore_core::model::{LyricLine, Word};

fn parse_stamp(s: &str) -> Option<f64> {
    // mm:ss(.xx) or mm:ss:xx
    let (m, rest) = s.split_once(':')?;
    let m: f64 = m.trim().parse().ok()?;
    let rest = rest.replace(':', ".");
    let sec: f64 = rest.trim().parse().ok()?;
    if !(0.0..60.0 + 1e-9).contains(&sec) || m < 0.0 {
        return None;
    }
    Some(m * 60.0 + sec)
}

fn words(text: &str, line_start: f64) -> (String, Option<Vec<Word>>) {
    if !text.contains('<') {
        return (text.trim().to_string(), None);
    }
    let mut out = Vec::new();
    let mut plain = String::new();
    let mut rest = text;
    let mut cur_t: Option<f64> = None;
    while let Some(i) = rest.find('<') {
        let before = &rest[..i];
        if let Some(t) = cur_t {
            if !before.trim().is_empty() {
                out.push(Word { t, end: None, text: before.to_string() });
            }
        } else if !before.trim().is_empty() {
            out.push(Word { t: line_start, end: None, text: before.to_string() });
        }
        plain.push_str(before);
        let Some(j) = rest[i..].find('>') else { break };
        cur_t = parse_stamp(&rest[i + 1..i + j]);
        rest = &rest[i + j + 1..];
    }
    if let Some(t) = cur_t {
        if !rest.trim().is_empty() {
            out.push(Word { t, end: None, text: rest.to_string() });
        }
    }
    plain.push_str(rest);
    // Each word ends where the next begins.
    for k in 0..out.len().saturating_sub(1) {
        out[k].end = Some(out[k + 1].t);
    }
    let plain = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    (plain, (!out.is_empty()).then_some(out))
}

pub fn parse(src: &str) -> Vec<LyricLine> {
    let mut offset = 0.0;
    let mut lines: Vec<LyricLine> = Vec::new();
    for raw in src.lines() {
        let mut s = raw.trim();
        let mut stamps = Vec::new();
        while s.starts_with('[') {
            let Some(end) = s.find(']') else { break };
            let tag = &s[1..end];
            if let Some(o) = tag.strip_prefix("offset:") {
                // Positive offset shifts lyrics earlier (ms).
                offset = o.trim().parse::<f64>().map(|ms| ms / 1000.0).unwrap_or(0.0);
            } else if let Some(t) = parse_stamp(tag) {
                stamps.push(t);
            }
            s = s[end + 1..].trim_start();
        }
        if stamps.is_empty() {
            continue;
        }
        for t in stamps {
            let t = (t - offset).max(0.0);
            let (text, w) = words(s, t);
            lines.push(LyricLine { t, end: None, text, words: w });
        }
    }
    lines.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
    // Empty lines mark instrumental gaps: they end the previous line.
    let mut out: Vec<LyricLine> = Vec::new();
    for l in lines {
        if let Some(prev) = out.last_mut() {
            if prev.end.is_none() {
                prev.end = Some(l.t);
            }
        }
        if !l.text.is_empty() {
            out.push(l);
        } else if let Some(prev) = out.last_mut() {
            prev.end = Some(l.t);
        }
    }
    out.retain(|l| l.t.is_finite());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_and_multi_stamp() {
        let l = parse("[ar:Someone]\n[00:01.50]Hello\n[00:05.00][00:20.25]Chorus\n[00:10.00]\n[00:12.345]World");
        let ts: Vec<f64> = l.iter().map(|x| x.t).collect();
        assert_eq!(ts, vec![1.5, 5.0, 12.345, 20.25]);
        assert_eq!(l[1].text, "Chorus");
        assert_eq!(l[1].end, Some(10.0), "empty line ends the previous one");
        assert_eq!(l[3].text, "Chorus");
    }

    #[test]
    fn offset_tag() {
        let l = parse("[offset:+500]\n[00:02.00]a\n[00:03.00]b");
        assert!((l[0].t - 1.5).abs() < 1e-9);
    }

    #[test]
    fn enhanced_words() {
        let l = parse("[00:10.00]<00:10.00>Hel <00:10.50>lo <00:11.20>world");
        assert_eq!(l[0].text, "Hel lo world");
        let w = l[0].words.as_ref().unwrap();
        assert_eq!(w.len(), 3);
        assert_eq!(w[1].t, 10.5);
        assert_eq!(w[0].end, Some(10.5));
    }

    #[test]
    fn junk_is_ignored() {
        assert!(parse("no tags here\n[xx:yy]bad").is_empty());
    }
}
