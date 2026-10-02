pub mod backup;

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};

use crate::error::{AppError, AppResult};

/// Ordered migrations; index + 1 is the resulting `user_version`. Never edit a shipped entry.
const MIGRATIONS: &[&str] = &[
    include_str!("migrations/0001_foundation.sql"),
    include_str!("migrations/0002_library.sql"),
    include_str!("migrations/0003_metadata.sql"),
    include_str!("migrations/0004_csv_import.sql"),
    include_str!("migrations/0005_tags.sql"),
    include_str!("migrations/0006_album_detail.sql"),
    include_str!("migrations/0007_listening.sql"),
    include_str!("migrations/0008_next_up.sql"),
    include_str!("migrations/0009_discography.sql"),
    include_str!("migrations/0010_catalogue_credits.sql"),
];

pub const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

pub struct Database {
    conn: Connection,
    backup_dir: PathBuf,
}

impl Database {
    /// Open (creating if absent) and migrate. An unreadable, damaged, or newer-schema
    /// file is reported as an error and left exactly as found — never reset.
    /// An existing database is backed up before any migration touches it.
    pub fn open(path: &Path) -> AppResult<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        // Reading user_version first surfaces NotADatabase before any write happens.
        let found = user_version(&conn)?;
        if found > SCHEMA_VERSION {
            return Err(AppError::SchemaTooNew {
                found,
                supported: SCHEMA_VERSION,
            });
        }
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        let backup_dir = backup::backup_dir_for(path)?;
        let mut db = Self { conn, backup_dir };
        db.migrate(found)?;
        Ok(db)
    }

    fn migrate(&mut self, from: i64) -> AppResult<()> {
        if from == SCHEMA_VERSION {
            return Ok(());
        }
        if from > 0 {
            // Online backup API: a consistent copy even with WAL, never a raw file copy.
            backup::create(
                &self.conn,
                &self.backup_dir,
                &format!("pre-v{SCHEMA_VERSION}"),
            )?;
        }
        // SQLite's table-rebuild procedure: foreign keys off (only possible outside a
        // transaction) so dropping a rebuilt parent cannot cascade, then every reference is
        // verified before commit and enforcement is restored whatever the outcome.
        self.conn.pragma_update(None, "foreign_keys", false)?;
        let result = self.apply_migrations(from);
        self.conn.pragma_update(None, "foreign_keys", true)?;
        result
    }

    fn apply_migrations(&mut self, from: i64) -> AppResult<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(from as usize) {
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", i as i64 + 1)?;
        }
        check_foreign_keys(&tx)?;
        tx.commit()?;
        Ok(())
    }

    /// Read-only access for queries.
    pub fn read<T>(&self, f: impl FnOnce(&Connection) -> AppResult<T>) -> AppResult<T> {
        f(&self.conn)
    }

    /// Run `f` in one IMMEDIATE transaction: all of its writes commit together, or none do.
    pub fn write<T>(&mut self, f: impl FnOnce(&Transaction<'_>) -> AppResult<T>) -> AppResult<T> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let value = f(&tx)?; // dropping `tx` on error rolls back
        tx.commit()?;
        Ok(value)
    }

    pub fn schema_version(&self) -> AppResult<i64> {
        user_version(&self.conn)
    }

    /// Fast structural check; anything other than "ok" is reported, not repaired.
    pub fn quick_check(&self) -> AppResult<()> {
        quick_check(&self.conn)
    }

    pub fn backup_dir(&self) -> &Path {
        &self.backup_dir
    }
}

fn user_version(conn: &Connection) -> AppResult<i64> {
    Ok(conn.pragma_query_value(None, "user_version", |row| row.get(0))?)
}

fn quick_check(conn: &Connection) -> AppResult<()> {
    let result: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if result == "ok" {
        Ok(())
    } else {
        Err(AppError::StorageCorrupt(result))
    }
}

fn check_foreign_keys(conn: &Connection) -> AppResult<()> {
    let mut stmt = conn.prepare("PRAGMA foreign_key_check")?;
    let mut rows = stmt.query([])?;
    if let Some(row) = rows.next()? {
        let table: String = row.get(0)?;
        return Err(AppError::StorageCorrupt(format!(
            "foreign key violation in {table}"
        )));
    }
    Ok(())
}

