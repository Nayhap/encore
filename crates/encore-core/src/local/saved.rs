//! Things the user keeps on this computer (lane C): bookmarks (D25), saved
//! queues (D33) and folders (D34). Tables in `local::SCHEMA`. Every method
//! takes the owner captured when the operation started, and never touches
//! another owner's rows.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::{AppError, Result};
use crate::local::Owner;
use crate::model::{ArtRef, CollectionCard, CollectionKind, ItemId, Track};
use crate::queue::Queue;
use crate::store::{Store, db_err, now};

/// A page that can be opened again: the same routes as the UI's `View`.
/// Links keep their `params` (an artist's "All songs" needs them), never a
/// continuation cursor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "name", rename_all = "camelCase")]
pub enum PageRef {
    Collection {
        id: ItemId,
        params: Option<String>,
    },
    Artist {
        id: ItemId,
    },
    Page {
        #[serde(rename = "browseId")]
        browse_id: String,
        params: Option<String>,
        title: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum BookmarkKind {
    Page,
    Track,
    /// A song at a time: opening it plays from there.
    Moment,
}

impl BookmarkKind {
    fn as_str(self) -> &'static str {
        match self {
            BookmarkKind::Page => "page",
            BookmarkKind::Track => "track",
            BookmarkKind::Moment => "moment",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "page" => Some(BookmarkKind::Page),
            "track" => Some(BookmarkKind::Track),
            "moment" => Some(BookmarkKind::Moment),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum BookmarkTarget {
    Page(PageRef),
    Track(Track),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Bookmark {
    pub id: String,
    pub kind: BookmarkKind,
    pub target: BookmarkTarget,
    /// Seconds into the song, for a moment.
    pub position: Option<f64>,
    pub title: String,
    pub subtitle: String,
    pub art: Option<ArtRef>,
    /// Unix seconds, as a string on the wire (D13).
    pub created: String,
}

/// A saved queue as listed (the queue itself stays in Rust until restored).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SavedQueueInfo {
    pub id: String,
    pub name: String,
    pub songs: u32,
    pub duration: Option<f64>,
    /// The song it restores to.
    pub current: Option<Track>,
    pub position: f64,
    pub updated: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: String,
    pub name: String,
    /// An opaque local art key (served by `art://`, never a path).
    pub cover: Option<String>,
    pub collapsed: bool,
    pub items: Vec<FolderItem>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FolderItem {
    pub item_id: ItemId,
    pub card: CollectionCard,
}

/// Names and labels are the user's, but bounded.
const NAME_MAX: usize = 120;
const LABEL_MAX: usize = 300;

fn clip(s: &str, max: usize) -> String {
    let s: String = s.chars().filter(|c| !c.is_control()).collect();
    let s = s.trim();
    s.chars().take(max).collect()
}

/// A name for a folder or a saved queue: trimmed, one line, not empty.
pub fn clean_name(s: &str) -> Result<String> {
    let n = clip(s, NAME_MAX);
    if n.is_empty() { Err(AppError::invalid("Give it a name.")) } else { Ok(n) }
}

fn json<T: Serialize>(v: &T) -> Result<String> {
    serde_json::to_string(v).map_err(|e| AppError::internal(e.to_string()))
}

fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Two moments of one song this close together are the same bookmark.
const SAME_MOMENT: f64 = 2.0;

impl BookmarkTarget {
    fn same(&self, other: &BookmarkTarget) -> bool {
        match (self, other) {
            (BookmarkTarget::Page(a), BookmarkTarget::Page(b)) => a == b,
            (BookmarkTarget::Track(a), BookmarkTarget::Track(b)) => a.id == b.id,
            _ => false,
        }
    }
}

impl Store {
    // ---------- bookmarks (D25) ----------

    /// Newest first.
    pub fn bookmarks(&self, owner: &Owner) -> Result<Vec<Bookmark>> {
        let c = self.conn.lock();
        let mut st = c.prepare("SELECT id, kind, target, position, title, subtitle, art, created FROM bookmarks WHERE owner = ?1 ORDER BY created DESC, rowid DESC").map_err(db_err)?;
        let rows = st
            .query_map(params![owner.as_str()], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<f64>>(3)?, r.get::<_, String>(4)?, r.get::<_, String>(5)?, r.get::<_, Option<String>>(6)?, r.get::<_, i64>(7)?))
            })
            .map_err(db_err)?;
        let mut out = Vec::new();
        for row in rows {
            let (id, kind, target, position, title, subtitle, art, created) = row.map_err(db_err)?;
            // A row this version can't read is skipped, not fatal.
            let (Some(kind), Ok(target)) = (BookmarkKind::parse(&kind), serde_json::from_str(&target)) else { continue };
            let art = art.and_then(|a| serde_json::from_str(&a).ok());
            out.push(Bookmark { id, kind, target, position, title, subtitle, art, created: created.to_string() });
        }
        Ok(out)
    }

    /// Bookmark a page, a song, or a song at `position` (a moment). The same
    /// page or song again, or a moment within two seconds of one already
    /// kept, returns the bookmark that exists.
    pub fn bookmark_add(&self, owner: &Owner, target: BookmarkTarget, position: Option<f64>, title: &str, subtitle: &str, art: Option<ArtRef>) -> Result<Bookmark> {
        let position = match position {
            Some(p) if !p.is_finite() || p < 0.0 => return Err(AppError::invalid("That time isn't in the song.")),
            p => p,
        };
        let kind = match (&target, position) {
            (BookmarkTarget::Page(_), None) => BookmarkKind::Page,
            (BookmarkTarget::Page(_), Some(_)) => return Err(AppError::invalid("A page has no time to bookmark.")),
            (BookmarkTarget::Track(_), None) => BookmarkKind::Track,
            (BookmarkTarget::Track(_), Some(_)) => BookmarkKind::Moment,
        };
        let target = match target {
            // Likes depend on the account, not on the bookmark.
            BookmarkTarget::Track(mut t) => {
                t.liked = None;
                BookmarkTarget::Track(t)
            }
            p => p,
        };
        let existing = self.bookmarks(owner)?.into_iter().find(|b| {
            b.kind == kind
                && b.target.same(&target)
                && match (b.position, position) {
                    (Some(a), Some(p)) => (a - p).abs() < SAME_MOMENT,
                    _ => true,
                }
        });
        if let Some(b) = existing {
            return Ok(b);
        }
        let b = Bookmark { id: id(), kind, target, position, title: clip(title, LABEL_MAX), subtitle: clip(subtitle, LABEL_MAX), art, created: now().to_string() };
        self.conn
            .lock()
            .execute(
                "INSERT INTO bookmarks (id, owner, kind, target, position, title, subtitle, art, created) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![b.id, owner.as_str(), kind.as_str(), json(&b.target)?, b.position, b.title, b.subtitle, b.art.as_ref().map(json).transpose()?, now()],
            )
            .map_err(db_err)?;
        Ok(b)
    }

    pub fn bookmark_remove(&self, owner: &Owner, id: &str) -> Result<bool> {
        let n = self.conn.lock().execute("DELETE FROM bookmarks WHERE id = ?1 AND owner = ?2", params![id, owner.as_str()]).map_err(db_err)?;
        Ok(n > 0)
    }

    // ---------- saved queues (D33) ----------

    /// Most recently saved first.
    pub fn saved_queues(&self, owner: &Owner) -> Result<Vec<SavedQueueInfo>> {
        let c = self.conn.lock();
        let mut st = c.prepare("SELECT id, name, queue, position, songs, duration, updated FROM saved_queues WHERE owner = ?1 ORDER BY updated DESC, rowid DESC").map_err(db_err)?;
        let rows = st
            .query_map(params![owner.as_str()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, f64>(3)?, r.get::<_, i64>(4)?, r.get::<_, Option<f64>>(5)?, r.get::<_, i64>(6)?)))
            .map_err(db_err)?;
        let mut out = Vec::new();
        for row in rows {
            let (id, name, queue, position, songs, duration, updated) = row.map_err(db_err)?;
            let current = serde_json::from_str::<Queue>(&queue).ok().and_then(|q| q.current().map(|e| e.track.clone()));
            out.push(SavedQueueInfo { id, name, songs: songs.max(0) as u32, duration, current, position, updated: updated.to_string() });
        }
        Ok(out)
    }

    /// Keep a copy of `queue`, at `position` in its current song, under
    /// `name`. Saving under a name that exists replaces that copy.
    pub fn saved_queue_save(&self, owner: &Owner, name: &str, queue: &Queue, position: f64) -> Result<SavedQueueInfo> {
        let name = clean_name(name)?;
        if queue.is_empty() {
            return Err(AppError::invalid("The queue is empty."));
        }
        let position = if position.is_finite() { position.max(0.0) } else { 0.0 };
        let songs = queue.len() as i64;
        let durations: Vec<Option<f64>> = queue.entries().iter().map(|e| e.track.duration).collect();
        // The total only when every length is known.
        let duration = durations.iter().copied().sum::<Option<f64>>();
        let t = now();
        let c = self.conn.lock();
        let found: Option<String> = c.query_row("SELECT id FROM saved_queues WHERE owner = ?1 AND name = ?2", params![owner.as_str(), name], |r| r.get(0)).optional().map_err(db_err)?;
        let id = found.unwrap_or_else(id);
        c.execute(
            "INSERT INTO saved_queues (id, owner, name, queue, position, songs, duration, created, updated) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
             ON CONFLICT(id) DO UPDATE SET queue = excluded.queue, position = excluded.position, songs = excluded.songs, duration = excluded.duration, updated = excluded.updated",
            params![id, owner.as_str(), name, json(queue)?, position, songs, duration, t],
        )
        .map_err(db_err)?;
        Ok(SavedQueueInfo { id, name, songs: songs as u32, duration, current: queue.current().map(|e| e.track.clone()), position, updated: t.to_string() })
    }

    /// The queue and the position in its current song, to restore.
    pub fn saved_queue_load(&self, owner: &Owner, id: &str) -> Result<Option<(Queue, f64)>> {
        let row: Option<(String, f64)> = self.conn.lock().query_row("SELECT queue, position FROM saved_queues WHERE id = ?1 AND owner = ?2", params![id, owner.as_str()], |r| Ok((r.get(0)?, r.get(1)?))).optional().map_err(db_err)?;
        let Some((q, pos)) = row else { return Ok(None) };
        let q = serde_json::from_str::<Queue>(&q).map_err(|_| AppError::internal("This saved queue can't be read by this version of Encore."))?;
        Ok(Some((q, pos)))
    }

    pub fn saved_queue_remove(&self, owner: &Owner, id: &str) -> Result<bool> {
        let n = self.conn.lock().execute("DELETE FROM saved_queues WHERE id = ?1 AND owner = ?2", params![id, owner.as_str()]).map_err(db_err)?;
        Ok(n > 0)
    }

    // ---------- folders (D34) ----------

    /// In the order they were made, each with its items in order.
    pub fn folders(&self, owner: &Owner) -> Result<Vec<Folder>> {
        let c = self.conn.lock();
        let mut st = c.prepare("SELECT id, name, cover, collapsed FROM folders WHERE owner = ?1 ORDER BY position, created").map_err(db_err)?;
        let mut folders: Vec<Folder> = st
            .query_map(params![owner.as_str()], |r| Ok(Folder { id: r.get(0)?, name: r.get(1)?, cover: r.get(2)?, collapsed: r.get(3)?, items: Vec::new() }))
            .map_err(db_err)?
            .collect::<std::result::Result<_, _>>()
            .map_err(db_err)?;
        let mut items = c.prepare("SELECT item_id, card FROM folder_items WHERE folder_id = ?1 ORDER BY position, added").map_err(db_err)?;
        for f in &mut folders {
            let rows = items.query_map(params![f.id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map_err(db_err)?;
            for row in rows {
                let (item, card) = row.map_err(db_err)?;
                if let Ok(card) = serde_json::from_str::<CollectionCard>(&card) {
                    f.items.push(FolderItem { item_id: ItemId(item), card });
                }
            }
        }
        Ok(folders)
    }

    pub fn folder(&self, owner: &Owner, id: &str) -> Result<Option<Folder>> {
        Ok(self.folders(owner)?.into_iter().find(|f| f.id == id))
    }

    fn folder_mine(c: &rusqlite::Connection, owner: &Owner, id: &str) -> Result<()> {
        let n: i64 = c.query_row("SELECT COUNT(*) FROM folders WHERE id = ?1 AND owner = ?2", params![id, owner.as_str()], |r| r.get(0)).map_err(db_err)?;
        if n == 0 { Err(AppError::not_found("That folder is gone.")) } else { Ok(()) }
    }

    /// A new folder, last in the list, holding `items` (filed out of any
    /// other folder they were in).
    pub fn folder_create(&self, owner: &Owner, name: &str, items: &[CollectionCard]) -> Result<Folder> {
        let name = clean_name(name)?;
        let id = id();
        {
            let mut c = self.conn.lock();
            let tx = c.transaction().map_err(db_err)?;
            let pos: i64 = tx.query_row("SELECT COALESCE(MAX(position), -1) + 1 FROM folders WHERE owner = ?1", params![owner.as_str()], |r| r.get(0)).map_err(db_err)?;
            let t = now();
            tx.execute("INSERT INTO folders (id, owner, name, cover, position, collapsed, created, updated) VALUES (?1, ?2, ?3, NULL, ?4, 0, ?5, ?5)", params![id, owner.as_str(), name, pos, t]).map_err(db_err)?;
            for card in items {
                file_into(&tx, owner, &id, card)?;
            }
            tx.commit().map_err(db_err)?;
        }
        self.folder(owner, &id)?.ok_or_else(|| AppError::internal("folder vanished"))
    }

    pub fn folder_rename(&self, owner: &Owner, id: &str, name: &str) -> Result<()> {
        let name = clean_name(name)?;
        let n = self.conn.lock().execute("UPDATE folders SET name = ?3, updated = ?4 WHERE id = ?1 AND owner = ?2", params![id, owner.as_str(), name, now()]).map_err(db_err)?;
        if n == 0 { Err(AppError::not_found("That folder is gone.")) } else { Ok(()) }
    }

    pub fn folder_set_collapsed(&self, owner: &Owner, id: &str, collapsed: bool) -> Result<()> {
        self.conn.lock().execute("UPDATE folders SET collapsed = ?3 WHERE id = ?1 AND owner = ?2", params![id, owner.as_str(), collapsed]).map_err(db_err)?;
        Ok(())
    }

    /// Set or clear the cover; returns the key it replaced, whose file the
    /// caller deletes.
    pub fn folder_set_cover(&self, owner: &Owner, id: &str, cover: Option<&str>) -> Result<Option<String>> {
        let mut c = self.conn.lock();
        let tx = c.transaction().map_err(db_err)?;
        let old: Option<Option<String>> = tx.query_row("SELECT cover FROM folders WHERE id = ?1 AND owner = ?2", params![id, owner.as_str()], |r| r.get(0)).optional().map_err(db_err)?;
        let Some(old) = old else { return Err(AppError::not_found("That folder is gone.")) };
        tx.execute("UPDATE folders SET cover = ?3, updated = ?4 WHERE id = ?1 AND owner = ?2", params![id, owner.as_str(), cover, now()]).map_err(db_err)?;
        tx.commit().map_err(db_err)?;
        Ok(old.filter(|o| Some(o.as_str()) != cover))
    }

    /// Delete a folder (never the playlists in it); returns its cover key.
    pub fn folder_remove(&self, owner: &Owner, id: &str) -> Result<Option<String>> {
        let mut c = self.conn.lock();
        let tx = c.transaction().map_err(db_err)?;
        let cover: Option<Option<String>> = tx.query_row("SELECT cover FROM folders WHERE id = ?1 AND owner = ?2", params![id, owner.as_str()], |r| r.get(0)).optional().map_err(db_err)?;
        tx.execute("DELETE FROM folder_items WHERE folder_id = (SELECT id FROM folders WHERE id = ?1 AND owner = ?2)", params![id, owner.as_str()]).map_err(db_err)?;
        tx.execute("DELETE FROM folders WHERE id = ?1 AND owner = ?2", params![id, owner.as_str()]).map_err(db_err)?;
        tx.commit().map_err(db_err)?;
        Ok(cover.flatten())
    }

    /// File a playlist or album in a folder, last. It leaves any other
    /// folder of this owner: a list lives in one place in the sidebar.
    pub fn folder_add(&self, owner: &Owner, id: &str, card: &CollectionCard) -> Result<()> {
        let mut c = self.conn.lock();
        let tx = c.transaction().map_err(db_err)?;
        Self::folder_mine(&tx, owner, id)?;
        file_into(&tx, owner, id, card)?;
        tx.execute("UPDATE folders SET updated = ?2 WHERE id = ?1", params![id, now()]).map_err(db_err)?;
        tx.commit().map_err(db_err)
    }

    /// Take a playlist or album out of a folder (the list itself stays).
    pub fn folder_remove_item(&self, owner: &Owner, id: &str, item: &ItemId) -> Result<()> {
        let c = self.conn.lock();
        Self::folder_mine(&c, owner, id)?;
        c.execute("DELETE FROM folder_items WHERE folder_id = ?1 AND item_id = ?2", params![id, item.0]).map_err(db_err)?;
        Ok(())
    }

    /// Move an item within its folder, before `before` (or to the end).
    pub fn folder_move_item(&self, owner: &Owner, id: &str, item: &ItemId, before: Option<&ItemId>) -> Result<()> {
        let mut c = self.conn.lock();
        let tx = c.transaction().map_err(db_err)?;
        Self::folder_mine(&tx, owner, id)?;
        let mut order: Vec<String> = {
            let mut st = tx.prepare("SELECT item_id FROM folder_items WHERE folder_id = ?1 ORDER BY position, added").map_err(db_err)?;
            st.query_map(params![id], |r| r.get(0)).map_err(db_err)?.collect::<std::result::Result<_, _>>().map_err(db_err)?
        };
        let Some(from) = order.iter().position(|x| *x == item.0) else { return Err(AppError::not_found("That isn't in this folder any more.")) };
        let moving = order.remove(from);
        let to = before.and_then(|b| order.iter().position(|x| *x == b.0)).unwrap_or(order.len());
        order.insert(to, moving);
        for (i, it) in order.iter().enumerate() {
            tx.execute("UPDATE folder_items SET position = ?3 WHERE folder_id = ?1 AND item_id = ?2", params![id, it, i as i64]).map_err(db_err)?;
        }
        tx.commit().map_err(db_err)
    }
}

fn file_into(tx: &rusqlite::Transaction<'_>, owner: &Owner, folder: &str, card: &CollectionCard) -> Result<()> {
    let item = &card.id.0;
    tx.execute("DELETE FROM folder_items WHERE item_id = ?1 AND folder_id IN (SELECT id FROM folders WHERE owner = ?2 AND id <> ?3)", params![item, owner.as_str(), folder]).map_err(db_err)?;
    let exists: i64 = tx.query_row("SELECT COUNT(*) FROM folder_items WHERE folder_id = ?1 AND item_id = ?2", params![folder, item], |r| r.get(0)).map_err(db_err)?;
    if exists > 0 {
        // Already there: keep its place, refresh the card.
        tx.execute("UPDATE folder_items SET card = ?3 WHERE folder_id = ?1 AND item_id = ?2", params![folder, item, json(card)?]).map_err(db_err)?;
        return Ok(());
    }
    let pos: i64 = tx.query_row("SELECT COALESCE(MAX(position), -1) + 1 FROM folder_items WHERE folder_id = ?1", params![folder], |r| r.get(0)).map_err(db_err)?;
    tx.execute("INSERT INTO folder_items (folder_id, item_id, card, position, added) VALUES (?1, ?2, ?3, ?4, ?5)", params![folder, item, json(card)?, pos, now()]).map_err(db_err)?;
    Ok(())
}

// ---------- the .encorefolder file (D34) ----------

/// What a folder file holds: its name and links to what's in it, for
/// someone else's Encore to open. No covers, songs, accounts or anything
/// that only works on this computer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FolderFile {
    /// Always "encorefolder"; tells the file apart from other JSON.
    pub format: String,
    pub version: u32,
    pub name: String,
    pub items: Vec<FolderFileItem>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FolderFileItem {
    /// `https://music.youtube.com/playlist?list=…` or `…/browse/MPRE…`.
    pub link: String,
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    #[serde(default)]
    pub kind: Option<CollectionKind>,
    /// The cover's address, only when it is on an allow-listed host (D14).
    #[serde(default)]
    pub art: Option<String>,
}

pub const FOLDER_FORMAT: &str = "encorefolder";
pub const FOLDER_FILE_MAX: usize = 1024 * 1024;
const FOLDER_ITEMS_MAX: usize = 500;

/// The YouTube Music link for a playlist or album id.
pub fn collection_link(id: &ItemId) -> Option<String> {
    let r = id.remote();
    if !safe_id(r) {
        return None;
    }
    Some(if r.starts_with("MPRE") { format!("https://music.youtube.com/browse/{r}") } else { format!("https://music.youtube.com/playlist?list={}", r.strip_prefix("VL").unwrap_or(r)) })
}

fn safe_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 80 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The id a link names, if it is a YouTube Music playlist or album link.
pub fn link_id(link: &str) -> Option<ItemId> {
    let u = url::Url::parse(link.trim()).ok()?;
    if u.scheme() != "https" || !matches!(u.host_str()?, "music.youtube.com" | "www.youtube.com" | "youtube.com") {
        return None;
    }
    let path = u.path().trim_end_matches('/');
    if path == "/playlist" {
        let list = u.query_pairs().find(|(k, _)| k == "list")?.1.to_string();
        if !safe_id(&list) {
            return None;
        }
        // Playlists are browsed as VL….
        return Some(ItemId::ytm(&if list.starts_with("VL") { list } else { format!("VL{list}") }));
    }
    let b = path.strip_prefix("/browse/")?;
    (safe_id(b) && (b.starts_with("MPRE") || b.starts_with("VL"))).then(|| ItemId::ytm(b))
}

impl FolderFile {
    pub fn of(f: &Folder) -> Self {
        let items = f
            .items
            .iter()
            .filter_map(|it| {
                Some(FolderFileItem {
                    link: collection_link(&it.item_id)?,
                    title: it.card.title.clone(),
                    subtitle: it.card.subtitle.clone(),
                    kind: Some(it.card.kind),
                    art: it.card.art.as_ref().and_then(|a| crate::art::public_url(&a.key)),
                })
            })
            .collect();
        FolderFile { format: FOLDER_FORMAT.into(), version: 1, name: f.name.clone(), items }
    }

    /// Read a file someone sent: strictly, whatever is in it. Links that
    /// aren't YouTube Music playlists or albums are dropped; covers only
    /// from allow-listed hosts.
    pub fn parse(bytes: &[u8]) -> Result<(String, Vec<CollectionCard>)> {
        let bad = || AppError::invalid("This isn't an Encore folder file.");
        if bytes.len() > FOLDER_FILE_MAX {
            return Err(AppError::invalid("This file is too large to be an Encore folder."));
        }
        let f: FolderFile = serde_json::from_slice(bytes).map_err(|_| bad())?;
        if f.format != FOLDER_FORMAT {
            return Err(bad());
        }
        if f.version > 1 {
            return Err(AppError::invalid("This folder was made by a newer Encore. Update Encore to open it."));
        }
        let name = clean_name(&f.name).unwrap_or_else(|_| "Imported folder".into());
        let mut seen = std::collections::HashSet::new();
        let cards = f
            .items
            .into_iter()
            .take(FOLDER_ITEMS_MAX)
            .filter_map(|it| {
                let id = link_id(&it.link)?;
                if !seen.insert(id.0.clone()) {
                    return None;
                }
                let kind = it.kind.unwrap_or(if id.remote().starts_with("MPRE") { CollectionKind::Album } else { CollectionKind::Playlist });
                let title = clip(&it.title, LABEL_MAX);
                Some(CollectionCard {
                    title: if title.is_empty() { "Untitled".into() } else { title },
                    subtitle: clip(&it.subtitle, LABEL_MAX),
                    kind,
                    artists: vec![],
                    year: None,
                    art: it.art.as_deref().and_then(crate::art::register),
                    play_id: None,
                    id,
                })
            })
            .collect();
        Ok((name, cards))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Credit, TrackKind};
    use crate::queue::{Origin, QueueSource};

    fn track(id: &str, dur: Option<f64>) -> Track {
        Track { id: ItemId::ytm(id), title: format!("Song {id}"), artists: vec![Credit { name: "Artist".into(), id: None }], album: None, duration: dur, explicit: false, art: None, kind: TrackKind::Song, plays: None, liked: Some(true), playable: true, counterpart: None }
    }

    fn card(id: &str) -> CollectionCard {
        CollectionCard { id: ItemId::ytm(id), kind: CollectionKind::Playlist, title: format!("List {id}"), subtitle: "Playlist".into(), artists: vec![], year: None, art: None, play_id: None }
    }

    #[test]
    fn bookmarks_are_per_owner_and_not_repeated() {
        let s = Store::open_memory().unwrap();
        let (a, b) = (Owner::new("a"), Owner::anonymous());
        let page = BookmarkTarget::Page(PageRef::Collection { id: ItemId::ytm("VLx"), params: None });
        let one = s.bookmark_add(&a, page.clone(), None, "List", "Playlist", None).unwrap();
        assert_eq!(one.kind, BookmarkKind::Page);
        assert_eq!(s.bookmark_add(&a, page.clone(), None, "List", "Playlist", None).unwrap().id, one.id);
        // Same page with other params is another page.
        s.bookmark_add(&a, BookmarkTarget::Page(PageRef::Collection { id: ItemId::ytm("VLx"), params: Some("p".into()) }), None, "List", "", None).unwrap();
        let song = s.bookmark_add(&a, BookmarkTarget::Track(track("v1", Some(200.0))), None, "Song", "", None).unwrap();
        assert_eq!(song.kind, BookmarkKind::Track);
        let m1 = s.bookmark_add(&a, BookmarkTarget::Track(track("v1", Some(200.0))), Some(42.0), "Song", "", None).unwrap();
        assert_eq!(m1.kind, BookmarkKind::Moment);
        assert_eq!(s.bookmark_add(&a, BookmarkTarget::Track(track("v1", Some(200.0))), Some(43.5), "Song", "", None).unwrap().id, m1.id);
        s.bookmark_add(&a, BookmarkTarget::Track(track("v1", Some(200.0))), Some(90.0), "Song", "", None).unwrap();
        assert_eq!(s.bookmarks(&a).unwrap().len(), 5);
        assert!(s.bookmarks(&b).unwrap().is_empty());
        let BookmarkTarget::Track(t) = &m1.target else { panic!() };
        assert_eq!(t.liked, None, "likes are the account's");
        assert!(!s.bookmark_remove(&b, &one.id).unwrap(), "another owner can't remove it");
        assert!(s.bookmark_remove(&a, &one.id).unwrap());
        assert_eq!(s.bookmarks(&a).unwrap().len(), 4);
        assert!(s.bookmark_add(&a, page, Some(3.0), "", "", None).is_err());
        assert!(s.bookmark_add(&a, BookmarkTarget::Track(track("v2", None)), Some(f64::NAN), "", "", None).is_err());
    }

    #[test]
    fn saved_queues_keep_order_uids_and_position() {
        let s = Store::open_memory().unwrap();
        let o = Owner::new("a");
        let mut q = Queue::new(7);
        q.replace(vec![track("v1", Some(100.0)), track("v2", Some(50.0)), track("v1", Some(100.0))], 1, Some(QueueSource { label: "Mix".into(), collection_id: None, radio_playlist: None }), false, Origin::Collection).unwrap();
        let info = s.saved_queue_save(&o, "  Evening\n", &q, 31.5).unwrap();
        assert_eq!((info.name.as_str(), info.songs, info.duration), ("Evening", 3, Some(250.0)));
        assert_eq!(info.current.as_ref().unwrap().id, ItemId::ytm("v2"));
        let (back, pos) = s.saved_queue_load(&o, &info.id).unwrap().unwrap();
        assert_eq!(pos, 31.5);
        assert_eq!(back.index(), Some(1));
        let uids = |q: &Queue| q.entries().iter().map(|e| e.uid.clone()).collect::<Vec<_>>();
        assert_eq!(uids(&back), uids(&q));
        // The same name replaces; an unknown length drops the total.
        q.append(vec![track("v3", None)], Origin::Manual);
        let again = s.saved_queue_save(&o, "Evening", &q, 0.0).unwrap();
        assert_eq!(again.id, info.id);
        assert_eq!((again.songs, again.duration), (4, None));
        assert_eq!(s.saved_queues(&o).unwrap().len(), 1);
        assert!(s.saved_queue_load(&Owner::anonymous(), &info.id).unwrap().is_none());
        assert!(s.saved_queue_save(&o, " ", &q, 0.0).is_err());
        assert!(s.saved_queue_save(&o, "Empty", &Queue::default(), 0.0).is_err());
        assert!(s.saved_queue_remove(&o, &info.id).unwrap());
        assert!(s.saved_queues(&o).unwrap().is_empty());
    }

    #[test]
    fn folders_file_each_list_once() {
        let s = Store::open_memory().unwrap();
        let o = Owner::new("a");
        let f1 = s.folder_create(&o, "Work", &[card("VL1"), card("VL2")]).unwrap();
        let f2 = s.folder_create(&o, "Play", &[]).unwrap();
        assert_eq!(f1.items.len(), 2);
        // Filing a list somewhere else moves it.
        s.folder_add(&o, &f2.id, &card("VL1")).unwrap();
        let all = s.folders(&o).unwrap();
        assert_eq!(all.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["Work", "Play"]);
        assert_eq!(all[0].items.iter().map(|i| i.item_id.0.as_str()).collect::<Vec<_>>(), ["ytm:VL2"]);
        assert_eq!(all[1].items.len(), 1);
        s.folder_add(&o, &f2.id, &card("VL3")).unwrap();
        s.folder_add(&o, &f2.id, &card("VL4")).unwrap();
        s.folder_move_item(&o, &f2.id, &ItemId::ytm("VL4"), Some(&ItemId::ytm("VL1"))).unwrap();
        let order = |o: &Owner| s.folder(o, &f2.id).unwrap().unwrap().items.iter().map(|i| i.item_id.0.clone()).collect::<Vec<_>>();
        assert_eq!(order(&o), ["ytm:VL4", "ytm:VL1", "ytm:VL3"]);
        s.folder_remove_item(&o, &f2.id, &ItemId::ytm("VL1")).unwrap();
        assert_eq!(order(&o).len(), 2);
        // Another owner sees nothing and changes nothing.
        let other = Owner::anonymous();
        assert!(s.folders(&other).unwrap().is_empty());
        assert!(s.folder_add(&other, &f2.id, &card("VL9")).is_err());
        assert!(s.folder_rename(&other, &f2.id, "Mine").is_err());
        s.folder_rename(&o, &f2.id, "Weekend").unwrap();
        assert_eq!(s.folder_set_cover(&o, &f2.id, Some("l-1")).unwrap(), None);
        assert_eq!(s.folder_set_cover(&o, &f2.id, Some("l-2")).unwrap().as_deref(), Some("l-1"));
        s.folder_set_collapsed(&o, &f2.id, true).unwrap();
        let f = s.folder(&o, &f2.id).unwrap().unwrap();
        assert_eq!((f.name.as_str(), f.cover.as_deref(), f.collapsed), ("Weekend", Some("l-2"), true));
        assert_eq!(s.folder_remove(&o, &f2.id).unwrap().as_deref(), Some("l-2"));
        let n: i64 = s.conn.lock().query_row("SELECT COUNT(*) FROM folder_items WHERE folder_id = ?1", params![f2.id], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn folder_files_round_trip_and_refuse_strangers() {
        let mut album = card("MPREb_abc");
        album.kind = CollectionKind::Album;
        let f = Folder { id: "f".into(), name: "Road trip".into(), cover: Some("l-x".into()), collapsed: false, items: vec![FolderItem { item_id: card("VLPL1").id, card: card("VLPL1") }, FolderItem { item_id: album.id.clone(), card: album }] };
        let file = FolderFile::of(&f);
        assert_eq!(file.items[0].link, "https://music.youtube.com/playlist?list=PL1");
        assert_eq!(file.items[1].link, "https://music.youtube.com/browse/MPREb_abc");
        let bytes = serde_json::to_vec(&file).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("l-x"), "covers stay on this computer");
        let (name, cards) = FolderFile::parse(&bytes).unwrap();
        assert_eq!(name, "Road trip");
        assert_eq!(cards.iter().map(|c| c.id.0.as_str()).collect::<Vec<_>>(), ["ytm:VLPL1", "ytm:MPREb_abc"]);
        assert_eq!(cards[1].kind, CollectionKind::Album);

        let hostile = serde_json::json!({
            "format": "encorefolder", "version": 1, "name": "\u{0007}Hi",
            "items": [
                { "link": "https://evil.example.com/playlist?list=PL1", "title": "x" },
                { "link": "http://music.youtube.com/playlist?list=PL2", "title": "x" },
                { "link": "https://music.youtube.com/playlist?list=PL3&x=../../", "title": "ok", "art": "https://evil.example.com/a.jpg" },
                { "link": "https://music.youtube.com/playlist?list=../etc", "title": "x" },
                { "link": "https://music.youtube.com/watch?v=abc", "title": "x" },
                { "link": "https://music.youtube.com/playlist?list=PL3", "title": "dup" }
            ]
        });
        let (name, cards) = FolderFile::parse(hostile.to_string().as_bytes()).unwrap();
        assert_eq!(name, "Hi");
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].id.0, "ytm:VLPL3");
        assert!(cards[0].art.is_none(), "covers only from allow-listed hosts");
        assert!(FolderFile::parse(b"{\"format\":\"other\",\"version\":1,\"name\":\"x\",\"items\":[]}").is_err());
        assert!(FolderFile::parse(b"not json").is_err());
        assert!(FolderFile::parse(b"{\"format\":\"encorefolder\",\"version\":9,\"name\":\"x\",\"items\":[]}").is_err());
    }
}
