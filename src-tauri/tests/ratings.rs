//! The shared rating service against real storage.

mod common;

use common::{album, artist, count, edition, open_temp};
use mudraft_lib::db::Database;
use mudraft_lib::domain::rating::{HalfStars, RatingSource};
use mudraft_lib::library::listing::{self, ListQuery, SortOrder, Source};
use mudraft_lib::library::{album as album_view, personal, ratings};

fn hs(v: u8) -> Option<HalfStars> {
    Some(HalfStars::new(i64::from(v)).unwrap())
}

/// Album with one edition of `n` tracks; returns (album, edition, track ids).
fn setup(db: &mut Database, title: &str, n: i64) -> (String, String, Vec<String>) {
    let a = artist(db, "Artist");
    let al = album(db, title, &[&a]);
    let (ed, tracks) = edition(db, &al, "Standard", n);
    (al, ed, tracks)
}

fn rate(db: &mut Database, track: &str, half_stars: Option<i64>) {
    db.write(|tx| ratings::set_track(tx, track, half_stars))
        .unwrap();
}

#[test]
fn all_zero_and_mixed_zero_unrated_tracks_are_rated_zero() {
    let (_d, mut db) = open_temp();
    let (_, ed, t) = setup(&mut db, "Zeros", 4);
    for id in &t[..3] {
        rate(&mut db, id, Some(0));
    }
    let s = db.read(|c| ratings::summary(c, &ed)).unwrap();
    assert_eq!(
        (s.effective, s.source, s.rated_tracks, s.total_tracks),
        (hs(0), RatingSource::Calculated, 3, 4)
    );

    rate(&mut db, &t[1], None);
    let s = db.read(|c| ratings::summary(c, &ed)).unwrap();
    assert_eq!(
        (s.effective, s.rated_tracks),
        (hs(0), 2),
        "cleared track no longer counts; zeros still do"
    );
}

#[test]
fn partial_ratings_round_ties_up_and_unrated_stays_unrated() {
    let (_d, mut db) = open_temp();
    let (_, ed, t) = setup(&mut db, "Partial", 5);
    let s = db.read(|c| ratings::summary(c, &ed)).unwrap();
    assert_eq!((s.effective, s.source), (None, RatingSource::Unrated));
    rate(&mut db, &t[0], Some(7));
    rate(&mut db, &t[1], Some(8));
    let s = db.read(|c| ratings::summary(c, &ed)).unwrap();
    assert_eq!(
        (s.effective, s.rated_tracks, s.total_tracks),
        (hs(8), 2, 5),
        "3.75 stars → 4 (tie up)"
    );
    rate(&mut db, &t[2], Some(7));
    assert_eq!(
        db.read(|c| ratings::summary(c, &ed)).unwrap().effective,
        hs(7),
        "3.67 stars → 3.5"
    );
}

#[test]
fn explicit_zero_overrides_and_clearing_restores_the_calculation() {
    let (_d, mut db) = open_temp();
    let (_, ed, t) = setup(&mut db, "Override", 2);
    rate(&mut db, &t[0], Some(10));
    rate(&mut db, &t[1], Some(9));
    let s = db.write(|tx| ratings::set_album(tx, &ed, Some(0))).unwrap();
    assert_eq!(
        (s.effective, s.source, s.calculated),
        (hs(0), RatingSource::Explicit, hs(10))
    );
    let untouched = db.read(|c| personal::track_rating(c, &t[0])).unwrap();
    assert_eq!(
        untouched.rating,
        hs(10),
        "an explicit rating never changes track ratings"
    );
    let s = db.write(|tx| ratings::set_album(tx, &ed, None)).unwrap();
    assert_eq!((s.effective, s.source), (hs(10), RatingSource::Calculated));
    assert_eq!(
        db.write(|tx| ratings::set_album(tx, &ed, Some(11)))
            .unwrap_err()
            .code(),
        "validation"
    );
    assert_eq!(
        db.write(|tx| ratings::set_track(tx, &t[0], Some(-1)))
            .unwrap_err()
            .code(),
        "validation"
    );
}