pub fn sqlite_version() -> &'static str {
    rusqlite::version()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v8_rebuild_keeps_next_up_history_and_foreign_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v7.sqlite3");
        {
            let conn = Connection::open(&path).unwrap();
            for (i, sql) in MIGRATIONS.iter().take(7).enumerate() {
                conn.execute_batch(sql).unwrap();
                conn.pragma_update(None, "user_version", i as i64 + 1)
                    .unwrap();
            }
            conn.execute_batch(
                "INSERT INTO album (id, title) VALUES ('al', 'A');
                 INSERT INTO edition (id, album_id, name) VALUES ('ed', 'al', 'Standard');
                 INSERT INTO selection_session (id, mode) VALUES ('s1', 'completely_random');
                 INSERT INTO selection_attempt (id, session_id, sequence, pool_cycle, album_id, edition_id, source)
                     VALUES ('at', 's1', 1, 2, 'al', 'ed', 'completely_random');
                 INSERT INTO current_selection (slot, attempt_id) VALUES (1, 'at');
                 INSERT INTO listen_event (id, edition_id, album_id, date_status, is_full, selection_attempt_id)
                     VALUES ('le', 'ed', 'al', 'undated', 1, 'at');",
            )
            .unwrap();
        }
        let db = Database::open(&path).unwrap();
        let c = &db.conn;
        let (cycle, ended): (i64, Option<String>) = c
            .query_row(
                "SELECT pool_cycle, ended_at FROM selection_session WHERE id = 's1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(cycle, 2);
        assert!(ended.is_some(), "pre-v8 sessions are closed");
        let kept: i64 = c
            .query_row(
                "SELECT (SELECT COUNT(*) FROM selection_attempt) + (SELECT COUNT(*) FROM current_selection)
                      + (SELECT COUNT(*) FROM listen_event WHERE selection_attempt_id = 'at')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kept, 3);
        // References still point at the rebuilt table and are enforced again.
        assert!(
            c.execute(
                "INSERT INTO selection_attempt (id, session_id, sequence, album_id, edition_id, source)
                 VALUES ('x', 'missing', 1, 'al', 'ed', 'manual')",
                [],
            )
            .is_err()
        );
        c.execute("DELETE FROM selection_session WHERE id = 's1'", [])
            .unwrap();
        let attempts: i64 = c
            .query_row("SELECT COUNT(*) FROM selection_attempt", [], |r| r.get(0))
            .unwrap();
        assert_eq!(attempts, 0, "cascade from the rebuilt parent still works");
    }

    #[test]
    fn creates_and_migrates_new_database() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("t.sqlite3")).unwrap();
        assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);
        db.quick_check().unwrap();
        let fk: bool = db
            .conn
            .pragma_query_value(None, "foreign_keys", |r| r.get(0))
            .unwrap();
        assert!(fk);
    }

    #[test]
    fn reopening_preserves_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite3");
        {
            let db = Database::open(&path).unwrap();
            db.conn
                .execute("INSERT INTO app_meta VALUES ('k', 'v')", [])
                .unwrap();
        }
        let db = Database::open(&path).unwrap();
        let v: String = db
            .conn
            .query_row("SELECT value FROM app_meta WHERE key = 'k'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(v, "v");
    }

    #[test]
    fn refuses_newer_schema_without_modifying_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite3");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE future (x); PRAGMA user_version = 99;")
                .unwrap();
        }
        let before = std::fs::read(&path).unwrap();
        let err = Database::open(&path).err().unwrap();
        assert_eq!(
            err,
            AppError::SchemaTooNew {
                found: 99,
                supported: SCHEMA_VERSION
            }
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn reports_non_database_file_and_leaves_it_intact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite3");
        let garbage = vec![0xAB_u8; 8192];
        std::fs::write(&path, &garbage).unwrap();
        let err = Database::open(&path).err().unwrap();
        assert_eq!(err.code(), "storage_corrupt");
        assert_eq!(std::fs::read(&path).unwrap(), garbage);
    }

    #[test]
    fn failed_migration_rolls_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite3");
        {
            // Pre-existing conflicting table makes migration 1 fail mid-transaction.
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE app_meta (other INTEGER);")
                .unwrap();
        }
        assert!(Database::open(&path).is_err());
        let conn = Connection::open(&path).unwrap();
        assert_eq!(user_version(&conn).unwrap(), 0);
    }
}
