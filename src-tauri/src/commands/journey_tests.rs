//! The whole product journey against a real SQLite profile, with MusicBrainz served by a
//! scripted transport: CSV → match → tag → guided pick → reroll → rate tracks → log listen
//! → Collection → Artist/Stats → export → restore. Edge cases ride along: a manual-only
//! row, an explicit zero rating, Deluxe bonus tracks, an unused tag, no Listen ASAP
//! candidates, and earlier listening with unknown dates.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use super::metadata::MetadataService;
use super::profile::{export_to, inspect, restore_from};
use crate::csv_import::commit::{self, TagDecisions};
use crate::csv_import::{enrich, mapping, parse, staging};
use crate::domain::picker::{EmptyReason, Method};
use crate::library::listening::{self, ListenKind, LogListen};
use crate::library::next_up::{self, RollOutcome};
use crate::library::{catalogue, discography, listing, ratings, stats, tags};
use crate::metadata::cache::MemoryCache;
use crate::metadata::musicbrainz::MusicBrainz;
use crate::metadata::testing::{MockTransport, Reply};
use crate::paths::StorageProfile;
use crate::state::AppState;

const BJORK: &str = "87c5dedd-371d-4a53-9f7f-80522fb7f3cb";
const RG: &str = "10000000-0000-4000-8000-000000000001";
const STD: &str = "20000000-0000-4000-8000-000000000001";
const DLX: &str = "20000000-0000-4000-8000-000000000002";
const ASAP: &str = crate::library::catalogue::LISTEN_ASAP_TAG_ID;
const YEAR: i32 = 2026;

fn credit() -> String {
    format!(r#"[{{"name": "Björk", "artist": {{"id": "{BJORK}", "name": "Björk"}}}}]"#)
}

fn responder(url: &str, _: usize) -> Reply {
    if url.contains("/release-group/") {
        return Reply::Json(
            200,
            format!(
                r#"{{"id": "{RG}", "title": "Homogenic", "first-release-date": "1997-09-22",
                    "artist-credit": {}, "genres": [{{"name": "electronic", "count": 3}}], "tags": []}}"#,
                credit()
            ),
        );
    }
    // Standard: 2 tracks. Deluxe: the same 2 recordings plus a bonus track.
    let (id, title, extra) = if url.contains(DLX) {
        (
            DLX,
            "Homogenic (Deluxe)",
            r#", {"id": "30000000-0000-4000-8000-000000000013", "position": 3, "title": "Bonus",
                "length": 200000, "recording": {"id": "40000000-0000-4000-8000-000000000003", "title": "Bonus"}}"#,
        )
    } else {
        (STD, "Homogenic", "")
    };
    let n = if id == DLX { 1 } else { 0 };
    Reply::Json(
        200,
        format!(
            r#"{{"id": "{id}", "title": "{title}", "date": "1997-09-22", "release-group": {{"id": "{RG}"}},
                "artist-credit": {},
                "media": [{{"position": 1, "format": "CD", "tracks": [
                    {{"id": "30000000-0000-4000-8000-00000000{n}011", "position": 1, "title": "Hunter", "length": 250000,
                      "recording": {{"id": "40000000-0000-4000-8000-000000000001", "title": "Hunter"}}}},
                    {{"id": "30000000-0000-4000-8000-00000000{n}012", "position": 2, "title": "Jóga", "length": 300000,
                      "recording": {{"id": "40000000-0000-4000-8000-000000000002", "title": "Jóga"}}}}{extra}]}}]}}"#,
            credit()
        ),
    )
}

