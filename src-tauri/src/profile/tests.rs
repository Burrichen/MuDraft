use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

use super::export::dump_table;
use super::restore::{self, Phase};
use super::*;
use crate::commands::profile::{export_to, inspect, restore_from};
use crate::domain::ids::new_id;
use crate::library::catalogue::{
    self, CreditInput, LISTEN_ASAP_TAG_ID, NewAlbum, NewArtist, NewEdition, NewRecording, NewTrack,
};
use crate::library::listening::{self, ListenKind, LogListen};
use crate::library::preferences::{self, UiPreferencesPatch};
use crate::library::selection::{self, SelectionMode, SelectionSource};
use crate::library::{personal, ratings, settings, tags};
use crate::paths::StorageProfile;
use crate::state::AppState;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nnot-really-an-image-but-bytes";

fn state(dir: &Path) -> AppState {
    AppState::open(StorageProfile::Development, dir.to_path_buf())
}

fn credit(id: &str) -> CreditInput {
    CreditInput {
        artist_id: id.into(),
        credited_name: None,
        join_phrase: Some(" & ".into()),
    }
}

fn track(p: i64, title: &str, recording: Option<&String>, credits: Vec<CreditInput>) -> NewTrack {
    NewTrack {
        disc_number: None,
        position: p,
        title: title.into(),
        length_ms: Some(200_000),
        recording_id: recording.cloned(),
        musicbrainz_track_id: None,
        credits,
    }
}

