//! Staged CSV import: parse → map → review → commit, without network access.

mod common;

use common::{count, open_temp};
use mudraft_lib::csv_import::commit::{self, CommitReport, TagDecisions};
use mudraft_lib::csv_import::mapping::{self, ColumnMapping};
use mudraft_lib::csv_import::parse;
use mudraft_lib::csv_import::staging::{
    self, Decision, DecisionInput, ExistingReason, Readiness, RowFilter,
};
use mudraft_lib::db::Database;
use mudraft_lib::domain::rating::HalfStars;
use mudraft_lib::error::AppError;
use mudraft_lib::library::personal;

const HEADER: &str =
    "Album,Artist,Year,Edition,Tags,MusicBrainz Release Group ID,MusicBrainz Release ID\n";

fn stage(db: &mut Database, csv: &str) -> String {
    let parsed = parse::parse(csv.as_bytes()).unwrap();
    db.write(|tx| {
        let id = staging::create_session(tx, "albums.csv", &parsed)?;
        staging::apply_mapping(tx, &id, &mapping::suggest(&parsed.headers))?;
        Ok(id)
    })
    .unwrap()
}

fn commit_all(db: &mut Database, session: &str, create: &[&str]) -> Result<CommitReport, AppError> {
    let tags = TagDecisions {
        create: create.iter().map(|s| (*s).into()).collect(),
        skip: vec![],
    };
    db.write(|tx| commit::commit(tx, session, &tags))
}

fn rows(db: &Database, session: &str) -> Vec<staging::StagedRow> {
    db.read(|c| staging::all_rows(c, session)).unwrap()
}

fn library_counts(db: &Database) -> Vec<i64> {
    [
        "artist",
        "album",
        "edition",
        "listen_list_entry",
        "album_tag",
        "tag",
        "imported_row",
    ]
    .iter()
    .map(|t| count(db, t))
    .collect()
}

#[test]
fn offline_import_creates_manual_entries_on_the_listen_list() {
    let (_d, mut db) = open_temp();
    let s = stage(
        &mut db,
        &format!("{HEADER}Homogenic,Björk,1997,,,,\nVespertine,Björk,2001,,,,\n"),
    );
    let summary = db.read(|c| staging::summary(c, &s)).unwrap();
    assert_eq!(
        (
            summary.counts.rows,
            summary.counts.ready,
            summary.counts.manual
        ),
        (2, 2, 2)
    );
    // Staging alone changes nothing in the library.
    assert_eq!(count(&db, "album"), 0);

    let report = commit_all(&mut db, &s, &[]).unwrap();
    assert_eq!((report.albums_created, report.added_to_listen_list), (2, 2));
    assert_eq!(
        count(&db, "artist"),
        1,
        "same written artist within a file is one artist"
    );
    let (date, precision): (String, String) = db
        .read(|c| Ok(c.query_row("SELECT original_date, original_date_precision FROM album WHERE title = 'Homogenic'", [], |r| Ok((r.get(0)?, r.get(1)?)))?))
        .unwrap();
    assert_eq!((date.as_str(), precision.as_str()), ("1997", "year"));
    let source: String = db
        .read(|c| {
            Ok(
                c.query_row("SELECT DISTINCT source FROM metadata_provenance", [], |r| {
                    r.get(0)
                })?,
            )
        })
        .unwrap();
    assert_eq!(source, "csv");
    let status = db.read(|c| staging::summary(c, &s)).unwrap().status;
    assert_eq!(status, "committed");
    assert_eq!(
        commit_all(&mut db, &s, &[]).unwrap_err().code(),
        "conflict",
        "a session commits once"
    );
}

