//! Listen logging and the Collection.

mod common;

use common::{album, artist, count, edition, open_temp};
use mudraft_lib::db::Database;
use mudraft_lib::domain::ids::new_id;
use mudraft_lib::domain::rating::HalfStars;
use mudraft_lib::library::listening::{self, ListenFields, ListenKind, LogListen};
use mudraft_lib::library::listing::{self, ListQuery, SortOrder, Source};
use mudraft_lib::library::selection::{self, SelectionMode, SelectionSource};
use mudraft_lib::library::settings::{self, Settings};
use mudraft_lib::library::{album as album_view, personal, ratings};

fn setup(db: &mut Database, title: &str, tracks: i64) -> (String, String, Vec<String>) {
    let a = artist(db, "Artist");
    let al = album(db, title, &[&a]);
    let (ed, t) = edition(db, &al, "Standard", tracks);
    (al, ed, t)
}

fn full(ed: &str, date: Option<&str>, kind: ListenKind) -> LogListen {
    LogListen {
        edition_id: ed.into(),
        listened_on: date.map(Into::into),
        kind,
        earlier_undated: false,
        track_ids: None,
        attempt_id: None,
    }
}

fn log(db: &mut Database, input: &LogListen) -> listening::Logged {
    db.write(|tx| listening::log(tx, Some(&new_id()), input))
        .unwrap()
}

fn listened_titles(db: &Database, al: &str) -> Vec<(String, bool)> {
    db.read(|c| album_view::detail(c, al, None))
        .unwrap()
        .discs
        .iter()
        .flat_map(|d| d.tracks.iter().map(|t| (t.title.clone(), t.listened)))
        .collect()
}

#[test]
fn backdated_and_unknown_dates_persist_and_update_memberships() {
    let (dir, mut db) = open_temp();
    let (al, ed, _) = setup(&mut db, "Backdated", 3);
    db.write(|tx| personal::add_to_listen_list(tx, &ed))
        .unwrap();
    let out = log(
        &mut db,
        &LogListen {
            earlier_undated: true,
            ..full(&ed, Some("1999-12-31"), ListenKind::First)
        },
    );
    assert!(out.is_full && out.added_to_collection && out.removed_from_listen_list);
    assert_eq!((out.coverage.as_str(), out.covered_tracks), ("tracks", 3));
    log(&mut db, &full(&ed, None, ListenKind::Relisten));
    let bad =
        db.write(|tx| listening::log(tx, None, &full(&ed, Some("2025-02-30"), ListenKind::First)));
    assert_eq!(bad.unwrap_err().code(), "validation");

    drop(db);
    let db = Database::open(&dir.path().join("mudraft.sqlite3")).unwrap();
    let h = db.read(|c| listening::history(c, &al)).unwrap();
    assert_eq!(h.summary.last_listened.as_deref(), Some("1999-12-31"));
    assert_eq!(
        (
            h.summary.listen_count,
            h.summary.undated_listens,
            h.summary.earlier_undated
        ),
        (2, 1, true)
    );
    assert_eq!(
        h.listens[0].listened_on.as_deref(),
        Some("1999-12-31"),
        "dated before undated"
    );
    assert_eq!(h.listens[1].listened_on, None, "unknown stays unknown");
    assert_eq!(count(&db, "listen_list_entry"), 0);
    assert_eq!(count(&db, "collection_entry"), 1);
}

#[test]
fn full_listens_respect_the_listen_list_setting() {
    let (_d, mut db) = open_temp();
    let (_, ed, _) = setup(&mut db, "Keep", 1);
    db.write(|tx| {
        settings::save(
            tx,
            &Settings {
                full_listen_removes_from_listen_list: false,
            },
        )?;
        personal::add_to_listen_list(tx, &ed)
    })
    .unwrap();
    assert!(
        !log(&mut db, &full(&ed, Some("2026-10-01"), ListenKind::First)).removed_from_listen_list
    );
    assert_eq!(count(&db, "listen_list_entry"), 1);
}

