//! Importing reviewed provider metadata: identity, collisions, locks, and safe refresh.

mod common;

use common::{count, open_temp};
use mudraft_lib::db::Database;
use mudraft_lib::domain::ids::new_id;
use mudraft_lib::domain::rating::HalfStars;
use mudraft_lib::library::metadata_import::{
    self, ImportOutcome, ImportRequest, ManualAlbum, current_genres,
};
use mudraft_lib::library::personal::{self, NewListen};
use mudraft_lib::library::provenance::{self, EntityKind, MetadataSource, Provenance};
use mudraft_lib::metadata::{
    CreditPart, ProviderDate, ReleaseDetail, ReleaseGroupDetail, SourceTag, TrackDetail,
};

const RG: &str = "b1392450-e666-3926-a536-22c65f834433";
const REL: &str = "11111111-1111-4111-8111-111111111111";
const REL_DELUXE: &str = "11111111-1111-4111-8111-222222222222";
const RADIOHEAD: &str = "a74b1b7f-71a5-4011-9441-d0b5e4122711";
const FETCHED: &str = "2026-09-30T12:00:00.000Z";

fn credit(id: &str, name: &str) -> CreditPart {
    CreditPart {
        artist_id: id.into(),
        artist_name: name.into(),
        sort_name: None,
        disambiguation: None,
        credited_name: None,
        join_phrase: String::new(),
    }
}

fn tag(name: &str, votes: i64) -> SourceTag {
    SourceTag {
        name: name.into(),
        votes,
    }
}

fn group() -> ReleaseGroupDetail {
    ReleaseGroupDetail {
        id: RG.into(),
        title: "OK Computer".into(),
        disambiguation: None,
        annotation: None,
        original_date: ProviderDate::parse(Some("1997-05-21")),
        artist_credit: vec![credit(RADIOHEAD, "Radiohead")],
        genres: vec![
            tag("alternative rock", 20),
            tag("art rock", 9),
            tag("electronic", 8),
        ],
        tags: vec![tag("seen live", 40), tag("british", 3)],
    }
}

fn track(n: u32, title: &str) -> TrackDetail {
    TrackDetail {
        id: format!("22222222-2222-4222-8222-{n:012}"),
        disc: 1,
        position: n,
        title: title.into(),
        length_ms: Some(200_000 + i64::from(n)),
        recording_id: Some(format!("33333333-3333-4333-8333-{n:012}")),
        recording_title: Some(title.into()),
        artist_credit: vec![credit(RADIOHEAD, "Radiohead")],
    }
}

fn release(id: &str, tracks: Vec<TrackDetail>) -> ReleaseDetail {
    ReleaseDetail {
        id: id.into(),
        title: "OK Computer".into(),
        release_group_id: Some(RG.into()),
        disambiguation: None,
        date: ProviderDate::parse(Some("1997-06")),
        country: Some("GB".into()),
        formats: vec!["CD".into()],
        artist_credit: vec![credit(RADIOHEAD, "Radiohead")],
        tracks,
    }
}

fn import(
    db: &mut Database,
    rg: &ReleaseGroupDetail,
    rel: &ReleaseDetail,
    name: Option<&str>,
) -> Result<ImportOutcome, mudraft_lib::error::AppError> {
    db.write(|tx| {
        metadata_import::import_release(
            tx,
            &ImportRequest {
                release_group: rg,
                release: rel,
                edition_name: name,
                group_fetched_at: FETCHED,
                release_fetched_at: FETCHED,
            },
        )
    })
}

fn three_tracks() -> Vec<TrackDetail> {
    vec![
        track(1, "Airbag"),
        track(2, "Paranoid Android"),
        track(3, "Subterranean Homesick Alien"),
    ]
}

fn scalar<T: rusqlite::types::FromSql>(db: &Database, sql: &str, p: &[&dyn rusqlite::ToSql]) -> T {
    db.read(|c| Ok(c.query_row(sql, p, |r| r.get(0))?)).unwrap()
}

