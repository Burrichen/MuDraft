//! Lists: progress against the Collection and adding albums to the Listen List.

mod common;

use common::{album, artist, count, edition, open_temp};
use mudraft_lib::csv_import::commit::{self, TagDecisions};
use mudraft_lib::csv_import::mapping::{self, RowFields};
use mudraft_lib::db::Database;
use mudraft_lib::library::listening::{ListenKind, LogListen};
use mudraft_lib::library::lists::{self, EntryStatus, NewEntry};
use mudraft_lib::library::{listening, personal};

fn entry(artist: &str, title: &str, year: Option<i64>, position: Option<i64>) -> NewEntry {
    NewEntry {
        artist_name: artist.into(),
        title: title.into(),
        year,
        position,
    }
}

fn sample(db: &mut Database) -> String {
    db.write(|tx| {
        lists::create(
            tx,
            "  Best of  ",
            Some("https://pitchfork.com/story/x/"),
            &[
                entry(
                    "Pavement",
                    "Crooked Rain, Crooked Rain",
                    Some(1994),
                    Some(1),
                ),
                entry("Liz Phair", "Exile in Guyville", Some(1993), Some(2)),
                entry("Sonic Youth", "Daydream Nation", Some(1988), Some(3)),
            ],
        )
    })
    .unwrap()
}

fn statuses(db: &Database, list: &str) -> Vec<EntryStatus> {
    db.read(|c| lists::detail(c, list))
        .unwrap()
        .entries
        .iter()
        .map(|e| e.status)
        .collect()
}

fn collect(db: &mut Database, edition_id: &str) {
    db.write(|tx| personal::add_to_collection(tx, edition_id, personal::CollectionSource::Manual))
        .unwrap();
}

#[test]
fn creates_lists_with_ordered_entries_and_a_trimmed_name() {
    let (_d, mut db) = open_temp();
    let id = sample(&mut db);
    let d = db.read(|c| lists::detail(c, &id)).unwrap();
    assert_eq!(d.name, "Best of");
    assert_eq!(
        d.source_url.as_deref(),
        Some("https://pitchfork.com/story/x/")
    );
    let order: Vec<_> = d.entries.iter().map(|e| e.position).collect();
    assert_eq!(order, [1, 2, 3]);
    assert_eq!((d.progress.total, d.progress.listened), (3, 0));
    assert!(d.entries.iter().all(|e| e.status == EntryStatus::Missing));
}

#[test]
fn only_the_collection_counts_as_listened() {
    let (_d, mut db) = open_temp();
    let list = sample(&mut db);
    let pav = artist(&mut db, "Pavement");
    let crooked = album(&mut db, "Crooked Rain, Crooked Rain", &[&pav]);
    let (crooked_ed, _) = edition(&mut db, &crooked, "Standard", 1);
    let sy = artist(&mut db, "Sonic Youth");
    let daydream = album(&mut db, "Daydream Nation", &[&sy]);
    let (daydream_ed, _) = edition(&mut db, &daydream, "Standard", 1);

    // In the library only: neither listened nor on the Listen List.
    assert_eq!(
        statuses(&db, &list),
        [
            EntryStatus::InLibrary,
            EntryStatus::Missing,
            EntryStatus::InLibrary
        ]
    );

    // On the Listen List: reported, but not counted.
    db.write(|tx| personal::add_to_listen_list(tx, &daydream_ed))
        .unwrap();
    let d = db.read(|c| lists::detail(c, &list)).unwrap();
    assert_eq!(d.entries[2].status, EntryStatus::ListenList);
    assert_eq!((d.progress.listened, d.progress.on_listen_list), (0, 1));

    // In the Collection: counted, and it wins over the Listen List.
    collect(&mut db, &crooked_ed);
    collect(&mut db, &daydream_ed);
    let d = db.read(|c| lists::detail(c, &list)).unwrap();
    assert_eq!(d.progress.listened, 2);
    assert_eq!(d.progress.on_listen_list, 0);
    assert_eq!(d.entries[0].album_id.as_deref(), Some(crooked.as_str()));

    let summary = db.read(lists::overview).unwrap();
    assert_eq!(summary.len(), 1);
    assert_eq!(
        (summary[0].progress.total, summary[0].progress.listened),
        (3, 2)
    );
}

