//! Storage lifecycle: creation, upgrade, restart, and refusing to reset damaged data.

mod common;

use common::{album, artist, count, edition, open_temp};
use mudraft_lib::db::{Database, SCHEMA_VERSION};
use mudraft_lib::library::catalogue::LISTEN_ASAP_TAG_ID;
use mudraft_lib::library::personal;
use rusqlite::Connection;

const V1_FIXTURE: &str = include_str!("fixtures/v1.sql");

fn write_v1_fixture(path: &std::path::Path) {
    Connection::open(path)
        .unwrap()
        .execute_batch(V1_FIXTURE)
        .unwrap();
}

#[test]
fn fresh_database_has_full_schema_and_builtin_tag() {
    let (dir, db) = open_temp();
    assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);
    db.quick_check().unwrap();
    let (fk, journal, asap): (bool, String, String) = db
        .read(|c| {
            Ok((
                c.pragma_query_value(None, "foreign_keys", |r| r.get(0))?,
                c.pragma_query_value(None, "journal_mode", |r| r.get(0))?,
                c.query_row(
                    "SELECT name FROM tag WHERE id = ?1",
                    [LISTEN_ASAP_TAG_ID],
                    |r| r.get(0),
                )?,
            ))
        })
        .unwrap();
    assert!(fk);
    assert_eq!(journal, "wal");
    assert_eq!(asap, "Listen ASAP");
    // A brand-new database has nothing to protect, so no backup is taken.
    assert!(db.list_backups().unwrap().is_empty());
    assert!(dir.path().join("mudraft.sqlite3").is_file());
}

#[test]
fn upgrades_prior_fixture_preserving_data_with_backup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mudraft.sqlite3");
    write_v1_fixture(&path);

    let db = Database::open(&path).unwrap();
    assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);
    let kept: String = db
        .read(|c| {
            Ok(c.query_row(
                "SELECT value FROM app_meta WHERE key = 'installed_by'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(kept, "mudraft 0.1.0");
    assert_eq!(count(&db, "artist"), 0);

    let backups = db.list_backups().unwrap();
    assert_eq!(backups.len(), 1);
    assert!(
        backups[0]
            .file_name
            .ends_with(&format!("-pre-v{SCHEMA_VERSION}.sqlite3"))
    );
    let backup = Connection::open(db.backup_dir().join(&backups[0].file_name)).unwrap();
    let v: i64 = backup
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(v, 1, "backup holds the pre-upgrade database");
}

#[test]
fn failed_upgrade_rolls_back_and_keeps_prior_version() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mudraft.sqlite3");
    write_v1_fixture(&path);
    // A stray table named like a v2 table makes the upgrade fail midway.
    Connection::open(&path)
        .unwrap()
        .execute_batch("CREATE TABLE artist (x);")
        .unwrap();

    assert!(Database::open(&path).is_err());
    let conn = Connection::open(&path).unwrap();
    let v: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(v, 1);
    let tables: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE name IN ('album', 'edition', 'tag')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tables, 0, "no partial v2 schema left behind");
    let kept: i64 = conn
        .query_row("SELECT COUNT(*) FROM app_meta", [], |r| r.get(0))
        .unwrap();
    assert_eq!(kept, 1);
}

#[test]
fn data_persists_across_restart() {
    let (dir, mut db) = open_temp();
    let a = artist(&mut db, "Radiohead");
    let al = album(&mut db, "OK Computer", &[&a]);
    let (ed, _) = edition(&mut db, &al, "Standard", 3);
    db.write(|tx| personal::add_to_listen_list(tx, &ed))
        .unwrap();
    drop(db);

    let db = Database::open(&dir.path().join("mudraft.sqlite3")).unwrap();
    assert_eq!(count(&db, "track"), 3);
    let list = db.read(personal::listen_list).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].edition_id, ed);
}

#[test]
fn damaged_or_newer_files_fail_without_being_reset() {
    let dir = tempfile::tempdir().unwrap();

    let garbage_path = dir.path().join("garbage.sqlite3");
    let garbage = vec![0x5A_u8; 16_384];
    std::fs::write(&garbage_path, &garbage).unwrap();
    assert_eq!(
        Database::open(&garbage_path).err().unwrap().code(),
        "storage_corrupt"
    );
    assert_eq!(std::fs::read(&garbage_path).unwrap(), garbage);

    let newer_path = dir.path().join("newer.sqlite3");
    Connection::open(&newer_path)
        .unwrap()
        .execute_batch("CREATE TABLE artist (id TEXT); INSERT INTO artist VALUES ('keep'); PRAGMA user_version = 99;")
        .unwrap();
    let before = std::fs::read(&newer_path).unwrap();
    assert_eq!(
        Database::open(&newer_path).err().unwrap().code(),
        "schema_too_new"
    );
    assert_eq!(std::fs::read(&newer_path).unwrap(), before);

    // A wrong-typed stored setting is reported, not replaced by the default.
    let (_d, mut db) = open_temp();
    db.write(|tx| Ok(tx.execute("INSERT INTO setting (key, value) VALUES ('listen.full_listen_removes_from_listen_list', '\"yes\"')", [])?))
        .unwrap();
    assert_eq!(
        db.read(mudraft_lib::library::settings::load)
            .unwrap_err()
            .code(),
        "validation"
    );
}
