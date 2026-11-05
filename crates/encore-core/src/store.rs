//! SQLite storage (D11): browse/library cache, play state, settings, lyrics
//! cache and the artwork index. Rows are scoped (`account:<key>`, `lyrics`,
//! `settings`, `art`, ...) so a sign-out or channel switch can purge exactly the
//! personal data of that account. Nothing secret is stored here. The
//! downloads index (D23) has its own tables, in [`crate::downloads`], and the
//! user's own data (bookmarks, folders, the play log, ...) has the durable
//! tables of [`crate::local`], which no purge touches.
//!
//! Because the file now holds things that can't be fetched again, it is
//! backed up whole: `VACUUM INTO` a temporary file, checked, then renamed
//! into `backups/` beside it, once a day and before every migration, keeping
//! the newest three. A database that turns out damaged when Encore starts is
//! replaced by the newest backup that opens cleanly; only when none does is
//! it set aside and a fresh one started, as before.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{AppError, Result};

pub struct Store {
    pub(crate) conn: Mutex<Connection>,
    /// The database file; `None` in memory.
    path: Option<PathBuf>,
    recovery: Option<Recovery>,
}

/// What `Store::open` had to do because the database was damaged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recovery {
    /// Replaced by the backup made at `made` (unix seconds): changes since
    /// then are gone.
    Restored { made: i64 },
    /// No backup opened cleanly: a fresh, empty database.
    Fresh,
}

/// A whole-database backup in `backups/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Backup {
    pub path: PathBuf,
    /// Unix seconds.
    pub made: i64,
    /// `daily`, or `schema<N>` for the copy taken before migrating from N.
    pub why: String,
}

/// How many backups are kept.
pub const BACKUPS_KEPT: usize = 3;
/// A new daily backup once the newest is this old.
pub const BACKUP_EVERY: i64 = 24 * 3600;

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub(crate) fn db_err(e: rusqlite::Error) -> AppError {
    AppError::internal(format!("storage error: {e}"))
}

/// The file isn't a usable SQLite database (as opposed to busy or unwritable).
fn is_corrupt(e: &rusqlite::Error) -> bool {
    matches!(e.sqlite_error_code(), Some(rusqlite::ErrorCode::NotADatabase | rusqlite::ErrorCode::DatabaseCorrupt))
}

fn corrupt(what: &str) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CORRUPT), Some(what.to_string()))
}

const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS kv (
    scope   TEXT NOT NULL,
    key     TEXT NOT NULL,
    value   TEXT NOT NULL,
    updated INTEGER NOT NULL,
    expires INTEGER,
    PRIMARY KEY (scope, key)
);
CREATE INDEX IF NOT EXISTS kv_expires ON kv(expires);
CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v TEXT NOT NULL);
"#;

/// The schema this Encore writes. Each step in `migrate` brings an older
/// database up by one version.
const SCHEMA_VERSION: i64 = 3;

fn schema_version(conn: &Connection) -> rusqlite::Result<i64> {
    let v: Option<String> = conn.query_row("SELECT v FROM meta WHERE k = 'schema'", [], |r| r.get(0)).optional()?;
    Ok(v.and_then(|v| v.parse().ok()).unwrap_or(1))
}

/// Anything worth keeping: a schema mark or a row in the cache. A database
/// just created has neither, and needs no backup before its first migration.
fn has_data(conn: &Connection) -> rusqlite::Result<bool> {
    conn.query_row("SELECT EXISTS(SELECT 1 FROM meta WHERE k = 'schema') OR EXISTS(SELECT 1 FROM kv)", [], |r| r.get(0))
}

