//! Persistence contracts for catalogue, personal data, and selection history.

mod common;

use common::{album, artist, count, credit, edition, new_edition, open_temp, track};
use mudraft_lib::domain::ids::new_id;
use mudraft_lib::domain::picker::GuidedCriteria;
use mudraft_lib::domain::rating::HalfStars;
use mudraft_lib::library::catalogue::{self, LISTEN_ASAP_TAG_ID, NewAlbum};
use mudraft_lib::library::personal::{self, CollectionSource, NewListen};
use mudraft_lib::library::provenance::{self, EntityKind, MetadataSource, Provenance};
use mudraft_lib::library::selection::{self, AttemptStatus, SelectionMode, SelectionSource};
use mudraft_lib::library::settings::{self, Settings};

fn hs(v: i64) -> Option<HalfStars> {
    Some(HalfStars::new(v).unwrap())
}

#[test]
fn failed_multi_record_operation_rolls_back_entirely() {
    let (_d, mut db) = open_temp();
    let a = artist(&mut db, "Artist");
    let al = album(&mut db, "Album", &[&a]);
    // Second track duplicates position 1, so the whole edition must be rejected.
    let input = new_edition(
        &al,
        "Standard",
        vec![track(1, "One"), track(2, "Two"), track(1, "Dup")],
    );
    let err = db
        .write(|tx| catalogue::create_edition(tx, None, &input))
        .unwrap_err();
    assert_eq!(err.code(), "validation");
    assert_eq!(count(&db, "edition"), 0);
    assert_eq!(count(&db, "track"), 0);

    // A failure after earlier writes inside one `write` also rolls those writes back.
    let (ed, tracks) = edition(&mut db, &al, "Standard", 2);
    let err = db
        .write(|tx| {
            personal::add_to_listen_list(tx, &ed)?;
            personal::set_track_rating(tx, &tracks[0], hs(8))?;
            personal::set_track_rating(tx, "missing-track", hs(8))
        })
        .unwrap_err();
    assert_eq!(err.code(), "not_found");
    assert_eq!(count(&db, "listen_list_entry"), 0);
    assert_eq!(count(&db, "track_rating"), 0);
}

#[test]
fn invalid_references_are_rejected() {
    let (_d, mut db) = open_temp();
    let missing = new_id();
    let input = new_edition(&missing, "Standard", vec![]);
    assert_eq!(
        db.write(|tx| catalogue::create_edition(tx, None, &input))
            .unwrap_err()
            .code(),
        "not_found"
    );

    let bad_album = NewAlbum {
        title: "X".into(),
        original_date: None,
        musicbrainz_release_group_id: None,
        credits: vec![credit(&missing)],
        genres: vec![],
    };
    assert_eq!(
        db.write(|tx| catalogue::create_album(tx, None, &bad_album))
            .unwrap_err()
            .code(),
        "not_found"
    );
    assert_eq!(
        db.write(|tx| personal::add_to_listen_list(tx, &missing))
            .unwrap_err()
            .code(),
        "not_found"
    );
    assert_eq!(count(&db, "album"), 0);

    // SQLite enforces the edition↔album pairing even if Rust validation were bypassed.
    let a = artist(&mut db, "A");
    let al1 = album(&mut db, "One", &[&a]);
    let al2 = album(&mut db, "Two", &[&a]);
    let (ed1, _) = edition(&mut db, &al1, "Standard", 1);
    let err = db
        .write(|tx| {
            Ok(tx.execute(
                "INSERT INTO listen_list_entry (album_id, edition_id) VALUES (?1, ?2)",
                [&al2, &ed1],
            )?)
        })
        .unwrap_err();
    assert_eq!(err.code(), "validation");

    // Listen evidence must come from the listened edition.
    let (_ed2, other_tracks) = edition(&mut db, &al2, "Standard", 1);
    let listen = NewListen {
        edition_id: ed1.clone(),
        listened_on: Some("2026-09-30".into()),
        is_full: false,
        track_ids: other_tracks,
    };
    assert_eq!(
        db.write(|tx| personal::record_listen(tx, None, &listen))
            .unwrap_err()
            .code(),
        "validation"
    );
    assert_eq!(count(&db, "listen_event"), 0);
}

