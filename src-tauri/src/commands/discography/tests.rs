use std::sync::Arc;

use super::*;
use crate::commands::metadata::MetadataService;
use crate::domain::ids::new_id;
use crate::library::catalogue::{
    self, CreditInput, NewAlbum, NewArtist, NewEdition, NewRecording, NewTrack,
};
use crate::library::discography::{CoverageStatus, EntrySource, ReferenceKind};
use crate::library::listening::{self, ListenKind, LogListen};
use crate::library::personal;
use crate::metadata::cache::MemoryCache;
use crate::metadata::musicbrainz::MusicBrainz;
use crate::metadata::testing::{MockTransport, Reply};
use crate::paths::StorageProfile;

const ARTIST_MB: &str = "a74b1b7f-71a5-4011-9441-d0b5e4122711";
const PARTNER_MB: &str = "b0000000-0000-4000-8000-000000000001";

fn rg(n: u32) -> String {
    format!("10000000-0000-4000-8000-{n:012}")
}
fn rel(n: u32) -> String {
    format!("20000000-0000-4000-8000-{n:012}")
}
fn mbtrack(n: u32) -> String {
    format!("30000000-0000-4000-8000-{n:012}")
}
fn rec(n: u32) -> String {
    format!("40000000-0000-4000-8000-{n:012}")
}

fn offset_of(url: &str) -> u32 {
    url.split("offset=")
        .nth(1)
        .and_then(|s| s.split('&').next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn group_json(n: u32, primary: &str, secondary: &[&str], collab: bool) -> String {
    let partner = if collab {
        format!(
            r#", {{"name": "Partner", "joinphrase": "", "artist": {{"id": "{PARTNER_MB}", "name": "Partner"}}}}"#
        )
    } else {
        String::new()
    };
    format!(
        r#"{{"id": "{}", "title": "Album {n}", "primary-type": "{primary}",
            "secondary-types": {}, "first-release-date": "{}",
            "artist-credit": [{{"name": "Artist", "joinphrase": "{}", "artist": {{"id": "{ARTIST_MB}", "name": "Artist"}}}}{partner}]}}"#,
        rg(n),
        serde_json::to_string(secondary).unwrap(),
        1960 + n,
        if collab { " & " } else { "" },
    )
}

/// A catalogue of `total` release groups served 100 per page. Group 1 is a collaboration,
/// group 2 an EP, group 3 a live album, group 4 a compilation; the rest are studio albums.
fn browse(url: &str, total: u32) -> Reply {
    let offset = offset_of(url);
    let groups: Vec<String> = (offset..(offset + 100).min(total))
        .map(|i| {
            let n = i + 1;
            match n {
                1 => group_json(n, "Album", &[], true),
                2 => group_json(n, "EP", &[], false),
                3 => group_json(n, "Album", &["Live"], false),
                4 => group_json(n, "Album", &["Compilation"], false),
                _ => group_json(n, "Album", &[], false),
            }
        })
        .collect();
    Reply::Json(
        200,
        format!(
            r#"{{"release-group-count": {total}, "release-group-offset": {offset}, "release-groups": [{}]}}"#,
            groups.join(",")
        ),
    )
}

/// Three pressings of every album: an official original (2 tracks), a later official
/// deluxe (3 tracks), and an earlier bootleg.
fn editions(url: &str) -> Reply {
    let n: u32 = url
        .split("release-group=")
        .nth(1)
        .and_then(|s| s.split('&').next())
        .and_then(|id| id.rsplit('-').next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let release = |m: u32, status: &str, date: &str, tracks: u32| {
        format!(
            r#"{{"id": "{}", "title": "Album {n}", "status": "{status}", "date": "{date}",
                "media": [{{"format": "CD", "track-count": {tracks}}}]}}"#,
            rel(n * 10 + m)
        )
    };
    Reply::Json(
        200,
        format!(
            r#"{{"release-count": 3, "releases": [{}, {}, {}]}}"#,
            release(2, "Official", "2017-06-23", 3),
            release(1, "Official", "1997-05-21", 2),
            release(3, "Bootleg", "1990", 2)
        ),
    )
}