/// All steps in one transaction: a migration stopped halfway leaves the old
/// schema, not half of the new one.
fn migrate(conn: &mut Connection) -> rusqlite::Result<()> {
    let v = schema_version(conn)?;
    let tx = conn.transaction()?;
    if v < 2 {
        // 2: the downloads index (D23).
        tx.execute_batch(crate::downloads::SCHEMA)?;
    }
    if v < 3 {
        // 3: the local data of the feature wave (bookmarks, folders, the
        // play log, ...): every table at once, see `crate::local`.
        tx.execute_batch(crate::local::SCHEMA)?;
    }
    tx.execute("INSERT INTO meta (k, v) VALUES ('schema', ?1) ON CONFLICT(k) DO UPDATE SET v = excluded.v", params![SCHEMA_VERSION.max(v).to_string()])?;
    tx.commit()
}

/// SQLite's own check of the pages and indexes (quicker than a full
/// `integrity_check`): damage it finds counts as a damaged database.
fn check(conn: &Connection) -> rusqlite::Result<()> {
    let r: String = conn.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
    if r == "ok" { Ok(()) } else { Err(corrupt(&r)) }
}

fn backup_dir(db: &Path) -> PathBuf {
    db.parent().unwrap_or(Path::new(".")).join("backups")
}

#[cfg(unix)]
fn private(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}
#[cfg(not(unix))]
fn private(_: &Path, _: u32) {}

/// Backups in `dir`, newest first.
pub fn list_backups(dir: &Path) -> Vec<Backup> {
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let mut v: Vec<Backup> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let rest = name.strip_prefix("encore-")?.strip_suffix(".sqlite")?;
            let (made, why) = rest.split_once('-')?;
            Some(Backup { path: e.path(), made: made.parse().ok()?, why: why.to_string() })
        })
        .collect();
    v.sort_by(|a, b| b.made.cmp(&a.made).then_with(|| b.why.cmp(&a.why)));
    v
}