#[test]
fn zero_is_a_valid_rating_distinct_from_unrated() {
    let (_d, mut db) = open_temp();
    let a = artist(&mut db, "A");
    let al = album(&mut db, "Album", &[&a]);
    let (ed, tracks) = edition(&mut db, &al, "Standard", 3);

    db.write(|tx| {
        personal::set_track_rating(tx, &tracks[0], hs(0))?;
        personal::set_track_rating(tx, &tracks[1], hs(0))
    })
    .unwrap();
    assert_eq!(
        db.read(|c| personal::track_rating(c, &tracks[0]))
            .unwrap()
            .rating,
        hs(0)
    );
    assert_eq!(
        db.read(|c| personal::track_rating(c, &tracks[2]))
            .unwrap()
            .rating,
        None
    );
    // Computed from tracks (unrated third ignored), and not stored as an explicit rating.
    assert_eq!(
        db.read(|c| personal::effective_rating(c, &ed)).unwrap(),
        hs(0)
    );
    assert_eq!(
        db.read(|c| personal::album_review(c, &ed)).unwrap().rating,
        None
    );

    db.write(|tx| personal::set_album_rating(tx, &ed, hs(0)))
        .unwrap();
    db.write(|tx| personal::set_track_rating(tx, &tracks[2], hs(10)))
        .unwrap();
    assert_eq!(
        db.read(|c| personal::effective_rating(c, &ed)).unwrap(),
        hs(0),
        "explicit zero wins"
    );

    db.write(|tx| personal::set_album_rating(tx, &ed, None))
        .unwrap();
    assert_eq!(
        db.read(|c| personal::effective_rating(c, &ed)).unwrap(),
        hs(3)
    ); // (0+0+10)/3 = 3.33

    let raw = db.write(|tx| Ok(tx.execute("UPDATE track_rating SET rating = 11", [])?));
    assert!(raw.is_err(), "schema rejects out-of-range ratings");
    assert!(HalfStars::new(11).is_err());
}

#[test]
fn two_editions_share_album_tags_but_keep_their_own_personal_data() {
    let (_d, mut db) = open_temp();
    let a = artist(&mut db, "A");
    let al = album(&mut db, "Album", &[&a]);
    let (standard, std_tracks) = edition(&mut db, &al, "Standard", 10);
    let (deluxe, dlx_tracks) = edition(&mut db, &al, "Deluxe", 14);

    let editions = db.read(|c| catalogue::editions_of_album(c, &al)).unwrap();
    assert_eq!(editions.len(), 2);
    assert_eq!(editions[0].release_date_precision, "year");
    assert_eq!(std_tracks.len(), 10);
    assert_eq!(dlx_tracks.len(), 14);

    // An accidental second "standard" edition is refused.
    let dup = new_edition(&al, "standard", vec![]);
    assert_eq!(
        db.write(|tx| catalogue::create_edition(tx, None, &dup))
            .unwrap_err()
            .code(),
        "conflict"
    );

    db.write(|tx| catalogue::tag_album(tx, &al, LISTEN_ASAP_TAG_ID))
        .unwrap();
    db.write(|tx| catalogue::tag_album(tx, &al, LISTEN_ASAP_TAG_ID))
        .unwrap(); // idempotent
    let tags = db.read(|c| catalogue::album_tags(c, &al)).unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].builtin_key.as_deref(), Some("listen_asap"));

    // One Listen List entry per album; re-adding with the other edition switches it.
    db.write(|tx| personal::add_to_listen_list(tx, &standard))
        .unwrap();
    db.write(|tx| personal::add_to_listen_list(tx, &deluxe))
        .unwrap();
    let list = db.read(personal::listen_list).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].edition_id, deluxe);

    // Ratings are per edition.
    db.write(|tx| personal::set_album_rating(tx, &standard, hs(6)))
        .unwrap();
    db.write(|tx| personal::set_album_rating(tx, &deluxe, hs(9)))
        .unwrap();
    assert_eq!(
        db.read(|c| personal::effective_rating(c, &standard))
            .unwrap(),
        hs(6)
    );
    assert_eq!(
        db.read(|c| personal::effective_rating(c, &deluxe)).unwrap(),
        hs(9)
    );

    // Both editions may be collected deliberately.
    db.write(|tx| {
        personal::add_to_collection(tx, &standard, CollectionSource::Manual)?;
        personal::add_to_collection(tx, &deluxe, CollectionSource::Manual)?;
        personal::add_to_collection(tx, &deluxe, CollectionSource::Manual)
    })
    .unwrap();
    assert_eq!(db.read(personal::collection_edition_ids).unwrap().len(), 2);
}