fn release(url: &str) -> Reply {
    let id = url
        .split("/release/")
        .nth(1)
        .unwrap()
        .split('?')
        .next()
        .unwrap();
    let n: u32 = id.rsplit('-').next().unwrap().parse().unwrap();
    let group = n / 10;
    let count = if n % 10 == 2 { 3 } else { 2 };
    let tracks: Vec<String> = (1..=count)
        .map(|p| {
            format!(
                r#"{{"id": "{}", "position": {p}, "title": "Song {p}", "recording": {{"id": "{}", "title": "Song {p}"}}}}"#,
                mbtrack(n * 10 + p),
                rec(group * 10 + p)
            )
        })
        .collect();
    Reply::Json(
        200,
        format!(
            r#"{{"id": "{id}", "title": "Album {group}", "release-group": {{"id": "{}"}},
                "media": [{{"position": 1, "tracks": [{}]}}]}}"#,
            rg(group),
            tracks.join(",")
        ),
    )
}

fn route(url: &str, total: u32) -> Reply {
    if url.contains("/release-group/") {
        let id = url
            .split("/release-group/")
            .nth(1)
            .unwrap()
            .split('?')
            .next()
            .unwrap();
        Reply::Json(
            200,
            format!(
                r#"{{"id": "{id}", "title": "Album", "artist-credit": [{{"name": "Artist",
                    "artist": {{"id": "{ARTIST_MB}", "name": "Artist"}}}}], "genres": [], "tags": []}}"#
            ),
        )
    } else if url.contains("/release-group?") {
        browse(url, total)
    } else if url.contains("/release?") {
        editions(url)
    } else {
        release(url)
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    state: AppState,
    svc: MetadataService,
    transport: Arc<MockTransport>,
    cache: Arc<MemoryCache>,
    artist: String,
}

fn fixture(responder: impl Fn(&str, usize) -> Reply + Send + Sync + 'static) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::open(StorageProfile::Development, dir.path().to_path_buf());
    let transport = Arc::new(MockTransport::new(responder));
    let cache = Arc::new(MemoryCache::default());
    let svc = MetadataService::new(Arc::new(MusicBrainz::new(transport.clone(), cache.clone())));
    let artist = artist(&state, "Artist", Some(ARTIST_MB));
    Fixture {
        _dir: dir,
        state,
        svc,
        transport,
        cache,
        artist,
    }
}

fn artist(state: &AppState, name: &str, mbid: Option<&str>) -> String {
    state
        .with_db(|db| {
            db.write(|tx| {
                catalogue::create_artist(
                    tx,
                    None,
                    &NewArtist {
                        name: name.into(),
                        sort_name: None,
                        musicbrainz_id: mbid.map(Into::into),
                    },
                )
            })
        })
        .unwrap()
}

fn view(f: &Fixture) -> ArtistCatalogue {
    f.state
        .with_db(|db| db.read(|c| discography::catalogue(c, &f.artist)))
        .unwrap()
}

#[tokio::test(start_paused = true)]
async fn fetches_every_page_with_official_filter_and_labels_types() {
    let f = fixture(|url, _| route(url, 150));
    let run = fetch_catalogue(&f.svc, &f.state, &new_id(), &f.artist)
        .await
        .unwrap();
    assert_eq!((run.pages_fetched, run.complete), (2, true));
    let urls = f.transport.urls();
    assert_eq!(urls.len(), 2);
    assert!(
        urls.iter()
            .all(|u| u.contains("release-group-status=website-default"))
    );
    assert_eq!(
        offset_of(&urls[1]),
        100,
        "advances by what the page returned"
    );

    let v = view(&f);
    assert_eq!(v.coverage.status, CoverageStatus::Complete);
    assert_eq!(
        (v.coverage.provider_total, v.coverage.fetched_entries),
        (Some(150), 150)
    );
    // Default scope: studio albums only. EP, live album, and compilation are labelled
    // and listed, not counted.
    assert_eq!(v.progress.albums_counted, 147);
    let label = |t: &str| v.entries.iter().find(|e| e.title == t).unwrap();
    assert_eq!(label("Album 2").type_label, "EP");
    assert_eq!(label("Album 3").type_label, "Album · Live");
    assert_eq!(label("Album 4").type_label, "Album · Compilation");
    assert!(!label("Album 2").counted && !label("Album 3").counted);
    assert!(v.denominator.contains("type Album with no secondary type"));
    // Collaborations are in the catalogue and flagged.
    let collab = label("Album 1");
    assert!(collab.collaboration && collab.counted);
    assert_eq!(collab.credit, "Artist & Partner");
    // Discovery never touches the lists.
    let counts = f
        .state
        .with_db(|db| {
            db.read(|c| {
                Ok(c.query_row(
                    "SELECT (SELECT COUNT(*) FROM listen_list_entry) + (SELECT COUNT(*) FROM collection_entry)
                          + (SELECT COUNT(*) FROM album)",
                    [],
                    |r| r.get::<_, i64>(0),
                )?)
            })
        })
        .unwrap();
    assert_eq!(counts, 0);

    // Explicitly including EPs and live albums widens the denominator.
    f.state
        .with_db(|db| {
            db.write(|tx| {
                discography::set_scope(
                    tx,
                    &f.artist,
                    &Scope {
                        primary_types: vec!["album".into(), "EP".into()],
                        secondary_types: vec!["Live".into()],
                    },
                )
            })
        })
        .unwrap();
    assert_eq!(view(&f).progress.albums_counted, 149);
}