/// A profile touching every exported table.
fn rich_profile(s: &AppState) {
    let art_dir = crate::artwork::artwork_dir(s);
    std::fs::create_dir_all(&art_dir).unwrap();
    let art_name = format!("edition-x-{}.png", new_id());
    std::fs::write(art_dir.join(&art_name), PNG).unwrap();
    s.with_db(|db| {
        db.write(|tx| {
            let a = catalogue::create_artist(tx, None, &NewArtist {
                name: "Alpha".into(),
                sort_name: Some("Alpha, The".into()),
                musicbrainz_id: Some("a74b1b7f-71a5-4011-9441-d0b5e4122711".into()),
            })?;
            let b = catalogue::create_artist(tx, None, &NewArtist {
                name: "Beta".into(),
                sort_name: None,
                musicbrainz_id: None,
            })?;
            let album = catalogue::create_album(tx, Some(&new_id()), &NewAlbum {
                title: "Shared".into(),
                original_date: Some("1997-05-21".into()),
                musicbrainz_release_group_id: Some("b1392450-e666-3926-a536-22c65f834433".into()),
                credits: vec![credit(&a), credit(&b)],
                genres: vec!["Rock".into(), "Electronic".into()],
            })?;
            let r1 = catalogue::create_recording(tx, None, &NewRecording {
                title: "One".into(),
                length_ms: Some(200_000),
                musicbrainz_recording_id: Some("40000000-0000-4000-8000-000000000001".into()),
            })?;
            let std_ed = catalogue::create_edition(tx, None, &NewEdition {
                album_id: album.clone(),
                name: "Standard".into(),
                release_date: Some("1997".into()),
                musicbrainz_release_id: None,
                tracks: vec![track(1, "One", Some(&r1), vec![]), track(2, "Two", None, vec![credit(&b)])],
            })?;
            let dlx = catalogue::create_edition(tx, None, &NewEdition {
                album_id: album.clone(),
                name: "Deluxe".into(),
                release_date: Some("2017-06".into()),
                musicbrainz_release_id: None,
                tracks: vec![track(1, "One", Some(&r1), vec![]), track(2, "Bonus", None, vec![])],
            })?;
            let road = tags::create(tx, "Road trip", Some("#1d4ed8"))?;
            tx.execute("UPDATE tag SET color = '#ef4444' WHERE id = ?1", [LISTEN_ASAP_TAG_ID])?;
            catalogue::tag_album(tx, &album, LISTEN_ASAP_TAG_ID)?;
            catalogue::tag_album(tx, &album, &road.id)?;
            personal::add_to_listen_list(tx, &dlx.edition_id)?;
            settings::save(tx, &settings::Settings { full_listen_removes_from_listen_list: false })?;
            settings::set_artwork_download(tx, true)?;
            preferences::update(tx, &serde_json::from_str::<UiPreferencesPatch>(
                r#"{"startPage": "/stats", "albumLayout": "list"}"#,
            ).unwrap())?;
            // Not a user preference: must not travel.
            tx.execute("INSERT INTO setting (key, value) VALUES ('dev.secret_flag', 'true')", [])?;
            let log = |edition: &str, date: Option<&str>, kind, tracks: Option<Vec<String>>, earlier| {
                listening::log(tx, Some(&new_id()), &LogListen {
                    edition_id: edition.into(),
                    listened_on: date.map(Into::into),
                    kind,
                    earlier_undated: earlier,
                    track_ids: tracks,
                    attempt_id: None,
                })
            };
            log(&std_ed.edition_id, Some("2026-08-10"), ListenKind::First, None, true)?;
            log(&dlx.edition_id, None, ListenKind::Relisten, Some(vec![dlx.track_ids[1].clone()]), false)?;
            let gone = log(&std_ed.edition_id, Some("2026-07-01"), ListenKind::Unspecified, None, false)?;
            listening::delete(tx, &gone.listen_id)?;
            ratings::set_track(tx, &std_ed.track_ids[0], Some(0))?;
            ratings::set_album(tx, &dlx.edition_id, Some(7))?;
            ratings::set_review(tx, &std_ed.edition_id, Some("Still great."))?;
            ratings::set_favourite(tx, &dlx.track_ids[1], true)?;
            let session = selection::start_session(tx, SelectionMode::CompletelyRandom, None)?;
            selection::record_attempt(tx, &session, &dlx.edition_id, SelectionSource::CompletelyRandom)?;
            tx.execute(
                "INSERT INTO metadata_provenance (entity_type, entity_id, field, source, provider_value, is_override)
                 VALUES ('album', ?1, 'title', 'musicbrainz', 'Shared (Remastered)', 1)",
                [&album],
            )?;
            tx.execute(
                "INSERT INTO artwork (owner_type, owner_id, source, file_name, mime, bytes)
                 VALUES ('edition', ?1, 'local', ?2, 'image/png', ?3)",
                rusqlite::params![std_ed.edition_id, art_name, PNG.len() as i64],
            )?;
            tx.execute(
                "INSERT INTO artwork (owner_type, owner_id, source) VALUES ('edition', ?1, 'removed')",
                [&dlx.edition_id],
            )?;
            tx.execute(
                "INSERT INTO artist_catalogue (artist_id, musicbrainz_artist_id, provider_total, completed_at, scope)
                 VALUES (?1, 'a74b1b7f-71a5-4011-9441-d0b5e4122711', 1, '2026-09-01T00:00:00.000Z',
                         '{\"primaryTypes\":[\"Album\",\"EP\"],\"secondaryTypes\":[]}')",
                [&a],
            )?;
            tx.execute(
                "INSERT INTO catalogue_entry (id, artist_id, source, musicbrainz_release_group_id, title,
                     primary_type, credit, credit_artists, credit_parts, excluded)
                 VALUES (?1, ?2, 'musicbrainz', 'b1392450-e666-3926-a536-22c65f834433', 'Shared', 'Album',
                         'Alpha & Beta', 2, '[{\"musicbrainzArtistId\":\"x\",\"name\":\"Alpha\",\"joinPhrase\":\" & \"}]', 1)",
                rusqlite::params![new_id(), a],
            )?;
            tx.execute(
                "INSERT INTO catalogue_entry (id, artist_id, source, title, original_year) VALUES (?1, ?2, 'manual', 'Demo', 2003)",
                rusqlite::params![new_id(), b],
            )?;
            tx.execute(
                "INSERT INTO catalogue_reference (musicbrainz_release_group_id, musicbrainz_release_id, title, track_count)
                 VALUES ('10000000-0000-4000-8000-000000000001', '20000000-0000-4000-8000-000000000001', 'Rep', 1)",
                [],
            )?;
            tx.execute(
                "INSERT INTO catalogue_track (musicbrainz_release_id, disc, position, title, musicbrainz_recording_id)
                 VALUES ('20000000-0000-4000-8000-000000000001', 1, 1, 'Song', '40000000-0000-4000-8000-000000000001')",
                [],
            )?;
            tx.execute(
                "INSERT INTO imported_row (fingerprint, album_id, edition_id) VALUES ('fp-1', ?1, ?2)",
                [&album, &std_ed.edition_id],
            )?;
            // Disposable: must not travel.
            tx.execute(
                "INSERT INTO metadata_cache (key, body, fetched_at, expires_at) VALUES ('k', '{}', 'x', 'y')",
                [],
            )?;
            Ok(())
        })
    })
    .unwrap();
}

