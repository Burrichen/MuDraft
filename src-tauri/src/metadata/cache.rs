//! Provider response cache. Fresh entries skip the network entirely; expired entries are
//! kept so they can be served when the network is unavailable. Cache failures are logged
//! and treated as a miss: the cache only holds re-fetchable provider data, never user data.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedBody {
    pub body: String,
    pub fetched_at: String,
    pub fresh: bool,
}

pub trait ResponseCache: Send + Sync {
    fn get(&self, key: &str) -> Option<CachedBody>;
    fn put(&self, key: &str, body: &str, fetched_at: &str, ttl: Duration);
}

/// Cache in the app database's `metadata_cache` table, on its own connection so network
/// work never holds the main database lock.
pub struct SqliteCache {
    conn: Mutex<Connection>,
}

impl SqliteCache {
    /// Open an already-migrated database file.
    pub fn open(path: &Path) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(Duration::from_secs(5))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

impl ResponseCache for SqliteCache {
    fn get(&self, key: &str) -> Option<CachedBody> {
        let conn = self.conn.lock().ok()?;
        conn.query_row(
            "SELECT body, fetched_at, expires_at > strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             FROM metadata_cache WHERE key = ?1",
            params![key],
            |r| {
                Ok(CachedBody {
                    body: r.get(0)?,
                    fetched_at: r.get(1)?,
                    fresh: r.get(2)?,
                })
            },
        )
        .optional()
        .unwrap_or_else(|e| {
            eprintln!("MuDraft: metadata cache read failed: {e}");
            None
        })
    }

    fn put(&self, key: &str, body: &str, fetched_at: &str, ttl: Duration) {
        let Ok(conn) = self.conn.lock() else { return };
        let modifier = format!("+{} seconds", ttl.as_secs());
        if let Err(e) = conn.execute(
            "INSERT INTO metadata_cache (key, body, fetched_at, expires_at)
             VALUES (?1, ?2, ?3, strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?4))
             ON CONFLICT (key) DO UPDATE SET body = excluded.body,
                 fetched_at = excluded.fetched_at, expires_at = excluded.expires_at",
            params![key, body, fetched_at, modifier],
        ) {
            eprintln!("MuDraft: metadata cache write failed: {e}");
        }
    }
}

/// Process-local cache: used when storage is unavailable, and in tests.
#[derive(Default)]
pub struct MemoryCache {
    entries: Mutex<HashMap<String, CachedBody>>,
}

impl MemoryCache {
    /// Mark every entry expired (tests simulate time passing).
    pub fn expire_all(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            for entry in entries.values_mut() {
                entry.fresh = false;
            }
        }
    }
}

impl ResponseCache for MemoryCache {
    fn get(&self, key: &str) -> Option<CachedBody> {
        self.entries.lock().ok()?.get(key).cloned()
    }

    fn put(&self, key: &str, body: &str, fetched_at: &str, ttl: Duration) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(
                key.to_owned(),
                CachedBody {
                    body: body.to_owned(),
                    fetched_at: fetched_at.to_owned(),
                    fresh: !ttl.is_zero(),
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    #[test]
    fn sqlite_cache_round_trips_and_expires() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite3");
        drop(Database::open(&path).unwrap());
        let cache = SqliteCache::open(&path).unwrap();
        assert_eq!(cache.get("k"), None);
        cache.put(
            "k",
            "{}",
            "2026-09-30T00:00:00.000Z",
            Duration::from_secs(60),
        );
        let hit = cache.get("k").unwrap();
        assert!(hit.fresh);
        assert_eq!(hit.fetched_at, "2026-09-30T00:00:00.000Z");
        cache.put("k", "[]", "2026-09-30T00:00:01.000Z", Duration::ZERO);
        let stale = cache.get("k").unwrap();
        assert!(!stale.fresh, "expired entries are kept for offline use");
        assert_eq!(stale.body, "[]");
    }
}
