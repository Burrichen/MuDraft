//! Next Up: methods, session pools, stale candidates, provenance, and completion.

mod common;

use std::collections::HashSet;

use common::{artist, count, credit, edition, open_temp};
use mudraft_lib::db::Database;
use mudraft_lib::domain::ids::new_id;
use mudraft_lib::domain::picker::{
    EmptyReason, Filter, GuidedCriteria, Method, RandomSource, YearChoice,
};
use mudraft_lib::error::AppResult;
use mudraft_lib::library::catalogue::{self, NewAlbum};
use mudraft_lib::library::listening::{self, ListenKind, LogListen};
use mudraft_lib::library::next_up::{self, Eligibility, RollOutcome};
use mudraft_lib::library::selection::{AttemptStatus, SelectionSource};
use mudraft_lib::library::{personal, tags};

const LISTEN_ASAP: &str = "00000000-0000-7000-8000-000000000001";
const YEAR: i32 = 2026;

/// Always the first unseen candidate (candidates are ordered by album ID).
struct First;
impl RandomSource for First {
    fn below(&mut self, _len: usize) -> AppResult<usize> {
        Ok(0)
    }
}

/// Creates an album with one edition and puts it on the Listen List.
fn entry(db: &mut Database, title: &str, date: Option<&str>, genres: &[&str]) -> (String, String) {
    let a = artist(db, &format!("{title} artist"));
    let al = db
        .write(|tx| {
            catalogue::create_album(
                tx,
                None,
                &NewAlbum {
                    title: title.into(),
                    original_date: date.map(Into::into),
                    musicbrainz_release_group_id: None,
                    credits: vec![credit(&a)],
                    genres: genres.iter().map(|g| (*g).into()).collect(),
                },
            )
        })
        .unwrap();
    let (ed, _) = edition(db, &al, "Standard", 2);
    db.write(|tx| personal::add_to_listen_list(tx, &ed))
        .unwrap();
    (al, ed)
}

fn method(json: serde_json::Value) -> Method {
    serde_json::from_value(json).unwrap()
}

fn roll_with(db: &mut Database, m: &Method, year: i32) -> AppResult<RollOutcome> {
    db.write(|tx| next_up::roll(tx, Some(&new_id()), 1, m.clone(), year, &mut First))
}

fn roll(db: &mut Database, m: &Method) -> RollOutcome {
    roll_with(db, m, YEAR).unwrap()
}

fn picked(outcome: RollOutcome) -> next_up::Shown {
    match outcome {
        RollOutcome::Picked { shown, .. } => shown,
        other => panic!("expected a pick, got {other:?}"),
    }
}

fn pool_size(outcome: &RollOutcome) -> usize {
    match outcome {
        RollOutcome::Picked { pool_size, .. } | RollOutcome::Exhausted { pool_size } => *pool_size,
        RollOutcome::Empty { .. } => 0,
    }
}

/// One slot's view; an unused slot has neither a pick nor a session.
fn slot(db: &Database, n: i64) -> next_up::SlotState {
    db.read(|c| next_up::state(c, YEAR))
        .unwrap()
        .slots
        .into_iter()
        .find(|s| s.slot == n)
        .unwrap_or(next_up::SlotState {
            slot: n,
            current: None,
            session: None,
        })
}

fn attempt_status(db: &Database, id: &str) -> String {
    db.read(|c| {
        Ok(c.query_row(
            "SELECT status FROM selection_attempt WHERE id = ?1",
            [id],
            |r| r.get(0),
        )?)
    })
    .unwrap()
}