/// Every exported table, with settings filtered the way export filters them.
fn dump(s: &AppState) -> Vec<TableData> {
    s.with_db(|db| {
        db.read(|c| {
            TABLES
                .iter()
                .map(|t| {
                    let mut d = dump_table(c, t)?;
                    if *t == "setting" {
                        d.rows.retain(|r| {
                            r[0].as_str()
                                .is_some_and(|k| SETTING_PREFIXES.iter().any(|p| k.starts_with(p)))
                        });
                    }
                    Ok(d)
                })
                .collect()
        })
    })
    .unwrap()
}

/// Equal dumps, or a panic naming the first differing table and row.
fn assert_same(a: &[TableData], b: &[TableData], what: &str) {
    for (x, y) in a.iter().zip(b) {
        assert_eq!(x.columns, y.columns, "{what}: columns of {}", x.table);
        for (i, (rx, ry)) in x.rows.iter().zip(&y.rows).enumerate() {
            assert_eq!(rx, ry, "{what}: {} row {i}", x.table);
        }
        assert_eq!(x.rows.len(), y.rows.len(), "{what}: {} row count", x.table);
    }
    assert_eq!(a.len(), b.len());
}

fn count(s: &AppState, table: &str) -> i64 {
    s.with_db(|db| {
        db.read(|c| Ok(c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?))
    })
    .unwrap()
}

fn restore(s: &AppState, archive: &Path) -> AppResult<restore::Preview> {
    let preview = inspect(s, archive)?;
    restore_from(None, s, archive, &preview.sha256).map(|o| o.preview)
}

fn nothing_left_behind(dir: &Path) {
    for leftover in [
        restore::STAGING_DIR,
        restore::PREVIOUS_DIR,
        restore::JOURNAL,
    ] {
        assert!(!dir.join(leftover).exists(), "{leftover} left behind");
    }
    let scratch = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .any(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(".profile-inspect-")
        });
    assert!(!scratch, "inspection scratch left behind");
}

type Entries = BTreeMap<String, Vec<u8>>;
type Edit = Box<dyn FnOnce(&mut Manifest, &mut Entries)>;

fn read_entries(archive: &Path) -> Entries {
    let mut zip = ZipArchive::new(std::fs::File::open(archive).unwrap()).unwrap();
    let mut out = BTreeMap::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).unwrap();
        let mut bytes = Vec::new();
        f.read_to_end(&mut bytes).unwrap();
        out.insert(f.name().to_owned(), bytes);
    }
    out
}

