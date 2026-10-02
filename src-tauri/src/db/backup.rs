//! Backups use SQLite's online backup API, so the copy is transactionally consistent
//! even while the live database is open in WAL mode. A live file is never copied raw.

use std::path::{Path, PathBuf};

use rusqlite::backup::Backup;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use super::{Database, SCHEMA_VERSION, check_foreign_keys, quick_check, user_version};
use crate::error::{AppError, AppResult};

const PREFIX: &str = "mudraft-";
const SUFFIX: &str = ".sqlite3";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub file_name: String,
    pub bytes: u64,
}

/// Backups live in `<data dir>/backups`, next to the authoritative database.
pub fn backup_dir_for(db_path: &Path) -> AppResult<PathBuf> {
    db_path
        .parent()
        .map(|p| p.join("backups"))
        .ok_or_else(|| AppError::InvalidPath(format!("{} has no parent", db_path.display())))
}

/// Write a validated backup of `conn` and return its file name. The copy is written to a
/// temporary name and only renamed into place once it passes an integrity check.
pub fn create(conn: &Connection, dir: &Path, label: &str) -> AppResult<String> {
    if label.is_empty()
        || !label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(AppError::Backup(format!("invalid label '{label}'")));
    }
    std::fs::create_dir_all(dir)
        .map_err(|e| AppError::Backup(format!("cannot create {}: {e}", dir.display())))?;
    let stamp: String =
        conn.query_row("SELECT strftime('%Y%m%dT%H%M%f', 'now')", [], |r| r.get(0))?;
    let base = format!("{PREFIX}{stamp}-{label}").replace('.', "");
    let (file_name, final_path) = (0..100)
        .map(|n| {
            let name = if n == 0 {
                format!("{base}{SUFFIX}")
            } else {
                format!("{base}-{n}{SUFFIX}")
            };
            let path = dir.join(&name);
            (name, path)
        })
        .find(|(_, p)| !p.exists())
        .ok_or_else(|| AppError::Backup("too many backups with the same timestamp".into()))?;
    let partial = final_path.with_extension("partial");

    let result = (|| {
        let mut dst = Connection::open(&partial)?;
        Backup::new(conn, &mut dst)?.run_to_completion(256, std::time::Duration::ZERO, None)?;
        // The copy inherits WAL mode from the live database. A backup must be one
        // self-contained file, so switch it to a rollback journal before closing.
        let mode: String =
            dst.pragma_update_and_check(None, "journal_mode", "DELETE", |r| r.get(0))?;
        if !mode.eq_ignore_ascii_case("delete") {
            return Err(AppError::Backup(format!("backup stayed in {mode} mode")));
        }
        quick_check(&dst)?;
        dst.close().map_err(|(_, e)| AppError::from(e))?;
        std::fs::rename(&partial, &final_path)
            .map_err(|e| AppError::Backup(format!("cannot finalize backup: {e}")))
    })();
    if let Err(e) = result {
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let mut side = partial.clone().into_os_string();
            side.push(suffix);
            let _ = std::fs::remove_file(side);
        }
        return Err(match e {
            AppError::Backup(_) => e,
            other => AppError::Backup(other.to_string()),
        });
    }
    Ok(file_name)
}

/// Completed backups, oldest first (names sort by timestamp).
pub fn list(dir: &Path) -> AppResult<Vec<BackupInfo>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(AppError::Backup(format!(
                "cannot read {}: {e}",
                dir.display()
            )));
        }
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| AppError::Backup(e.to_string()))?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if is_backup_name(&name) {
            let bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
            out.push(BackupInfo {
                file_name: name,
                bytes,
            });
        }
    }
    out.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    Ok(out)
}

fn is_backup_name(name: &str) -> bool {
    name.starts_with(PREFIX)
        && name.ends_with(SUFFIX)
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
        && !name.contains("..")
}

fn open_read_only(path: &Path) -> AppResult<Connection> {
    Ok(Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?)
}

impl Database {
    pub fn create_backup(&self, label: &str) -> AppResult<String> {
        create(&self.conn, &self.backup_dir, label)
    }

    pub fn list_backups(&self) -> AppResult<Vec<BackupInfo>> {
        list(&self.backup_dir)
    }