#[test]
fn zero_one_and_many_candidates_never_repeat_until_reset() {
    let (_d, mut db) = open_temp();
    let random = Method::CompletelyRandom;
    assert_eq!(
        roll(&mut db, &random),
        RollOutcome::Empty {
            reason: EmptyReason::ListenListEmpty
        }
    );
    assert_eq!(count(&db, "selection_attempt"), 0);

    let (only, _) = entry(&mut db, "Only", Some("1990"), &["Rock"]);
    let first = picked(roll(&mut db, &random));
    assert_eq!(first.album_id, only);
    // The first result counts as shown: with one candidate the pool is now used up.
    assert_eq!(
        roll(&mut db, &random),
        RollOutcome::Exhausted { pool_size: 1 }
    );

    entry(&mut db, "Two", Some("1991"), &["Rock"]);
    entry(&mut db, "Three", None, &["Rock"]);
    let mut shown: Vec<String> = vec![first.album_id.clone()];
    let mut previous = first.attempt_id.clone();
    for _ in 0..2 {
        let s = picked(roll(&mut db, &random));
        assert!(!shown.contains(&s.album_id), "no repeats within a pool");
        assert_eq!(attempt_status(&db, &previous), "skipped");
        shown.push(s.album_id);
        previous = s.attempt_id;
    }
    assert_eq!(
        roll(&mut db, &random),
        RollOutcome::Exhausted { pool_size: 3 }
    );
    // Exhaustion changes nothing: the last pick stays current and shown.
    let state = slot(&db, 1);
    let current = state.current.unwrap();
    assert_eq!(current.attempt_id, previous);
    assert_eq!(current.status, AttemptStatus::Shown);
    assert!(state.session.as_ref().unwrap().exhausted);

    assert_eq!(db.write(|tx| next_up::reset_pool(tx, 1)).unwrap(), 2);
    assert_eq!(
        db.write(|tx| next_up::reset_pool(tx, 1)).unwrap(),
        2,
        "double reset is one reset"
    );
    let after = picked(roll(&mut db, &random));
    assert_ne!(
        after.album_id, current.album_id,
        "a reroll changes the album"
    );
    let distinct: HashSet<String> = shown.into_iter().collect();
    assert_eq!(distinct.len(), 3);
}

#[test]
fn weighted_random_uses_only_listen_asap_without_fallback() {
    let (_d, mut db) = open_temp();
    let weighted = Method::WeightedRandom;
    entry(&mut db, "A", Some("2000"), &["Pop"]);
    let (b, _) = entry(&mut db, "B", Some("2001"), &["Pop"]);
    entry(&mut db, "C", Some("2002"), &["Pop"]);
    assert_eq!(
        roll(&mut db, &weighted),
        RollOutcome::Empty {
            reason: EmptyReason::NoListenAsap
        }
    );
    assert!(slot(&db, 1).current.is_none());

    db.write(|tx| catalogue::tag_album(tx, &b, LISTEN_ASAP))
        .unwrap();
    let s = picked(roll(&mut db, &weighted));
    assert_eq!(
        (s.album_id.as_str(), s.source),
        (b.as_str(), SelectionSource::WeightedRandom)
    );
    assert_eq!(
        roll(&mut db, &weighted),
        RollOutcome::Exhausted { pool_size: 1 }
    );
    let options = |db: &Database| {
        db.read(|c| next_up::options(c, GuidedCriteria::default(), YEAR))
            .unwrap()
    };
    assert!(options(&db).listen_asap_tag);

    // With the tag deleted, Weighted Random has nothing to pick from: still no fallback.
    db.write(|tx| tags::delete(tx, LISTEN_ASAP)).unwrap();
    assert!(!options(&db).listen_asap_tag);
    assert_eq!(options(&db).listen_asap, 0);
    assert_eq!(
        roll_in(&mut db, 2, &weighted),
        RollOutcome::Empty {
            reason: EmptyReason::NoListenAsap
        }
    );
}