fn write_entries(path: &Path, entries: &Entries) {
    let mut zip = ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, bytes) in entries {
        zip.start_file(name.as_str(), SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}

/// Rewrite an archive. With `fix`, sizes and checksums in the manifest are recomputed so
/// only the content (not the bookkeeping) is wrong.
fn tamper(src: &Path, dst: &Path, fix: bool, edit: impl FnOnce(&mut Manifest, &mut Entries)) {
    let mut entries = read_entries(src);
    let mut manifest: Manifest = serde_json::from_slice(&entries["manifest.json"]).unwrap();
    edit(&mut manifest, &mut entries);
    if fix {
        for t in &mut manifest.tables {
            if let Some(b) = entries.get(&t.path) {
                t.bytes = b.len() as u64;
                t.sha256 = sha256_hex(b);
            }
        }
    }
    entries.insert(
        "manifest.json".into(),
        serde_json::to_vec(&manifest).unwrap(),
    );
    write_entries(dst, &entries);
}

fn edit_table(entries: &mut Entries, table: &str, edit: impl FnOnce(&mut TableData)) {
    let key = format!("data/{table}.json");
    let mut data: TableData = serde_json::from_slice(&entries[&key]).unwrap();
    edit(&mut data);
    entries.insert(key, serde_json::to_vec(&data).unwrap());
}

struct Setup {
    _dir: tempfile::TempDir,
    root: PathBuf,
    source: AppState,
    archive: PathBuf,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let source = state(&root.join("source"));
    rich_profile(&source);
    let archive = root.join("profile.mudraft");
    export_to(&source, &archive, true).unwrap();
    Setup {
        _dir: dir,
        root,
        source,
        archive,
    }
}

#[test]
fn every_table_is_exported_or_deliberately_excluded() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path());
    let names: Vec<String> = s
        .with_db(|db| {
            db.read(|c| {
                let mut st = c.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")?;
                Ok(st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?)
            })
        })
        .unwrap();
    for n in names {
        assert!(
            TABLES.contains(&n.as_str()) || EXCLUDED.iter().any(|(e, _)| *e == n),
            "table {n} is neither exported nor excluded"
        );
    }
}

#[test]
fn round_trip_is_exact_and_excludes_transient_data() {
    let s = setup();
    let original = dump(&s.source);
    assert!(count(&s.source, "applied_mutation") > 0);

    let target = state(&s.root.join("target"));
    let preview = restore(&target, &s.archive).unwrap();
    assert_eq!(preview.schema_version, crate::db::SCHEMA_VERSION);
    assert_eq!(preview.counts.albums, 1);
    assert_eq!(preview.counts.artwork_files, 1);
    assert_same(
        &dump(&target),
        &original,
        "every exported table restores exactly",
    );
    // Built-in tag restored once, with its colour; transient data left behind.
    assert_eq!(count(&target, "tag WHERE builtin_key = 'listen_asap'"), 1);
    assert_eq!(count(&target, "tag WHERE color = '#ef4444'"), 1);
    assert_eq!(count(&target, "setting WHERE key LIKE 'dev.%'"), 0);
    assert_eq!(
        (
            count(&target, "metadata_cache"),
            count(&target, "applied_mutation")
        ),
        (0, 0)
    );
    // The image came along, byte for byte.
    let name: String = target
        .with_db(|db| {
            db.read(|c| {
                Ok(c.query_row(
                    "SELECT file_name FROM artwork WHERE file_name IS NOT NULL",
                    [],
                    |r| r.get(0),
                )?)
            })
        })
        .unwrap();
    assert_eq!(
        std::fs::read(crate::artwork::artwork_dir(&target).join(name)).unwrap(),
        PNG
    );
    nothing_left_behind(&s.root.join("target"));
    // The restored profile keeps working: writes succeed.
    target
        .with_db(|db| db.write(|tx| tags::create(tx, "After restore", None)))
        .unwrap();
}

#[test]
fn lightweight_and_missing_artwork_are_optional() {
    let s = setup();
    let light = s.root.join("light.mudraft");
    let out = export_to(&s.source, &light, false).unwrap();
    assert_eq!(
        (out.artwork, out.counts.artwork_files),
        (ArtworkMode::Omitted, 0)
    );
    assert!(
        !read_entries(&light)
            .keys()
            .any(|k| k.starts_with("artwork/"))
    );
    let target = state(&s.root.join("light-target"));
    let preview = restore(&target, &light).unwrap();
    assert_eq!(preview.artwork, ArtworkMode::Omitted);
    assert_eq!(count(&target, "artwork WHERE file_name IS NOT NULL"), 0);
    assert_eq!(
        count(&target, "artwork WHERE source = 'removed'"),
        1,
        "user decisions are kept"
    );

    // A cached file that vanished from disk is left out, not fabricated.
    for f in std::fs::read_dir(crate::artwork::artwork_dir(&s.source)).unwrap() {
        std::fs::remove_file(f.unwrap().path()).unwrap();
    }
    let gap = s.root.join("gap.mudraft");
    assert_eq!(export_to(&s.source, &gap, true).unwrap().artwork_missing, 1);
    restore(&state(&s.root.join("gap-target")), &gap).unwrap();
}