/// Keep the newest `BACKUPS_KEPT`, and drop copies that never finished.
fn prune(dir: &Path) {
    for b in list_backups(dir).into_iter().skip(BACKUPS_KEPT) {
        let _ = std::fs::remove_file(&b.path);
    }
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|d| d.as_secs() > 3600);
            if name.starts_with(".encore-") && name.ends_with(".part") && old {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

/// Copy the whole database into `dir` and check the copy before it counts.
fn backup_to(conn: &Connection, dir: &Path, why: &str) -> Result<PathBuf> {
    let io = |what: &str, e: std::io::Error| AppError::internal(format!("cannot {what}: {}", e.kind()));
    std::fs::create_dir_all(dir).map_err(|e| io("create the backups folder", e))?;
    private(dir, 0o700);
    let name = format!("encore-{}-{why}.sqlite", now());
    let part = dir.join(format!(".{name}.part"));
    let _ = std::fs::remove_file(&part);
    let target = part.to_str().ok_or_else(|| AppError::internal("the backups folder has a name SQLite can't use"))?;
    let made = conn.execute("VACUUM INTO ?1", params![target]).map_err(db_err).and_then(|_| {
        let c = Connection::open_with_flags(&part, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(db_err)?;
        check(&c).map_err(db_err)
    });
    if let Err(e) = made {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    private(&part, 0o600);
    let path = dir.join(&name);
    std::fs::rename(&part, &path).map_err(|e| io("finish the backup", e))?;
    prune(dir);
    Ok(path)
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).map_err(|e| AppError::internal(format!("cannot create data dir: {}", e.kind())))?;
        }
        match Self::open_at(path) {
            Ok(s) => Ok(s),
            // A damaged database (power loss, a bad disk, a stray file) would
            // stop Encore from starting every time. Set it aside and put the
            // newest backup that opens cleanly in its place; with none, start
            // a fresh one.
            Err(e) if is_corrupt(&e) => {
                let aside = path.with_extension(format!("sqlite.damaged-{}", now()));
                tracing::warn!(error = %e, aside = %aside.display(), "storage was damaged; setting it aside");
                std::fs::rename(path, &aside).map_err(|e| AppError::internal(format!("cannot move the damaged database aside: {}", e.kind())))?;
                let drop_journal = || {
                    for ext in ["sqlite-wal", "sqlite-shm"] {
                        let _ = std::fs::remove_file(path.with_extension(ext));
                    }
                };
                drop_journal();
                for b in list_backups(&backup_dir(path)) {
                    match Self::restore_from(&b.path, path) {
                        Ok(mut s) => {
                            tracing::warn!(made = b.made, "storage restored from a backup");
                            s.recovery = Some(Recovery::Restored { made: b.made });
                            return Ok(s);
                        }
                        Err(e) => {
                            tracing::warn!(backup = %b.path.display(), error = %e, "a backup didn't open; trying an older one");
                            let _ = std::fs::remove_file(path);
                            drop_journal();
                        }
                    }
                }
                tracing::warn!("no usable backup; starting a fresh database");
                let mut s = Self::open_at(path).map_err(db_err)?;
                s.recovery = Some(Recovery::Fresh);
                Ok(s)
            }
            Err(e) => Err(db_err(e)),
        }
    }

    /// Copy a backup into place (by temporary file and rename) and open it.
    fn restore_from(backup: &Path, path: &Path) -> rusqlite::Result<Self> {
        let tmp = path.with_extension("sqlite.restoring");
        std::fs::copy(backup, &tmp).and_then(|_| std::fs::rename(&tmp, path)).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CANTOPEN), Some(format!("cannot copy the backup: {}", e.kind())))
        })?;
        private(path, 0o600);
        Self::open_at(path)
    }

    fn open_at(path: &Path) -> rusqlite::Result<Self> {
        let mut conn = Connection::open(path)?;
        // Another Encore (or a slow disk) holding a write lock is waited for,
        // not reported as "database is locked".
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(SCHEMA)?;
        check(&conn)?;
        if schema_version(&conn)? < SCHEMA_VERSION && has_data(&conn)? {
            // The old schema, as it was, in case the new one goes wrong.
            let why = format!("schema{}", schema_version(&conn)?);
            if let Err(e) = backup_to(&conn, &backup_dir(path), &why) {
                tracing::warn!(error = %e.message, "couldn't back up before migrating; migrating anyway");
            }
        }
        migrate(&mut conn)?;
        let s = Self { conn: Mutex::new(conn), path: Some(path.to_path_buf()), recovery: None };
        s.purge_expired();
        Ok(s)
    }

    pub fn open_memory() -> Result<Self> {
        let mut conn = Connection::open_in_memory().map_err(db_err)?;
        conn.execute_batch(SCHEMA).map_err(db_err)?;
        migrate(&mut conn).map_err(db_err)?;
        Ok(Self { conn: Mutex::new(conn), path: None, recovery: None })
    }

    /// Set when this start had to replace a damaged database.
    pub fn recovery(&self) -> Option<&Recovery> {
        self.recovery.as_ref()
    }

    /// Back up the whole database now (`why`: `daily`, ...). Holds the
    /// connection while it copies, a moment for a database of this size.
    pub fn backup(&self, why: &str) -> Result<PathBuf> {
        let path = self.path.as_ref().ok_or_else(|| AppError::internal("an in-memory database has no backups"))?;
        let c = self.conn.lock();
        backup_to(&c, &backup_dir(path), why)
    }

    /// The daily backup, if the newest one is a day old (or there is none).
    pub fn backup_if_due(&self) -> Result<Option<PathBuf>> {
        let Some(path) = &self.path else { return Ok(None) };
        let newest = list_backups(&backup_dir(path)).first().map(|b| b.made);
        if newest.is_some_and(|m| now() - m < BACKUP_EVERY) {
            return Ok(None);
        }
        self.backup("daily").map(Some)
    }

    /// This database's backups, newest first.
    pub fn backups(&self) -> Vec<Backup> {
        self.path.as_ref().map(|p| list_backups(&backup_dir(p))).unwrap_or_default()
    }

    pub fn put_raw(&self, scope: &str, key: &str, value: &str, ttl_secs: Option<i64>) -> Result<()> {
        let n = now();
        self.conn
            .lock()
            .execute(
                "INSERT INTO kv (scope, key, value, updated, expires) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(scope, key) DO UPDATE SET value = excluded.value, updated = excluded.updated, expires = excluded.expires",
                params![scope, key, value, n, ttl_secs.map(|t| n + t)],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// Returns `(value, age_secs)` if present and not expired.
    pub fn get_raw(&self, scope: &str, key: &str) -> Result<Option<(String, i64)>> {
        let n = now();
        self.conn
            .lock()
            .query_row(
                "SELECT value, updated FROM kv WHERE scope = ?1 AND key = ?2 AND (expires IS NULL OR expires > ?3)",
                params![scope, key, n],
                |r| Ok((r.get::<_, String>(0)?, n - r.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(db_err)
    }

    /// Like `get`, but gives up (`None`) rather than wait longer than
    /// `wait` for the connection, which a backup can hold for a while.
    pub fn get_quick<T: DeserializeOwned>(&self, scope: &str, key: &str, wait: std::time::Duration) -> Option<T> {
        let c = self.conn.try_lock_for(wait)?;
        let v: Option<String> = c
            .query_row("SELECT value FROM kv WHERE scope = ?1 AND key = ?2 AND (expires IS NULL OR expires > ?3)", params![scope, key, now()], |r| r.get(0))
            .optional()
            .ok()
            .flatten();
        v.and_then(|v| serde_json::from_str(&v).ok())
    }

    pub fn put<T: Serialize>(&self, scope: &str, key: &str, value: &T, ttl_secs: Option<i64>) -> Result<()> {
        let s = serde_json::to_string(value).map_err(|e| AppError::internal(e.to_string()))?;
        self.put_raw(scope, key, &s, ttl_secs)
    }

    pub fn get<T: DeserializeOwned>(&self, scope: &str, key: &str) -> Result<Option<T>> {
        Ok(self.get_raw(scope, key)?.and_then(|(v, _)| serde_json::from_str(&v).ok()))
    }

    /// Like `get`, but also returns stale-but-present rows when `allow_stale`.
    pub fn get_any<T: DeserializeOwned>(&self, scope: &str, key: &str) -> Result<Option<(T, i64, bool)>> {
        let n = now();
        let row = self
            .conn
            .lock()
            .query_row("SELECT value, updated, expires FROM kv WHERE scope = ?1 AND key = ?2", params![scope, key], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, Option<i64>>(2)?))
            })
            .optional()
            .map_err(db_err)?;
        Ok(row.and_then(|(v, upd, exp)| serde_json::from_str(&v).ok().map(|t| (t, n - upd, exp.is_some_and(|e| e <= n)))))
    }

    pub fn delete(&self, scope: &str, key: &str) -> Result<()> {
        self.conn.lock().execute("DELETE FROM kv WHERE scope = ?1 AND key = ?2", params![scope, key]).map_err(db_err)?;
        Ok(())
    }

    /// Remove every row of a scope (sign-out / channel switch purge).
    pub fn purge_scope(&self, scope: &str) -> Result<usize> {
        self.conn.lock().execute("DELETE FROM kv WHERE scope = ?1", params![scope]).map_err(db_err)
    }

    pub fn list(&self, scope: &str) -> Result<Vec<(String, String)>> {
        let c = self.conn.lock();
        let mut st = c.prepare("SELECT key, value FROM kv WHERE scope = ?1").map_err(db_err)?;
        let rows = st.query_map(params![scope], |r| Ok((r.get(0)?, r.get(1)?))).map_err(db_err)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Adds rows that aren't there yet, in one transaction; rows already
    /// there (and their expiry) are left as they are.
    pub fn put_absent<T: Serialize>(&self, scope: &str, rows: &[(String, T)], ttl_secs: Option<i64>) -> Result<()> {
        let n = now();
        let mut c = self.conn.lock();
        let tx = c.transaction().map_err(db_err)?;
        {
            let mut st = tx.prepare("INSERT OR IGNORE INTO kv (scope, key, value, updated, expires) VALUES (?1, ?2, ?3, ?4, ?5)").map_err(db_err)?;
            for (k, v) in rows {
                let v = serde_json::to_string(v).map_err(|e| AppError::internal(e.to_string()))?;
                st.execute(params![scope, k, v, n, ttl_secs.map(|t| n + t)]).map_err(db_err)?;
            }
        }
        tx.commit().map_err(db_err)
    }

    pub fn purge_expired(&self) {
        let _ = self.conn.lock().execute("DELETE FROM kv WHERE expires IS NOT NULL AND expires <= ?1", params![now()]);
    }

    pub fn count(&self, scope: &str) -> usize {
        self.conn.lock().query_row("SELECT COUNT(*) FROM kv WHERE scope = ?1", params![scope], |r| r.get::<_, i64>(0)).unwrap_or(0) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damaged_database_is_set_aside() {
        let dir = std::env::temp_dir().join(format!("encore-store-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("encore.sqlite");
        std::fs::write(&db, vec![0x5au8; 8192]).unwrap();
        let s = Store::open(&db).expect("a damaged database must not stop startup");
        s.put("settings", "k", &1, None).unwrap();
        assert_eq!(s.get::<i32>("settings", "k").unwrap(), Some(1));
        let aside = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).any(|e| e.file_name().to_string_lossy().contains("damaged"));
        assert!(aside, "the damaged file is kept for inspection");
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_databases_are_migrated() {
        let dir = std::env::temp_dir().join(format!("encore-store-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("encore.sqlite");
        {
            // A version 1 database, as Encore 0.3 left it.
            let c = Connection::open(&db).unwrap();
            c.execute_batch(SCHEMA).unwrap();
            c.execute("INSERT OR IGNORE INTO meta (k, v) VALUES ('schema', '1')", []).unwrap();
        }
        let s = Store::open(&db).unwrap();
        assert_eq!(schema_version(&s.conn.lock()).unwrap(), SCHEMA_VERSION);
        assert!(s.download_rows().unwrap().is_empty());
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("encore-store-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tables(s: &Store) -> Vec<String> {
        let c = s.conn.lock();
        let mut st = c.prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name").unwrap();
        st.query_map([], |r| r.get(0)).unwrap().filter_map(|r| r.ok()).collect()
    }

    /// A version 2 database, as Encore 0.4 left it, with one setting.
    fn schema_2(db: &Path) {
        let c = Connection::open(db).unwrap();
        c.execute_batch(SCHEMA).unwrap();
        c.execute_batch(crate::downloads::SCHEMA).unwrap();
        c.execute("INSERT INTO meta (k, v) VALUES ('schema', '2')", []).unwrap();
        c.execute("INSERT INTO kv (scope, key, value, updated) VALUES ('settings', 'k', '1', 0)", []).unwrap();
    }

    #[test]
    fn schema_2_gains_every_local_table_and_keeps_a_backup() {
        let dir = temp_dir();
        let db = dir.join("encore.sqlite");
        schema_2(&db);
        let s = Store::open(&db).unwrap();
        assert_eq!(schema_version(&s.conn.lock()).unwrap(), 3);
        let t = tables(&s);
        for name in crate::local::DURABLE.iter().chain(crate::local::CACHES) {
            assert!(t.iter().any(|x| x == name), "{name} missing from {t:?}");
        }
        assert_eq!(s.get::<i32>("settings", "k").unwrap(), Some(1), "old data stays");
        let b = s.backups();
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].why, "schema2");
        let old = Connection::open_with_flags(&b[0].path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        assert_eq!(schema_version(&old).unwrap(), 2, "the backup is the database before the migration");
        drop(s);
        // Opening again migrates nothing and backs up nothing.
        let s = Store::open(&db).unwrap();
        assert_eq!(s.backups().len(), 1);
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_database_needs_no_backup_before_its_first_migration() {
        let dir = temp_dir();
        let s = Store::open(&dir.join("encore.sqlite")).unwrap();
        assert!(s.backups().is_empty());
        assert!(s.recovery().is_none());
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn backups_rotate_and_are_daily() {
        let dir = temp_dir();
        let db = dir.join("encore.sqlite");
        let s = Store::open(&db).unwrap();
        assert!(s.backup_if_due().unwrap().is_some(), "none yet: one now");
        assert!(s.backup_if_due().unwrap().is_none(), "not again the same day");
        // Older ones, as if from earlier days.
        let bdir = dir.join("backups");
        for (i, made) in [100, 200, 300, 400].iter().enumerate() {
            std::fs::copy(&s.backups()[0].path, bdir.join(format!("encore-{made}-daily{i}.sqlite"))).unwrap();
        }
        // (Two backups in the same second for the same reason are one file.)
        s.backup("test1").unwrap();
        s.backup("test2").unwrap();
        let kept: Vec<i64> = s.backups().iter().map(|b| b.made).collect();
        assert_eq!(kept.len(), BACKUPS_KEPT);
        assert!(kept.iter().all(|m| *m > 400), "the newest three stay: {kept:?}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&s.backups()[0].path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn damage(db: &Path) {
        std::fs::write(db, vec![0x5au8; 8192]).unwrap();
        for ext in ["sqlite-wal", "sqlite-shm"] {
            let _ = std::fs::remove_file(db.with_extension(ext));
        }
    }

    #[test]
    fn a_damaged_database_comes_back_from_the_newest_good_backup() {
        let dir = temp_dir();
        let db = dir.join("encore.sqlite");
        let s = Store::open(&db).unwrap();
        s.put("settings", "k", &1, None).unwrap();
        s.conn.lock().execute("INSERT INTO bookmarks (id, owner, kind, target, title, created) VALUES ('b1', 'anonymous', 'page', '{}', 'Kept', 0)", []).unwrap();
        let good = s.backup("daily").unwrap();
        s.put("settings", "k", &2, None).unwrap();
        drop(s);
        // A newer backup that is itself damaged is passed over.
        let made = list_backups(&dir.join("backups"))[0].made + 10;
        std::fs::write(dir.join("backups").join(format!("encore-{made}-daily.sqlite")), vec![0x5au8; 8192]).unwrap();
        damage(&db);
        let s = Store::open(&db).unwrap();
        let from = list_backups(&dir.join("backups")).into_iter().find(|b| b.path == good).unwrap().made;
        assert_eq!(s.recovery(), Some(&Recovery::Restored { made: from }));
        assert_eq!(s.get::<i32>("settings", "k").unwrap(), Some(1), "as it was at the backup");
        let n: i64 = s.conn.lock().query_row("SELECT COUNT(*) FROM bookmarks", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1, "durable data is back");
        assert!(std::fs::read_dir(&dir).unwrap().flatten().any(|e| e.file_name().to_string_lossy().contains("damaged")), "the damaged file is kept");
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_a_good_backup_a_damaged_database_starts_fresh() {
        let dir = temp_dir();
        let db = dir.join("encore.sqlite");
        std::fs::create_dir_all(dir.join("backups")).unwrap();
        std::fs::write(dir.join("backups").join("encore-100-daily.sqlite"), vec![0x5au8; 8192]).unwrap();
        damage(&db);
        let s = Store::open(&db).unwrap();
        assert_eq!(s.recovery(), Some(&Recovery::Fresh));
        s.put("settings", "k", &1, None).unwrap();
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn purges_never_touch_durable_tables() {
        let s = Store::open_memory().unwrap();
        s.put("account:a", "home", &1, None).unwrap();
        s.conn.lock().execute("INSERT INTO blocked (owner, kind, target, label, created) VALUES ('a', 'track', 'ytm:x', 'X', 0)", []).unwrap();
        s.purge_scope("account:a").unwrap();
        let n: i64 = s.conn.lock().query_row("SELECT COUNT(*) FROM blocked", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
    }

    /// Migrate a copy of a real database: `ENCORE_TEST_DB=/path/to/copy.sqlite
    /// cargo test -p encore-core real_database -- --ignored`. The file given
    /// is changed (migrated, backed up beside it): pass a copy.
    #[test]
    #[ignore]
    fn real_database_migrates() {
        let db = PathBuf::from(std::env::var("ENCORE_TEST_DB").expect("ENCORE_TEST_DB"));
        let before = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY).and_then(|c| schema_version(&c)).unwrap();
        let s = Store::open(&db).unwrap();
        assert_eq!(schema_version(&s.conn.lock()).unwrap(), SCHEMA_VERSION);
        assert!(s.recovery().is_none());
        let t = tables(&s);
        for name in crate::local::DURABLE.iter().chain(crate::local::CACHES) {
            assert!(t.iter().any(|x| x == name), "{name} missing");
        }
        let b = s.backups();
        assert!(b.iter().any(|b| b.why == format!("schema{before}")), "{b:?}");
        let kv: i64 = s.conn.lock().query_row("SELECT COUNT(*) FROM kv", [], |r| r.get(0)).unwrap();
        let downloads = s.download_count().unwrap();
        println!("migrated {} from schema {before}; {kv} kv rows, {downloads} downloads; backup {}", db.display(), b[0].path.display());
    }

    /// Then damage that copy and open it again: it comes back from the
    /// newest backup beside it (same variable, same warning: a copy).
    #[test]
    #[ignore]
    fn real_database_restores() {
        let db = PathBuf::from(std::env::var("ENCORE_TEST_DB").expect("ENCORE_TEST_DB"));
        let kv = |s: &Store| -> i64 { s.conn.lock().query_row("SELECT COUNT(*) FROM kv", [], |r| r.get(0)).unwrap() };
        let before = kv(&Store::open(&db).unwrap());
        let newest = list_backups(&backup_dir(&db)).first().cloned().expect("run real_database_migrates first");
        damage(&db);
        let s = Store::open(&db).unwrap();
        assert_eq!(s.recovery(), Some(&Recovery::Restored { made: newest.made }));
        assert_eq!(schema_version(&s.conn.lock()).unwrap(), SCHEMA_VERSION);
        let after = kv(&s);
        println!("restored {} from {} ({before} kv rows before, {after} after)", db.display(), newest.path.display());
        assert!(after > 0);
    }

    #[test]
    fn put_get_ttl_and_purge() {
        let s = Store::open_memory().unwrap();
        s.put("account:a", "home", &vec![1, 2, 3], Some(60)).unwrap();
        s.put("account:b", "home", &vec![9], Some(60)).unwrap();
        assert_eq!(s.get::<Vec<i32>>("account:a", "home").unwrap(), Some(vec![1, 2, 3]));
        s.put("account:a", "old", &1, Some(-1)).unwrap();
        assert_eq!(s.get::<i32>("account:a", "old").unwrap(), None);
        let (v, _, stale) = s.get_any::<i32>("account:a", "old").unwrap().unwrap();
        assert_eq!(v, 1);
        assert!(stale);
        s.purge_scope("account:a").unwrap();
        assert_eq!(s.get::<Vec<i32>>("account:a", "home").unwrap(), None);
        // other accounts untouched
        assert_eq!(s.get::<Vec<i32>>("account:b", "home").unwrap(), Some(vec![9]));
    }
}