#[test]
fn fresh_import_creates_identity_provenance_and_separate_raw_tags() {
    let (_d, mut db) = open_temp();
    let out = import(&mut db, &group(), &release(REL, three_tracks()), None).unwrap();
    assert!(out.album_created && out.edition_created);
    assert_eq!(out.genres, vec!["Rock", "Electronic"]);
    assert_eq!(
        db.read(|c| current_genres(c, &out.album_id)).unwrap(),
        vec!["Rock", "Electronic"]
    );

    // Raw provider tags are stored as received, apart from user tags.
    assert_eq!(count(&db, "album_source_tag"), 5);
    assert_eq!(count(&db, "album_tag"), 0);
    // Catalogue import alone creates no personal membership.
    assert_eq!(count(&db, "listen_list_entry"), 0);
    assert_eq!(count(&db, "collection_entry"), 0);

    let (rgid, date, precision): (String, String, String) = db
        .read(|c| Ok(c.query_row("SELECT musicbrainz_release_group_id, original_date, original_date_precision FROM album", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?))
        .unwrap();
    assert_eq!(
        (rgid.as_str(), date.as_str(), precision.as_str()),
        (RG, "1997-05-21", "day")
    );
    let (name, edition_precision): (String, String) = db
        .read(|c| {
            Ok(c.query_row(
                "SELECT name, release_date_precision FROM edition",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .unwrap();
    assert_eq!(
        (name.as_str(), edition_precision.as_str()),
        ("Standard", "month")
    );
    assert_eq!(count(&db, "track"), 3);
    assert_eq!(count(&db, "recording"), 3);
    assert_eq!(
        count(&db, "track_artist_credit"),
        0,
        "track credits equal to the release credit are implied"
    );

    let prov = db
        .read(|c| provenance::for_entity(c, EntityKind::Album, &out.album_id))
        .unwrap();
    let title = prov.iter().find(|p| p.field == "title").unwrap();
    assert_eq!(title.source, MetadataSource::Musicbrainz);
    assert_eq!(title.source_ref.as_deref(), Some(RG));
    let fetched: String = scalar(
        &db,
        "SELECT fetched_at FROM metadata_provenance WHERE entity_type = 'album' AND field = 'title'",
        &[],
    );
    assert_eq!(fetched, FETCHED);
}

#[test]
fn reimport_refreshes_in_place_without_duplicates() {
    let (_d, mut db) = open_temp();
    let first = import(&mut db, &group(), &release(REL, three_tracks()), None).unwrap();
    let mut renamed = group();
    renamed.title = "OK Computer (corrected)".into();
    let again = import(&mut db, &renamed, &release(REL, three_tracks()), None).unwrap();
    assert_eq!(
        (again.album_id.as_str(), again.edition_id.as_str()),
        (first.album_id.as_str(), first.edition_id.as_str())
    );
    assert!(!again.album_created && !again.edition_created);
    for (table, n) in [
        ("artist", 1),
        ("album", 1),
        ("edition", 1),
        ("track", 3),
        ("recording", 3),
        ("album_source_tag", 5),
    ] {
        assert_eq!(count(&db, table), n, "{table}");
    }
    let title: String = scalar(&db, "SELECT title FROM album", &[]);
    assert_eq!(title, "OK Computer (corrected)");
}

#[test]
fn artists_are_matched_by_provider_id_never_by_name() {
    let (_d, mut db) = open_temp();
    // A manual "Radiohead" already exists; import must not merge into it.
    let manual = db
        .write(|tx| {
            metadata_import::add_manual_album(
                tx,
                None,
                &ManualAlbum {
                    title: "Demo".into(),
                    artist_name: Some("Radiohead".into()),
                    artist_id: None,
                    year: None,
                    edition_name: None,
                },
            )
        })
        .unwrap();
    import(&mut db, &group(), &release(REL, three_tracks()), None).unwrap();
    assert_eq!(count(&db, "artist"), 2);
    let manual_mbid: Option<String> = scalar(
        &db,
        "SELECT musicbrainz_id FROM artist WHERE id = ?1",
        &[&manual.artist_id],
    );
    assert_eq!(manual_mbid, None);

    // Two different provider artists sharing a name stay two artists.
    let other = "0e5d1f2a-0000-4000-8000-00000000abcd";
    let mut rg = group();
    rg.id = "b1392450-0000-4000-8000-000000000999".into();
    rg.artist_credit = vec![CreditPart {
        disambiguation: Some("tribute band".into()),
        ..credit(other, "Radiohead")
    }];
    let mut rel = release(
        "11111111-1111-4111-8111-999999999999",
        vec![track(9, "Creep")],
    );
    rel.release_group_id = Some(rg.id.clone());
    import(&mut db, &rg, &rel, None).unwrap();
    let same_name: i64 = scalar(
        &db,
        "SELECT COUNT(*) FROM artist WHERE name = 'Radiohead'",
        &[],
    );
    assert_eq!(same_name, 3);
}

#[test]
fn collisions_are_reported_not_merged() {
    let (_d, mut db) = open_temp();
    import(&mut db, &group(), &release(REL, three_tracks()), None).unwrap();

    // Explicit edition name already used on this album.
    let err = import(
        &mut db,
        &group(),
        &release(REL_DELUXE, vec![track(4, "Lucky")]),
        Some("standard"),
    )
    .unwrap_err();
    assert_eq!(err.code(), "conflict");

    // Release that belongs to a different release group.
    let mut stray = release(REL_DELUXE, vec![]);
    stray.release_group_id = Some("0a1b2c3d-0000-4000-8000-000000000002".into());
    assert_eq!(
        import(&mut db, &group(), &stray, None).unwrap_err().code(),
        "validation"
    );

    // Same release ID already linked to another album.
    let mut other_group = group();
    other_group.id = "0a1b2c3d-0000-4000-8000-000000000002".into();
    let mut moved = release(REL, three_tracks());
    moved.release_group_id = Some(other_group.id.clone());
    assert_eq!(
        import(&mut db, &other_group, &moved, None)
            .unwrap_err()
            .code(),
        "conflict"
    );
    assert_eq!(
        count(&db, "album"),
        1,
        "failed imports leave nothing behind"
    );
    assert_eq!(count(&db, "edition"), 1);
}

#[test]
fn second_edition_gets_distinct_default_name_and_shares_recordings() {
    let (_d, mut db) = open_temp();
    import(&mut db, &group(), &release(REL, three_tracks()), None).unwrap();
    let mut deluxe = release(REL_DELUXE, {
        let mut t = three_tracks();
        for (i, tr) in t.iter_mut().enumerate() {
            tr.id = format!("44444444-4444-4444-8444-{i:012}");
        }
        t.push(track(4, "Lucky"));
        t
    });
    deluxe.disambiguation = Some("20th anniversary".into());
    let out = import(&mut db, &group(), &deluxe, None).unwrap();
    assert!(!out.album_created && out.edition_created);
    let names: Vec<String> = db
        .read(|c| {
            let mut s = c.prepare("SELECT name FROM edition ORDER BY name")?;
            Ok(s.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?)
        })
        .unwrap();
    assert_eq!(names, vec!["20th anniversary", "Standard"]);
    assert_eq!(count(&db, "track"), 7);
    assert_eq!(
        count(&db, "recording"),
        4,
        "same recordings are shared across editions"
    );
}

#[test]
fn refresh_respects_user_locked_fields() {
    let (_d, mut db) = open_temp();
    let out = import(&mut db, &group(), &release(REL, three_tracks()), None).unwrap();
    db.write(|tx| {
        metadata_import::set_album_genres_manually(tx, &out.album_id, &["Jazz".into()])?;
        tx.execute(
            "UPDATE album SET title = 'My Title' WHERE id = ?1",
            [&out.album_id],
        )?;
        provenance::set(
            tx,
            &Provenance {
                entity: EntityKind::Album,
                entity_id: out.album_id.clone(),
                field: "title".into(),
                source: MetadataSource::Manual,
                source_ref: None,
                provider_value: None,
                is_override: true,
            },
        )
    })
    .unwrap();

    let mut updated = group();
    updated.title = "OK Computer (provider change)".into();
    let refreshed = import(&mut db, &updated, &release(REL, three_tracks()), None).unwrap();
    assert_eq!(
        refreshed.locked_fields_kept,
        vec!["album.genres", "album.title"]
    );
    assert_eq!(refreshed.genres, vec!["Jazz"]);
    let title: String = scalar(&db, "SELECT title FROM album", &[]);
    assert_eq!(title, "My Title");
    let (source, lock, provider_value): (String, bool, String) = db
        .read(|c| Ok(c.query_row("SELECT source, is_override, provider_value FROM metadata_provenance WHERE entity_type = 'album' AND field = 'title'", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?))
        .unwrap();
    assert_eq!((source.as_str(), lock), ("manual", true));
    assert_eq!(
        provider_value, "OK Computer (provider change)",
        "provider's latest value is kept for reference"
    );
    // Raw tags still refresh even though normalized genres are locked.
    assert_eq!(count(&db, "album_source_tag"), 5);
}

#[test]
fn refresh_never_drops_or_remaps_rated_or_listened_tracks() {
    let (_d, mut db) = open_temp();
    let out = import(&mut db, &group(), &release(REL, three_tracks()), None).unwrap();
    let id_of = |db: &Database, pos: i64| -> String {
        scalar(db, "SELECT id FROM track WHERE position = ?1", &[&pos])
    };
    let (t1, t2, t3) = (id_of(&db, 1), id_of(&db, 2), id_of(&db, 3));
    db.write(|tx| {
        personal::set_track_rating(tx, &t3, Some(HalfStars::new(9)?))?;
        personal::record_listen(
            tx,
            None,
            &NewListen {
                edition_id: out.edition_id.clone(),
                listened_on: Some("2026-09-29".into()),
                is_full: false,
                track_ids: vec![t2.clone()],
            },
        )?;
        Ok(())
    })
    .unwrap();

    // Provider now lists: new track at 1, old track 1 moved to 2; tracks 2 and 3 gone.
    let mut moved = track(1, "Airbag");
    moved.position = 2;
    let refreshed = import(
        &mut db,
        &group(),
        &release(REL, vec![track(7, "New Opener").with_position(1), moved]),
        None,
    )
    .unwrap();
    assert_eq!(refreshed.retained_tracks, 2);

    let tracks: Vec<(String, i64, String)> = db
        .read(|c| {
            let mut s = c.prepare("SELECT id, position, title FROM track ORDER BY position")?;
            Ok(s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<Result<_, _>>()?)
        })
        .unwrap();
    assert_eq!(tracks.len(), 4);
    assert_eq!(
        tracks[1],
        (t1.clone(), 2, "Airbag".into()),
        "matched by provider ID, same local ID"
    );
    assert_eq!(
        (tracks[2].0.as_str(), tracks[3].0.as_str()),
        (t2.as_str(), t3.as_str()),
        "kept after the provider's tracks, in order"
    );
    let rating: i64 = scalar(
        &db,
        "SELECT rating FROM track_rating WHERE track_id = ?1",
        &[&t3],
    );
    assert_eq!(rating, 9);
    let evidence: String = scalar(&db, "SELECT track_id FROM listen_event_track", &[]);
    assert_eq!(evidence, t2);
}

#[test]
fn invalid_provider_data_rolls_back_the_whole_import() {
    let (_d, mut db) = open_temp();
    let dup = vec![track(1, "A"), track(2, "B").with_position(1)];
    assert_eq!(
        import(&mut db, &group(), &release(REL, dup), None)
            .unwrap_err()
            .code(),
        "provider_error"
    );
    for table in [
        "artist",
        "album",
        "edition",
        "track",
        "album_source_tag",
        "metadata_provenance",
    ] {
        assert_eq!(count(&db, table), 0, "{table}");
    }
}

#[test]
fn manual_entries_never_merge_and_genres_can_be_corrected() {
    let (_d, mut db) = open_temp();
    let key = new_id();
    let input = ManualAlbum {
        title: "Untitled Demo".into(),
        artist_name: Some("Band".into()),
        artist_id: None,
        year: Some(2003),
        edition_name: None,
    };
    let a = db
        .write(|tx| metadata_import::add_manual_album(tx, Some(&key), &input))
        .unwrap();
    let retry = db
        .write(|tx| metadata_import::add_manual_album(tx, Some(&key), &input))
        .unwrap();
    assert_eq!(a, retry);
    let b = db
        .write(|tx| metadata_import::add_manual_album(tx, None, &input))
        .unwrap();
    assert_ne!(
        a.artist_id, b.artist_id,
        "same name, still a separate artist unless chosen"
    );
    let chosen = ManualAlbum {
        artist_id: Some(a.artist_id.clone()),
        artist_name: None,
        ..input.clone()
    };
    let c = db
        .write(|tx| metadata_import::add_manual_album(tx, None, &chosen))
        .unwrap();
    assert_eq!(c.artist_id, a.artist_id);

    let (date, precision): (String, String) = db
        .read(|c2| {
            Ok(c2.query_row(
                "SELECT original_date, original_date_precision FROM album WHERE id = ?1",
                [&a.album_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .unwrap();
    assert_eq!((date.as_str(), precision.as_str()), ("2003", "year"));

    let none = ManualAlbum {
        artist_name: Some("  ".into()),
        ..input.clone()
    };
    assert_eq!(
        db.write(|tx| metadata_import::add_manual_album(tx, None, &none))
            .unwrap_err()
            .code(),
        "validation"
    );

    let set = db
        .write(|tx| {
            metadata_import::set_album_genres_manually(
                tx,
                &a.album_id,
                &["hip hop".into(), "Jazz".into()],
            )
        })
        .unwrap();
    assert_eq!(set, vec!["Hip Hop", "Jazz"]);
    let bad = db.write(|tx| {
        metadata_import::set_album_genres_manually(tx, &a.album_id, &["Vaporwave".into()])
    });
    assert_eq!(bad.unwrap_err().code(), "validation");
    assert_eq!(
        db.read(|c2| current_genres(c2, &a.album_id)).unwrap(),
        vec!["Hip Hop", "Jazz"]
    );
}

trait WithPosition {
    fn with_position(self, position: u32) -> Self;
}

impl WithPosition for TrackDetail {
    fn with_position(mut self, position: u32) -> Self {
        self.position = position;
        self
    }
}