#[test]
fn shared_artists_and_ordered_collaboration_credits() {
    let (_d, mut db) = open_temp();
    let bowie = artist(&mut db, "David Bowie");
    let queen = artist(&mut db, "Queen");
    let solo = album(&mut db, "Low", &[&bowie]);
    let collab = album(&mut db, "Under Pressure", &[&queen, &bowie]);

    assert_eq!(
        db.read(|c| catalogue::albums_for_artist(c, &bowie))
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        db.read(|c| catalogue::albums_for_artist(c, &queen))
            .unwrap(),
        vec![collab.clone()]
    );
    assert_eq!(
        db.read(|c| catalogue::album_credit_artists(c, &collab))
            .unwrap(),
        vec![queen.clone(), bowie.clone()]
    );
    assert_eq!(
        db.read(|c| catalogue::album_credit_artists(c, &solo))
            .unwrap(),
        vec![bowie.clone()]
    );
    assert_eq!(count(&db, "artist"), 2);

    let twice = NewAlbum {
        title: "Dup".into(),
        original_date: None,
        musicbrainz_release_group_id: None,
        credits: vec![credit(&bowie), credit(&bowie)],
        genres: vec![],
    };
    assert_eq!(
        db.write(|tx| catalogue::create_album(tx, None, &twice))
            .unwrap_err()
            .code(),
        "validation"
    );
    // A credited artist cannot be deleted out from under their albums.
    assert!(
        db.write(|tx| Ok(tx.execute("DELETE FROM artist WHERE id = ?1", [&bowie])?))
            .is_err()
    );
}

#[test]
fn catalogue_metadata_alone_creates_no_personal_membership() {
    let (_d, mut db) = open_temp();
    let a = artist(&mut db, "A");
    let al = album(&mut db, "Album", &[&a]);
    edition(&mut db, &al, "Standard", 5);
    for table in [
        "listen_list_entry",
        "collection_entry",
        "listen_event",
        "track_rating",
        "album_review",
        "album_tag",
    ] {
        assert_eq!(count(&db, table), 0, "{table}");
    }
}