#[test]
fn restoring_the_automatic_backup_brings_the_old_profile_back() {
    let s = setup();
    let original = dump(&s.source);
    // Another, smaller profile replaces the rich one…
    let other = state(&s.root.join("other"));
    other
        .with_db(|db| db.write(|tx| tags::create(tx, "Other only", None)))
        .unwrap();
    let other_archive = s.root.join("other.mudraft");
    export_to(&other, &other_archive, true).unwrap();
    let preview = inspect(&s.source, &other_archive).unwrap();
    let outcome = restore_from(None, &s.source, &other_archive, &preview.sha256).unwrap();
    assert_eq!(count(&s.source, "album"), 0);
    // …and the backup written first restores it exactly.
    let backup = s
        .root
        .join("source")
        .join("backups")
        .join(&outcome.backup_file_name);
    assert!(backup.is_file());
    restore(&s.source, &backup).unwrap();
    assert_same(&dump(&s.source), &original, "dump");
}

#[test]
fn corrupt_archives_are_rejected_and_nothing_changes() {
    let s = setup();
    let before = dump(&s.source);
    let bad = s.root.join("bad.mudraft");
    let cases: Vec<(&str, bool, Edit)> = vec![
        (
            "checksum",
            false,
            Box::new(|_, e| edit_table(e, "album", |d| d.rows[0][1] = "Changed".into())),
        ),
        ("missing manifest", false, Box::new(|_, _| {})),
        (
            "row count",
            true,
            Box::new(|m, _| {
                m.tables
                    .iter_mut()
                    .find(|t| t.name == "album")
                    .unwrap()
                    .rows = 5
            }),
        ),
        (
            "broken reference",
            true,
            Box::new(|_, e| {
                edit_table(e, "album_artist_credit", |d| {
                    let i = d.columns.iter().position(|c| c == "artist_id").unwrap();
                    d.rows[0][i] = new_id().into();
                })
            }),
        ),
        (
            "rating out of range",
            true,
            Box::new(|_, e| {
                edit_table(e, "track_rating", |d| {
                    let i = d.columns.iter().position(|c| c == "rating").unwrap();
                    d.rows[0][i] = 11.into();
                })
            }),
        ),
        (
            "bad date",
            true,
            Box::new(|_, e| {
                edit_table(e, "listen_event", |d| {
                    let i = d.columns.iter().position(|c| c == "listened_on").unwrap();
                    let r = d.rows.iter_mut().find(|r| !r[i].is_null()).unwrap();
                    r[i] = "2026-02-30".into();
                })
            }),
        ),
        (
            "non-canonical id",
            true,
            Box::new(|_, e| {
                edit_table(e, "artist", |d| {
                    d.rows[0][0] = d.rows[0][0].as_str().unwrap().to_uppercase().into();
                })
            }),
        ),
        (
            "unexpected file",
            false,
            Box::new(|_, e| {
                e.insert("notes.txt".into(), b"hi".to_vec());
            }),
        ),
        (
            "unknown column",
            true,
            Box::new(|_, e| {
                edit_table(e, "tag", |d| {
                    d.columns.push("password".into());
                    for r in &mut d.rows {
                        r.push("x".into());
                    }
                })
            }),
        ),
        (
            "duplicate built-in tag",
            true,
            Box::new(|_, e| {
                edit_table(e, "tag", |d| {
                    let mut copy = d.rows[0].clone();
                    copy[0] = new_id().into();
                    d.rows.push(copy);
                })
            }),
        ),
    ];
    for (name, fix, edit) in cases {
        let missing_manifest = name == "missing manifest";
        tamper(&s.archive, &bad, fix, edit);
        if missing_manifest {
            let mut e = read_entries(&bad);
            e.remove("manifest.json");
            write_entries(&bad, &e);
        }
        if name == "duplicate built-in tag" {
            let mut m: Manifest =
                serde_json::from_slice(&read_entries(&bad)["manifest.json"]).unwrap();
            m.tables.iter_mut().find(|t| t.name == "tag").unwrap().rows += 1;
            let mut e = read_entries(&bad);
            e.insert("manifest.json".into(), serde_json::to_vec(&m).unwrap());
            write_entries(&bad, &e);
        }
        let err = inspect(&s.source, &bad).expect_err(name);
        assert_eq!(err.code(), "validation", "{name}: {err}");
        assert_same(
            &dump(&s.source),
            &before,
            "{name} changed the current profile",
        );
        nothing_left_behind(&s.root.join("source"));
    }
    std::fs::write(&bad, b"this is not a zip file").unwrap();
    assert_eq!(inspect(&s.source, &bad).unwrap_err().code(), "validation");
}