#[test]
fn matching_ignores_case_accents_punctuation_and_a_leading_the() {
    let (_d, mut db) = open_temp();
    let list = db
        .write(|tx| {
            lists::create(
                tx,
                "L",
                None,
                &[
                    entry("The Replacements", "Let it Be", None, None),
                    entry("Björk", "Homogenic", None, None),
                ],
            )
        })
        .unwrap();
    let rep = artist(&mut db, "Replacements");
    let letitbe = album(&mut db, "Let It Be!", &[&rep]);
    let (ed, _) = edition(&mut db, &letitbe, "Standard", 1);
    collect(&mut db, &ed);
    let bjork = artist(&mut db, "Bjork");
    let hg = album(&mut db, "Homogenic", &[&bjork]);
    edition(&mut db, &hg, "Standard", 1);
    assert_eq!(
        statuses(&db, &list),
        [EntryStatus::Listened, EntryStatus::InLibrary]
    );
    // Same title by another artist is a different album.
    let other = artist(&mut db, "Someone Else");
    let imposter = album(&mut db, "Homogenic", &[&other]);
    let (imposter_ed, _) = edition(&mut db, &imposter, "Standard", 1);
    collect(&mut db, &imposter_ed);
    assert_eq!(statuses(&db, &list)[1], EntryStatus::InLibrary);
}

#[test]
fn adding_entries_skips_duplicates_and_appends_after_the_last_rank() {
    let (_d, mut db) = open_temp();
    let list = sample(&mut db);
    let added = db
        .write(|tx| {
            lists::add_entries(
                tx,
                &list,
                &[
                    entry("pavement", "crooked rain crooked rain", None, None),
                    entry("Slint", "Spiderland", Some(1991), None),
                    entry("Slint", "Spiderland", Some(1991), None),
                ],
            )
        })
        .unwrap();
    assert_eq!((added.added, added.duplicates), (1, 2));
    let d = db.read(|c| lists::detail(c, &list)).unwrap();
    assert_eq!(d.entries.last().unwrap().position, 4);
    assert_eq!(d.entries.last().unwrap().title, "Spiderland");
}

#[test]
fn rejects_bad_input_and_changes_nothing() {
    let (_d, mut db) = open_temp();
    let list = sample(&mut db);
    for (name, entries) in [
        ("", vec![]),
        ("   ", vec![]),
        ("L", vec![entry("", "T", None, None)]),
        ("L", vec![entry("A", "T", Some(99), None)]),
        ("L", vec![entry("A", "T", None, Some(0))]),
    ] {
        let err = db
            .write(|tx| lists::create(tx, name, None, &entries))
            .unwrap_err();
        assert_eq!(err.code(), "validation");
    }
    assert!(
        db.write(|tx| lists::create(tx, "L", Some("http://x.example/"), &[]))
            .is_err()
    );
    assert_eq!(
        count(&db, "list"),
        1,
        "failed creates roll back whole lists"
    );
    assert_eq!(
        db.write(|tx| lists::add_entries(tx, &list, &[entry("A", "", None, None)]))
            .unwrap_err()
            .code(),
        "validation"
    );
    assert_eq!(count(&db, "list_entry"), 3);
    let missing = "0190f5c3-0000-7000-8000-000000000001";
    assert_eq!(
        db.read(|c| lists::detail(c, missing)).unwrap_err().code(),
        "not_found"
    );
}

#[test]
fn unlistened_albums_in_the_library_go_on_the_listen_list_and_the_rest_need_a_match() {
    let (_d, mut db) = open_temp();
    let list = sample(&mut db);
    // One album is listened, one is in the library only, one isn't in the library.
    let pav = artist(&mut db, "Pavement");
    let crooked = album(&mut db, "Crooked Rain, Crooked Rain", &[&pav]);
    let (crooked_ed, _) = edition(&mut db, &crooked, "Standard", 1);
    collect(&mut db, &crooked_ed);
    let lp = artist(&mut db, "Liz Phair");
    let exile = album(&mut db, "Exile in Guyville", &[&lp]);
    edition(&mut db, &exile, "Standard", 1);

    let r = db
        .write(|tx| lists::add_unlistened_to_listen_list(tx, &list))
        .unwrap();
    assert_eq!((r.added, r.needs_match), (1, 1));
    assert_eq!(
        statuses(&db, &list),
        [
            EntryStatus::Listened,
            EntryStatus::ListenList,
            EntryStatus::Missing
        ]
    );
    assert_eq!(count(&db, "listen_list_entry"), 1);
    // Nothing was invented: no albums or artists were created for the missing one.
    assert_eq!(count(&db, "album"), 2);
    assert_eq!(count(&db, "artist"), 2);

    // Repeating changes nothing.
    let again = db
        .write(|tx| lists::add_unlistened_to_listen_list(tx, &list))
        .unwrap();
    assert_eq!((again.added, again.needs_match), (0, 1));
    assert_eq!(count(&db, "listen_list_entry"), 1);
}