#[test]
fn reimporting_the_same_file_duplicates_nothing_and_keeps_personal_data() {
    let (_d, mut db) = open_temp();
    let csv =
        format!("{HEADER}Homogenic,Björk,1997,,Listen ASAP,,\nHomogenic,Björk,1997,Deluxe,,,\n");
    let first = stage(&mut db, &csv);
    let report = commit_all(&mut db, &first, &[]).unwrap();
    assert_eq!(
        (report.albums_created, report.editions_created),
        (1, 2),
        "Standard and Deluxe are one album, two editions"
    );
    assert_eq!(report.added_to_listen_list, 1);
    assert_eq!(
        report.already_on_listen_list, 1,
        "the album keeps its first edition on the list"
    );

    // The user rates, reviews, and removes nothing; then re-imports the same file.
    let edition: String = db
        .read(|c| Ok(c.query_row("SELECT edition_id FROM listen_list_entry", [], |r| r.get(0))?))
        .unwrap();
    let track_free_rating = HalfStars::new(0).unwrap();
    db.write(|tx| {
        personal::set_album_rating(tx, &edition, Some(track_free_rating))?;
        personal::set_album_review(tx, &edition, Some("Still great."))
    })
    .unwrap();
    let before = library_counts(&db);

    let second = stage(&mut db, &csv);
    let staged = rows(&db, &second);
    assert!(staged.iter().all(|r| matches!(
        r.decision,
        Decision::UseExisting {
            reason: ExistingReason::PreviousImport,
            ..
        }
    )));
    let again = commit_all(&mut db, &second, &[]).unwrap();
    assert_eq!(
        (
            again.albums_created,
            again.editions_created,
            again.reused_existing
        ),
        (0, 0, 2)
    );
    assert_eq!(library_counts(&db), before);
    let review = db.read(|c| personal::album_review(c, &edition)).unwrap();
    assert_eq!(review.rating, Some(track_free_rating), "zero rating kept");
    assert_eq!(review.review.as_deref(), Some("Still great."));
}

