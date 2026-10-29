//! SQLite storage (D11): browse/library cache, play state, settings, lyrics
//! cache and the artwork index. Rows are scoped (`account:<key>`, `lyrics`,
//! `settings`, `art`, ...) so a sign-out or channel switch can purge exactly the
//! personal data of that account. Nothing secret is stored here.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{AppError, Result};

pub struct Store {
    conn: Mutex<Connection>,
}

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn db_err(e: rusqlite::Error) -> AppError {
    AppError::internal(format!("storage error: {e}"))
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

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).map_err(|e| AppError::internal(format!("cannot create data dir: {}", e.kind())))?;
        }
        let conn = Connection::open(path).map_err(db_err)?;
        conn.execute_batch(SCHEMA).map_err(db_err)?;
        conn.execute("INSERT OR IGNORE INTO meta (k, v) VALUES ('schema', '1')", []).map_err(db_err)?;
        let s = Self { conn: Mutex::new(conn) };
        s.purge_expired();
        Ok(s)
    }

    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(db_err)?;
        conn.execute_batch(SCHEMA).map_err(db_err)?;
        Ok(Self { conn: Mutex::new(conn) })
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