#[tokio::test(start_paused = true)]
async fn failure_and_cancellation_keep_pages_and_resume() {
    // Page 2 is offline the first time; afterwards it works.
    let f = fixture(|url, n| {
        if offset_of(url) == 100 && n == 1 {
            Reply::Offline
        } else {
            route(url, 250)
        }
    });
    let err = fetch_catalogue(&f.svc, &f.state, &new_id(), &f.artist)
        .await
        .unwrap_err();
    assert_eq!(err.code(), "offline");
    let v = view(&f);
    assert_eq!(v.coverage.status, CoverageStatus::Partial);
    assert_eq!(
        (v.coverage.fetched_entries, v.coverage.next_offset),
        (100, 100)
    );
    assert!(v.coverage.last_error.is_some());
    assert!(v.coverage.note.contains("100 of 250"));

    // Cancel while page 3 hangs.
    let hanging = fixture(|url, _| {
        if offset_of(url) == 200 {
            Reply::Hang
        } else {
            route(url, 250)
        }
    });
    let id = new_id();
    let (result, _) = tokio::join!(
        fetch_catalogue(&hanging.svc, &hanging.state, &id, &hanging.artist),
        async {
            while hanging.transport.call_count() < 3 {
                tokio::task::yield_now().await;
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            hanging.svc.cancel(&id).unwrap();
        }
    );
    assert_eq!(result.unwrap_err().code(), "cancelled");
    assert_eq!(view(&hanging).coverage.next_offset, 200);

    // Resume the first fixture: it continues at offset 100, not from the start.
    let before = f.transport.call_count();
    let run = fetch_catalogue(&f.svc, &f.state, &new_id(), &f.artist)
        .await
        .unwrap();
    assert!(run.complete);
    assert_eq!(offset_of(&f.transport.urls()[before]), 100);
    let v = view(&f);
    assert_eq!(
        (v.coverage.status, v.coverage.fetched_entries),
        (CoverageStatus::Complete, 250)
    );
    assert!(v.coverage.last_error.is_none());
}

#[tokio::test(start_paused = true)]
async fn offline_refresh_uses_the_cache_and_keeps_discoveries() {
    let f = fixture(|url, n| {
        if n < 1 {
            route(url, 20)
        } else {
            Reply::Offline
        }
    });
    fetch_catalogue(&f.svc, &f.state, &new_id(), &f.artist)
        .await
        .unwrap();
    f.cache.expire_all();
    let run = fetch_catalogue(&f.svc, &f.state, &new_id(), &f.artist)
        .await
        .unwrap();
    assert!(run.used_offline_cache && run.complete);
    assert_eq!(view(&f).coverage.fetched_entries, 20);
}

#[tokio::test(start_paused = true)]
async fn tracklists_use_one_representative_and_report_partial_coverage() {
    // Album 6's release lookups fail on the server.
    let f = fixture(|url, _| {
        if url.contains(&format!("/release/{}", rel(61))) {
            Reply::Json(500, String::new())
        } else {
            route(url, 7)
        }
    });
    fetch_catalogue(&f.svc, &f.state, &new_id(), &f.artist)
        .await
        .unwrap();
    let run = load_tracklists(&f.svc, &f.state, &new_id(), &f.artist)
        .await
        .unwrap();
    // Counted: albums 1, 5, 6, 7 (2–4 are EP/live/compilation and are not loaded).
    assert_eq!(run.loaded, 3);
    assert_eq!(run.failed.len(), 1);
    assert_eq!(run.failed[0].release_group_id, rg(6));
    assert!(
        run.stopped.is_none(),
        "a single album's failure doesn't stop the run"
    );

    let v = view(&f);
    let five = v.entries.iter().find(|e| e.title == "Album 5").unwrap();
    // Earliest *official* pressing, not the bootleg; its 2 tracks, not the deluxe's 3
    // and never the union of every pressing.
    assert_eq!(five.reference.kind, ReferenceKind::Representative);
    assert_eq!(
        five.reference.musicbrainz_release_id.as_deref(),
        Some(rel(51).as_str())
    );
    assert_eq!(five.reference.track_count, Some(2));
    assert!(five.reference.note.contains("3 MusicBrainz editions"));
    let six = v.entries.iter().find(|e| e.title == "Album 6").unwrap();
    assert_eq!(six.reference.track_count, None, "unknown, not zero");
    assert!(six.tracks.is_none() && six.reference.last_error.is_some());
    assert_eq!(v.progress.tracks_total, 6);
    assert_eq!(v.progress.albums_without_tracklist, 1);
    assert_eq!(v.coverage.tracklists_known, 3);

    // Resuming only retries what is still missing.
    let before = f.transport.call_count();
    let again = load_tracklists(&f.svc, &f.state, &new_id(), &f.artist)
        .await
        .unwrap();
    assert_eq!((again.loaded, again.failed.len()), (0, 1));
    let urls = &f.transport.urls()[before..];
    assert!(
        urls.iter()
            .all(|u| u.contains(&rg(6)) || u.contains(&rel(61)))
    );
}

#[tokio::test(start_paused = true)]
async fn offline_tracklist_run_stops_and_cancellation_keeps_loaded_albums() {
    let f = fixture(|url, n| if n < 3 { route(url, 7) } else { Reply::Offline });
    fetch_catalogue(&f.svc, &f.state, &new_id(), &f.artist)
        .await
        .unwrap();
    let run = load_tracklists(&f.svc, &f.state, &new_id(), &f.artist)
        .await
        .unwrap();
    assert_eq!(run.loaded, 1);
    assert_eq!(run.stopped.as_deref(), Some("offline"));
    assert_eq!(run.remaining, 3);

    // Calls: 1 browse page, then an editions browse and a release lookup per album.
    let g = fixture(|url, n| if n < 5 { route(url, 7) } else { Reply::Hang });
    fetch_catalogue(&g.svc, &g.state, &new_id(), &g.artist)
        .await
        .unwrap();
    let id = new_id();
    let (result, _) = tokio::join!(load_tracklists(&g.svc, &g.state, &id, &g.artist), async {
        while g.transport.call_count() < 6 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        g.svc.cancel(&id).unwrap();
    });
    assert_eq!(result.unwrap_err().code(), "cancelled");
    assert_eq!(
        view(&g).coverage.tracklists_known,
        2,
        "albums finished before cancel are kept"
    );
}

#[tokio::test(start_paused = true)]
async fn your_edition_is_the_reference_and_recordings_link_editions_not_titles() {
    let f = fixture(|url, _| route(url, 7));
    fetch_catalogue(&f.svc, &f.state, &new_id(), &f.artist)
        .await
        .unwrap();
    // The user has album 5 in two editions: A (listened) and B (on the Listen List).
    let (a_tracks, b_edition, b_tracks) = f
        .state
        .with_db(|db| {
            db.write(|tx| {
                let r1 = catalogue::create_recording(
                    tx,
                    None,
                    &NewRecording {
                        title: "Song 1".into(),
                        length_ms: None,
                        musicbrainz_recording_id: Some(rec(51)),
                    },
                )?;
                let album = catalogue::create_album(
                    tx,
                    None,
                    &NewAlbum {
                        title: "Album 5".into(),
                        original_date: Some("1965".into()),
                        musicbrainz_release_group_id: Some(rg(5)),
                        credits: vec![CreditInput {
                            artist_id: f.artist.clone(),
                            credited_name: None,
                            join_phrase: None,
                        }],
                        genres: vec![],
                    },
                )?;
                let track = |p: i64, title: &str, recording: Option<&String>| NewTrack {
                    disc_number: None,
                    position: p,
                    title: title.into(),
                    length_ms: None,
                    recording_id: recording.cloned(),
                    musicbrainz_track_id: None,
                    credits: vec![],
                };
                let a = catalogue::create_edition(
                    tx,
                    None,
                    &NewEdition {
                        album_id: album.clone(),
                        name: "Original".into(),
                        release_date: None,
                        musicbrainz_release_id: None,
                        tracks: vec![track(1, "Song 1", Some(&r1)), track(2, "Same Title", None)],
                    },
                )?;
                let b = catalogue::create_edition(
                    tx,
                    None,
                    &NewEdition {
                        album_id: album,
                        name: "Remaster".into(),
                        release_date: None,
                        musicbrainz_release_id: None,
                        tracks: vec![
                            track(1, "Song 1", Some(&r1)),
                            track(2, "Same Title", None),
                            track(3, "Bonus", None),
                        ],
                    },
                )?;
                personal::add_to_listen_list(tx, &b.edition_id)?;
                Ok((a, b.edition_id, b.track_ids))
            })
        })
        .unwrap();
    // Heard both tracks of edition A, and rated one of them.
    f.state
        .with_db(|db| {
            db.write(|tx| {
                listening::log(
                    tx,
                    None,
                    &LogListen {
                        edition_id: a_tracks.edition_id.clone(),
                        listened_on: None,
                        kind: ListenKind::First,
                        earlier_undated: false,
                        track_ids: Some(a_tracks.track_ids.clone()),
                        attempt_id: None,
                    },
                )?;
                personal::set_track_rating(
                    tx,
                    &a_tracks.track_ids[0],
                    Some(crate::domain::rating::HalfStars::new(8)?),
                )
            })
        })
        .unwrap();

    let v = view(&f);
    let fives: Vec<_> = v.entries.iter().filter(|e| e.title == "Album 5").collect();
    assert_eq!(fives.len(), 1, "two editions, one canonical album");
    let five = fives[0];
    assert_eq!(five.reference.kind, ReferenceKind::YourEdition);
    assert_eq!(
        five.reference.edition_id.as_deref(),
        Some(b_edition.as_str())
    );
    assert_eq!(five.reference.note, "Your Listen List edition.");
    // Track 1 shares recording R1 with the heard track; "Same Title" has no recording
    // identity, so it is not merged by title; the bonus track was never heard.
    let tracks = five.tracks.as_ref().unwrap();
    assert_eq!((tracks.listened, tracks.total), (1, 3));
    assert!(!five.listened);
    // Ratings stay on the edition that was rated.
    let b_rating = f
        .state
        .with_db(|db| db.read(|c| personal::track_rating(c, &b_tracks[0])))
        .unwrap();
    assert_eq!(b_rating.rating, None);
    // No representative is fetched for an album you own an edition of.
    let run = load_tracklists(&f.svc, &f.state, &new_id(), &f.artist)
        .await
        .unwrap();
    assert!(
        !f.transport
            .urls()
            .iter()
            .any(|u| u.contains(&rg(5)) && u.contains("/release?"))
    );
    assert_eq!(run.failed.len(), 0);
}

#[tokio::test(start_paused = true)]
async fn manual_artists_and_corrections_never_claim_a_complete_catalogue() {
    let f = fixture(|url, _| route(url, 7));
    let manual = artist(&f.state, "Local Band", None);
    let err = fetch_catalogue(&f.svc, &f.state, &new_id(), &manual)
        .await
        .unwrap_err();
    assert_eq!(err.code(), "validation");
    assert_eq!(f.transport.call_count(), 0);

    let entry = f
        .state
        .with_db(|db| {
            db.write(|tx| {
                discography::add_manual(
                    tx,
                    &manual,
                    &ManualEntry {
                        title: "Basement Tape".into(),
                        year: Some(2003),
                        primary_type: Some("album".into()),
                        secondary_types: vec![],
                    },
                )
            })
        })
        .unwrap();
    let v = f
        .state
        .with_db(|db| db.read(|c| discography::catalogue(c, &manual)))
        .unwrap();
    assert_eq!(v.coverage.status, CoverageStatus::LocalOnly);
    assert!(v.coverage.note.contains("not a complete catalogue"));
    assert_eq!(v.entries[0].source, EntrySource::Manual);
    assert_eq!(v.entries[0].reference.kind, ReferenceKind::None);
    assert_eq!(
        (
            v.progress.albums_counted,
            v.progress.albums_without_tracklist
        ),
        (1, 1)
    );
    assert_eq!(v.progress.tracks_total, 0);

    // Excluding a fetched entry survives a refetch.
    fetch_catalogue(&f.svc, &f.state, &new_id(), &f.artist)
        .await
        .unwrap();
    let wrong = view(&f)
        .entries
        .into_iter()
        .find(|e| e.title == "Album 7")
        .unwrap();
    f.state
        .with_db(|db| {
            db.write(|tx| discography::set_excluded(tx, wrong.id.as_deref().unwrap(), true))
        })
        .unwrap();
    fetch_catalogue(&f.svc, &f.state, &new_id(), &f.artist)
        .await
        .unwrap();
    let again = view(&f)
        .entries
        .into_iter()
        .find(|e| e.title == "Album 7")
        .unwrap();
    assert!(again.excluded && !again.counted);

    f.state
        .with_db(|db| db.write(|tx| discography::remove_manual(tx, &entry)))
        .unwrap();
    assert_eq!(
        f.state
            .with_db(
                |db| db.write(|tx| discography::remove_manual(tx, wrong.id.as_deref().unwrap()))
            )
            .unwrap_err()
            .code(),
        "not_found",
        "fetched entries can be excluded, not deleted"
    );
}

/// Local album for release group `n` with a Standard (recordings n1, n2) and a Deluxe
/// (n1, n2, plus a bonus track without a recording ID). Returns (album, standard, deluxe).
fn standard_and_deluxe(
    f: &Fixture,
    n: u32,
) -> (String, catalogue::EditionCreated, catalogue::EditionCreated) {
    f.state
        .with_db(|db| {
            db.write(|tx| {
                let recording = |m: u32| {
                    catalogue::create_recording(
                        tx,
                        None,
                        &NewRecording {
                            title: format!("Song {m}"),
                            length_ms: None,
                            musicbrainz_recording_id: Some(rec(n * 10 + m)),
                        },
                    )
                };
                let (r1, r2) = (recording(1)?, recording(2)?);
                let album = catalogue::create_album(
                    tx,
                    None,
                    &NewAlbum {
                        title: format!("Album {n}"),
                        original_date: Some(format!("{}", 1960 + n)),
                        musicbrainz_release_group_id: Some(rg(n)),
                        credits: vec![CreditInput {
                            artist_id: f.artist.clone(),
                            credited_name: None,
                            join_phrase: None,
                        }],
                        genres: vec![],
                    },
                )?;
                let track = |p: i64, title: &str, recording: Option<&String>| NewTrack {
                    disc_number: None,
                    position: p,
                    title: title.into(),
                    length_ms: None,
                    recording_id: recording.cloned(),
                    musicbrainz_track_id: None,
                    credits: vec![],
                };
                let edition = |name: &str, tracks: Vec<NewTrack>| {
                    catalogue::create_edition(
                        tx,
                        None,
                        &NewEdition {
                            album_id: album.clone(),
                            name: name.into(),
                            release_date: None,
                            musicbrainz_release_id: None,
                            tracks,
                        },
                    )
                };
                let standard = edition(
                    "Standard",
                    vec![track(1, "Song 1", Some(&r1)), track(2, "Song 2", Some(&r2))],
                )?;
                let deluxe = edition(
                    "Deluxe",
                    vec![
                        track(1, "Song 1", Some(&r1)),
                        track(2, "Song 2", Some(&r2)),
                        track(3, "Song 1", None),
                    ],
                )?;
                Ok((album, standard, deluxe))
            })
        })
        .unwrap()
}

fn log(f: &Fixture, edition: &str, tracks: Option<Vec<String>>, earlier: bool) -> String {
    f.state
        .with_db(|db| {
            db.write(|tx| {
                listening::log(
                    tx,
                    Some(&new_id()),
                    &LogListen {
                        edition_id: edition.into(),
                        listened_on: Some("2026-09-30".into()),
                        kind: ListenKind::Unspecified,
                        earlier_undated: earlier,
                        track_ids: tracks,
                        attempt_id: None,
                    },
                )
            })
        })
        .unwrap()
        .listen_id
}

fn entry(v: &ArtistCatalogue, title: &str) -> discography::Entry {
    v.entries
        .iter()
        .find(|e| e.title == title)
        .cloned()
        .unwrap()
}

#[tokio::test(start_paused = true)]
async fn completion_counts_canonical_albums_and_distinct_recordings() {
    let f = fixture(|url, _| route(url, 7));
    fetch_catalogue(&f.svc, &f.state, &new_id(), &f.artist)
        .await
        .unwrap();
    let (_, standard, deluxe) = standard_and_deluxe(&f, 5);
    // Keep the Deluxe on the Listen List (and so the reference) after full listens.
    f.state
        .with_db(|db| {
            db.write(|tx| {
                crate::library::settings::save(
                    tx,
                    &crate::library::settings::Settings {
                        full_listen_removes_from_listen_list: false,
                    },
                )?;
                personal::add_to_listen_list(tx, &deluxe.edition_id)
            })
        })
        .unwrap();

    // Track-only listening never completes the album.
    log(
        &f,
        &standard.edition_id,
        Some(vec![standard.track_ids[0].clone()]),
        false,
    );
    let v = view(&f);
    let five = entry(&v, "Album 5");
    assert_eq!(
        five.reference.edition_id.as_deref(),
        Some(deluxe.edition_id.as_str())
    );
    assert!(!five.listened);
    // Recording 51 was heard on the Standard edition and counts for the Deluxe; the bonus
    // track shares a *title* with it but has no recording ID, so it needs its own evidence.
    assert_eq!(
        five.tracks.as_ref().map(|t| (t.listened, t.total)),
        Some((1, 3))
    );
    assert_eq!(v.progress.albums_listened, 0);

    // Full listens of Standard, twice: one album, and re-listens don't inflate anything.
    let first = log(&f, &standard.edition_id, None, false);
    let second = log(&f, &standard.edition_id, None, false);
    let v = view(&f);
    assert_eq!(v.entries.iter().filter(|e| e.title == "Album 5").count(), 1);
    assert!(entry(&v, "Album 5").listened);
    assert_eq!(v.progress.albums_listened, 1);
    assert_eq!(entry(&v, "Album 5").tracks.unwrap().listened, 2);
    assert_eq!(
        (v.progress.tracks_listened, v.progress.tracks_total),
        (2, 3)
    );

    // Undo (delete) both full listens: back to track-only evidence.
    f.state
        .with_db(|db| {
            db.write(|tx| {
                listening::delete(tx, &first)?;
                listening::delete(tx, &second)
            })
        })
        .unwrap();
    let v = view(&f);
    assert!(!entry(&v, "Album 5").listened);
    assert_eq!(entry(&v, "Album 5").tracks.unwrap().listened, 1);

    // Confirmed earlier listening (undated history) counts as a full listen.
    log(
        &f,
        &standard.edition_id,
        Some(vec![standard.track_ids[1].clone()]),
        true,
    );
    assert!(entry(&view(&f), "Album 5").listened);
}

#[tokio::test(start_paused = true)]
async fn album_scores_average_rated_editions_including_zero() {
    let f = fixture(|url, _| route(url, 7));
    fetch_catalogue(&f.svc, &f.state, &new_id(), &f.artist)
        .await
        .unwrap();
    let (_, standard, deluxe) = standard_and_deluxe(&f, 5);
    let (_, other, _) = standard_and_deluxe(&f, 6);
    f.state
        .with_db(|db| {
            db.write(|tx| {
                crate::library::ratings::set_album(tx, &standard.edition_id, Some(0))?;
                crate::library::ratings::set_album(tx, &deluxe.edition_id, Some(8))?;
                crate::library::ratings::set_album(tx, &other.edition_id, Some(3))
            })
        })
        .unwrap();
    let v = view(&f);
    let five = entry(&v, "Album 5");
    let score = five.score.unwrap();
    assert!(
        (score.stars - 2.0).abs() < 1e-9,
        "(0 + 4) / 2 stars: zero is a rating"
    );
    assert_eq!(score.editions.len(), 2);
    assert_eq!(five.rank, Some(1));
    assert_eq!(entry(&v, "Album 6").rank, Some(2));
    assert_eq!(v.average.rated_albums, 2, "each canonical album once");
    assert!((v.average.stars.unwrap() - 1.75).abs() < 1e-9);
    assert!(entry(&v, "Album 7").score.is_none());
}

#[tokio::test(start_paused = true)]
async fn collaborators_link_and_various_artists_compilations_stay_unattributed() {
    let f = fixture(|url, _| route(url, 7));
    let partner = artist(&f.state, "Partner", Some(PARTNER_MB));
    fetch_catalogue(&f.svc, &f.state, &new_id(), &f.artist)
        .await
        .unwrap();
    let credits = entry(&view(&f), "Album 1").credits;
    assert_eq!(credits.len(), 2);
    assert_eq!(credits[1].artist_id.as_deref(), Some(partner.as_str()));
    assert_eq!(credits[0].join_phrase, " & ");

    // A Various Artists compilation with one track credited to our artist.
    let various = artist(&f.state, "Various Artists", None);
    f.state
        .with_db(|db| {
            db.write(|tx| {
                let album = catalogue::create_album(
                    tx,
                    None,
                    &NewAlbum {
                        title: "Hits Compilation".into(),
                        original_date: None,
                        musicbrainz_release_group_id: None,
                        credits: vec![CreditInput {
                            artist_id: various.clone(),
                            credited_name: None,
                            join_phrase: None,
                        }],
                        genres: vec![],
                    },
                )?;
                catalogue::create_edition(
                    tx,
                    None,
                    &NewEdition {
                        album_id: album,
                        name: "Standard".into(),
                        release_date: None,
                        musicbrainz_release_id: None,
                        tracks: vec![NewTrack {
                            disc_number: None,
                            position: 1,
                            title: "Our Song".into(),
                            length_ms: None,
                            recording_id: None,
                            musicbrainz_track_id: None,
                            credits: vec![CreditInput {
                                artist_id: f.artist.clone(),
                                credited_name: None,
                                join_phrase: None,
                            }],
                        }],
                    },
                )
            })
        })
        .unwrap();
    assert!(
        !view(&f)
            .entries
            .iter()
            .any(|e| e.title == "Hits Compilation")
    );
}

#[tokio::test(start_paused = true)]
async fn discovered_albums_are_added_to_the_listen_list_only_on_request() {
    let f = fixture(|url, _| route(url, 7));
    fetch_catalogue(&f.svc, &f.state, &new_id(), &f.artist)
        .await
        .unwrap();
    let six = entry(&view(&f), "Album 6");
    assert!(!six.on_listen_list && six.album_id.is_none());
    let added = add_to_listen_list(&f.svc, &f.state, &new_id(), six.id.as_deref().unwrap())
        .await
        .unwrap();
    assert!(added.imported);
    let v = view(&f);
    let six = entry(&v, "Album 6");
    assert!(six.on_listen_list);
    assert_eq!(six.album_id.as_deref(), Some(added.album_id.as_str()));
    // Imported edition is the representative (earliest official) release.
    assert!(
        f.transport
            .urls()
            .iter()
            .any(|u| u.contains(&format!("/release/{}", rel(61))))
    );
    assert!(
        entry(&v, "Album 7").album_id.is_none(),
        "nothing else was added"
    );

    // An album already in the library reuses its edition.
    let (_, standard, _) = standard_and_deluxe(&f, 5);
    let five = entry(&view(&f), "Album 5");
    let again = add_to_listen_list(&f.svc, &f.state, &new_id(), five.id.as_deref().unwrap())
        .await
        .unwrap();
    assert!(!again.imported);
    assert_eq!(again.edition_id, standard.edition_id);
}