    /// Replace live contents with a named backup from the backup directory. The backup is
    /// validated first and the current state is itself backed up, so a restore is undoable.
    pub fn restore_backup(&mut self, file_name: &str) -> AppResult<()> {
        if !is_backup_name(file_name) {
            return Err(AppError::validation(
                "backup",
                format!("'{file_name}' is not a backup name"),
            ));
        }
        let path = self.backup_dir.join(file_name);
        if !path.is_file() {
            return Err(AppError::not_found("backup", file_name));
        }
        let src = open_read_only(&path)?;
        quick_check(&src)?;
        let found = user_version(&src)?;
        if found > SCHEMA_VERSION {
            return Err(AppError::SchemaTooNew {
                found,
                supported: SCHEMA_VERSION,
            });
        }
        self.create_backup("pre-restore")?;
        Backup::new(&src, &mut self.conn)?.run_to_completion(
            256,
            std::time::Duration::ZERO,
            None,
        )?;
        drop(src);
        self.conn.pragma_update(None, "foreign_keys", true)?;
        self.migrate(found)?;
        quick_check(&self.conn)?;
        check_foreign_keys(&self.conn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_restricted_to_backup_dir_entries() {
        assert!(is_backup_name("mudraft-20260930T120000123-manual.sqlite3"));
        assert!(!is_backup_name("../mudraft-x.sqlite3"));
        assert!(!is_backup_name("mudraft-x/../../y.sqlite3"));
        assert!(!is_backup_name("mudraft-x.sqlite3-wal"));
        assert!(!is_backup_name("other.sqlite3"));
    }

    #[test]
    fn backup_and_restore_round_trip_through_live_wal_database() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Database::open(&dir.path().join("live.sqlite3")).unwrap();
        db.write(|tx| Ok(tx.execute("INSERT INTO app_meta VALUES ('k', 'before')", [])?))
            .unwrap();
        let name = db.create_backup("manual").unwrap();
        db.write(|tx| Ok(tx.execute("UPDATE app_meta SET value = 'after'", [])?))
            .unwrap();

        db.restore_backup(&name).unwrap();
        let v: String = db
            .read(|c| Ok(c.query_row("SELECT value FROM app_meta", [], |r| r.get(0))?))
            .unwrap();
        assert_eq!(v, "before");

        let names: Vec<_> = db
            .list_backups()
            .unwrap()
            .into_iter()
            .map(|b| b.file_name)
            .collect();
        assert_eq!(names.len(), 2);
        assert!(names.iter().any(|n| n.ends_with("-pre-restore.sqlite3")));
        // Only complete, self-contained files: no partials and no detached WAL/SHM.
        let mut on_disk: Vec<_> = std::fs::read_dir(db.backup_dir())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        on_disk.sort();
        assert_eq!(on_disk, names);
        for name in &names {
            let copy = open_read_only(&db.backup_dir().join(name)).unwrap();
            let mode: String = copy
                .pragma_query_value(None, "journal_mode", |r| r.get(0))
                .unwrap();
            assert_eq!(mode, "delete");
        }
    }

    #[test]
    fn refuses_damaged_or_unknown_backups_and_keeps_live_data() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Database::open(&dir.path().join("live.sqlite3")).unwrap();
        db.write(|tx| Ok(tx.execute("INSERT INTO app_meta VALUES ('k', 'live')", [])?))
            .unwrap();
        std::fs::create_dir_all(db.backup_dir()).unwrap();
        std::fs::write(
            db.backup_dir().join("mudraft-bad.sqlite3"),
            vec![7_u8; 4096],
        )
        .unwrap();

        assert_eq!(
            db.restore_backup("mudraft-bad.sqlite3").unwrap_err().code(),
            "storage_corrupt"
        );
        assert_eq!(
            db.restore_backup("mudraft-missing.sqlite3")
                .unwrap_err()
                .code(),
            "not_found"
        );
        assert_eq!(
            db.restore_backup("../live.sqlite3").unwrap_err().code(),
            "validation"
        );
        let v: String = db
            .read(|c| Ok(c.query_row("SELECT value FROM app_meta", [], |r| r.get(0))?))
            .unwrap();
        assert_eq!(v, "live");
    }
}