#[test]
fn one_entry_in_the_library_is_added_and_one_that_isnt_is_left_for_review() {
    let (_d, mut db) = open_temp();
    let list = sample(&mut db);
    let lp = artist(&mut db, "Liz Phair");
    let exile = album(&mut db, "Exile in Guyville", &[&lp]);
    edition(&mut db, &exile, "Standard", 1);
    let d = db.read(|c| lists::detail(c, &list)).unwrap();
    let (liz, sonic) = (d.entries[1].id.clone(), d.entries[2].id.clone());
    let r = db
        .write(|tx| lists::add_entry_to_listen_list(tx, &liz))
        .unwrap();
    assert_eq!((r.added, r.needs_match), (1, 0));
    let r = db
        .write(|tx| lists::add_entry_to_listen_list(tx, &sonic))
        .unwrap();
    assert_eq!((r.added, r.needs_match), (0, 1));
    assert_eq!(count(&db, "album"), 1);
}

#[test]
fn listening_to_a_listed_album_moves_it_into_progress() {
    let (_d, mut db) = open_temp();
    let list = sample(&mut db);
    let lp = artist(&mut db, "Liz Phair");
    let exile = album(&mut db, "Exile in Guyville", &[&lp]);
    let (ed, _) = edition(&mut db, &exile, "Standard", 1);
    db.write(|tx| lists::add_unlistened_to_listen_list(tx, &list))
        .unwrap();
    assert_eq!(statuses(&db, &list)[1], EntryStatus::ListenList);
    db.write(|tx| {
        listening::log(
            tx,
            None,
            &LogListen {
                edition_id: ed.clone(),
                listened_on: Some("2026-01-02".into()),
                kind: ListenKind::First,
                earlier_undated: false,
                attempt_id: None,
            },
        )
    })
    .unwrap();
    let d = db.read(|c| lists::detail(c, &list)).unwrap();
    assert_eq!((d.progress.listened, d.progress.on_listen_list), (1, 0));
    assert_eq!(d.entries[1].status, EntryStatus::Listened);
}