#[test]
fn row_errors_are_reported_and_excluded_but_do_not_block_the_rest() {
    let (_d, mut db) = open_temp();
    let csv = format!(
        "{HEADER}Good,Artist,1999,,,,\n,No Album,,,,,\nBad Year,Artist,97,,,,\nBad Id,Artist,,,,xyz,\nToo,Many,1,2,3,4,5,6\n\"Multi\nLine, Title\",\"Crosby, Stills & Nash\",,,,,\n"
    );
    let s = stage(&mut db, &csv);
    let all = rows(&db, &s);
    let errors: Vec<(u32, String)> = all
        .iter()
        .filter(|r| r.readiness == Readiness::Error)
        .map(|r| (r.row_number, r.issues[0].message.clone()))
        .collect();
    assert_eq!(errors.len(), 4);
    assert!(errors[0].1.contains("album is required"));
    assert!(errors[1].1.contains("“97” isn’t a year"));
    assert!(errors[2].1.contains("isn’t a MusicBrainz ID"));
    assert!(errors[3].1.contains("unquoted comma"));
    // An error row can only be skipped.
    let err = db
        .write(|tx| staging::set_decision(tx, &s, errors[0].0, &DecisionInput::Manual, None))
        .unwrap_err();
    assert_eq!(err.code(), "validation");

    let report = commit_all(&mut db, &s, &[]).unwrap();
    assert_eq!((report.albums_created, report.errors), (2, 4));
    let artist: String = db
        .read(|c| Ok(c.query_row("SELECT ar.name FROM album a JOIN album_artist_credit ac ON ac.album_id = a.id JOIN artist ar ON ar.id = ac.artist_id WHERE a.title LIKE 'Multi%'", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(
        artist, "Crosby, Stills & Nash",
        "never split into collaborators"
    );
    assert_eq!(count(&db, "artist"), 2);
    let bad = report.rows.iter().find(|r| r.row_number == 4).unwrap();
    assert!(bad.detail.as_deref().unwrap().contains("97"));
}

#[test]
fn duplicates_within_the_file_are_skipped_but_editions_stay_distinct() {
    let (_d, mut db) = open_temp();
    let csv = format!(
        "{HEADER}Homogenic,Björk,,,,,\n  homogenic ,BJÖRK,,,,,\nHomogenic,Björk,,Deluxe,,,\nHomogenic,Bjo\u{308}rk,,deluxe,,,\n"
    );
    let s = stage(&mut db, &csv);
    let all = rows(&db, &s);
    assert_eq!(
        all.iter().map(|r| r.duplicate_of).collect::<Vec<_>>(),
        vec![None, Some(2), None, Some(4)]
    );
    let dupes = db
        .read(|c| staging::rows(c, &s, 0, 50, RowFilter::Duplicates))
        .unwrap();
    assert_eq!(dupes.total, 2);
    let report = commit_all(&mut db, &s, &[]).unwrap();
    assert_eq!(
        (
            report.albums_created,
            report.editions_created,
            report.skipped
        ),
        (1, 2, 2)
    );
    assert!(
        report
            .rows
            .iter()
            .any(|r| r.detail.as_deref() == Some("duplicate of row 2"))
    );
}

#[test]
fn existing_library_albums_are_detected_and_can_be_kept_separate() {
    let (_d, mut db) = open_temp();
    let s = stage(&mut db, &format!("{HEADER}Homogenic,Björk,,,,,\n"));
    commit_all(&mut db, &s, &[]).unwrap();
    // Remove the fingerprint so detection must use the library itself.
    db.write(|tx| Ok(tx.execute("DELETE FROM imported_row", [])?))
        .unwrap();

    let s2 = stage(&mut db, &format!("{HEADER}HOMOGENIC,björk,,,,,\n"));
    let row = &rows(&db, &s2)[0];
    assert!(matches!(
        row.decision,
        Decision::UseExisting {
            reason: ExistingReason::SameName,
            ..
        }
    ));
    assert_eq!(row.existing.as_ref().unwrap().album_title, "Homogenic");
    let attention = db
        .read(|c| staging::rows(c, &s2, 0, 50, RowFilter::Attention))
        .unwrap();
    assert_eq!(
        attention.total, 1,
        "same-name matches are flagged for review"
    );

    // The user says it's a different album: Keep as Manual creates a separate one.
    db.write(|tx| staging::set_decision(tx, &s2, 2, &DecisionInput::Manual, None))
        .unwrap();
    commit_all(&mut db, &s2, &[]).unwrap();
    assert_eq!(count(&db, "album"), 2);
}

#[test]
fn exact_ids_already_in_the_library_resolve_predictably_offline() {
    let (_d, mut db) = open_temp();
    let rel = "11111111-1111-4111-8111-111111111111";
    let rg = "b1392450-e666-3926-a536-22c65f834433";
    db.write(|tx| {
        tx.execute("INSERT INTO album (id, title, musicbrainz_release_group_id) VALUES ('a1', 'OK Computer', ?1)", [rg])?;
        tx.execute("INSERT INTO edition (id, album_id, name, musicbrainz_release_id) VALUES ('e1', 'a1', 'Standard', ?1)", [rel])?;
        tx.execute("INSERT INTO edition (id, album_id, name) VALUES ('e2', 'a1', 'Deluxe')", [])?;
        Ok(())
    })
    .unwrap();
    let csv = format!(
        "{HEADER}Anything,Whoever,,,,,{rel}\nOK Computer,Radiohead,,Deluxe,,{rg},\nOK Computer,Radiohead,,Japan,,{rg},\n"
    );
    let s = stage(&mut db, &csv);
    let all = rows(&db, &s);
    match &all[0].decision {
        Decision::UseExisting {
            edition_id, reason, ..
        } => assert_eq!(
            (edition_id.as_str(), *reason),
            ("e1", ExistingReason::ReleaseId)
        ),
        d => panic!("{d:?}"),
    }
    match &all[1].decision {
        Decision::UseExisting {
            edition_id, reason, ..
        } => assert_eq!(
            (edition_id.as_str(), *reason),
            ("e2", ExistingReason::ReleaseGroupEdition)
        ),
        d => panic!("{d:?}"),
    }
    // Known album, unknown edition: needs an edition choice before commit.
    assert_eq!(all[2].readiness, Readiness::NeedsEdition);
    let err = commit_all(&mut db, &s, &[]).unwrap_err();
    assert!(
        err.to_string().contains("rows 4 need a MusicBrainz lookup"),
        "{err}"
    );
    assert_eq!(count(&db, "listen_list_entry"), 0);

    // Offline fallback: keep it as a manual edition, then commit works.
    db.write(|tx| staging::set_decision(tx, &s, 4, &DecisionInput::Skip, None))
        .unwrap();
    let report = commit_all(&mut db, &s, &[]).unwrap();
    assert_eq!((report.reused_existing, report.albums_created), (2, 0));
}

#[test]
fn unknown_tags_must_be_confirmed_and_can_be_skipped() {
    let (_d, mut db) = open_temp();
    let csv = format!("{HEADER}A,X,,,\"listen asap; road trip; Café\",,\nB,X,,,road trip,,\n");
    let s = stage(&mut db, &csv);
    let unknown = db.read(|c| staging::summary(c, &s)).unwrap().unknown_tags;
    let names: Vec<(&str, u32)> = unknown.iter().map(|t| (t.name.as_str(), t.rows)).collect();
    assert_eq!(
        names,
        vec![("Café", 1), ("road trip", 2)],
        "built-in Listen ASAP matches case-insensitively"
    );

    let err = commit_all(&mut db, &s, &["road trip"]).unwrap_err();
    assert!(err.to_string().contains("Café"));
    assert_eq!(count(&db, "album"), 0);

    let tags = TagDecisions {
        create: vec!["road trip".into()],
        skip: vec!["café".into()],
    };
    let report = db.write(|tx| commit::commit(tx, &s, &tags)).unwrap();
    assert_eq!(report.tags_created, vec!["road trip"]);
    assert_eq!(report.tags_skipped, vec!["Café"]);
    assert_eq!(report.tag_links_added, 3);
    assert_eq!(count(&db, "tag"), 2, "Listen ASAP plus road trip");
}

#[test]
fn discarding_before_commit_changes_nothing() {
    let (_d, mut db) = open_temp();
    let before = library_counts(&db);
    let s = stage(&mut db, &format!("{HEADER}A,B,,,new tag,,\n"));
    db.write(|tx| staging::discard(tx, &s)).unwrap();
    assert_eq!(library_counts(&db), before);
    assert_eq!(count(&db, "import_row"), 0);
    assert_eq!(
        db.read(|c| staging::summary(c, &s)).unwrap_err().code(),
        "not_found"
    );
}

#[test]
fn a_failure_during_commit_rolls_back_every_row() {
    let (_d, mut db) = open_temp();
    let s = stage(
        &mut db,
        &format!("{HEADER}First,Artist,,,,,\nSecond,Artist,,,,,\n"),
    );
    // Sabotage: a trigger makes the second album insert fail mid-commit.
    db.write(|tx| {
        tx.execute_batch("CREATE TEMP TRIGGER fail_second BEFORE INSERT ON album WHEN NEW.title = 'Second' BEGIN SELECT RAISE(ABORT, 'disk full'); END;")?;
        Ok(())
    })
    .unwrap();
    let err = commit_all(&mut db, &s, &[]).unwrap_err();
    assert!(err.to_string().contains("row 3"), "{err}");
    for table in [
        "artist",
        "album",
        "edition",
        "listen_list_entry",
        "imported_row",
    ] {
        assert_eq!(count(&db, table), 0, "{table}");
    }
    assert_eq!(
        db.read(|c| staging::summary(c, &s)).unwrap().status,
        "open",
        "the session can be retried"
    );
}

#[test]
fn remapping_columns_restages_every_row() {
    let (_d, mut db) = open_temp();
    let parsed = parse::parse("Name,Band,When\nHomogenic,Björk,1997\n".as_bytes()).unwrap();
    let s = db
        .write(|tx| staging::create_session(tx, "albums.csv", &parsed))
        .unwrap();
    let summary = db.read(|c| staging::summary(c, &s)).unwrap();
    assert_eq!(
        summary.suggested_mapping.album, None,
        "unrecognized headers aren't guessed"
    );
    assert!(
        db.write(|tx| staging::apply_mapping(tx, &s, &ColumnMapping::default()))
            .is_err()
    );
    let mapping = ColumnMapping {
        album: Some(0),
        artist: Some(1),
        year: Some(2),
        ..Default::default()
    };
    db.write(|tx| staging::apply_mapping(tx, &s, &mapping))
        .unwrap();
    let row = &rows(&db, &s)[0];
    let fields = row.fields.as_ref().unwrap();
    assert_eq!(
        (fields.album.as_str(), fields.artist.as_str(), fields.year),
        ("Homogenic", "Björk", Some(1997))
    );
    assert_eq!(row.readiness, Readiness::Ready);
}

#[test]
fn a_maximum_size_file_stages_reviews_and_commits() {
    let (_d, mut db) = open_temp();
    let mut csv = String::from(HEADER);
    for i in 0..mudraft_lib::csv_import::MAX_ROWS {
        csv.push_str(&format!(
            "Album {i},Artist {},{},,,,\n",
            i % 400,
            1950 + i % 70
        ));
    }
    let start = std::time::Instant::now();
    let s = stage(&mut db, &csv);
    let staged = start.elapsed();
    let summary = db.read(|c| staging::summary(c, &s)).unwrap();
    let page = db
        .read(|c| staging::rows(c, &s, 19_950, 50, RowFilter::All))
        .unwrap();
    let reviewed = start.elapsed();
    assert_eq!(summary.counts.ready, 20_000);
    assert_eq!(page.rows.len(), 50);
    let report = commit_all(&mut db, &s, &[]).unwrap();
    println!(
        "20k rows: staged {staged:?}, summary+page {:?}, committed {:?}",
        reviewed - staged,
        start.elapsed() - reviewed
    );
    assert_eq!(report.albums_created, 20_000);
    assert_eq!(count(&db, "artist"), 400);
}