#[test]
fn guided_filters_combine_and_years_follow_the_supplied_calendar() {
    let (_d, mut db) = open_temp();
    let road = db
        .write(|tx| tags::create(tx, "Road trip", None))
        .unwrap()
        .id;
    let (nineties_rock, _) = entry(&mut db, "Nineties", Some("1994-03"), &["Rock"]);
    db.write(|tx| catalogue::tag_album(tx, &nineties_rock, &road))
        .unwrap();
    entry(&mut db, "This year", Some("2026-01-02"), &["Pop"]);
    entry(&mut db, "Early 2020s", Some("2021"), &["Rock"]);
    entry(&mut db, "Undated", None, &["Jazz"]);

    let guided = |criteria: serde_json::Value| {
        method(serde_json::json!({ "mode": "guided", "criteria": criteria }))
    };
    let size = |db: &mut Database, criteria: serde_json::Value, year: i32| {
        pool_size(&roll_with(db, &guided(criteria), year).unwrap())
    };
    // Current year plus its decade: two albums, the 2026 one counted once.
    let both = serde_json::json!({ "years": { "any_of": [{ "decade": 2020 }, "current_year"] } });
    assert_eq!(size(&mut db, both, 2026), 2);
    let this_year = serde_json::json!({ "years": { "any_of": ["current_year"] } });
    assert_eq!(size(&mut db, this_year.clone(), 2026), 1);
    assert_eq!(
        roll_with(&mut db, &guided(this_year), 2027).unwrap(),
        RollOutcome::Empty {
            reason: EmptyReason::NoGuidedMatches
        }
    );
    // OR within genres; unknown years qualify because years are agnostic.
    let genres = serde_json::json!({ "genres": { "any_of": ["rock", "JAZZ"] } });
    assert_eq!(size(&mut db, genres, 2026), 3);
    // AND between categories.
    let and = serde_json::json!({
        "years": { "any_of": [{ "decade": 1990 }, { "decade": 2020 }] },
        "genres": { "any_of": ["Rock"] },
        "tagIds": { "any_of": [road] }
    });
    assert_eq!(size(&mut db, and, 2026), 1);

    for bad in [
        serde_json::json!({ "genres": { "any_of": [] } }),
        serde_json::json!({ "years": { "any_of": [{ "decade": 1995 }] } }),
    ] {
        assert_eq!(
            roll_with(&mut db, &guided(bad), 2026).unwrap_err().code(),
            "validation"
        );
    }
    let unknown_tag = serde_json::json!({ "tagIds": { "any_of": [new_id()] } });
    assert_eq!(
        roll_with(&mut db, &guided(unknown_tag), 2026)
            .unwrap_err()
            .code(),
        "not_found"
    );

    let options = db
        .read(|c| next_up::options(c, GuidedCriteria::default(), 2026))
        .unwrap();
    let decades: Vec<(i32, usize)> = options.decades.iter().map(|d| (d.value, d.count)).collect();
    assert_eq!(decades, [(1990, 1), (2020, 2)]);
    assert_eq!(
        options.current_year.map(|c| (c.value, c.count)),
        Some((2026, 1))
    );
    assert_eq!(options.unknown_years, 1);
    assert!(
        db.read(|c| next_up::options(c, GuidedCriteria::default(), 2030))
            .unwrap()
            .current_year
            .is_none()
    );
}

#[test]
fn changing_method_or_criteria_starts_a_new_session() {
    let (_d, mut db) = open_temp();
    for n in 0..3 {
        entry(&mut db, &format!("Album {n}"), Some("1999"), &["Rock"]);
    }
    let random = Method::CompletelyRandom;
    let a = picked(roll(&mut db, &random));
    let b = picked(roll(&mut db, &random));
    assert_eq!(a.session_id, b.session_id);

    let rock = method(serde_json::json!({
        "mode": "guided", "criteria": { "genres": { "any_of": ["Rock"] } }
    }));
    let g = picked(roll(&mut db, &rock));
    assert_ne!(g.session_id, b.session_id);
    assert_eq!(attempt_status(&db, &b.attempt_id), "skipped");
    // Equivalent criteria (case, order, duplicates) continue the same session.
    let same = method(serde_json::json!({
        "mode": "guided", "criteria": { "genres": { "any_of": ["ROCK", "rock"] } }
    }));
    assert_eq!(picked(roll(&mut db, &same)).session_id, g.session_id);
    // Back to random: a fresh pool, so earlier albums may be shown again.
    let again = picked(roll(&mut db, &random));
    assert_ne!(again.session_id, a.session_id);
    assert_eq!(slot(&db, 1).session.unwrap().remaining, 2);
}

