//! Album detail, manual edits, and deliberate edition management.

mod common;

use common::{count, open_temp};
use mudraft_lib::db::Database;
use mudraft_lib::domain::rating::HalfStars;
use mudraft_lib::library::album::{self, AlbumEdit};
use mudraft_lib::library::editions;
use mudraft_lib::library::metadata_import::{self, ImportRequest};
use mudraft_lib::library::personal::{self, NewListen};
use mudraft_lib::metadata::{
    CreditPart, ProviderDate, ReleaseDetail, ReleaseGroupDetail, TrackDetail,
};

const RG: &str = "b1392450-e666-3926-a536-22c65f834433";
const STD: &str = "11111111-1111-4111-8111-000000000001";
const DLX: &str = "11111111-1111-4111-8111-000000000002";
const ARTIST: &str = "a74b1b7f-71a5-4011-9441-d0b5e4122711";

fn credit() -> CreditPart {
    CreditPart {
        artist_id: ARTIST.into(),
        artist_name: "Radiohead".into(),
        sort_name: None,
        disambiguation: None,
        credited_name: None,
        join_phrase: String::new(),
    }
}

fn group(annotation: Option<&str>) -> ReleaseGroupDetail {
    ReleaseGroupDetail {
        id: RG.into(),
        title: "OK Computer".into(),
        disambiguation: None,
        annotation: annotation.map(Into::into),
        original_date: ProviderDate::parse(Some("1997-05-21")),
        artist_credit: vec![credit()],
        genres: vec![],
        tags: vec![],
    }
}

/// Track on `disc` at `pos` with recording `rec` (None = unknown recording).
fn track(
    id: u32,
    disc: u32,
    pos: u32,
    title: &str,
    rec: Option<u32>,
    len: Option<i64>,
) -> TrackDetail {
    TrackDetail {
        id: format!("22222222-2222-4222-8222-{id:012}"),
        disc,
        position: pos,
        title: title.into(),
        length_ms: len,
        recording_id: rec.map(|r| format!("33333333-3333-4333-8333-{r:012}")),
        recording_title: None,
        artist_credit: vec![credit()],
    }
}

fn release(id: &str, tracks: Vec<TrackDetail>) -> ReleaseDetail {
    ReleaseDetail {
        id: id.into(),
        title: "OK Computer".into(),
        release_group_id: Some(RG.into()),
        disambiguation: None,
        date: ProviderDate::parse(Some("1997")),
        country: None,
        formats: vec!["CD".into()],
        artist_credit: vec![credit()],
        tracks,
    }
}

fn standard_tracks() -> Vec<TrackDetail> {
    vec![
        track(1, 1, 1, "Airbag", Some(1), Some(284_000)),
        track(2, 1, 2, "Paranoid Android", Some(2), Some(383_000)),
        track(3, 1, 3, "Lucky", Some(3), None),
    ]
}

/// Deluxe: same recordings plus a live version (different recording) and a bonus track,
/// in a different order — positions deliberately don't line up with Standard.
fn deluxe_tracks() -> Vec<TrackDetail> {
    vec![
        track(11, 1, 1, "Paranoid Android", Some(2), Some(383_000)),
        track(12, 1, 2, "Airbag", Some(1), Some(284_000)),
        track(13, 1, 3, "Lucky", Some(3), Some(259_000)),
        track(14, 2, 1, "Lucky (live)", Some(30), Some(270_000)),
        track(15, 2, 2, "Lull", Some(4), Some(200_000)),
    ]
}

fn import(
    db: &mut Database,
    g: &ReleaseGroupDetail,
    r: &ReleaseDetail,
    name: Option<&str>,
) -> (String, String) {
    let out = db
        .write(|tx| {
            metadata_import::import_release(
                tx,
                &ImportRequest {
                    release_group: g,
                    release: r,
                    edition_name: name,
                    group_fetched_at: "2026-10-01T00:00:00.000Z",
                    release_fetched_at: "2026-10-01T00:00:00.000Z",
                },
            )
        })
        .unwrap();
    (out.album_id, out.edition_id)
}

