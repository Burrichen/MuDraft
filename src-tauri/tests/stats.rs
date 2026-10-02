//! Stats selectors against a hand-built fixture. Every expected number below is worked
//! out by hand in the comments, not taken from the implementation.

mod common;

use common::{artist, open_temp};
use mudraft_lib::db::Database;
use mudraft_lib::domain::ids::new_id;
use mudraft_lib::library::catalogue::{
    self, CreditInput, NewAlbum, NewEdition, NewRecording, NewTrack,
};
use mudraft_lib::library::listening::{self, ListenKind, LogListen};
use mudraft_lib::library::selection::{self, AttemptStatus, SelectionMode, SelectionSource};
use mudraft_lib::library::stats::{self, Best};
use mudraft_lib::library::{personal, ratings};

const ASAP: &str = "00000000-0000-7000-8000-000000000001";
const NOW: &str = "2026-10-02T00:00:00.000Z";

fn credit(id: &str) -> CreditInput {
    CreditInput {
        artist_id: id.into(),
        credited_name: None,
        join_phrase: Some(" & ".into()),
    }
}

fn album(
    db: &mut Database,
    title: &str,
    artists: &[&str],
    date: Option<&str>,
    genres: &[&str],
) -> String {
    db.write(|tx| {
        catalogue::create_album(
            tx,
            None,
            &NewAlbum {
                title: title.into(),
                original_date: date.map(Into::into),
                musicbrainz_release_group_id: None,
                credits: artists.iter().map(|a| credit(a)).collect(),
                genres: genres.iter().map(|g| (*g).into()).collect(),
            },
        )
    })
    .unwrap()
}

/// Tracks given as (length in seconds or None, local recording ID or None).
fn edition(
    db: &mut Database,
    album: &str,
    name: &str,
    release: Option<&str>,
    tracks: &[(Option<i64>, Option<&String>)],
) -> catalogue::EditionCreated {
    db.write(|tx| {
        catalogue::create_edition(
            tx,
            None,
            &NewEdition {
                album_id: album.into(),
                name: name.into(),
                release_date: release.map(Into::into),
                musicbrainz_release_id: None,
                tracks: tracks
                    .iter()
                    .enumerate()
                    .map(|(i, (secs, rec))| NewTrack {
                        disc_number: None,
                        position: i as i64 + 1,
                        title: format!("{name} track {}", i + 1),
                        length_ms: secs.map(|s| s * 1000),
                        recording_id: rec.cloned(),
                        musicbrainz_track_id: None,
                        credits: vec![],
                    })
                    .collect(),
            },
        )
    })
    .unwrap()
}

fn recording(db: &mut Database, title: &str) -> String {
    db.write(|tx| {
        catalogue::create_recording(
            tx,
            None,
            &NewRecording {
                title: title.into(),
                length_ms: None,
                musicbrainz_recording_id: None,
            },
        )
    })
    .unwrap()
}

fn listen(
    db: &mut Database,
    edition: &str,
    date: Option<&str>,
    kind: ListenKind,
    tracks: Option<Vec<String>>,
    earlier: bool,
) -> String {
    db.write(|tx| {
        listening::log(
            tx,
            Some(&new_id()),
            &LogListen {
                edition_id: edition.into(),
                listened_on: date.map(Into::into),
                kind,
                earlier_undated: earlier,
                track_ids: tracks,
                attempt_id: None,
            },
        )
    })
    .unwrap()
    .listen_id
}