#[test]
fn relistens_add_events_not_album_entries_and_ratings_stay_independent() {
    let (_d, mut db) = open_temp();
    let (al, ed, t) = setup(&mut db, "Again", 2);
    db.write(|tx| ratings::set_track(tx, &t[0], Some(6)))
        .unwrap();
    for d in ["2026-01-01", "2026-03-01", "2026-02-01"] {
        log(&mut db, &full(&ed, Some(d), ListenKind::Relisten));
    }
    assert_eq!(count(&db, "collection_entry"), 1);
    let s = db.read(|c| listening::summary(c, &al)).unwrap();
    assert_eq!(
        (s.listen_count, s.last_listened.as_deref()),
        (3, Some("2026-03-01"))
    );
    let rating = db.read(|c| personal::track_rating(c, &t[0])).unwrap();
    assert_eq!(rating.rating, Some(HalfStars::new(6).unwrap()));
    assert_eq!(
        db.read(|c| personal::track_rating(c, &t[1]))
            .unwrap()
            .rating,
        None,
        "listening never rates"
    );
}

#[test]
fn partial_track_listens_cover_only_those_tracks() {
    let (_d, mut db) = open_temp();
    let (al, ed, t) = setup(&mut db, "Partial", 3);
    db.write(|tx| personal::add_to_listen_list(tx, &ed))
        .unwrap();
    let out = log(
        &mut db,
        &LogListen {
            track_ids: Some(vec![t[1].clone(), t[1].clone()]),
            ..full(&ed, Some("2026-10-01"), ListenKind::Unspecified)
        },
    );
    assert!(
        !out.is_full && !out.removed_from_listen_list,
        "partial listens keep the album listed"
    );
    assert_eq!(out.covered_tracks, 1);
    assert_eq!(
        listened_titles(&db, &al),
        vec![
            ("Track 1".into(), false),
            ("Track 2".into(), true),
            ("Track 3".into(), false)
        ]
    );

    let (_, _, other) = setup(&mut db, "Other", 1);
    let err = db.write(|tx| {
        listening::log(
            tx,
            None,
            &LogListen {
                track_ids: Some(other.clone()),
                ..full(&ed, None, ListenKind::First)
            },
        )
    });
    assert_eq!(err.unwrap_err().code(), "validation");
    let none = db.write(|tx| {
        listening::log(
            tx,
            None,
            &LogListen {
                track_ids: Some(vec![]),
                ..full(&ed, None, ListenKind::First)
            },
        )
    });
    assert_eq!(none.unwrap_err().code(), "validation");
}

#[test]
fn a_double_click_records_one_listen() {
    let (_d, mut db) = open_temp();
    let (_, ed, _) = setup(&mut db, "Twice", 2);
    let key = new_id();
    let input = full(&ed, Some("2026-10-01"), ListenKind::First);
    let a = db
        .write(|tx| listening::log(tx, Some(&key), &input))
        .unwrap();
    let b = db
        .write(|tx| listening::log(tx, Some(&key), &input))
        .unwrap();
    assert_eq!(a, b);
    assert_eq!(count(&db, "listen_event"), 1);
    let changed = LogListen {
        listened_on: Some("2026-09-30".into()),
        ..input
    };
    assert_eq!(
        db.write(|tx| listening::log(tx, Some(&key), &changed))
            .unwrap_err()
            .code(),
        "conflict"
    );
}

#[test]
fn undoing_a_log_restores_list_membership_exactly() {
    let (_d, mut db) = open_temp();
    let (al, ed, _) = setup(&mut db, "Undo", 2);
    db.write(|tx| personal::add_to_listen_list(tx, &ed))
        .unwrap();
    let out = log(
        &mut db,
        &LogListen {
            earlier_undated: true,
            ..full(&ed, Some("2026-10-01"), ListenKind::First)
        },
    );
    db.write(|tx| listening::undo_log(tx, &out.undo)).unwrap();
    assert_eq!(count(&db, "listen_list_entry"), 1);
    assert_eq!(
        count(&db, "collection_entry"),
        0,
        "the collection entry this log created is gone"
    );
    let s = db.read(|c| listening::summary(c, &al)).unwrap();
    assert_eq!((s.listen_count, s.earlier_undated), (0, false));
    assert!(listened_titles(&db, &al).iter().all(|(_, l)| !l));

    // An explicit Collection entry that existed before stays after undo.
    db.write(|tx| personal::add_to_collection(tx, &ed, personal::CollectionSource::Manual))
        .unwrap();
    let out = log(&mut db, &full(&ed, None, ListenKind::Unspecified));
    assert!(!out.added_to_collection);
    db.write(|tx| listening::undo_log(tx, &out.undo)).unwrap();
    assert_eq!(count(&db, "collection_entry"), 1);
}