#[test]
fn review_stages_only_albums_missing_from_the_library_for_musicbrainz_matching() {
    let (_d, mut db) = open_temp();
    let list = sample(&mut db);
    let lp = artist(&mut db, "Liz Phair");
    let exile = album(&mut db, "Exile in Guyville", &[&lp]);
    edition(&mut db, &exile, "Standard", 1);

    let summary = db
        .write(|tx| lists::review_session(tx, &list, None))
        .unwrap();
    assert_eq!(summary.file_name, "List: Best of");
    assert!(summary.mapping.is_some(), "columns are mapped for the user");
    let rows: Vec<(String, String, Option<i64>)> = db
        .read(|c| {
            let mut stmt = c.prepare(
                "SELECT json_extract(fields, '$.album'), json_extract(fields, '$.artist'),
                        json_extract(fields, '$.year')
                 FROM import_row WHERE session_id = ?1 ORDER BY row_number",
            )?;
            let rows = stmt.query_map([&summary.session_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
        .unwrap();
    assert_eq!(
        rows,
        [
            (
                "Crooked Rain, Crooked Rain".to_owned(),
                "Pavement".to_owned(),
                Some(1994)
            ),
            (
                "Daydream Nation".to_owned(),
                "Sonic Youth".to_owned(),
                Some(1988)
            ),
        ]
    );
    assert_eq!(
        count(&db, "album"),
        1,
        "staging adds nothing to the library"
    );

    // One entry alone, and nothing to review when everything is in the library.
    let d = db.read(|c| lists::detail(c, &list)).unwrap();
    let one = db
        .write(|tx| lists::review_session(tx, &list, Some(&d.entries[2].id)))
        .unwrap();
    assert_eq!(one.counts.rows, 1);
    let err = db
        .write(|tx| lists::review_session(tx, &list, Some(&d.entries[1].id)))
        .unwrap_err();
    assert_eq!(err.code(), "validation");
}

#[test]
fn a_reviewed_import_links_the_album_even_when_its_title_differs() {
    let (_d, mut db) = open_temp();
    let list = db
        .write(|tx| {
            lists::create(
                tx,
                "L",
                None,
                &[entry(
                    "Yeah Yeah Yeahs",
                    "Yeah Yeah Yeahs EP",
                    Some(2001),
                    None,
                )],
            )
        })
        .unwrap();
    assert_eq!(statuses(&db, &list), [EntryStatus::Missing]);

    // MusicBrainz knows it as "Yeah Yeah Yeahs": titles differ, so name matching misses it.
    let yyy = artist(&mut db, "Yeah Yeah Yeahs");
    let ep = album(&mut db, "Yeah Yeah Yeahs", &[&yyy]);
    let (ed, _) = edition(&mut db, &ep, "Standard", 1);
    assert_eq!(statuses(&db, &list), [EntryStatus::Missing]);

    // Committing the reviewed import records the row's fingerprint against the album.
    let fp = mapping::fingerprint(&RowFields {
        album: "Yeah Yeah Yeahs EP".into(),
        artist: "Yeah Yeah Yeahs".into(),
        year: Some(2001),
        edition: None,
        tags: vec![],
        release_group_id: None,
        release_id: None,
    });
    db.write(|tx| {
        tx.execute(
            "INSERT INTO imported_row (fingerprint, album_id, edition_id) VALUES (?1, ?2, ?3)",
            rusqlite::params![fp, ep, ed],
        )?;
        Ok(())
    })
    .unwrap();
    assert_eq!(statuses(&db, &list), [EntryStatus::InLibrary]);
    collect(&mut db, &ed);
    assert_eq!(statuses(&db, &list), [EntryStatus::Listened]);
}

#[test]
fn an_entry_can_be_linked_to_the_album_the_user_chose() {
    let (_d, mut db) = open_temp();
    let list = sample(&mut db);
    let a = artist(&mut db, "Someone");
    let other = album(&mut db, "Another Title", &[&a]);
    let (ed, _) = edition(&mut db, &other, "Standard", 1);
    let entry_id = db.read(|c| lists::detail(c, &list)).unwrap().entries[0]
        .id
        .clone();
    db.write(|tx| lists::link_entry(tx, &entry_id, &other))
        .unwrap();
    collect(&mut db, &ed);
    let d = db.read(|c| lists::detail(c, &list)).unwrap();
    assert_eq!(d.entries[0].status, EntryStatus::Listened);
    assert_eq!(d.entries[0].album_id.as_deref(), Some(other.as_str()));
    let missing = "0190f5c3-0000-7000-8000-000000000001";
    assert_eq!(
        db.write(|tx| lists::link_entry(tx, &entry_id, missing))
            .unwrap_err()
            .code(),
        "not_found"
    );
}

#[test]
fn editing_and_deleting_lists_never_touches_the_library() {
    let (_d, mut db) = open_temp();
    let list = sample(&mut db);
    let lp = artist(&mut db, "Liz Phair");
    let exile = album(&mut db, "Exile in Guyville", &[&lp]);
    edition(&mut db, &exile, "Standard", 1);
    db.write(|tx| lists::add_unlistened_to_listen_list(tx, &list))
        .unwrap();
    db.write(|tx| lists::rename(tx, &list, "Renamed")).unwrap();
    assert_eq!(
        db.read(|c| lists::detail(c, &list)).unwrap().name,
        "Renamed"
    );
    let entry_id = db.read(|c| lists::detail(c, &list)).unwrap().entries[0]
        .id
        .clone();
    db.write(|tx| lists::remove_entry(tx, &entry_id)).unwrap();
    assert_eq!(
        db.read(|c| lists::detail(c, &list)).unwrap().progress.total,
        2
    );
    db.write(|tx| lists::delete(tx, &list)).unwrap();
    assert_eq!(count(&db, "list"), 0);
    assert_eq!(count(&db, "list_entry"), 0);
    assert_eq!(count(&db, "album"), 1);
    assert_eq!(count(&db, "listen_list_entry"), 1);
}

#[test]
fn committing_the_reviewed_import_puts_the_albums_on_the_listen_list_and_links_them() {
    let (_d, mut db) = open_temp();
    let list = sample(&mut db);
    let summary = db
        .write(|tx| lists::review_session(tx, &list, None))
        .unwrap();
    // Offline, rows default to manual entries; the commit is the real import path.
    let report = db
        .write(|tx| {
            commit::commit(
                tx,
                &summary.session_id,
                &TagDecisions {
                    create: vec![],
                    skip: vec![],
                },
            )
        })
        .unwrap();
    assert_eq!(report.added_to_listen_list, 3);
    assert_eq!(
        statuses(&db, &list),
        [EntryStatus::ListenList; 3],
        "the list sees what the import created"
    );
    let d = db.read(|c| lists::detail(c, &list)).unwrap();
    assert_eq!((d.progress.listened, d.progress.on_listen_list), (0, 3));
    // Nothing is left to match, and asking again adds nothing.
    assert_eq!(
        db.write(|tx| lists::review_session(tx, &list, None))
            .unwrap_err()
            .code(),
        "validation"
    );
    let again = db
        .write(|tx| lists::add_unlistened_to_listen_list(tx, &list))
        .unwrap();
    assert_eq!((again.added, again.needs_match), (0, 0));
    assert_eq!(count(&db, "album"), 3);
}