#[test]
fn editions_are_rated_separately() {
    let (_d, mut db) = open_temp();
    let a = artist(&mut db, "A");
    let al = album(&mut db, "Two editions", &[&a]);
    let (std, std_tracks) = edition(&mut db, &al, "Standard", 2);
    let (dlx, _) = edition(&mut db, &al, "Deluxe", 3);
    rate(&mut db, &std_tracks[0], Some(6));
    db.write(|tx| ratings::set_album(tx, &std, Some(9)))
        .unwrap();
    let dlx_summary = db.read(|c| ratings::summary(c, &dlx)).unwrap();
    assert_eq!(
        (
            dlx_summary.effective,
            dlx_summary.rated_tracks,
            dlx_summary.total_tracks
        ),
        (None, 0, 3)
    );
    let detail = db.read(|c| album_view::detail(c, &al, Some(&dlx))).unwrap();
    assert_eq!(detail.rating.effective, None);
    let detail = db.read(|c| album_view::detail(c, &al, Some(&std))).unwrap();
    assert_eq!(
        (detail.rating.effective, detail.rating.source),
        (hs(9), RatingSource::Explicit)
    );
}

#[test]
fn rating_favouriting_and_reviews_never_mark_anything_listened() {
    let (dir, mut db) = open_temp();
    let (al, ed, t) = setup(&mut db, "Quiet", 2);
    rate(&mut db, &t[0], Some(5));
    db.write(|tx| {
        ratings::set_favourite(tx, &t[1], true)?;
        ratings::set_album(tx, &ed, Some(4))?;
        ratings::set_review(tx, &ed, Some("  Notes about the mix.  "))
    })
    .unwrap();
    for table in [
        "listen_event",
        "listen_event_track",
        "collection_entry",
        "listen_list_entry",
    ] {
        assert_eq!(count(&db, table), 0, "{table}");
    }
    drop(db);
    let mut db = Database::open(&dir.path().join("mudraft.sqlite3")).unwrap();
    let detail = db.read(|c| album_view::detail(c, &al, None)).unwrap();
    assert_eq!(detail.review.as_deref(), Some("Notes about the mix."));
    assert!(detail.discs[0].tracks[1].favourite);
    assert!(!detail.discs[0].tracks[1].listened);
    assert_eq!(detail.discs[0].tracks[0].rating, Some(5));
    db.write(|tx| ratings::set_review(tx, &ed, Some("   ")))
        .unwrap();
    assert_eq!(
        db.read(|c| album_view::detail(c, &al, None))
            .unwrap()
            .review,
        None
    );
}

#[test]
fn cards_and_sorting_use_the_same_calculation() {
    let (_d, mut db) = open_temp();
    let (_, high, ht) = setup(&mut db, "High", 2);
    let (_, low, _) = setup(&mut db, "Low", 1);
    let (_, none, _) = setup(&mut db, "Unrated", 1);
    rate(&mut db, &ht[0], Some(9));
    rate(&mut db, &ht[1], Some(10));
    db.write(|tx| {
        ratings::set_album(tx, &low, Some(0))?;
        for e in [&high, &low, &none] {
            personal::add_to_listen_list(tx, e)?;
        }
        Ok(())
    })
    .unwrap();
    let list = |sort| {
        db.read(|c| {
            listing::query(
                c,
                Source::ListenList,
                &ListQuery {
                    sort,
                    ..Default::default()
                },
            )
        })
        .unwrap()
    };
    let r = list(SortOrder::RatingHighest);
    let titles: Vec<&str> = r.items.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(titles, vec!["High", "Low", "Unrated"]);
    assert_eq!(
        r.items[0].rating,
        db.read(|c| ratings::summary(c, &high)).unwrap()
    );
    assert_eq!(r.items[0].rating.effective, hs(10));
    let titles: Vec<String> = list(SortOrder::RatingLowest)
        .items
        .iter()
        .map(|i| i.title.clone())
        .collect();
    assert_eq!(
        titles,
        vec!["Low", "High", "Unrated"],
        "explicit zero is lowest; unrated always last"
    );
}