fn set_added(db: &mut Database, album: &str, at: &str) {
    db.write(|tx| {
        tx.execute(
            "UPDATE listen_list_entry SET added_at = ?2 WHERE album_id = ?1",
            [album, at],
        )?;
        Ok(())
    })
    .unwrap();
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn group(best: &Best, label: &str) -> (f64, u32) {
    let g = best.groups.iter().find(|g| g.label == label).unwrap();
    (g.mean_stars, g.albums)
}

#[test]
fn empty_library_reports_zeros_and_no_winners() {
    let (_d, db) = open_temp();
    let s = db.read(|c| stats::overview(c, NOW)).unwrap();
    assert_eq!(
        (s.listen_list.entries, s.listen_list.distinct_artists),
        (0, 0)
    );
    assert!(s.listen_list.waiting.is_none());
    assert_eq!(s.listen_list.runtime.known_ms, 0);
    assert_eq!(
        (s.collection.albums, s.collection.ratings.rated_albums),
        (0, 0)
    );
    assert!(s.collection.ratings.average_stars.is_none());
    assert!(s.collection.best_year.winners.is_empty());
    assert!(s.collection.runtime.complete && s.collection.runtime.sessions == 0);
    assert_eq!(s.selection.len(), 5);
    assert!(s.selection.iter().all(|m| m.shown == 0));
}

#[test]
fn fixture_matches_hand_calculated_results() {
    let (_d, mut db) = open_temp();
    let a = artist(&mut db, "Alpha");
    let b = artist(&mut db, "Beta");
    let c = artist(&mut db, "Gamma");

    // ---- Collection albums
    // One (Alpha, 1994, Rock+Pop): Standard [200s r1, 100s r2], Deluxe [200s r1, 100s r2, bonus ?s].
    let one = album(&mut db, "One", &[&a], Some("1994"), &["Rock", "Pop"]);
    let (r1, r2) = (recording(&mut db, "r1"), recording(&mut db, "r2"));
    let one_std = edition(
        &mut db,
        &one,
        "Standard",
        None,
        &[(Some(200), Some(&r1)), (Some(100), Some(&r2))],
    );
    let one_dlx = edition(
        &mut db,
        &one,
        "Deluxe",
        None,
        &[(Some(200), Some(&r1)), (Some(100), Some(&r2)), (None, None)],
    );
    // Two (Alpha & Beta collaboration, 1997, Rock): one 300s track.
    let two = album(&mut db, "Two", &[&a, &b], Some("1997"), &["Rock"]);
    let two_std = edition(&mut db, &two, "Standard", None, &[(Some(300), None)]);
    // Three (Beta, original 1997, Jazz) — only a 2017 Deluxe reissue edition: one 60s track.
    let three = album(&mut db, "Three", &[&b], Some("1997"), &["Jazz"]);
    let three_dlx = edition(&mut db, &three, "Deluxe", Some("2017"), &[(Some(60), None)]);
    // Four (Gamma, unknown year, no genre): one 120s track.
    let four = album(&mut db, "Four", &[&c], None, &[]);
    let four_std = edition(&mut db, &four, "Standard", None, &[(Some(120), None)]);
    // Seven (Gamma, 2010, Rock): no tracklist; unrated.
    let seven = album(&mut db, "Seven", &[&c], Some("2010"), &["Rock"]);
    let seven_std = edition(&mut db, &seven, "Standard", None, &[]);

    // ---- Listen List albums
    // Five (Gamma & Alpha, 2003, Pop): 100s + 200s. Six (Alpha, 2001, Rock, Listen ASAP): no tracks.
    let five = album(&mut db, "Five", &[&c, &a], Some("2003"), &["Pop"]);
    let five_std = edition(
        &mut db,
        &five,
        "Standard",
        None,
        &[(Some(100), None), (Some(200), None)],
    );
    let five_alt = edition(&mut db, &five, "Japan", None, &[(Some(100), None)]);
    let six = album(&mut db, "Six", &[&a], Some("2001"), &["Rock"]);
    let six_std = edition(&mut db, &six, "Standard", None, &[]);
    // Four also waits on the Listen List (unknown year, no genre, 120s).
    db.write(|tx| {
        personal::add_to_listen_list(tx, &five_alt.edition_id)?;
        // A second edition of the same album replaces the first: still one entry.
        personal::add_to_listen_list(tx, &five_std.edition_id)?;
        personal::add_to_listen_list(tx, &six_std.edition_id)?;
        personal::add_to_listen_list(tx, &four_std.edition_id)?;
        catalogue::tag_album(tx, &six, ASAP)
    })
    .unwrap();
    set_added(&mut db, &four, "2026-09-01T00:00:00.000Z"); // 31 days before NOW
    set_added(&mut db, &five, "2026-09-22T00:00:00.000Z"); // 10 days
    set_added(&mut db, &six, "2026-10-01T12:00:00.000Z"); // 0.5 → 0 days

    // Keep Listen List membership fixed while logging.
    db.write(|tx| {
        mudraft_lib::library::settings::save(
            tx,
            &mudraft_lib::library::settings::Settings {
                full_listen_removes_from_listen_list: false,
            },
        )
    })
    .unwrap();

    // ---- Listens (9 live, non-historical)
    use ListenKind::{First, Relisten, Unspecified};
    listen(
        &mut db,
        &one_std.edition_id,
        Some("2026-08-10"),
        First,
        None,
        false,
    );
    // Same session: the Deluxe copy of recording r1 must not add its length again.
    listen(
        &mut db,
        &one_dlx.edition_id,
        Some("2026-08-10"),
        Unspecified,
        Some(vec![one_dlx.track_ids[0].clone()]),
        false,
    );
    listen(
        &mut db,
        &one_dlx.edition_id,
        Some("2026-09-05"),
        Relisten,
        None,
        false,
    );
    listen(
        &mut db,
        &two_std.edition_id,
        Some("2026-08-10"),
        First,
        None,
        false,
    );
    listen(
        &mut db,
        &two_std.edition_id,
        Some("2026-09-01"),
        Relisten,
        None,
        false,
    );
    listen(&mut db, &two_std.edition_id, None, Relisten, None, false);
    listen(
        &mut db,
        &three_dlx.edition_id,
        Some("2025-12-31"),
        Unspecified,
        None,
        false,
    );
    // Track-only listen plus "listened before, dates unknown" (one historical record).
    listen(
        &mut db,
        &four_std.edition_id,
        Some("2026-09-15"),
        Unspecified,
        Some(four_std.track_ids.clone()),
        true,
    );
    listen(
        &mut db,
        &seven_std.edition_id,
        None,
        Unspecified,
        None,
        false,
    );
    // A deleted listen counts nowhere.
    let gone = listen(
        &mut db,
        &one_std.edition_id,
        Some("2026-07-01"),
        First,
        None,
        false,
    );
    db.write(|tx| listening::delete(tx, &gone)).unwrap();

    // ---- Ratings
    db.write(|tx| {
        ratings::set_album(tx, &one_std.edition_id, Some(0))?; // zero is a rating
        ratings::set_album(tx, &one_dlx.edition_id, Some(6))?; // One = (0 + 3) / 2 = 1.5★
        ratings::set_album(tx, &two_std.edition_id, Some(8))?; // Two = 4★
        ratings::set_track(tx, &three_dlx.track_ids[0], Some(5))?; // Three = 2.5★ (calculated)
        ratings::set_album(tx, &four_std.edition_id, Some(7))?; // Four = 3.5★
        ratings::set_favourite(tx, &one_std.track_ids[0], true)?;
        ratings::set_favourite(tx, &five_std.track_ids[0], true) // not in the Collection
    })
    .unwrap();

    // ---- Next Up provenance
    db.write(|tx| {
        let s1 = selection::start_session(tx, SelectionMode::CompletelyRandom, None)?;
        selection::record_attempt(
            tx,
            &s1,
            &five_std.edition_id,
            SelectionSource::CompletelyRandom,
        )?;
        selection::record_attempt(
            tx,
            &s1,
            &six_std.edition_id,
            SelectionSource::CompletelyRandom,
        )?;
        let s2 = selection::start_session(tx, SelectionMode::WeightedRandom, None)?;
        let w = selection::record_attempt(
            tx,
            &s2,
            &six_std.edition_id,
            SelectionSource::WeightedRandom,
        )?;
        selection::resolve_attempt(tx, &w, AttemptStatus::Completed)?;
        let s3 = selection::start_session(tx, SelectionMode::Manual, None)?;
        selection::record_attempt(tx, &s3, &five_std.edition_id, SelectionSource::Manual)?;
        // Today's tags must not rewrite history.
        catalogue::untag_album(tx, &six, ASAP)
    })
    .unwrap();

    let s = db.read(|c| stats::overview(c, NOW)).unwrap();

    // ======== Listen List: Four, Five, Six
    let l = &s.listen_list;
    assert_eq!((l.entries, l.distinct_albums), (3, 3));
    // Alpha: Five (collab) + Six = 2; Gamma: Five + Four = 2 → a tie, listed alphabetically.
    assert_eq!(l.distinct_artists, 2);
    let artists: Vec<(&str, u32)> = l
        .artists
        .iter()
        .map(|c| (c.label.as_str(), c.count))
        .collect();
    assert_eq!(artists, [("Alpha", 2), ("Gamma", 2)]);
    let years: Vec<(&str, u32)> = l.years.iter().map(|c| (c.key.as_str(), c.count)).collect();
    assert_eq!(years, [("2001", 1), ("2003", 1)]);
    assert_eq!(l.decades[0].key, "2000");
    assert_eq!((l.decades[0].count, l.unknown_years), (2, 1));
    let genres: Vec<(&str, u32)> = l
        .genres
        .iter()
        .map(|c| (c.label.as_str(), c.count))
        .collect();
    assert_eq!(genres, [("Pop", 1), ("Rock", 1)]);
    assert_eq!(l.albums_without_genre, 1);
    assert_eq!(l.listen_asap, 0, "untagged after the weighted pick");
    let oldest: Vec<(&str, i64)> = l
        .oldest_waiting
        .iter()
        .map(|w| (w.title.as_str(), w.waiting_days))
        .collect();
    assert_eq!(oldest, [("Four", 31), ("Five", 10), ("Six", 0)]);
    assert_eq!(l.recently_added[0].title, "Six");
    let w = l.waiting.as_ref().unwrap();
    assert!(close(w.mean_days, 41.0 / 3.0) && close(w.median_days, 10.0));
    assert_eq!(w.longest_days, 31);
    // Five 300s + Four 120s known; Six has no tracklist.
    assert_eq!(
        (
            l.runtime.known_ms,
            l.runtime.editions_known,
            l.runtime.editions_unknown
        ),
        (420_000, 2, 1)
    );

    // ======== Collection: One (2 editions), Two, Three, Four, Seven
    let col = &s.collection;
    assert_eq!((col.albums, col.distinct_listened_albums), (5, 5));
    let n = &col.listens;
    // Dated: One×3, Two×2, Three, Four = 7. Undated: Two, Seven = 2. Historical: Four's 1.
    assert_eq!((n.dated, n.undated, n.historical), (7, 2, 1));
    assert_eq!((n.first, n.relisten, n.unspecified), (2, 3, 4));
    assert_eq!((n.full_album, n.tracks_only), (7, 2));
    let months: Vec<(&str, u32)> = col
        .by_month
        .iter()
        .map(|c| (c.key.as_str(), c.count))
        .collect();
    assert_eq!(months, [("2025-12", 1), ("2026-08", 3), ("2026-09", 3)]);
    let years: Vec<(&str, u32)> = col
        .by_year
        .iter()
        .map(|c| (c.key.as_str(), c.count))
        .collect();
    assert_eq!(years, [("2025", 1), ("2026", 6)]);

    // Scores: One 1.5, Two 4, Three 2.5, Four 3.5; Seven unrated. Mean = 11.5 / 4.
    let r = &col.ratings;
    assert_eq!((r.rated_albums, r.unrated_albums), (4, 1));
    assert!(close(r.average_stars.unwrap(), 2.875));
    let dist: Vec<(f64, u32)> = r.distribution.iter().map(|b| (b.stars, b.albums)).collect();
    assert_eq!(dist, [(1.5, 1), (2.5, 1), (3.5, 1), (4.0, 1)]);

    // Best year uses original years: 1997 = (Two 4 + Three 2.5) / 2 = 3.25 from 2 albums
    // (Three's 2017 reissue and 2025 listen don't move it); 1994 = 1.5; Four unplaced.
    assert_eq!(col.best_year.winners.len(), 1);
    assert_eq!(col.best_year.winners[0].key, "1997");
    assert_eq!(group(&col.best_year, "1997"), (3.25, 2));
    assert_eq!(col.best_year.unplaced_albums, 1);
    let (decade, count) = group(&col.best_decade, "1990s");
    assert!(close(decade, 8.0 / 3.0) && count == 3);
    // Genres overlap: Rock = (One 1.5 + Two 4) / 2 = 2.75; Pop = 1.5; Jazz = 2.5.
    assert_eq!(col.best_genre.winners[0].label, "Rock");
    assert_eq!(group(&col.best_genre, "Rock"), (2.75, 2));
    assert_eq!(group(&col.best_genre, "Pop"), (1.5, 1));
    assert_eq!(col.best_genre.unplaced_albums, 1);
    // Artists: Alpha (One, Two) 2.75; Beta (Two, Three) 3.25 — Two's three listens count
    // once; Gamma (Four) 3.5 from a single album wins: no minimum sample.
    assert_eq!(group(&col.best_artist, "Alpha"), (2.75, 2));
    assert_eq!(group(&col.best_artist, "Beta"), (3.25, 2));
    assert_eq!(col.best_artist.winners.len(), 1);
    assert_eq!(
        (
            col.best_artist.winners[0].label.as_str(),
            col.best_artist.winners[0].albums
        ),
        ("Gamma", 1)
    );

    let favs: Vec<&str> = col
        .favourite_tracks
        .iter()
        .map(|t| t.album_title.as_str())
        .collect();
    assert_eq!(favs, ["One"]);

    // Runtime: One 08-10 = 300s (r1 counted once); One 09-05 = 300s + 1 unknown bonus;
    // Two ×3 = 900s; Three 60s; Four 120s; Seven undated with no tracklist.
    let rt = &col.runtime;
    assert_eq!(rt.sessions, 8);
    assert_eq!(rt.known_ms, 1_680_000);
    assert_eq!((rt.tracks_counted, rt.tracks_without_length), (9, 1));
    assert_eq!(rt.listens_without_tracklist, 1);
    assert!(!rt.complete);

    // ======== Next Up provenance
    let m = |source: &str| {
        let x = s.selection.iter().find(|m| m.source == source).unwrap();
        (x.shown, x.skipped, x.completed, x.pending)
    };
    assert_eq!(m("completely_random"), (2, 2, 0, 0));
    assert_eq!(m("weighted_random"), (1, 0, 1, 0));
    assert_eq!(m("guided"), (0, 0, 0, 0));
    assert_eq!(m("manual"), (1, 0, 0, 1));
    assert_eq!(m("all"), (4, 2, 1, 1));
}

#[test]
fn tied_groups_are_all_winners() {
    let (_d, mut db) = open_temp();
    let a = artist(&mut db, "Alpha");
    let b = artist(&mut db, "Beta");
    for (title, who) in [("X", &a), ("Y", &b)] {
        let al = album(&mut db, title, &[who], Some("2000"), &["Rock"]);
        let ed = edition(&mut db, &al, "Standard", None, &[(Some(60), None)]);
        db.write(|tx| {
            personal::add_to_collection(tx, &ed.edition_id, personal::CollectionSource::Manual)?;
            ratings::set_album(tx, &ed.edition_id, Some(6)).map(|_| ())
        })
        .unwrap();
    }
    let col = db.read(stats::collection).unwrap();
    let winners: Vec<&str> = col
        .best_artist
        .winners
        .iter()
        .map(|w| w.label.as_str())
        .collect();
    assert_eq!(winners, ["Alpha", "Beta"]);
    assert_eq!(col.best_year.winners[0].albums, 2);
    assert_eq!(
        col.distinct_listened_albums, 0,
        "manual Collection entries aren't listens"
    );
}