#[test]
fn malicious_paths_symlinks_and_bombs_are_rejected() {
    let s = setup();
    let evil = s.root.join("evil.mudraft");
    for path in [
        "../escape.json",
        "/etc/passwd",
        "C:\\Windows\\x.json",
        "artwork/../../x.png",
        "data//x.json",
    ] {
        let mut e = read_entries(&s.archive);
        e.insert(path.into(), b"x".to_vec());
        write_entries(&evil, &e);
        assert_eq!(
            inspect(&s.source, &evil).unwrap_err().code(),
            "validation",
            "{path}"
        );
    }
    // A symbolic link entry.
    let mut zip = ZipWriter::new(std::fs::File::create(&evil).unwrap());
    for (name, bytes) in read_entries(&s.archive) {
        zip.start_file(name.as_str(), SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&bytes).unwrap();
    }
    zip.add_symlink(
        "artwork/link.png",
        "/etc/passwd",
        SimpleFileOptions::default(),
    )
    .unwrap();
    zip.finish().unwrap();
    assert!(
        inspect(&s.source, &evil)
            .unwrap_err()
            .to_string()
            .contains("symbolic link")
    );
    // A decompression bomb: 64 MB of zeros in one small entry.
    let mut e = read_entries(&s.archive);
    e.insert("artwork/bomb.png".into(), vec![0; 64 * 1024 * 1024]);
    let mut zip = ZipWriter::new(std::fs::File::create(&evil).unwrap());
    for (name, bytes) in &e {
        zip.start_file(
            name.as_str(),
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
        )
        .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
    assert!(
        inspect(&s.source, &evil)
            .unwrap_err()
            .to_string()
            .contains("compressed suspiciously")
    );
    nothing_left_behind(&s.root.join("source"));
}

#[test]
fn artwork_that_is_not_an_image_is_rejected() {
    let s = setup();
    let disguised = s.root.join("disguised.mudraft");
    tamper(&s.archive, &disguised, true, |m, e| {
        let asset = m.assets[0].clone();
        let html = b"<html><script>alert(1)</script></html>".to_vec();
        m.assets[0].bytes = html.len() as u64;
        m.assets[0].sha256 = sha256_hex(&html);
        e.insert(asset.path.clone(), html.clone());
        edit_table(e, "artwork", |d| {
            let i = d.columns.iter().position(|c| c == "bytes").unwrap();
            for r in &mut d.rows {
                if !r[i].is_null() {
                    r[i] = (html.len() as i64).into();
                }
            }
        });
    });
    assert!(
        inspect(&s.source, &disguised)
            .unwrap_err()
            .to_string()
            .contains("isn't a supported image")
    );
}

#[test]
fn future_versions_are_rejected_without_changes() {
    let s = setup();
    let before = dump(&s.source);
    let future = s.root.join("future.mudraft");
    tamper(&s.archive, &future, true, |m, _| {
        m.format_version = FORMAT_VERSION + 1
    });
    assert!(
        inspect(&s.source, &future)
            .unwrap_err()
            .to_string()
            .contains("newer version of MuDraft")
    );
    tamper(&s.archive, &future, true, |m, _| {
        m.schema_version = crate::db::SCHEMA_VERSION + 1
    });
    assert!(
        inspect(&s.source, &future)
            .unwrap_err()
            .to_string()
            .contains("newer version of MuDraft")
    );
    assert_same(&dump(&s.source), &before, "dump");
}

#[test]
fn older_schema_exports_are_upgraded_by_the_normal_migrations() {
    let s = setup();
    let older = s.root.join("older.mudraft");
    // As written by a MuDraft at schema 9 (before catalogue credit parts existed).
    tamper(&s.archive, &older, true, |m, e| {
        m.schema_version = 9;
        edit_table(e, "catalogue_entry", |d| {
            let i = d.columns.iter().position(|c| c == "credit_parts").unwrap();
            d.columns.remove(i);
            for r in &mut d.rows {
                r.remove(i);
            }
        });
    });
    let target = state(&s.root.join("older-target"));
    let preview = restore(&target, &older).unwrap();
    assert_eq!(
        (preview.schema_version, preview.current_schema_version),
        (9, crate::db::SCHEMA_VERSION)
    );
    assert_eq!(
        target.with_db(|db| db.schema_version()).unwrap(),
        crate::db::SCHEMA_VERSION
    );
    assert_eq!(
        count(&target, "catalogue_entry WHERE credit_parts = '[]'"),
        2
    );
}

#[test]
fn windows_style_archives_restore_on_any_platform_without_absolute_paths() {
    let s = setup();
    // Nothing machine-specific is written.
    let data_dir = s.root.join("source").to_string_lossy().into_owned();
    for (name, bytes) in read_entries(&s.archive) {
        assert!(
            !String::from_utf8_lossy(&bytes).contains(&data_dir),
            "{name} contains a local path"
        );
    }
    // An archive whose tool wrote backslashes (as some Windows zippers do).
    let windows = s.root.join("windows.mudraft");
    tamper(&s.archive, &windows, true, |m, e| {
        let renamed: Entries = std::mem::take(e)
            .into_iter()
            .map(|(k, v)| {
                (
                    if k == "manifest.json" {
                        k
                    } else {
                        k.replace('/', "\\")
                    },
                    v,
                )
            })
            .collect();
        *e = renamed;
        for t in &mut m.tables {
            t.path = t.path.replace('/', "\\");
        }
        for a in &mut m.assets {
            a.path = a.path.replace('/', "\\");
        }
    });
    let target = state(&s.root.join("windows-target"));
    restore(&target, &windows).unwrap();
    assert_same(&dump(&target), &dump(&s.source), "windows");
    let art = crate::artwork::artwork_dir(&target);
    assert_eq!(std::fs::read_dir(art).unwrap().count(), 1);
}

#[test]
fn interrupted_restores_finish_or_roll_back_on_restart() {
    let s = setup();
    let original = dump(&s.source);
    let dir = s.root.join("crash");
    let crashing = state(&dir);
    crashing
        .with_db(|db| db.write(|tx| tags::create(tx, "Only before", None)))
        .unwrap();
    let before = dump(&crashing);

    // Crash while staging (no journal yet): the old profile stays, staging is discarded.
    restore::stage(&s.archive, &restore::staging_dir(&dir)).unwrap();
    drop(crashing);
    let reopened = state(&dir);
    assert_same(&dump(&reopened), &before, "dump");
    nothing_left_behind(&dir);

    // Crash after moving the old profile out, before moving the new one in.
    restore::stage(&s.archive, &restore::staging_dir(&dir)).unwrap();
    drop(reopened);
    restore::write_journal(&dir, Phase::MovingOut).unwrap();
    restore::move_out(&dir).unwrap();
    assert!(
        !dir.join(crate::paths::DB_FILE_NAME).exists(),
        "the live database is gone mid-swap"
    );
    let recovered = state(&dir);
    assert_same(
        &dump(&recovered),
        &original,
        "restart completes the validated restore",
    );
    nothing_left_behind(&dir);
}
