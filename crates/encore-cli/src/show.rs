//! Human-readable printing of domain objects.

use encore_core::model::*;

pub fn dur(d: Option<f64>) -> String {
    match d {
        Some(s) => format!("{}:{:02}", (s as u64) / 60, (s as u64) % 60),
        None => "-:--".into(),
    }
}

pub fn track(t: &Track) -> String {
    let album = t.album.as_ref().map(|a| format!(" [{}]", a.name)).unwrap_or_default();
    let e = if t.explicit { " (E)" } else { "" };
    let p = if t.playable { "" } else { " (unplayable)" };
    format!("{}  {}{} — {}{}  {}{}  ({:?})", t.id, t.title, e, t.artist_line(), album, dur(t.duration), p, t.kind)
}

pub fn item(i: &ShelfItem) -> String {
    match i {
        ShelfItem::Track(t) => format!("song   {}", track(t)),
        ShelfItem::Collection(c) => format!("{:<6} {}  {} — {}", format!("{:?}", c.kind).to_lowercase(), c.id, c.title, c.subtitle),
        ShelfItem::Artist(a) => format!("artist {}  {} — {}", a.id, a.name, a.subtitle),
        ShelfItem::Link(l) => format!("link   {}{}  {}", l.browse_id, l.params.as_ref().map(|_| " (+params)").unwrap_or(""), l.title),
    }
}

pub fn page(p: &Page, max_items: usize) {
    if let Some(t) = &p.title {
        println!("# {t}");
    }
    if !p.chips.is_empty() {
        println!("chips: {}", p.chips.iter().map(|c| c.title.as_str()).collect::<Vec<_>>().join(" | "));
    }
    for s in &p.shelves {
        println!("\n== {} ({:?}, {} items){}", if s.title.is_empty() { "(untitled)" } else { &s.title }, s.layout, s.items.len(), s.more.as_ref().map(|m| format!(" more→{}", m.browse_id)).unwrap_or_default());
        for i in s.items.iter().take(max_items) {
            println!("  {}", item(i));
        }
        if s.items.len() > max_items {
            println!("  … {} more", s.items.len() - max_items);
        }
    }
    println!("\nshelves: {}  more pages: {}", p.shelves.len(), p.cursor.is_some());
}

pub fn collection(c: &Collection, max: usize) {
    println!("# {} ({:?})", c.title, c.kind);
    println!("id: {}  playlist: {}", c.id, c.playlist_id.as_deref().unwrap_or("-"));
    println!("by: {}", c.artists.iter().map(|a| format!("{}{}", a.name, a.id.as_ref().map(|i| format!(" <{i}>")).unwrap_or_default())).collect::<Vec<_>>().join(", "));
    println!("{} | {}", c.subtitle, c.stats.as_deref().unwrap_or(""));
    if let Some(d) = &c.description {
        let d: String = d.chars().take(160).collect();
        println!("about: {d}…");
    }
    println!("art: {}  saved: {:?}  editable: {}", c.art.as_ref().map(|a| a.key.as_str()).unwrap_or("-"), c.saved, c.editable);
    for (i, e) in c.entries.iter().enumerate().take(max) {
        println!("  {:>3}. {}", i + 1, track(&e.track));
    }
    if c.entries.len() > max {
        println!("  … {} more", c.entries.len() - max);
    }
    for s in &c.related {
        println!("  related shelf: {} ({} items)", s.title, s.items.len());
    }
}

pub fn artist(a: &Artist, max: usize) {
    println!("# {}  ({})", a.name, a.subscribers.as_deref().unwrap_or("?"));
    println!("id: {} channel: {} radio: {} shuffle: {} subscribed: {:?}", a.id, a.channel_id.as_deref().unwrap_or("-"), a.radio_playlist.as_deref().unwrap_or("-"), a.shuffle_playlist.as_deref().unwrap_or("-"), a.subscribed);
    if let Some(d) = &a.description {
        let d: String = d.chars().take(160).collect();
        println!("about: {d}…");
    }
    println!("top songs ({}){}:", a.top_songs.len(), a.top_songs_more.as_ref().map(|m| format!(" all→{}", m.browse_id)).unwrap_or_default());
    for t in a.top_songs.iter().take(max) {
        println!("  {}", track(t));
    }
    for s in &a.shelves {
        println!("== {} ({} items)", s.title, s.items.len());
        for i in s.items.iter().take(3) {
            println!("  {}", item(i));
        }
    }
}

pub fn search(r: &SearchResults, max: usize) {
    if let Some(t) = &r.top {
        println!("top: {}", item(t));
    }
    if let Some(d) = &r.did_you_mean {
        println!("did you mean: {d}");
    }
    let sec = |name: &str, lines: Vec<String>| {
        if !lines.is_empty() {
            println!("\n== {name} ({})", lines.len());
            for l in lines.iter().take(max) {
                println!("  {l}");
            }
        }
    };
    sec("songs", r.songs.iter().map(track).collect());
    sec("videos", r.videos.iter().map(track).collect());
    sec("albums", r.albums.iter().map(|c| item(&ShelfItem::Collection(c.clone()))).collect());
    sec("artists", r.artists.iter().map(|a| item(&ShelfItem::Artist(a.clone()))).collect());
    sec("playlists", r.playlists.iter().map(|c| item(&ShelfItem::Collection(c.clone()))).collect());
    println!("\nmore: {}", r.cursor.is_some());
}