#[test]
fn standard_and_deluxe_tracks_stay_separate_and_multi_disc_renders() {
    let (_d, mut db) = open_temp();
    let (album_id, std_ed) = import(
        &mut db,
        &group(None),
        &release(STD, standard_tracks()),
        None,
    );
    db.read(|c| editions::check_new_edition(c, &album_id, &release(DLX, deluxe_tracks())))
        .unwrap();
    let (_, dlx_ed) = import(
        &mut db,
        &group(None),
        &release(DLX, deluxe_tracks()),
        Some("Deluxe"),
    );
    assert_eq!(count(&db, "track"), 8, "each edition keeps its own tracks");

    let std_view = db
        .read(|c| album::detail(c, &album_id, Some(&std_ed)))
        .unwrap();
    assert_eq!(std_view.discs.len(), 1);
    assert_eq!(std_view.runtime.known_ms, 667_000);
    assert_eq!(
        std_view.runtime.unknown_tracks, 1,
        "missing durations stay unknown, not zero"
    );
    assert_eq!(std_view.discs[0].tracks[2].length_ms, None);

    let dlx_view = db
        .read(|c| album::detail(c, &album_id, Some(&dlx_ed)))
        .unwrap();
    assert_eq!(
        dlx_view
            .discs
            .iter()
            .map(|d| (d.number, d.tracks.len()))
            .collect::<Vec<_>>(),
        vec![(1, 3), (2, 2)]
    );
    assert_eq!(dlx_view.editions.len(), 2);
    assert!(dlx_view.discs[1].tracks[0].title.contains("live"));
    assert_eq!(dlx_view.runtime.unknown_tracks, 0);
}

#[test]
fn long_albums_and_missing_description_use_a_factual_summary() {
    let (_d, mut db) = open_temp();
    let many: Vec<TrackDetail> = (1..=150)
        .map(|i| {
            track(
                100 + i,
                1 + (i - 1) / 50,
                1 + (i - 1) % 50,
                &format!("Part {i}"),
                Some(100 + i),
                Some(60_000),
            )
        })
        .collect();
    let (album_id, _) = import(&mut db, &group(None), &release(STD, many), None);
    let view = db.read(|c| album::detail(c, &album_id, None)).unwrap();
    assert_eq!(view.discs.len(), 3);
    assert_eq!(view.runtime.known_ms, 9_000_000);
    assert_eq!(view.description, None);
    assert_eq!(
        view.summary,
        "“OK Computer” by Radiohead, first released 1997-05-21. The Standard edition has 150 tracks on 3 discs (2:30:00)."
    );
}

#[test]
fn sourced_descriptions_refresh_but_manual_edits_survive_refresh_and_restart() {
    let (dir, mut db) = open_temp();
    let (album_id, _) = import(
        &mut db,
        &group(Some("Third studio album.")),
        &release(STD, standard_tracks()),
        None,
    );
    let view = db.read(|c| album::detail(c, &album_id, None)).unwrap();
    let d = view.description.unwrap();
    assert_eq!(
        (d.text.as_str(), d.source.as_str()),
        ("Third studio album.", "musicbrainz")
    );

    db.write(|tx| {
        album::edit(
            tx,
            &album_id,
            &AlbumEdit {
                title: Some("OK Computer (my copy)".into()),
                original_date: Some("1997-06".into()),
                description: Some("Bought on cassette.".into()),
            },
        )
    })
    .unwrap();
    import(
        &mut db,
        &group(Some("Updated annotation.")),
        &release(STD, standard_tracks()),
        None,
    );
    drop(db);
    let db = Database::open(&dir.path().join("mudraft.sqlite3")).unwrap();
    let view = db.read(|c| album::detail(c, &album_id, None)).unwrap();
    assert_eq!(view.title, "OK Computer (my copy)");
    assert_eq!(
        (
            view.original_date.value.as_deref(),
            view.original_date.precision.as_str()
        ),
        (Some("1997-06"), "month")
    );
    let d = view.description.unwrap();
    assert_eq!(
        (d.text.as_str(), d.source.as_str()),
        ("Bought on cassette.", "manual")
    );
    assert_eq!(
        view.locked_fields,
        vec!["description", "original_date", "title"]
    );

    let mut db = db;
    db.write(|tx| album::unlock(tx, &album_id, "description"))
        .unwrap();
    import(
        &mut db,
        &group(Some("Updated annotation.")),
        &release(STD, standard_tracks()),
        None,
    );
    let view = db.read(|c| album::detail(c, &album_id, None)).unwrap();
    assert_eq!(view.description.unwrap().text, "Updated annotation.");
    assert_eq!(
        db.write(|tx| album::edit(
            tx,
            &album_id,
            &AlbumEdit {
                original_date: Some("1997-13".into()),
                ..Default::default()
            }
        ))
        .unwrap_err()
        .code(),
        "validation"
    );
}