#[tokio::test(start_paused = true)]
async fn full_journey_from_csv_to_restore() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::open(StorageProfile::Development, dir.path().join("live"));
    let svc = MetadataService::new(Arc::new(MusicBrainz::new(
        Arc::new(MockTransport::new(responder)),
        Arc::new(MemoryCache::default()),
    )));

    // ---- CSV → match: two editions with exact IDs, one manual-only row (no IDs, no network).
    let csv = format!(
        "Album,Artist,Year,Edition,Tags,MusicBrainz Release Group ID,MusicBrainz Release ID\n\
         Homogenic,Björk,1997,Standard,Night,{RG},{STD}\n\
         Homogenic,Björk,1997,Deluxe,,{RG},{DLX}\n\
         Basement Demo,Local Band,,,,,\n"
    );
    let parsed = parse::parse(csv.as_bytes()).unwrap();
    let session = state
        .with_db(|db| {
            db.write(|tx| {
                let id = staging::create_session(tx, "albums.csv", &parsed)?;
                staging::apply_mapping(tx, &id, &mapping::suggest(&parsed.headers))?;
                Ok(id)
            })
        })
        .unwrap();
    let progress = enrich::enrich(
        svc.provider(),
        &state,
        &session,
        10,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert!(progress.stopped.is_none());
    let report = state
        .with_db(|db| {
            db.write(|tx| {
                commit::commit(
                    tx,
                    &session,
                    &TagDecisions {
                        create: vec!["Night".into()],
                        skip: vec![],
                    },
                )
            })
        })
        .unwrap();
    assert_eq!(report.errors, 0, "{report:?}");
    assert_eq!((report.albums_created, report.editions_created), (2, 3));

    let list = |s: &AppState, source| {
        s.with_db(|db| db.read(|c| listing::query(c, source, &listing::ListQuery::default())))
            .unwrap()
    };
    let listen_list = list(&state, listing::Source::ListenList);
    assert_eq!(
        listen_list.total, 2,
        "one entry per album, however many editions"
    );
    let homogenic = listen_list
        .items
        .iter()
        .find(|i| i.title == "Homogenic")
        .unwrap()
        .clone();
    let manual = listen_list
        .items
        .iter()
        .find(|i| i.title == "Basement Demo")
        .unwrap()
        .clone();

    // ---- Tags: an unused tag; Listen ASAP on the matched album only.
    let unused = state
        .with_db(|db| db.write(|tx| tags::create(tx, "Unused", Some("#34d399"))))
        .unwrap();
    state
        .with_db(|db| db.write(|tx| catalogue::tag_album(tx, &homogenic.album_id, ASAP)))
        .unwrap();
    let options = state
        .with_db(|db| db.read(|c| next_up::options(c, Default::default(), YEAR)))
        .unwrap();
    let unused_opt = options.tags.iter().find(|t| t.id == unused.id).unwrap();
    assert_eq!((unused_opt.on_listen_list, unused_opt.anywhere), (0, 0));

    // ---- Guided pick → reroll (pool exhausted, nothing else changes).
    let night = state
        .with_db(|db| db.read(|c| tags::find_by_name(c, "Night", None)))
        .unwrap()
        .unwrap();
    let guided: Method = serde_json::from_value(serde_json::json!({
        "mode": "guided", "criteria": { "tagIds": { "any_of": [night.0] } }
    }))
    .unwrap();
    let mut rng = next_up::OsRandom;
    let pick = state
        .with_db(|db| db.write(|tx| next_up::roll(tx, None, guided.clone(), YEAR, &mut rng)))
        .unwrap();
    let RollOutcome::Picked { shown, .. } = pick else {
        panic!("expected a pick, got {pick:?}")
    };
    assert_eq!(shown.album_id, homogenic.album_id);
    let reroll = state
        .with_db(|db| db.write(|tx| next_up::roll(tx, None, guided.clone(), YEAR, &mut rng)))
        .unwrap();
    assert_eq!(reroll, RollOutcome::Exhausted { pool_size: 1 });

    // ---- Rate tracks on the Deluxe edition: an explicit zero is a rating.
    let (dlx_edition, dlx_tracks) = state
        .with_db(|db| {
            db.read(|c| {
                let id: String = c.query_row(
                    "SELECT id FROM edition WHERE musicbrainz_release_id = ?1",
                    [DLX],
                    |r| r.get(0),
                )?;
                let mut st =
                    c.prepare("SELECT id FROM track WHERE edition_id = ?1 ORDER BY position")?;
                let tracks: Vec<String> = st
                    .query_map([&id], |r| r.get(0))?
                    .collect::<Result<_, _>>()?;
                Ok((id, tracks))
            })
        })
        .unwrap();
    assert_eq!(dlx_tracks.len(), 3);
    state
        .with_db(|db| {
            db.write(|tx| {
                ratings::set_track(tx, &dlx_tracks[0], Some(0))?;
                ratings::set_track(tx, &dlx_tracks[1], Some(0))?;
                Ok(())
            })
        })
        .unwrap();

    // ---- Log the guided pick as listened (full album, Deluxe), with earlier undated history.
    let logged = state
        .with_db(|db| {
            db.write(|tx| {
                listening::log(
                    tx,
                    None,
                    &LogListen {
                        edition_id: dlx_edition.clone(),
                        listened_on: Some("2026-09-30".into()),
                        kind: ListenKind::Relisten,
                        earlier_undated: true,
                        track_ids: None,
                        attempt_id: Some(shown.attempt_id.clone()),
                    },
                )
            })
        })
        .unwrap();
    assert!(logged.completed_next_up && logged.added_to_collection);
    // ---- No Listen ASAP candidates left on the list: Weighted Random says so, no fallback.
    let weighted = state
        .with_db(|db| {
            db.write(|tx| next_up::roll(tx, None, Method::WeightedRandom, YEAR, &mut rng))
        })
        .unwrap();
    assert_eq!(
        weighted,
        RollOutcome::Empty {
            reason: EmptyReason::NoListenAsap
        }
    );

    // A listen of the manual album on an unknown date.
    let manual_listen = state
        .with_db(|db| {
            db.write(|tx| {
                listening::log(
                    tx,
                    None,
                    &LogListen {
                        edition_id: manual.edition_id.clone(),
                        listened_on: None,
                        kind: ListenKind::Unspecified,
                        earlier_undated: false,
                        track_ids: None,
                        attempt_id: None,
                    },
                )
            })
        })
        .unwrap();
    assert_eq!(
        manual_listen.coverage, "unknown",
        "no tracklist: coverage is unknown, not zero"
    );

    // ---- Collection, Artist, Stats.
    let collection = list(&state, listing::Source::Collection);
    assert_eq!(collection.total, 2);
    let bjork_id: String = state
        .with_db(|db| {
            db.read(|c| {
                Ok(c.query_row(
                    "SELECT id FROM artist WHERE musicbrainz_id = ?1",
                    [BJORK],
                    |r| r.get(0),
                )?)
            })
        })
        .unwrap();
    let artist = state
        .with_db(|db| db.read(|c| discography::catalogue(c, &bjork_id)))
        .unwrap();
    let album = artist
        .entries
        .iter()
        .find(|e| e.title == "Homogenic")
        .unwrap();
    assert!(album.listened);
    assert_eq!(
        album.reference.edition_id.as_deref(),
        Some(dlx_edition.as_str())
    );
    // The Deluxe full listen covered all three, bonus included.
    assert_eq!(
        album.tracks.as_ref().map(|t| (t.listened, t.total)),
        Some((3, 3))
    );
    assert_eq!(
        album.score.as_ref().map(|s| s.stars),
        Some(0.0),
        "explicit zeros count"
    );
    let overview = state
        .with_db(|db| db.read(|c| stats::overview(c, "2026-10-02T00:00:00.000Z")))
        .unwrap();
    assert_eq!(overview.collection.ratings.rated_albums, 1);
    assert_eq!(overview.collection.best_artist.winners[0].label, "Björk");
    assert_eq!(overview.collection.listens.historical, 1);
    assert_eq!(overview.collection.listens.undated, 1);
    let guided_stats = overview
        .selection
        .iter()
        .find(|m| m.source == "guided")
        .unwrap();
    assert_eq!((guided_stats.shown, guided_stats.completed), (1, 1));

    // ---- Export → restore elsewhere: the same Stats and Artist view come back.
    let archive = dir.path().join("journey.mudraft");
    export_to(&state, &archive, true).unwrap();
    let restored = AppState::open(StorageProfile::Development, dir.path().join("restored"));
    let preview = inspect(&restored, &archive).unwrap();
    restore_from(Some(&svc), &restored, &archive, &preview.sha256).unwrap();
    let again = restored
        .with_db(|db| db.read(|c| stats::overview(c, "2026-10-02T00:00:00.000Z")))
        .unwrap();
    assert_eq!(
        serde_json::to_value(&again).unwrap(),
        serde_json::to_value(&overview).unwrap()
    );
    let artist_again = restored
        .with_db(|db| db.read(|c| discography::catalogue(c, &bjork_id)))
        .unwrap();
    assert_eq!(
        serde_json::to_value(&artist_again.entries).unwrap(),
        serde_json::to_value(&artist.entries).unwrap()
    );
}