#[test]
fn correcting_and_deleting_are_undoable_and_recompute_progress() {
    let (_d, mut db) = open_temp();
    let (al, ed, t) = setup(&mut db, "Fix", 2);
    db.write(|tx| ratings::set_track(tx, &t[0], Some(9)))
        .unwrap();
    let first = log(&mut db, &full(&ed, Some("2026-09-01"), ListenKind::First));
    let second = log(
        &mut db,
        &LogListen {
            track_ids: Some(vec![t[0].clone()]),
            ..full(&ed, Some("2026-09-20"), ListenKind::Relisten)
        },
    );

    let before = db
        .write(|tx| {
            listening::correct(
                tx,
                &second.listen_id,
                &ListenFields {
                    listened_on: None,
                    kind: ListenKind::Unspecified,
                },
            )
        })
        .unwrap();
    assert_eq!(
        (before.listened_on.as_deref(), before.kind),
        (Some("2026-09-20"), ListenKind::Relisten)
    );
    assert_eq!(
        db.read(|c| listening::summary(c, &al))
            .unwrap()
            .last_listened
            .as_deref(),
        Some("2026-09-01")
    );
    db.write(|tx| listening::correct(tx, &second.listen_id, &before))
        .unwrap();
    assert_eq!(
        db.read(|c| listening::summary(c, &al))
            .unwrap()
            .last_listened
            .as_deref(),
        Some("2026-09-20")
    );

    db.write(|tx| listening::delete(tx, &first.listen_id))
        .unwrap();
    assert_eq!(
        listened_titles(&db, &al),
        vec![("Track 1".into(), true), ("Track 2".into(), false)],
        "other evidence remains"
    );
    assert_eq!(
        db.read(|c| listening::summary(c, &al))
            .unwrap()
            .listen_count,
        1
    );
    assert_eq!(
        db.write(|tx| listening::delete(tx, &first.listen_id))
            .unwrap_err()
            .code(),
        "not_found"
    );
    db.write(|tx| listening::restore(tx, &first.listen_id))
        .unwrap();
    assert_eq!(
        listened_titles(&db, &al),
        vec![("Track 1".into(), true), ("Track 2".into(), true)]
    );
    assert_eq!(
        db.read(|c| personal::track_rating(c, &t[0]))
            .unwrap()
            .rating,
        Some(HalfStars::new(9).unwrap())
    );
    assert_eq!(count(&db, "collection_entry"), 1);
}

#[test]
fn later_added_bonus_tracks_are_not_retroactively_listened() {
    let (_d, mut db) = open_temp();
    let (al, ed, _) = setup(&mut db, "Deluxe", 2);
    log(&mut db, &full(&ed, Some("2026-01-01"), ListenKind::First));
    // A refresh later adds a bonus track to this edition.
    db.write(|tx| {
        tx.execute(
            "INSERT INTO track (id, edition_id, disc_number, position, title) VALUES (?1, ?2, 1, 3, 'Bonus')",
            [&new_id(), &ed],
        )?;
        Ok(())
    })
    .unwrap();
    let titles = listened_titles(&db, &al);
    assert_eq!(titles[2], ("Bonus".into(), false));
    assert!(titles[0].1 && titles[1].1);
}