#[test]
fn ordinary_pressings_are_refused_but_different_recordings_are_allowed() {
    let (_d, mut db) = open_temp();
    let (album_id, _) = import(
        &mut db,
        &group(None),
        &release(STD, standard_tracks()),
        None,
    );
    // Same release again.
    let again =
        db.read(|c| editions::check_new_edition(c, &album_id, &release(STD, standard_tracks())));
    assert!(
        again
            .unwrap_err()
            .to_string()
            .contains("already in your library as the “Standard” edition")
    );
    // A different pressing with the same recordings in the same order.
    let pressing: Vec<TrackDetail> = standard_tracks()
        .into_iter()
        .map(|mut t| {
            t.id = t.id.replace("22222222", "44444444");
            t
        })
        .collect();
    let err = db
        .read(|c| {
            editions::check_new_edition(
                c,
                &album_id,
                &release("11111111-1111-4111-8111-000000000009", pressing),
            )
        })
        .unwrap_err();
    assert_eq!(err.code(), "conflict");
    assert!(err.to_string().contains("ordinary pressing"));
    // A live/remix version uses different recordings: allowed.
    let mut live = standard_tracks();
    live[2].recording_id = Some("33333333-3333-4333-8333-000000000099".into());
    live.iter_mut()
        .for_each(|t| t.id = t.id.replace("22222222", "55555555"));
    db.read(|c| {
        editions::check_new_edition(
            c,
            &album_id,
            &release("11111111-1111-4111-8111-000000000010", live),
        )
    })
    .unwrap();
    // A release of a different album.
    let mut other = release(DLX, deluxe_tracks());
    other.release_group_id = Some("0a1b2c3d-0000-4000-8000-000000000002".into());
    assert_eq!(
        db.read(|c| editions::check_new_edition(c, &album_id, &other))
            .unwrap_err()
            .code(),
        "validation"
    );
}