#[test]
fn record_listen_is_atomic_idempotent_and_dated_by_calendar() {
    let (_d, mut db) = open_temp();
    let a = artist(&mut db, "A");
    let al = album(&mut db, "Album", &[&a]);
    let (ed, tracks) = edition(&mut db, &al, "Standard", 3);
    db.write(|tx| personal::add_to_listen_list(tx, &ed))
        .unwrap();

    let key = new_id();
    let input = NewListen {
        edition_id: ed.clone(),
        listened_on: Some("2026-03-29".into()),
        is_full: true,
        track_ids: tracks.clone(),
    };
    let first = db
        .write(|tx| personal::record_listen(tx, Some(&key), &input))
        .unwrap();
    let retry = db
        .write(|tx| personal::record_listen(tx, Some(&key), &input))
        .unwrap();
    assert_eq!(first, retry);
    assert!(first.removed_from_listen_list);
    assert_eq!(count(&db, "listen_event"), 1);
    assert_eq!(count(&db, "listen_event_track"), 3);
    assert_eq!(
        db.read(personal::collection_edition_ids).unwrap(),
        vec![ed.clone()]
    );
    assert!(db.read(personal::listen_list).unwrap().is_empty());

    // Stored exactly as the local calendar date, not shifted through UTC.
    let stored: String = db
        .read(|c| Ok(c.query_row("SELECT listened_on FROM listen_event", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(stored, "2026-03-29");

    let changed = NewListen {
        is_full: false,
        ..input.clone()
    };
    assert_eq!(
        db.write(|tx| personal::record_listen(tx, Some(&key), &changed))
            .unwrap_err()
            .code(),
        "conflict"
    );

    // Undated prior history is distinct from a known date; repeated listens keep one collection entry.
    let undated = NewListen {
        edition_id: ed.clone(),
        listened_on: None,
        is_full: true,
        track_ids: vec![],
    };
    db.write(|tx| personal::record_listen(tx, None, &undated))
        .unwrap();
    let statuses: Vec<String> = db
        .read(|c| {
            let mut s = c.prepare("SELECT date_status FROM listen_event ORDER BY date_status")?;
            Ok(s.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?)
        })
        .unwrap();
    assert_eq!(statuses, vec!["known", "undated"]);
    assert_eq!(count(&db, "collection_entry"), 1);

    let bad = NewListen {
        listened_on: Some("2026-02-30".into()),
        ..undated
    };
    assert_eq!(
        db.write(|tx| personal::record_listen(tx, None, &bad))
            .unwrap_err()
            .code(),
        "validation"
    );
}

#[test]
fn full_listen_keeps_list_membership_when_setting_disabled() {
    let (_d, mut db) = open_temp();
    let a = artist(&mut db, "A");
    let al = album(&mut db, "Album", &[&a]);
    let (ed, _) = edition(&mut db, &al, "Standard", 1);
    assert_eq!(db.read(settings::load).unwrap(), Settings::default());
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
    let input = NewListen {
        edition_id: ed,
        listened_on: None,
        is_full: true,
        track_ids: vec![],
    };
    let res = db
        .write(|tx| personal::record_listen(tx, None, &input))
        .unwrap();
    assert!(!res.removed_from_listen_list);
    assert_eq!(db.read(personal::listen_list).unwrap().len(), 1);
}

#[test]
fn idempotent_creation_with_mutation_key() {
    let (_d, mut db) = open_temp();
    let a = artist(&mut db, "A");
    let input = NewAlbum {
        title: "Album".into(),
        original_date: Some("1970".into()),
        musicbrainz_release_group_id: None,
        credits: vec![credit(&a)],
        genres: vec!["Rock".into(), "rock".into(), "Jazz".into()],
    };
    let key = new_id();
    let first = db
        .write(|tx| catalogue::create_album(tx, Some(&key), &input))
        .unwrap();
    let again = db
        .write(|tx| catalogue::create_album(tx, Some(&key), &input))
        .unwrap();
    assert_eq!(first, again);
    assert_eq!(count(&db, "album"), 1);
    assert_eq!(
        count(&db, "album_genre"),
        2,
        "genres deduplicated case-insensitively"
    );
    assert_eq!(
        db.write(|tx| catalogue::create_album(tx, Some("not-a-uuid"), &input))
            .unwrap_err()
            .code(),
        "validation"
    );

    let four = NewAlbum {
        genres: vec!["A".into(), "B".into(), "C".into(), "D".into()],
        ..input
    };
    assert_eq!(
        db.write(|tx| catalogue::create_album(tx, None, &four))
            .unwrap_err()
            .code(),
        "validation"
    );
}

#[test]
fn selection_sessions_track_attempts_and_single_current_album() {
    let (_d, mut db) = open_temp();
    let a = artist(&mut db, "A");
    let al1 = album(&mut db, "One", &[&a]);
    let al2 = album(&mut db, "Two", &[&a]);
    let (ed1, _) = edition(&mut db, &al1, "Standard", 1);
    let (ed2, _) = edition(&mut db, &al2, "Standard", 1);

    let session = db
        .write(|tx| selection::start_session(tx, SelectionMode::CompletelyRandom, None))
        .unwrap();
    let first = db
        .write(|tx| {
            selection::record_attempt(tx, &session, &ed1, SelectionSource::CompletelyRandom)
        })
        .unwrap();
    let second = db
        .write(|tx| {
            selection::record_attempt(tx, &session, &ed2, SelectionSource::CompletelyRandom)
        })
        .unwrap();

    let current = db.read(selection::current_selection).unwrap().unwrap();
    assert_eq!(current.attempt_id, second);
    assert_eq!(current.edition_id, ed2);
    assert_eq!(current.status, AttemptStatus::Shown);
    assert_eq!(count(&db, "current_selection"), 1);

    // The reroll skipped the first; it cannot repeat until the pool cycle restarts.
    assert_eq!(
        db.write(|tx| selection::resolve_attempt(tx, &first, AttemptStatus::Completed))
            .unwrap_err()
            .code(),
        "conflict"
    );
    let repeat = db.write(|tx| {
        selection::record_attempt(tx, &session, &ed1, SelectionSource::CompletelyRandom)
    });
    assert_eq!(repeat.unwrap_err().code(), "conflict");
    assert_eq!(
        db.write(|tx| selection::reset_pool(tx, &session)).unwrap(),
        2
    );
    db.write(|tx| selection::record_attempt(tx, &session, &ed1, SelectionSource::CompletelyRandom))
        .unwrap();

    let current = db.read(selection::current_selection).unwrap().unwrap();
    db.write(|tx| selection::resolve_attempt(tx, &current.attempt_id, AttemptStatus::Completed))
        .unwrap();
    db.write(|tx| selection::resolve_attempt(tx, &current.attempt_id, AttemptStatus::Completed))
        .unwrap();
    assert_eq!(
        db.read(selection::current_selection)
            .unwrap()
            .unwrap()
            .status,
        AttemptStatus::Completed
    );

    assert_eq!(
        db.write(|tx| selection::start_session(tx, SelectionMode::Guided, None))
            .unwrap_err()
            .code(),
        "validation"
    );
    let criteria = GuidedCriteria::default();
    db.write(|tx| selection::start_session(tx, SelectionMode::Guided, Some(&criteria)))
        .unwrap();
}

#[test]
fn provenance_records_sources_and_overrides() {
    let (_d, mut db) = open_temp();
    let a = artist(&mut db, "A");
    let al = album(&mut db, "Album", &[&a]);
    let p = Provenance {
        entity: EntityKind::Album,
        entity_id: al.clone(),
        field: "title".into(),
        source: MetadataSource::Musicbrainz,
        source_ref: Some(new_id()),
        provider_value: Some("Album (Remastered)".into()),
        is_override: true,
    };
    db.write(|tx| provenance::set(tx, &p)).unwrap();
    db.write(|tx| provenance::set(tx, &p)).unwrap();
    assert_eq!(
        db.read(|c| provenance::for_entity(c, EntityKind::Album, &al))
            .unwrap(),
        vec![p.clone()]
    );

    let bad_field = Provenance {
        field: "colour".into(),
        ..p.clone()
    };
    assert_eq!(
        db.write(|tx| provenance::set(tx, &bad_field))
            .unwrap_err()
            .code(),
        "validation"
    );
    let missing = Provenance {
        entity_id: new_id(),
        ..p
    };
    assert_eq!(
        db.write(|tx| provenance::set(tx, &missing))
            .unwrap_err()
            .code(),
        "not_found"
    );
}