#[test]
fn stale_candidates_are_revalidated_after_list_and_tag_edits() {
    let (_d, mut db) = open_temp();
    let (a, _) = entry(&mut db, "A", Some("2010"), &["Rock"]);
    let (b, _) = entry(&mut db, "B", Some("2011"), &["Rock"]);
    for al in [&a, &b] {
        db.write(|tx| catalogue::tag_album(tx, al, LISTEN_ASAP))
            .unwrap();
    }
    let weighted = Method::WeightedRandom;
    let first = picked(roll(&mut db, &weighted));
    assert_eq!(first.album_id, a);
    let eligibility = |db: &Database| slot(db, 1).current.unwrap().eligibility;
    assert_eq!(eligibility(&db), Eligibility::Eligible);

    db.write(|tx| catalogue::untag_album(tx, &a, LISTEN_ASAP))
        .unwrap();
    assert_eq!(eligibility(&db), Eligibility::NoLongerMatches);
    let (_, other_edition) = {
        let (ed, _) = edition(&mut db, &a, "Deluxe", 3);
        db.write(|tx| personal::add_to_listen_list(tx, &ed))
            .unwrap();
        ((), ed)
    };
    assert_eq!(eligibility(&db), Eligibility::EditionChanged);
    db.write(|tx| personal::remove_from_listen_list(tx, &a))
        .unwrap();
    assert_eq!(eligibility(&db), Eligibility::NotOnListenList);
    assert_ne!(other_edition, first.edition_id);

    // B left the list too: the pool shrinks immediately instead of offering stale albums.
    db.write(|tx| personal::remove_from_listen_list(tx, &b))
        .unwrap();
    assert_eq!(
        roll(&mut db, &weighted),
        RollOutcome::Empty {
            reason: EmptyReason::ListenListEmpty
        }
    );
}

