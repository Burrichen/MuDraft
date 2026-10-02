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
    /// Release any open database handle (a profile restore is replacing the file).
    fn suspend(&self) {}
    /// Reopen after [`ResponseCache::suspend`].
    fn resume(&self) {}
}

/// Cache in the app database's `metadata_cache` table, on its own connection so network
/// work never holds the main database lock.
pub struct SqliteCache {
    path: std::path::PathBuf,
    /// `None` while suspended; reads miss and writes are skipped meanwhile.
    conn: Mutex<Option<Connection>>,
}

fn open_cache(path: &Path) -> Result<Connection, rusqlite::Error> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(Duration::from_secs(5))?;
    Ok(conn)
}

impl SqliteCache {
    /// Open an already-migrated database file.
    pub fn open(path: &Path) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            path: path.to_path_buf(),
            conn: Mutex::new(Some(open_cache(path)?)),
        })
    }
}

impl ResponseCache for SqliteCache {
    fn get(&self, key: &str) -> Option<CachedBody> {
        let guard = self.conn.lock().ok()?;
        let conn = guard.as_ref()?;
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
        let Ok(guard) = self.conn.lock() else { return };
        let Some(conn) = guard.as_ref() else { return };
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

    fn suspend(&self) {
        if let Ok(mut guard) = self.conn.lock() {
            guard.take();
        }
    }

    fn resume(&self) {
        if let Ok(mut guard) = self.conn.lock()
            && guard.is_none()
        {
            match open_cache(&self.path) {
                Ok(c) => *guard = Some(c),
                Err(e) => eprintln!("MuDraft: metadata cache unavailable: {e}"),
            }
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

/// Size of the stored MusicBrainz response cache (the app database's `metadata_cache`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheStats {
    pub entries: u32,
    pub bytes: i64,
    pub expired: u32,
}

pub fn stats(conn: &Connection) -> crate::error::AppResult<CacheStats> {
    Ok(conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(length(CAST(body AS BLOB))), 0),
                COUNT(*) FILTER (WHERE expires_at < strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         FROM metadata_cache",
        [],
        |r| {
            Ok(CacheStats {
                entries: r.get(0)?,
                bytes: r.get(1)?,
                expired: r.get(2)?,
            })
        },
    )?)
}

/// Forget cached provider responses. Library data, ratings, and listens are untouched;
/// only offline re-lookups of metadata lose their saved copy.
pub fn clear(tx: &rusqlite::Transaction<'_>) -> crate::error::AppResult<u32> {
    Ok(tx.execute("DELETE FROM metadata_cache", [])? as u32)
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

    #[test]
    fn stats_and_clear_touch_only_cached_responses() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.sqlite3");
        let mut db = Database::open(&path).unwrap();
        let cache = SqliteCache::open(&path).unwrap();
        cache.put(
            "https://example.org/a",
            "{}",
            "2026-10-01T00:00:00.000Z",
            Duration::from_secs(60),
        );
        cache.put(
            "https://example.org/b",
            "[1]",
            "2026-10-01T00:00:00.000Z",
            Duration::from_secs(60),
        );
        db.write(|tx| {
            tx.execute("INSERT INTO artist (id, name) VALUES ('keep', 'Kept')", [])?;
            Ok(())
        })
        .unwrap();
        let s = db.read(stats).unwrap();
        assert_eq!((s.entries, s.bytes), (2, 5));
        assert_eq!(db.write(clear).unwrap(), 2);
        assert_eq!(db.read(stats).unwrap().entries, 0);
        let kept: i64 = db
            .read(|c| Ok(c.query_row("SELECT COUNT(*) FROM artist", [], |r| r.get(0))?))
            .unwrap();
        assert_eq!(kept, 1);
    }
}