#[test]
fn unknown_coverage_is_inferred_only_after_confirmation() {
    let (_d, mut db) = open_temp();
    let (al, ed, _) = setup(&mut db, "No tracklist", 0);
    let out = log(&mut db, &full(&ed, Some("2026-01-01"), ListenKind::First));
    assert_eq!((out.coverage.as_str(), out.covered_tracks), ("unknown", 0));
    assert_eq!(
        db.write(|tx| listening::confirm_coverage(tx, &out.listen_id))
            .unwrap_err()
            .code(),
        "validation",
        "nothing to infer yet"
    );
    db.write(|tx| {
        for p in 1..=2 {
            tx.execute(
                "INSERT INTO track (id, edition_id, disc_number, position, title) VALUES (?1, ?2, 1, ?3, 'T')",
                rusqlite::params![new_id(), ed, p],
            )?;
        }
        Ok(())
    })
    .unwrap();
    assert!(
        listened_titles(&db, &al).iter().all(|(_, l)| !l),
        "not inferred silently"
    );
    assert_eq!(
        db.write(|tx| listening::confirm_coverage(tx, &out.listen_id))
            .unwrap(),
        2
    );
    assert!(listened_titles(&db, &al).iter().all(|(_, l)| *l));
    assert_eq!(
        db.write(|tx| listening::confirm_coverage(tx, &out.listen_id))
            .unwrap_err()
            .code(),
        "validation"
    );
}

#[test]
fn completing_a_next_up_pick_links_and_clears_it_without_picking_again() {
    let (_d, mut db) = open_temp();
    let (al, ed, _) = setup(&mut db, "Picked", 1);
    let (_, other_ed, _) = setup(&mut db, "Other", 1);
    let attempt = db
        .write(|tx| {
            let s = selection::start_session(tx, SelectionMode::CompletelyRandom, None)?;
            selection::record_attempt(tx, &s, &ed, SelectionSource::CompletelyRandom)
        })
        .unwrap();
    let wrong = db.write(|tx| {
        listening::log(
            tx,
            None,
            &LogListen {
                attempt_id: Some(attempt.clone()),
                ..full(&other_ed, None, ListenKind::First)
            },
        )
    });
    assert_eq!(wrong.unwrap_err().code(), "validation");

    let out = log(
        &mut db,
        &LogListen {
            attempt_id: Some(attempt.clone()),
            ..full(&ed, Some("2026-10-01"), ListenKind::First)
        },
    );
    assert!(out.completed_next_up);
    assert_eq!(
        db.read(selection::current_selection).unwrap(),
        None,
        "cleared, nothing new picked"
    );
    assert_eq!(count(&db, "selection_attempt"), 1);
    assert!(db.read(|c| listening::history(c, &al)).unwrap().listens[0].from_next_up);

    db.write(|tx| listening::undo_log(tx, &out.undo)).unwrap();
    let back = db.read(selection::current_selection).unwrap().unwrap();
    assert_eq!(
        (back.attempt_id.as_str(), back.status),
        (attempt.as_str(), selection::AttemptStatus::Shown)
    );
}

#[test]
fn collection_sorts_recently_listened_with_unknown_dates_transparently() {
    let (_d, mut db) = open_temp();
    let (_, recent, _) = setup(&mut db, "Recent", 1);
    let (_, older, _) = setup(&mut db, "Older", 1);
    let (_, undated, _) = setup(&mut db, "Undated", 1);
    let (_, never, _) = setup(&mut db, "Never", 1);
    log(
        &mut db,
        &full(&recent, Some("2026-09-30"), ListenKind::First),
    );
    log(
        &mut db,
        &full(&older, Some("2001-05-05"), ListenKind::First),
    );
    log(&mut db, &full(&undated, None, ListenKind::First));
    // Explicit membership without any listen.
    db.write(|tx| personal::add_to_collection(tx, &never, personal::CollectionSource::Manual))
        .unwrap();

    let r = db
        .read(|c| {
            listing::query(
                c,
                Source::Collection,
                &ListQuery {
                    sort: SortOrder::RecentlyListened,
                    ..Default::default()
                },
            )
        })
        .unwrap();
    let rows: Vec<(String, Option<String>, u32)> = r
        .items
        .iter()
        .map(|i| {
            (
                i.title.clone(),
                i.listening.last_listened.clone(),
                i.listening.listen_count,
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            ("Recent".into(), Some("2026-09-30".into()), 1),
            ("Older".into(), Some("2001-05-05".into()), 1),
            ("Undated".into(), None, 1),
            ("Never".into(), None, 0),
        ]
    );
}