#[test]
fn provenance_manual_picks_and_completion_link_the_listen() {
    let (_d, mut db) = open_temp();
    let (a, _) = entry(&mut db, "A", Some("2001"), &["Rock"]);
    let (b, ed_b) = entry(&mut db, "B", Some("2002"), &["Rock"]);
    let random = Method::CompletelyRandom;
    let before = (
        count(&db, "listen_event"),
        count(&db, "listen_list_entry"),
        count(&db, "collection_entry"),
        count(&db, "track_rating"),
        count(&db, "album_review"),
    );

    // Double click: the same request ID rolls once.
    let key = new_id();
    let once = db
        .write(|tx| next_up::roll(tx, Some(&key), 1, random.clone(), YEAR, &mut First))
        .unwrap();
    let twice = db
        .write(|tx| next_up::roll(tx, Some(&key), 1, random.clone(), YEAR, &mut First))
        .unwrap();
    assert_eq!(once, twice);
    assert_eq!(count(&db, "selection_attempt"), 1);
    let rolled = picked(once);
    assert_eq!(
        (rolled.album_id.as_str(), rolled.source),
        (a.as_str(), SelectionSource::CompletelyRandom)
    );

    let manual = db
        .write(|tx| next_up::choose(tx, Some(&new_id()), &ed_b, Some(1)))
        .unwrap();
    assert_eq!(manual.source, SelectionSource::Manual);
    assert_eq!(attempt_status(&db, &rolled.attempt_id), "skipped");
    let state = slot(&db, 1);
    assert_eq!(
        state.current.as_ref().unwrap().source,
        SelectionSource::Manual
    );
    // The open random pool is untouched by a manual pick.
    let pool = state.session.unwrap();
    assert_eq!(
        (pool.session_id, pool.remaining),
        (rolled.session_id.clone(), 1)
    );

    let (off_list, _) = {
        let x = artist(&mut db, "X");
        let al = common::album(&mut db, "Not listed", &[&x]);
        edition(&mut db, &al, "Standard", 1)
    };
    assert_eq!(
        db.write(|tx| next_up::choose(tx, None, &off_list, None))
            .unwrap_err()
            .code(),
        "validation"
    );

    // Rolls and manual picks never log listening or touch ratings and membership.
    let after = (
        count(&db, "listen_event"),
        count(&db, "listen_list_entry"),
        count(&db, "collection_entry"),
        count(&db, "track_rating"),
        count(&db, "album_review"),
    );
    assert_eq!(before, after);

    let logged = db
        .write(|tx| {
            listening::log(
                tx,
                Some(&new_id()),
                &LogListen {
                    edition_id: ed_b.clone(),
                    listened_on: Some("2026-09-30".into()),
                    kind: ListenKind::First,
                    earlier_undated: false,
                    attempt_id: Some(manual.attempt_id.clone()),
                },
            )
        })
        .unwrap();
    assert!(logged.completed_next_up);
    assert_eq!(attempt_status(&db, &manual.attempt_id), "completed");
    let linked: String = db
        .read(|c| {
            Ok(c.query_row(
                "SELECT selection_attempt_id FROM listen_event WHERE id = ?1",
                [&logged.listen_id],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(linked, manual.attempt_id);
    // Cleared, and nothing else is selected automatically.
    assert!(slot(&db, 1).current.is_none());
    assert_eq!(
        count(&db, "listen_list_entry"),
        1,
        "B left the list after its full listen"
    );
    let _ = b;
}

#[test]
fn current_selection_and_pool_survive_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mudraft.sqlite3");
    let shown = {
        let mut db = Database::open(&path).unwrap();
        entry(&mut db, "A", Some("2001"), &["Rock"]);
        entry(&mut db, "B", Some("2002"), &["Rock"]);
        picked(roll(&mut db, &Method::CompletelyRandom))
    };
    let mut db = Database::open(&path).unwrap();
    let state = slot(&db, 1);
    assert_eq!(state.current.unwrap().attempt_id, shown.attempt_id);
    assert_eq!(state.session.unwrap().remaining, 1);
    let next = picked(roll(&mut db, &Method::CompletelyRandom));
    assert_eq!(next.session_id, shown.session_id);
    assert_ne!(next.album_id, shown.album_id);
}

#[test]
fn os_random_is_in_range_and_reaches_every_index() {
    let mut rng = next_up::OsRandom;
    let mut hit = [false; 3];
    for _ in 0..500 {
        hit[rng.below(3).unwrap()] = true;
    }
    assert_eq!(hit, [true; 3]);
    assert_eq!(rng.below(1).unwrap(), 0);
}

#[test]
fn options_list_represented_decades_and_count_or_choices_independently() {
    let (_d, mut db) = open_temp();
    for (title, year, genre) in [
        ("A", "1962", "Jazz"),
        ("B", "1969", "Rock"),
        ("C", "2016", "Rock"),
        ("D", "2026", "Pop"),
    ] {
        entry(&mut db, title, Some(year), &[genre]);
    }
    let all = db
        .read(|c| next_up::options(c, GuidedCriteria::default(), YEAR))
        .unwrap();
    let decades: Vec<i32> = all.decades.iter().map(|d| d.value).collect();
    assert_eq!(decades, [1960, 2010, 2020], "no 2016 bubble");
    assert_eq!(all.current_year.as_ref().map(|c| c.value), Some(2026));
    assert_eq!(all.match_count, 4);

    // 1960s selected: 2010s still counts its own album (OR within Decade), while the
    // Genre counts reflect the Decade selection (AND between categories).
    let sixties = GuidedCriteria {
        years: Filter::AnyOf(vec![YearChoice::Decade(1960)]),
        ..GuidedCriteria::default()
    };
    let o = db.read(|c| next_up::options(c, sixties, YEAR)).unwrap();
    assert_eq!(o.match_count, 2);
    let counts: Vec<(i32, usize)> = o.decades.iter().map(|d| (d.value, d.count)).collect();
    assert_eq!(counts, [(1960, 2), (2010, 1), (2020, 1)]);
    let genres: Vec<(&str, usize)> = o
        .genres
        .iter()
        .map(|g| (g.name.as_str(), g.count))
        .collect();
    assert_eq!(genres, [("Jazz", 1), ("Pop", 0), ("Rock", 1)]);
}

#[test]
fn options_show_every_tag_with_where_it_is_used() {
    let (_d, mut db) = open_temp();
    let (listed, _) = entry(&mut db, "Listed", Some("2001"), &["Rock"]);
    let unused = db.write(|tx| tags::create(tx, "Unused", None)).unwrap().id;
    let elsewhere = db
        .write(|tx| tags::create(tx, "Elsewhere", None))
        .unwrap()
        .id;
    let on_list = db.write(|tx| tags::create(tx, "On list", None)).unwrap().id;
    let x = artist(&mut db, "X");
    let off = common::album(&mut db, "Off list", &[&x]);
    db.write(|tx| catalogue::tag_album(tx, &off, &elsewhere))
        .unwrap();
    db.write(|tx| catalogue::tag_album(tx, &listed, &on_list))
        .unwrap();

    let o = db
        .read(|c| next_up::options(c, GuidedCriteria::default(), YEAR))
        .unwrap();
    let summary: Vec<(&str, usize, usize)> = o
        .tags
        .iter()
        .map(|t| (t.name.as_str(), t.on_listen_list, t.anywhere))
        .collect();
    assert_eq!(
        summary,
        [
            ("Listen ASAP", 0, 0),
            ("Elsewhere", 0, 1),
            ("On list", 1, 1),
            ("Unused", 0, 0)
        ]
    );
    assert!(o.tags.iter().any(|t| t.id == unused));
}

#[test]
fn state_explains_the_match_and_clear_selects_nothing_new() {
    let (_d, mut db) = open_temp();
    let road = db
        .write(|tx| tags::create(tx, "Road trip", None))
        .unwrap()
        .id;
    let (al, _) = entry(&mut db, "Match", Some("1994"), &["Rock", "Pop"]);
    db.write(|tx| catalogue::tag_album(tx, &al, &road)).unwrap();
    let guided = method(serde_json::json!({ "mode": "guided", "criteria": {
        "years": { "any_of": [{ "decade": 1990 }, { "decade": 2000 }] },
        "genres": { "any_of": ["rock", "Jazz"] },
        "tagIds": { "any_of": [road] }
    }}));
    picked(roll(&mut db, &guided));
    let current = slot(&db, 1).current.unwrap();
    assert_eq!(current.item.title, "Match");
    assert_eq!(current.matched.years, [YearChoice::Decade(1990)]);
    assert_eq!(current.matched.genres, ["Rock"]);
    assert_eq!(current.matched.tags.len(), 1);
    let Some(Method::Guided(stored)) = current.matched.method else {
        panic!("guided method expected")
    };
    assert_eq!(
        stored.genres,
        Filter::AnyOf(vec!["jazz".into(), "rock".into()])
    );

    // Reading state repeatedly never rolls.
    db.read(|c| next_up::state(c, YEAR)).unwrap();
    assert_eq!(count(&db, "selection_attempt"), 1);

    assert!(db.write(|tx| next_up::clear(tx, 1)).unwrap());
    assert_eq!(attempt_status(&db, &current.attempt_id), "skipped");
    assert!(slot(&db, 1).current.is_none());
    assert!(!db.write(|tx| next_up::clear(tx, 1)).unwrap());
    assert_eq!(count(&db, "listen_event"), 0);
    assert_eq!(count(&db, "listen_list_entry"), 1);
}

fn roll_in(db: &mut Database, slot: i64, m: &Method) -> RollOutcome {
    db.write(|tx| next_up::roll(tx, Some(&new_id()), slot, m.clone(), YEAR, &mut First))
        .unwrap()
}

fn used(db: &Database) -> Vec<i64> {
    db.read(|c| next_up::state(c, YEAR))
        .unwrap()
        .slots
        .iter()
        .map(|s| s.slot)
        .collect()
}

#[test]
fn slots_pick_independently_and_never_share_an_album() {
    let (_d, mut db) = open_temp();
    let (a, _) = entry(&mut db, "A", Some("2001"), &["Rock"]);
    let (b, _) = entry(&mut db, "B", Some("2002"), &["Rock"]);
    let (c, _) = entry(&mut db, "C", Some("2003"), &["Pop"]);
    let random = Method::CompletelyRandom;

    let one = picked(roll_in(&mut db, 1, &random));
    assert_eq!(one.album_id, a);
    let two = picked(roll_in(&mut db, 2, &random));
    assert_eq!((two.slot, two.album_id.as_str()), (2, b.as_str()));
    assert_ne!(one.session_id, two.session_id, "each slot has its own pool");

    // Rerolling slot 2 leaves slot 1 alone and never offers its album.
    let again = picked(roll_in(&mut db, 2, &random));
    assert_eq!(
        (again.album_id.as_str(), again.session_id.as_str()),
        (c.as_str(), two.session_id.as_str())
    );
    assert_eq!(attempt_status(&db, &one.attempt_id), "shown");
    assert_eq!(attempt_status(&db, &two.attempt_id), "skipped");
    assert_eq!(
        roll_in(&mut db, 2, &random),
        RollOutcome::Exhausted { pool_size: 2 }
    );
    assert_eq!(used(&db), [1, 2]);
    assert_eq!(
        db.read(|c| next_up::state(c, YEAR)).unwrap().next_free_slot,
        Some(3)
    );
    // Slot 1 has seen A; C is slot 2's pick, so only B can still come up.
    assert_eq!(slot(&db, 1).session.unwrap().remaining, 1);
    assert!(slot(&db, 2).session.unwrap().exhausted);

    assert_eq!(picked(roll_in(&mut db, 3, &random)).album_id, b);
    // Every album is now some slot's pick, which differs from nothing matching at all;
    // neither outcome starts a slot.
    assert_eq!(
        roll_in(&mut db, 4, &random),
        RollOutcome::Empty {
            reason: EmptyReason::AllInNextUp
        }
    );
    assert_eq!(
        roll_in(&mut db, 4, &Method::WeightedRandom),
        RollOutcome::Empty {
            reason: EmptyReason::NoListenAsap
        }
    );
    assert_eq!(used(&db), [1, 2, 3]);

    // Pools reset per slot.
    assert_eq!(db.write(|tx| next_up::reset_pool(tx, 2)).unwrap(), 2);
    assert_eq!(slot(&db, 1).session.unwrap().pool_cycle, 1);
    assert_eq!(
        db.write(|tx| next_up::reset_pool(tx, 4))
            .unwrap_err()
            .code(),
        "not_found"
    );
    for bad in [0, 6] {
        let err = db
            .write(|tx| next_up::roll(tx, None, bad, random.clone(), YEAR, &mut First))
            .unwrap_err();
        assert_eq!(err.code(), "validation");
    }
}

#[test]
fn manual_picks_fill_free_slots_and_remove_or_listen_touch_one_slot() {
    let (_d, mut db) = open_temp();
    let albums: Vec<(String, String)> = (0..6)
        .map(|n| entry(&mut db, &format!("Album {n}"), Some("1999"), &["Rock"]))
        .collect();
    let choose = |db: &mut Database, ed: &str, slot: Option<i64>| {
        db.write(|tx| next_up::choose(tx, Some(&new_id()), ed, slot))
    };

    let rolled = picked(roll(&mut db, &Method::CompletelyRandom));
    assert_eq!(rolled.album_id, albums[0].0);
    for (i, (_, ed)) in albums[1..5].iter().enumerate() {
        let shown = choose(&mut db, ed, None).unwrap();
        assert_eq!(
            (shown.slot, shown.source),
            (i as i64 + 2, SelectionSource::Manual)
        );
    }
    assert_eq!(
        attempt_status(&db, &rolled.attempt_id),
        "shown",
        "adding never replaces"
    );
    // An album already picked can't be added twice; a sixth needs a slot to replace.
    assert_eq!(
        choose(&mut db, &albums[1].1, None).unwrap_err().code(),
        "validation"
    );
    assert_eq!(
        choose(&mut db, &albums[5].1, None).unwrap_err().code(),
        "validation"
    );
    let manual = choose(&mut db, &albums[5].1, Some(1)).unwrap();
    assert_eq!(manual.slot, 1);
    assert_eq!(attempt_status(&db, &rolled.attempt_id), "skipped");
    assert_eq!(
        slot(&db, 1).session.unwrap().session_id,
        rolled.session_id,
        "slot 1 keeps its picking method"
    );

    // Removing slot 3 frees it without touching the others.
    let third = slot(&db, 3).current.unwrap();
    assert!(db.write(|tx| next_up::clear(tx, 3)).unwrap());
    assert_eq!(attempt_status(&db, &third.attempt_id), "skipped");
    assert_eq!(used(&db), [1, 2, 4, 5]);
    assert_eq!(
        db.read(|c| next_up::state(c, YEAR)).unwrap().next_free_slot,
        Some(3)
    );
    assert!(!db.write(|tx| next_up::clear(tx, 3)).unwrap());

    // Listening empties slot 1 but keeps its method; undo puts the pick back there.
    let logged = db
        .write(|tx| {
            listening::log(
                tx,
                Some(&new_id()),
                &LogListen {
                    edition_id: albums[5].1.clone(),
                    listened_on: Some("2026-10-01".into()),
                    kind: ListenKind::First,
                    earlier_undated: false,
                    attempt_id: Some(manual.attempt_id.clone()),
                },
            )
        })
        .unwrap();
    assert!(logged.completed_next_up);
    let one = slot(&db, 1);
    assert!(one.current.is_none());
    assert_eq!(one.session.unwrap().session_id, rolled.session_id);
    assert_eq!(slot(&db, 2).current.unwrap().album_id, albums[1].0);
    db.write(|tx| listening::undo_log(tx, &logged.undo))
        .unwrap();
    assert_eq!(slot(&db, 1).current.unwrap().attempt_id, manual.attempt_id);
}
