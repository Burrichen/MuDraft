//! Deleting an extra edition from an album.

mod common;

use common::{album, artist, count, edition, open_temp};
use mudraft_lib::db::Database;
use mudraft_lib::library::listening::{self, ListenKind, LogListen};
use mudraft_lib::library::selection::{self, SelectionMode, SelectionSource};
use mudraft_lib::library::{editions, personal, ratings};

struct Fixture {
    album: String,
    standard: String,
    deluxe: String,
    deluxe_tracks: Vec<String>,
}

fn fixture(db: &mut Database) -> Fixture {
    let a = artist(db, "Artist");
    let album = album(db, "Album", &[&a]);
    let (standard, _) = edition(db, &album, "Standard", 2);
    let (deluxe, deluxe_tracks) = edition(db, &album, "Deluxe", 3);
    Fixture {
        album,
        standard,
        deluxe,
        deluxe_tracks,
    }
}

fn delete(db: &mut Database, f: &Fixture, edition: &str) -> Result<Option<String>, String> {
    db.write(|tx| editions::delete(tx, &f.album, edition))
        .map_err(|e| e.to_string())
}

fn log_listen(db: &mut Database, edition: &str) -> String {
    db.write(|tx| {
        listening::log(
            tx,
            None,
            &LogListen {
                edition_id: edition.into(),
                listened_on: Some("2026-02-03".into()),
                kind: ListenKind::First,
                earlier_undated: false,
                attempt_id: None,
            },
        )
    })
    .unwrap()
    .listen_id
}

#[test]
fn deleting_an_unused_edition_removes_it_and_its_tracks_only() {
    let (_d, mut db) = open_temp();
    let f = fixture(&mut db);
    delete(&mut db, &f, &f.deluxe).unwrap();
    assert_eq!(count(&db, "edition"), 1);
    assert_eq!(count(&db, "track"), 2, "only the Standard tracks remain");
    assert_eq!(count(&db, "album"), 1);
    let left: String = db
        .read(|c| Ok(c.query_row("SELECT id FROM edition", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(left, f.standard);
}

#[test]
fn the_last_edition_and_other_albums_editions_are_refused() {
    let (_d, mut db) = open_temp();
    let f = fixture(&mut db);
    delete(&mut db, &f, &f.deluxe).unwrap();
    let err = delete(&mut db, &f, &f.standard).unwrap_err();
    assert!(err.contains("only edition"), "{err}");
    assert_eq!(count(&db, "edition"), 1);

    let other = fixture(&mut db);
    let err = delete(&mut db, &f, &other.deluxe).unwrap_err();
    assert!(err.contains("different album"), "{err}");
    assert_eq!(count(&db, "edition"), 3);
}

#[test]
fn an_edition_with_listens_ratings_favourites_or_a_review_is_kept() {
    let (_d, mut db) = open_temp();
    let f = fixture(&mut db);

    let listen = log_listen(&mut db, &f.deluxe);
    let err = delete(&mut db, &f, &f.deluxe).unwrap_err();
    assert!(err.contains("1 listen"), "{err}");
    // A listen the user already deleted no longer protects it.
    db.write(|tx| listening::delete(tx, &listen)).unwrap();

    db.write(|tx| ratings::set_track(tx, &f.deluxe_tracks[0], Some(0)))
        .unwrap();
    assert!(
        delete(&mut db, &f, &f.deluxe)
            .unwrap_err()
            .contains("1 rated or liked track")
    );
    db.write(|tx| ratings::set_track(tx, &f.deluxe_tracks[0], None))
        .unwrap();
    db.write(|tx| ratings::set_favourite(tx, &f.deluxe_tracks[1], true))
        .unwrap();
    assert!(
        delete(&mut db, &f, &f.deluxe)
            .unwrap_err()
            .contains("rated or liked")
    );
    db.write(|tx| ratings::set_favourite(tx, &f.deluxe_tracks[1], false))
        .unwrap();

    db.write(|tx| ratings::set_review(tx, &f.deluxe, Some("Great")))
        .unwrap();
    assert!(
        delete(&mut db, &f, &f.deluxe)
            .unwrap_err()
            .contains("rating or review")
    );
    assert_eq!(count(&db, "edition"), 2, "nothing was removed by refusals");
    assert_eq!(count(&db, "album_review"), 1);

    // Once only empty placeholders remain, it can go, taking the deleted listen with it.
    db.write(|tx| ratings::set_review(tx, &f.deluxe, None))
        .unwrap();
    delete(&mut db, &f, &f.deluxe).unwrap();
    assert_eq!(count(&db, "edition"), 1);
    assert_eq!(count(&db, "listen_event"), 0);
    assert_eq!(count(&db, "track_rating"), 0);
    assert_eq!(count(&db, "album_review"), 0);
}

#[test]
fn the_listen_list_collection_and_next_up_history_follow_the_remaining_edition() {
    let (_d, mut db) = open_temp();
    let f = fixture(&mut db);
    db.write(|tx| personal::add_to_listen_list(tx, &f.deluxe))
        .unwrap();
    db.write(|tx| personal::add_to_collection(tx, &f.deluxe, personal::CollectionSource::Manual))
        .unwrap();
    db.write(|tx| {
        let session = selection::start_session(tx, 1, SelectionMode::CompletelyRandom, None)?;
        selection::record_attempt(tx, &session, &f.deluxe, SelectionSource::CompletelyRandom)
    })
    .unwrap();

    delete(&mut db, &f, &f.deluxe).unwrap();

    let on_list: String = db
        .read(|c| {
            Ok(c.query_row(
                "SELECT edition_id FROM listen_list_entry WHERE album_id = ?1",
                [&f.album],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(on_list, f.standard, "the album stays on the Listen List");
    let attempt: String = db
        .read(|c| Ok(c.query_row("SELECT edition_id FROM selection_attempt", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(attempt, f.standard, "Next Up history is kept");
    assert_eq!(count(&db, "collection_entry"), 0, "ownership of it ends");
}

#[test]
fn a_cached_artwork_file_is_reported_for_removal() {
    let (_d, mut db) = open_temp();
    let f = fixture(&mut db);
    db.write(|tx| {
        tx.execute(
            "INSERT INTO artwork (owner_type, owner_id, source, file_name, mime, bytes)
             VALUES ('edition', ?1, 'local', 'edition-x.png', 'image/png', 4)",
            [&f.deluxe],
        )?;
        Ok(())
    })
    .unwrap();
    assert_eq!(
        delete(&mut db, &f, &f.deluxe).unwrap().as_deref(),
        Some("edition-x.png")
    );
    assert_eq!(count(&db, "artwork"), 0);
}