#[test]
fn switching_editions_matches_recordings_never_positions() {
    let (_d, mut db) = open_temp();
    let (album_id, std_ed) = import(
        &mut db,
        &group(None),
        &release(STD, standard_tracks()),
        None,
    );
    let (_, dlx_ed) = import(
        &mut db,
        &group(None),
        &release(DLX, deluxe_tracks()),
        Some("Deluxe"),
    );
    let view = db
        .read(|c| album::detail(c, &album_id, Some(&std_ed)))
        .unwrap();
    let t = |i: usize| view.discs[0].tracks[i].id.clone();
    db.write(|tx| {
        personal::add_to_listen_list(tx, &std_ed)?;
        personal::set_track_rating(tx, &t(0), Some(HalfStars::new(8)?))?; // Airbag
        personal::set_track_favourite(tx, &t(1), true)?; // Paranoid Android
        personal::set_track_rating(tx, &t(2), Some(HalfStars::new(0)?))?; // Lucky (studio)
        personal::record_listen(
            tx,
            None,
            &NewListen {
                edition_id: std_ed.clone(),
                listened_on: Some("2026-09-30".into()),
                is_full: false,
                track_ids: vec![t(2)],
            },
        )?;
        Ok(())
    })
    .unwrap();
    let dlx = db
        .read(|c| album::detail(c, &album_id, Some(&dlx_ed)))
        .unwrap();
    // The Deluxe "Airbag" already has its own rating: it must not be overwritten.
    db.write(|tx| {
        personal::set_track_rating(tx, &dlx.discs[0].tracks[1].id, Some(HalfStars::new(3)?))
    })
    .unwrap();

    let preview = db
        .read(|c| editions::preview_switch(c, &album_id, &dlx_ed))
        .unwrap();
    let pairs: Vec<(String, String, bool)> = preview
        .carry
        .iter()
        .map(|c| (c.from.title.clone(), c.to.title.clone(), c.target_has_data))
        .collect();
    assert_eq!(
        pairs,
        vec![
            ("Airbag".into(), "Airbag".into(), true),
            ("Paranoid Android".into(), "Paranoid Android".into(), false),
            ("Lucky".into(), "Lucky".into(), false),
        ],
        "matched by recording although positions differ; the live Lucky is not a match"
    );
    assert!(preview.carry.iter().all(|c| !c.to.title.contains("live")));
    assert_eq!(preview.listens_kept, 1);
    assert!(
        preview
            .stay
            .iter()
            .any(|s| s.track.title == "Lucky" && s.listened)
    );

    let result = db
        .write(|tx| editions::switch(tx, &album_id, &dlx_ed, true))
        .unwrap();
    assert_eq!(result.copied, 2);
    let after = db
        .read(|c| album::detail(c, &album_id, Some(&dlx_ed)))
        .unwrap();
    let by_title = |title: &str| {
        after
            .discs
            .iter()
            .flat_map(|d| &d.tracks)
            .find(|x| x.title == title)
            .unwrap()
            .clone()
    };
    assert_eq!(by_title("Airbag").rating, Some(3), "existing rating kept");
    assert!(by_title("Paranoid Android").favourite);
    assert_eq!(
        by_title("Lucky").rating,
        Some(0),
        "zero is copied as a real rating"
    );
    assert_eq!(by_title("Lucky (live)").rating, None);
    let listed = db.read(personal::listen_list).unwrap();
    assert_eq!(listed[0].edition_id, dlx_ed);
    // The old edition keeps everything.
    let old = db
        .read(|c| album::detail(c, &album_id, Some(&std_ed)))
        .unwrap();
    assert_eq!(old.discs[0].tracks[0].rating, Some(8));
    assert!(old.discs[0].tracks[2].listened);
    assert_eq!(count(&db, "listen_event"), 1);
}

#[test]
fn switching_without_copying_moves_only_the_list_entry() {
    let (_d, mut db) = open_temp();
    let (album_id, std_ed) = import(
        &mut db,
        &group(None),
        &release(STD, standard_tracks()),
        None,
    );
    let (_, dlx_ed) = import(
        &mut db,
        &group(None),
        &release(DLX, deluxe_tracks()),
        Some("Deluxe"),
    );
    let first = db
        .read(|c| album::detail(c, &album_id, Some(&std_ed)))
        .unwrap()
        .discs[0]
        .tracks[0]
        .id
        .clone();
    db.write(|tx| {
        personal::add_to_listen_list(tx, &std_ed)?;
        personal::set_track_rating(tx, &first, Some(HalfStars::new(7)?))
    })
    .unwrap();
    assert_eq!(
        db.write(|tx| editions::switch(tx, &album_id, &dlx_ed, false))
            .unwrap()
            .copied,
        0
    );
    assert_eq!(count(&db, "track_rating"), 1);
    assert_eq!(
        db.read(|c| editions::preview_switch(c, &album_id, &dlx_ed))
            .unwrap_err()
            .code(),
        "validation",
        "already on that edition"
    );
    db.write(|tx| album::rename_edition(tx, &dlx_ed, "Special"))
        .unwrap();
    assert_eq!(
        db.write(|tx| album::rename_edition(tx, &dlx_ed, "standard"))
            .unwrap_err()
            .code(),
        "conflict"
    );
}
