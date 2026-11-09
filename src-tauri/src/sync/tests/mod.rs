//! Sync between two genuinely separate profiles (own data folders and databases) through
//! a mocked GitHub.

mod journeys;
mod live;
mod mock;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use rusqlite::{OptionalExtension, Transaction};
use serde_json::{Value, json};

use self::mock::{FaultReply, MockGitHub};
use super::github::Method;
use super::merge::{ConflictKind, Resolution, Side};
use super::secrets::SecretStore;
use super::secrets::memory::MemorySecretStore;
use super::state::{FORMAT_PATH, State};
use super::{ConfirmReason, FirstSync, Limits, SyncOutcome, SyncRequest, SyncService};
use crate::error::{AppError, AppResult};
use crate::library::catalogue::{self, CreditInput, NewAlbum, NewArtist, NewEdition, NewTrack};
use crate::library::discography;
use crate::library::listening::{self, ListenKind, LogListen};
use crate::library::personal::{self, CollectionSource};
use crate::library::selection::{self, SelectionMode};
use crate::library::{metadata_import, next_up, ratings, tags};
use crate::paths::StorageProfile;
use crate::state::AppState;

const TOKEN: &str = "github_pat_11TESTTOKEN0000_abcdefghijklmnopqrstuvwxyz0123456789ABCDEF";

struct Device {
    dir: tempfile::TempDir,
    gh: Arc<MockGitHub>,
    limits: Limits,
    state: AppState,
    sync: SyncService,
}

impl Device {
    async fn new(gh: &Arc<MockGitHub>) -> Self {
        Self::with_limits(gh, Limits::default()).await
    }

    async fn with_limits(gh: &Arc<MockGitHub>, limits: Limits) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::open(StorageProfile::Development, dir.path().join("data"));
        let secrets = Arc::new(MemorySecretStore::default());
        secrets.write(TOKEN).unwrap();
        let sync = SyncService::with_limits(gh.clone(), secrets, limits);
        sync.configure(&state, "me", "MuDraft-Sync").await.unwrap();
        Self {
            dir,
            gh: gh.clone(),
            limits,
            state,
            sync,
        }
    }

    /// Close and reopen the profile (an app restart).
    fn restart(self) -> Self {
        let Self {
            dir,
            gh,
            limits,
            state,
            ..
        } = self;
        drop(state);
        let state = AppState::open(StorageProfile::Development, dir.path().join("data"));
        let secrets = Arc::new(MemorySecretStore::default());
        secrets.write(TOKEN).unwrap();
        let sync = SyncService::with_limits(gh.clone(), secrets, limits);
        Self {
            dir,
            gh,
            limits,
            state,
            sync,
        }
    }

    async fn sync(&self) -> AppResult<SyncOutcome> {
        self.sync.sync(&self.state, &SyncRequest::default()).await
    }

    async fn sync_with(
        &self,
        first_sync: Option<FirstSync>,
        resolutions: Vec<Resolution>,
    ) -> AppResult<SyncOutcome> {
        let request = SyncRequest {
            first_sync,
            resolutions,
        };
        self.sync.sync(&self.state, &request).await
    }

    async fn init(&self) -> SyncOutcome {
        self.sync.initialize(&self.state, true).await.unwrap()
    }

    fn write<T>(&self, f: impl FnOnce(&Transaction<'_>) -> AppResult<T>) -> T {
        self.state.with_db(|db| db.write(f)).unwrap()
    }

    fn int(&self, sql: &str) -> Option<i64> {
        self.state
            .with_db(|db| db.read(|c| Ok(c.query_row(sql, [], |r| r.get(0)).optional()?.flatten())))
            .unwrap()
    }

    fn text(&self, sql: &str) -> Option<String> {
        self.state
            .with_db(|db| db.read(|c| Ok(c.query_row(sql, [], |r| r.get(0)).optional()?.flatten())))
            .unwrap()
    }

    /// The synced library as the sync engine sees it.
    fn dump(&self) -> State {
        let schema = self
            .state
            .with_db(|db| db.read(super::schema::Schema::load))
            .unwrap();
        super::read_snapshot(&self.state, &schema).unwrap().state
    }

    fn sync_backups(&self) -> usize {
        std::fs::read_dir(self.state.data_dir.join("backups"))
            .map(|d| {
                d.filter(|e| {
                    e.as_ref()
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .contains("pre-sync")
                })
                .count()
            })
            .unwrap_or(0)
    }
}

struct Album {
    album: String,
    edition: String,
    tracks: Vec<String>,
}

fn credit(artist_id: String) -> Vec<CreditInput> {
    vec![CreditInput {
        artist_id,
        credited_name: None,
        join_phrase: None,
    }]
}

fn tracks(n: i64) -> Vec<NewTrack> {
    (1..=n)
        .map(|p| NewTrack {
            disc_number: None,
            position: p,
            title: format!("Track {p}"),
            length_ms: Some(200_000),
            recording_id: None,
            musicbrainz_track_id: None,
            credits: vec![],
        })
        .collect()
}

fn add_album_with(
    dev: &Device,
    title: &str,
    artist_mbid: Option<&str>,
    group_mbid: Option<&str>,
) -> Album {
    dev.write(|tx| {
        let artist = catalogue::create_artist(
            tx,
            None,
            &NewArtist {
                name: "Radiohead".into(),
                sort_name: None,
                musicbrainz_id: artist_mbid.map(Into::into),
            },
        )?;
        let album = catalogue::create_album(
            tx,
            None,
            &NewAlbum {
                title: title.into(),
                original_date: Some("1997".into()),
                musicbrainz_release_group_id: group_mbid.map(Into::into),
                credits: credit(artist),
                genres: vec!["Rock".into()],
            },
        )?;
        let created = catalogue::create_edition(
            tx,
            None,
            &NewEdition {
                album_id: album.clone(),
                name: "Standard".into(),
                release_date: None,
                musicbrainz_release_id: None,
                tracks: tracks(3),
            },
        )?;
        personal::add_to_listen_list(tx, &created.edition_id)?;
        Ok(Album {
            album,
            edition: created.edition_id,
            tracks: created.track_ids,
        })
    })
}

fn add_album(dev: &Device, title: &str) -> Album {
    add_album_with(dev, title, None, None)
}

fn listen(dev: &Device, edition: &str, date: &str) -> String {
    dev.write(|tx| {
        listening::log(
            tx,
            None,
            &LogListen {
                edition_id: edition.into(),
                listened_on: Some(date.into()),
                kind: ListenKind::Unspecified,
                earlier_undated: false,
                attempt_id: None,
            },
        )
    })
    .listen_id
}

fn rate(dev: &Device, track: &str, rating: Option<i64>) {
    dev.write(|tx| ratings::set_track(tx, track, rating));
}

fn review(dev: &Device, edition: &str, text: &str) {
    dev.write(|tx| ratings::set_review(tx, edition, Some(text)));
}

fn review_of(dev: &Device, edition: &str) -> Option<String> {
    dev.text(&format!(
        "SELECT review FROM album_review WHERE edition_id = '{edition}'"
    ))
}

fn rating_of(dev: &Device, track: &str) -> Option<i64> {
    dev.int(&format!(
        "SELECT rating FROM track_rating WHERE track_id = '{track}'"
    ))
}

fn synced(outcome: AppResult<SyncOutcome>) -> SyncOutcome {
    match outcome {
        Ok(o @ (SyncOutcome::Synced { .. } | SyncOutcome::UpToDate { .. })) => o,
        other => panic!("expected a completed sync, got {other:?}"),
    }
}

fn conflicts(outcome: AppResult<SyncOutcome>) -> Vec<super::merge::Conflict> {
    match outcome {
        Ok(SyncOutcome::Conflicts { conflicts, .. }) => conflicts,
        other => panic!("expected conflicts, got {other:?}"),
    }
}

fn choose(conflicts: &[super::merge::Conflict], choice: Side) -> Vec<Resolution> {
    conflicts
        .iter()
        .map(|c| Resolution {
            id: c.id.clone(),
            choice,
        })
        .collect()
}

fn new_repo_with_readme() -> Arc<MockGitHub> {
    let gh = Arc::new(MockGitHub::new(TOKEN));
    gh.commit_files(&[("README.md", Some(b"# My sync\n"))]);
    gh
}

/// A initialises the repository with one album; B (empty) links and downloads it.
async fn linked_pair() -> (Arc<MockGitHub>, Device, Device, Album) {
    let gh = new_repo_with_readme();
    let a = Device::new(&gh).await;
    let album = add_album(&a, "OK Computer");
    assert!(matches!(a.init().await, SyncOutcome::Synced { .. }));
    let b = Device::new(&gh).await;
    synced(b.sync().await);
    assert_eq!(a.dump(), b.dump());
    (gh, a, b, album)
}

// ---------------------------------------------------------------- tests

#[tokio::test(start_paused = true)]
async fn initial_upload_to_an_empty_repository_then_download() {
    let gh = Arc::new(MockGitHub::new(TOKEN));
    let a = Device::new(&gh).await;
    let album = add_album(&a, "OK Computer");
    rate(&a, &album.tracks[0], Some(0));
    listen(&a, &album.edition, "2026-10-01");
    a.write(|tx| {
        let tag = tags::create(tx, "Chill", None)?;
        tags::set_album_tags(tx, std::slice::from_ref(&album.album), &[tag.id], &[])
    });

    // A populated library needs confirmation; nothing is written without it.
    match a.sync.initialize(&a.state, false).await.unwrap() {
        SyncOutcome::NeedsConfirmation { reason, local, .. } => {
            assert_eq!(reason, ConfirmReason::InitializeFromPopulated);
            assert_eq!(local.albums, 1);
        }
        other => panic!("{other:?}"),
    }
    assert!(gh.head().is_none());
    // Sync (not initialise) refuses an empty repository instead of treating it as data.
    assert_eq!(a.sync().await.unwrap_err().code(), "sync_setup");

    let SyncOutcome::Synced { uploaded_files, .. } = a.init().await else {
        panic!()
    };
    assert!(uploaded_files > 5);
    let paths = gh.paths();
    assert!(paths.iter().all(|p| p.starts_with("sync/")), "{paths:?}");
    assert!(paths.contains(&FORMAT_PATH.to_owned()));
    assert!(paths.contains(&"sync/state/listen_event.json".to_owned()));

    let b = Device::new(&gh).await;
    let SyncOutcome::Synced {
        local_changes,
        backup_file_name,
        ..
    } = synced(b.sync().await)
    else {
        panic!()
    };
    assert!(local_changes > 10);
    assert!(
        backup_file_name.is_some(),
        "a backup precedes local changes"
    );
    assert_eq!(a.dump(), b.dump());
    assert_eq!(
        rating_of(&b, &album.tracks[0]),
        Some(0),
        "zero is a rating, not unrated"
    );
    assert_eq!(b.int("SELECT COUNT(*) FROM listen_event"), Some(1));
    assert_eq!(
        b.text("SELECT name FROM tag WHERE builtin_key IS NULL"),
        Some("Chill".into())
    );

    let blobs = gh.count(Method::Post, "/git/blobs");
    assert!(matches!(
        a.sync().await.unwrap(),
        SyncOutcome::UpToDate { .. }
    ));
    assert!(matches!(
        b.sync().await.unwrap(),
        SyncOutcome::UpToDate { .. }
    ));
    assert_eq!(
        gh.count(Method::Post, "/git/blobs"),
        blobs,
        "nothing re-uploaded"
    );
    let status = b.sync.status(&b.state).unwrap();
    assert!(status.linked && status.has_token);
    assert_eq!(status.pending_local_changes, Some(false));
    assert_eq!(status.revision, gh.head());
}

#[tokio::test(start_paused = true)]
async fn independent_offline_changes_merge_without_duplicates() {
    let (gh, a, b, album) = linked_pair().await;
    rate(&a, &album.tracks[0], Some(8));
    listen(&a, &album.edition, "2026-10-01");
    a.write(|tx| {
        let tag = tags::create(tx, "Night", None)?;
        tags::set_album_tags(tx, std::slice::from_ref(&album.album), &[tag.id], &[])
    });
    let other = add_album(&b, "Kid A");
    rate(&b, &album.tracks[1], Some(3));
    review(&b, &album.edition, "Great");
    listen(&b, &album.edition, "2026-10-02");
    listen(&b, &other.edition, "2026-10-03");

    synced(a.sync().await);
    synced(b.sync().await);
    synced(a.sync().await);
    assert_eq!(a.dump(), b.dump());
    assert_eq!(a.int("SELECT COUNT(*) FROM listen_event"), Some(3));
    assert_eq!(a.int("SELECT COUNT(*) FROM album"), Some(2));
    assert_eq!(
        a.int("SELECT COUNT(*) FROM genre"),
        Some(1),
        "both computers' Rock genre unified"
    );
    assert_eq!(rating_of(&b, &album.tracks[0]), Some(8));
    assert_eq!(rating_of(&a, &album.tracks[1]), Some(3));
    assert_eq!(review_of(&a, &album.edition).as_deref(), Some("Great"));

    let blobs = gh.count(Method::Post, "/git/blobs");
    assert!(matches!(
        a.sync().await.unwrap(),
        SyncOutcome::UpToDate { .. }
    ));
    assert!(matches!(
        b.sync().await.unwrap(),
        SyncOutcome::UpToDate { .. }
    ));
    assert_eq!(gh.count(Method::Post, "/git/blobs"), blobs);
}

#[tokio::test(start_paused = true)]
async fn same_field_edits_conflict_instead_of_overwriting() {
    let (gh, a, b, album) = linked_pair().await;
    review(&a, &album.edition, "Loved it");
    review(&b, &album.edition, "Meh");
    synced(a.sync().await);
    let head = gh.head();
    let found = conflicts(b.sync().await);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].kind, ConflictKind::Field);
    assert_eq!(found[0].table, "album_review");
    assert_eq!(found[0].columns, vec!["review".to_owned()]);
    assert_eq!(found[0].local, json!({ "review": "Meh" }));
    assert_eq!(found[0].remote, json!({ "review": "Loved it" }));
    assert_eq!(
        review_of(&b, &album.edition).as_deref(),
        Some("Meh"),
        "nothing changed"
    );
    assert_eq!(gh.head(), head, "nothing published");

    synced(b.sync_with(None, choose(&found, Side::Remote)).await);
    assert_eq!(review_of(&b, &album.edition).as_deref(), Some("Loved it"));

    rate(&a, &album.tracks[0], Some(4));
    rate(&b, &album.tracks[0], Some(6));
    synced(a.sync().await);
    let found = conflicts(b.sync().await);
    assert_eq!(found[0].columns, vec!["rating".to_owned()]);
    synced(b.sync_with(None, choose(&found, Side::Local)).await);
    synced(a.sync().await);
    assert_eq!(rating_of(&a, &album.tracks[0]), Some(6));
    assert_eq!(a.dump(), b.dump());

    // A stale resolution (values changed since) doesn't apply silently.
    review(&a, &album.edition, "First");
    review(&b, &album.edition, "Second");
    synced(a.sync().await);
    let old = conflicts(b.sync().await);
    review(&b, &album.edition, "Third");
    let again = conflicts(b.sync_with(None, choose(&old, Side::Remote)).await);
    assert_ne!(again[0].id, old[0].id);
    assert_eq!(review_of(&b, &album.edition).as_deref(), Some("Third"));
}

#[tokio::test(start_paused = true)]
async fn deletions_propagate_and_delete_versus_edit_conflicts() {
    let (_gh, a, b, album) = linked_pair().await;
    let old = a.write(|tx| {
        let tag = tags::create(tx, "Old", None)?;
        tags::set_album_tags(
            tx,
            std::slice::from_ref(&album.album),
            std::slice::from_ref(&tag.id),
            &[],
        )?;
        Ok(tag.id)
    });
    synced(a.sync().await);
    synced(b.sync().await);
    assert_eq!(b.int("SELECT COUNT(*) FROM album_tag"), Some(1));

    a.write(|tx| {
        tags::delete(tx, &old)?;
        personal::remove_from_listen_list(tx, &album.album)
    });
    synced(a.sync().await);
    synced(b.sync().await);
    assert_eq!(
        b.int(&format!("SELECT COUNT(*) FROM tag WHERE id = '{old}'")),
        Some(0)
    );
    assert_eq!(b.int("SELECT COUNT(*) FROM album_tag"), Some(0));
    assert_eq!(b.int("SELECT COUNT(*) FROM listen_list_entry"), Some(0));

    let x = a.write(|tx| Ok(tags::create(tx, "X", None)?.id));
    synced(a.sync().await);
    synced(b.sync().await);
    a.write(|tx| tags::delete(tx, &x));
    b.write(|tx| tags::update(tx, &x, Some("Y"), None));
    synced(a.sync().await);
    let found = conflicts(b.sync().await);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].kind, ConflictKind::DeleteEdit);
    assert_eq!(found[0].remote, Value::Null);
    synced(b.sync_with(None, choose(&found, Side::Local)).await);
    synced(a.sync().await);
    assert_eq!(
        a.text(&format!("SELECT name FROM tag WHERE id = '{x}'")),
        Some("Y".into())
    );
    assert_eq!(a.dump(), b.dump());
}

#[tokio::test(start_paused = true)]
async fn genre_overrides_and_resets_sync_between_computers() {
    const RG: &str = "b1392450-e666-3926-a536-22c65f834433";
    let gh = new_repo_with_readme();
    let a = Device::new(&gh).await;
    let linked = add_album_with(&a, "OK Computer", None, Some(RG));
    let manual = add_album(&a, "Demo");
    // As a MusicBrainz import leaves it: raw provider genres and the genres provenance.
    a.write(|tx| {
        tx.execute(
            "INSERT INTO album_source_tag (album_id, source, kind, name, votes, fetched_at)
             VALUES (?1, 'musicbrainz', 'genre', 'alternative rock', 20, '2026-10-01T00:00:00.000Z'),
                    (?1, 'musicbrainz', 'genre', 'electronic', 8, '2026-10-01T00:00:00.000Z')",
            [&linked.album],
        )?;
        tx.execute(
            "INSERT INTO metadata_provenance (entity_type, entity_id, field, source, source_ref, is_override)
             VALUES ('album', ?1, 'genres', 'musicbrainz', ?2, 0)",
            [&linked.album, RG],
        )?;
        Ok(())
    });
    assert!(matches!(a.init().await, SyncOutcome::Synced { .. }));
    let b = Device::new(&gh).await;
    synced(b.sync().await);

    let genres = |dev: &Device, album: &str| {
        dev.state
            .with_db(|db| db.read(|c| metadata_import::current_genres(c, album)))
            .unwrap()
    };
    let locked = |dev: &Device, album: &str| {
        dev.int(&format!(
            "SELECT is_override FROM metadata_provenance WHERE entity_id = '{album}' AND field = 'genres'"
        ))
    };

    // An override on A arrives on B, still locked against refresh.
    a.write(|tx| {
        metadata_import::set_album_genres_manually(tx, &linked.album, &["Jazz".into()])?;
        metadata_import::set_album_genres_manually(tx, &manual.album, &["Folk".into()])
    });
    synced(a.sync().await);
    synced(b.sync().await);
    assert_eq!(genres(&b, &linked.album), vec!["Jazz"]);
    assert_eq!(locked(&b, &linked.album), Some(1));
    assert_eq!(genres(&b, &manual.album), vec!["Folk"]);

    // A reset on B arrives on A: defaults back, unlocked; the manual album's lock is gone.
    b.write(|tx| {
        metadata_import::reset_album_genres(tx, &linked.album)?;
        metadata_import::reset_album_genres(tx, &manual.album)
    });
    synced(b.sync().await);
    synced(a.sync().await);
    assert_eq!(genres(&a, &linked.album), vec!["Rock", "Electronic"]);
    assert_eq!(locked(&a, &linked.album), Some(0));
    assert!(genres(&a, &manual.album).is_empty());
    assert_eq!(locked(&a, &manual.album), None);
    assert_eq!(a.dump(), b.dump());

    // Editing on one computer while resetting on the other is one conflict, and the
    // chosen side's genres keep that side's lock (never edited genres left unlocked).
    let race = async |locked_like_default: bool, choice: Side| {
        let start = if locked_like_default {
            vec!["Rock".into(), "Electronic".into()]
        } else {
            vec!["Pop".into()]
        };
        a.write(|tx| metadata_import::set_album_genres_manually(tx, &linked.album, &start));
        synced(a.sync().await);
        synced(b.sync().await);
        a.write(|tx| {
            metadata_import::set_album_genres_manually(tx, &linked.album, &["Metal".into()])
        });
        b.write(|tx| metadata_import::reset_album_genres(tx, &linked.album));
        synced(a.sync().await);
        let found = conflicts(b.sync().await);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].kind, ConflictKind::Collection);
        assert_eq!(found[0].table, "album_genre");
        assert_eq!(found[0].record, "Genres of OK Computer");
        assert_eq!(
            found[0].local,
            json!({ "genres": ["Rock", "Electronic"], "edited": false })
        );
        assert_eq!(
            found[0].remote,
            json!({ "genres": ["Metal"], "edited": true })
        );
        synced(b.sync_with(None, choose(&found, choice)).await);
        synced(a.sync().await);
        assert_eq!(a.dump(), b.dump());
    };
    race(false, Side::Remote).await;
    assert_eq!(genres(&a, &linked.album), vec!["Metal"]);
    assert_eq!(locked(&a, &linked.album), Some(1));
    race(false, Side::Local).await;
    assert_eq!(genres(&a, &linked.album), vec!["Rock", "Electronic"]);
    assert_eq!(locked(&a, &linked.album), Some(0));
    // The reset leaves the list as it was and only unlocks it: still a conflict.
    race(true, Side::Remote).await;
    assert_eq!(genres(&a, &linked.album), vec!["Metal"]);
    assert_eq!(locked(&a, &linked.album), Some(1));
}

#[tokio::test(start_paused = true)]
async fn next_up_slots_merge_with_one_slot_per_album_and_one_session_per_slot() {
    let (_gh, a, b, _) = linked_pair().await;
    let kid_a = add_album(&a, "Kid A");
    synced(a.sync().await);
    synced(b.sync().await);

    // Both computers pick the same album by hand, into different slots.
    a.write(|tx| next_up::choose(tx, None, &kid_a.edition, Some(2)));
    b.write(|tx| next_up::choose(tx, None, &kid_a.edition, Some(4)));
    // Both open a picking method in slot 5, and one each in slots 1 and 3.
    a.write(|tx| selection::start_session(tx, 5, SelectionMode::CompletelyRandom, None));
    b.write(|tx| selection::start_session(tx, 5, SelectionMode::WeightedRandom, None));
    a.write(|tx| selection::start_session(tx, 1, SelectionMode::CompletelyRandom, None));
    b.write(|tx| selection::start_session(tx, 3, SelectionMode::WeightedRandom, None));
    synced(a.sync().await);
    synced(b.sync().await);
    synced(a.sync().await);
    assert_eq!(a.dump(), b.dump());

    // The album stays in the lowest slot only.
    assert_eq!(
        b.text("SELECT group_concat(slot) FROM current_selection"),
        Some("2".into())
    );
    // Slots keep their own sessions; slot 5's duplicate is closed.
    assert_eq!(
        b.text(
            "SELECT group_concat(slot) FROM (SELECT slot FROM selection_session
             WHERE ended_at IS NULL AND mode != 'manual' ORDER BY slot)"
        ),
        Some("1,3,5".into())
    );
}

#[tokio::test(start_paused = true)]
async fn deleting_listen_asap_on_one_computer_deletes_it_on_the_other() {
    let (_gh, a, b, album) = linked_pair().await;
    let asap = crate::library::catalogue::LISTEN_ASAP_TAG_ID;
    a.write(|tx| catalogue::tag_album(tx, &album.album, asap));
    synced(a.sync().await);
    synced(b.sync().await);
    assert_eq!(b.int("SELECT COUNT(*) FROM album_tag"), Some(1));

    a.write(|tx| tags::delete(tx, asap));
    synced(a.sync().await);
    synced(b.sync().await);
    for dev in [&a, &b] {
        assert_eq!(
            dev.int("SELECT COUNT(*) FROM tag WHERE builtin_key IS NOT NULL"),
            Some(0)
        );
        assert_eq!(dev.int("SELECT COUNT(*) FROM album_tag"), Some(0));
    }
    assert_eq!(a.dump(), b.dump());
}

const CAT_ARTIST: &str = "a74b1b7f-71a5-4011-9441-d0b5e4122711";

fn cat_group(n: u32) -> String {
    format!("c0000000-0000-4000-8000-00000000000{n}")
}

/// One complete page of the artist's catalogue, as fetched at `at`.
fn record_catalogue(dev: &Device, artist: &str, titles: &[&str], at: &str) {
    use crate::metadata::{CatalogueReleaseGroup, CreditPart, ProviderDate, ReleaseGroupPage};
    let page = ReleaseGroupPage {
        release_groups: titles
            .iter()
            .enumerate()
            .map(|(i, t)| CatalogueReleaseGroup {
                id: cat_group(i as u32 + 1),
                title: (*t).into(),
                disambiguation: None,
                primary_type: Some("Album".into()),
                secondary_types: vec![],
                original_date: ProviderDate::parse(Some("2000")),
                artist_credit: vec![CreditPart {
                    artist_id: CAT_ARTIST.into(),
                    artist_name: "Radiohead".into(),
                    sort_name: None,
                    disambiguation: None,
                    credited_name: None,
                    join_phrase: String::new(),
                }],
            })
            .collect(),
        total: titles.len() as u32,
        offset: 0,
        received: titles.len() as u32,
    };
    dev.write(|tx| {
        discography::begin_pass(tx, artist, CAT_ARTIST)?;
        discography::record_page(tx, artist, &page, at)
    });
}

fn catalogue_of(dev: &Device, artist: &str) -> discography::ArtistCatalogue {
    dev.state
        .with_db(|db| db.read(|c| discography::catalogue(c, artist)))
        .unwrap()
}

#[tokio::test(start_paused = true)]
async fn a_discography_fetched_on_one_computer_shows_progress_on_the_other() {
    let (_gh, a, b, _) = linked_pair().await;
    let kid_a = add_album_with(&a, "Kid A", Some(CAT_ARTIST), Some(&cat_group(1)));
    let artist = a
        .text(&format!(
            "SELECT id FROM artist WHERE musicbrainz_id = '{CAT_ARTIST}'"
        ))
        .unwrap();
    record_catalogue(
        &a,
        &artist,
        &["Kid A", "Amnesiac", "Hail to the Thief"],
        "2026-11-09T10:00:00Z",
    );
    synced(a.sync().await);
    synced(b.sync().await);

    // B never fetched, yet shows the same catalogue and progress.
    let on_b = catalogue_of(&b, &artist);
    assert_eq!(on_b.entries.len(), 3);
    assert_eq!(on_b.progress.albums_counted, 3);
    assert_eq!(on_b.progress.albums_listened, 0);

    // A listen logged on B updates progress on A after a sync.
    listen(&b, &kid_a.edition, "2026-11-08");
    synced(b.sync().await);
    synced(a.sync().await);
    assert_eq!(catalogue_of(&a, &artist).progress.albums_listened, 1);

    // The user's corrections travel too: an excluded album and a wider scope.
    let amnesiac = on_b.entries.iter().find(|e| e.title == "Amnesiac").unwrap();
    b.write(|tx| discography::set_excluded(tx, amnesiac.id.as_deref().unwrap(), true));
    b.write(|tx| {
        discography::set_scope(
            tx,
            &artist,
            &discography::Scope {
                primary_types: vec!["Album".into(), "EP".into()],
                secondary_types: vec![],
            },
        )
    });
    synced(b.sync().await);
    synced(a.sync().await);
    let on_a = catalogue_of(&a, &artist);
    assert!(
        on_a.entries
            .iter()
            .any(|e| e.title == "Amnesiac" && e.excluded)
    );
    assert_eq!(on_a.scope.primary_types, ["Album", "EP"]);
    assert_eq!(on_a.progress.albums_counted, 2);
    assert_eq!(a.dump(), b.dump());
}

#[tokio::test(start_paused = true)]
async fn both_computers_fetching_one_artist_merge_into_one_catalogue_without_conflicts() {
    let (_gh, a, b, _) = linked_pair().await;
    add_album_with(&a, "Kid A", Some(CAT_ARTIST), Some(&cat_group(1)));
    synced(a.sync().await);
    synced(b.sync().await);
    let artist = a
        .text(&format!(
            "SELECT id FROM artist WHERE musicbrainz_id = '{CAT_ARTIST}'"
        ))
        .unwrap();
    // Separate fetches: different entry IDs, times, and (later) a corrected title.
    record_catalogue(&a, &artist, &["Kid A", "Amnesiac"], "2026-11-09T10:00:00Z");
    record_catalogue(
        &b,
        &artist,
        &["Kid A", "Amnesiac (Remastered)"],
        "2026-11-09T12:00:00Z",
    );
    synced(a.sync().await);
    synced(b.sync().await);
    synced(a.sync().await);
    assert_eq!(a.dump(), b.dump());
    for dev in [&a, &b] {
        assert_eq!(dev.int("SELECT COUNT(*) FROM catalogue_entry"), Some(2));
        assert_eq!(dev.int("SELECT COUNT(*) FROM artist_catalogue"), Some(1));
        let titles: Vec<String> = catalogue_of(dev, &artist)
            .entries
            .iter()
            .map(|e| e.title.clone())
            .collect();
        assert!(
            titles.contains(&"Amnesiac (Remastered)".to_owned()),
            "{titles:?}: newer fetch wins"
        );
    }

    // A refetch on each side again (bookkeeping differs) still merges silently.
    record_catalogue(&a, &artist, &["Kid A", "Amnesiac"], "2026-11-10T09:00:00Z");
    record_catalogue(
        &b,
        &artist,
        &["Kid A", "Amnesiac (Remastered)"],
        "2026-11-10T08:00:00Z",
    );
    synced(a.sync().await);
    synced(b.sync().await);
    synced(a.sync().await);
    assert_eq!(a.dump(), b.dump());
    assert_eq!(
        a.text(&format!(
            "SELECT title FROM catalogue_entry WHERE musicbrainz_release_group_id = '{}'",
            cat_group(2)
        )),
        Some("Amnesiac".into())
    );
}

#[tokio::test(start_paused = true)]
async fn listen_list_progress_syncs_and_clears_everywhere_once_logged() {
    let (_gh, a, b, album) = linked_pair().await;
    let marked = |dev: &Device| dev.int("SELECT COUNT(*) FROM listen_list_track");
    a.write(|tx| personal::mark_listen_list_track(tx, &album.tracks[0], true));
    b.write(|tx| personal::mark_listen_list_track(tx, &album.tracks[2], true));
    synced(a.sync().await);
    synced(b.sync().await);
    synced(a.sync().await);
    assert_eq!((marked(&a), marked(&b)), (Some(2), Some(2)));

    listen(&b, &album.edition, "2026-10-05");
    synced(b.sync().await);
    synced(a.sync().await);
    assert_eq!((marked(&a), marked(&b)), (Some(0), Some(0)));
    assert_eq!(a.int("SELECT COUNT(*) FROM listen_list_entry"), Some(0));
    assert_eq!(a.dump(), b.dump());
}

#[tokio::test(start_paused = true)]
async fn replaying_the_same_listen_never_duplicates_it() {
    let (gh, a, b, album) = linked_pair().await;
    listen(&a, &album.edition, "2026-10-05");
    // GitHub accepts the commit but the local acknowledgement fails.
    a.sync.faults.fail_apply.store(true, Ordering::SeqCst);
    let before = a.text("SELECT base_revision FROM sync_state");
    assert_eq!(a.sync().await.unwrap_err().code(), "storage_unavailable");
    assert_ne!(gh.head(), before, "GitHub has the new state");
    assert_eq!(
        a.text("SELECT base_revision FROM sync_state"),
        before,
        "rolled back locally"
    );
    assert_eq!(
        a.int("SELECT COUNT(*) FROM listen_event"),
        Some(1),
        "local data intact"
    );

    let patches = gh.count(Method::Patch, "/git/refs");
    let SyncOutcome::Synced { uploaded_files, .. } = synced(a.sync().await) else {
        panic!()
    };
    assert_eq!(uploaded_files, 0, "recovered without publishing again");
    assert_eq!(gh.count(Method::Patch, "/git/refs"), patches);

    synced(b.sync().await);
    assert!(matches!(
        b.sync().await.unwrap(),
        SyncOutcome::UpToDate { .. }
    ));
    assert_eq!(b.int("SELECT COUNT(*) FROM listen_event"), Some(1));

    // Relinking B from scratch replays every event it already has.
    b.sync.disconnect(&b.state).await.unwrap();
    b.sync
        .configure(&b.state, "me", "MuDraft-Sync")
        .await
        .unwrap();
    assert!(matches!(
        b.sync().await.unwrap(),
        SyncOutcome::NeedsConfirmation {
            reason: ConfirmReason::MergeIntoPopulated,
            ..
        }
    ));
    synced(b.sync_with(Some(FirstSync::Merge), vec![]).await);
    assert_eq!(b.int("SELECT COUNT(*) FROM listen_event"), Some(1));
    assert_eq!(a.dump(), b.dump());
}

#[tokio::test(start_paused = true)]
async fn zero_and_cleared_ratings_stay_distinct() {
    let (_gh, a, b, album) = linked_pair().await;
    rate(&b, &album.tracks[1], Some(6));
    rate(&b, &album.tracks[2], Some(4));
    synced(b.sync().await);
    synced(a.sync().await);

    rate(&a, &album.tracks[0], Some(0));
    rate(&a, &album.tracks[1], None);
    synced(a.sync().await);
    synced(b.sync().await);
    assert_eq!(rating_of(&b, &album.tracks[0]), Some(0));
    assert_eq!(rating_of(&b, &album.tracks[1]), None);

    // 0 on one computer and "unrated" on the other is a real disagreement.
    rate(&a, &album.tracks[2], Some(0));
    rate(&b, &album.tracks[2], None);
    synced(a.sync().await);
    let found = conflicts(b.sync().await);
    assert_eq!(found[0].local, json!({ "rating": null }));
    assert_eq!(found[0].remote, json!({ "rating": 0 }));
}

#[tokio::test(start_paused = true)]
async fn a_deliberate_deluxe_edition_syncs_beside_the_standard() {
    let (_gh, a, b, album) = linked_pair().await;
    let deluxe = a.write(|tx| {
        catalogue::create_edition(
            tx,
            None,
            &NewEdition {
                album_id: album.album.clone(),
                name: "Deluxe".into(),
                release_date: Some("2017".into()),
                musicbrainz_release_id: None,
                tracks: tracks(5),
            },
        )
    });
    listen(&a, &deluxe.edition_id, "2026-10-06");
    a.write(|tx| personal::add_to_collection(tx, &deluxe.edition_id, CollectionSource::Manual));
    review(&b, &album.edition, "Standard is enough");
    synced(a.sync().await);
    synced(b.sync().await);
    synced(a.sync().await);
    assert_eq!(
        b.int(&format!(
            "SELECT COUNT(*) FROM edition WHERE album_id = '{}'",
            album.album
        )),
        Some(2)
    );
    assert_eq!(
        b.text("SELECT edition_id FROM listen_event"),
        Some(deluxe.edition_id.clone()),
        "the listen stays on the Deluxe edition"
    );
    assert_eq!(b.int("SELECT COUNT(*) FROM track"), Some(8));
    assert_eq!(
        review_of(&a, &album.edition).as_deref(),
        Some("Standard is enough")
    );
    assert_eq!(a.dump(), b.dump());
}

#[tokio::test(start_paused = true)]
async fn the_same_release_added_on_both_computers_is_unified() {
    let gh = new_repo_with_readme();
    let a = Device::new(&gh).await;
    a.init().await; // an empty library needs no confirmation
    let b = Device::new(&gh).await;
    synced(b.sync().await);
    let artist = "a74b1b7f-71a5-4011-9441-d0b5e4122711";
    let group = "b1392450-e666-3926-a536-22c65f834433";
    let on_a = add_album_with(&a, "OK Computer", Some(artist), Some(group));
    let on_b = add_album_with(&b, "OK Computer", Some(artist), Some(group));
    assert_ne!(on_a.album, on_b.album);
    rate(&b, &on_b.tracks[2], Some(9));
    synced(a.sync().await);
    synced(b.sync().await);
    synced(a.sync().await);
    for dev in [&a, &b] {
        assert_eq!(dev.int("SELECT COUNT(*) FROM album"), Some(1));
        assert_eq!(dev.int("SELECT COUNT(*) FROM artist"), Some(1));
        assert_eq!(dev.int("SELECT COUNT(*) FROM edition"), Some(1));
        assert_eq!(dev.int("SELECT COUNT(*) FROM track"), Some(3));
        assert_eq!(dev.int("SELECT MAX(rating) FROM track_rating"), Some(9));
    }
    assert_eq!(a.dump(), b.dump());
}

#[tokio::test(start_paused = true)]
async fn a_remote_head_race_is_refetched_and_merged_again() {
    let (gh, a, b, album) = linked_pair().await;
    review(&a, &album.edition, "From A");
    rate(&b, &album.tracks[1], Some(2));
    let start = gh.head().unwrap();
    synced(b.sync().await);
    let b_commit = gh.head().unwrap();
    // A reads the old head, then B's commit lands just before A moves the branch.
    gh.repo().refs.insert("main".into(), start);
    gh.repo().race = Some(b_commit.clone());
    let patches = gh.count(Method::Patch, "/git/refs");
    synced(a.sync().await);
    assert_eq!(
        gh.count(Method::Patch, "/git/refs"),
        patches + 2,
        "rejected once, then accepted"
    );
    synced(b.sync().await);
    assert_eq!(rating_of(&a, &album.tracks[1]), Some(2));
    assert_eq!(review_of(&b, &album.edition).as_deref(), Some("From A"));
    assert_eq!(a.dump(), b.dump());
}

#[tokio::test(start_paused = true)]
async fn interrupted_transfers_change_nothing_and_recover() {
    let (gh, a, b, album) = linked_pair().await;

    // Upload interrupted: local changes stay pending, the branch doesn't move.
    review(&a, &album.edition, "x");
    let head = gh.head();
    gh.fault(Method::Post, "/git/trees", 10, FaultReply::Transport);
    assert_eq!(a.sync().await.unwrap_err().code(), "sync_offline");
    assert_eq!(gh.head(), head);
    let status = a.sync.status(&a.state).unwrap();
    assert_eq!(status.pending_local_changes, Some(true));
    assert_eq!(status.last_error.unwrap().code, "sync_offline");
    gh.clear_faults();
    synced(a.sync().await);
    assert!(a.sync.status(&a.state).unwrap().last_error.is_none());

    // Download interrupted: nothing changes here, no backup is taken.
    let before = b.dump();
    let backups = b.sync_backups();
    gh.fault(Method::Get, "/git/blobs/", 10, FaultReply::Transport);
    assert_eq!(b.sync().await.unwrap_err().code(), "sync_offline");
    assert_eq!(b.dump(), before);
    assert_eq!(b.sync_backups(), backups);
    gh.clear_faults();
    synced(b.sync().await);
    assert_eq!(review_of(&b, &album.edition).as_deref(), Some("x"));

    // GitHub moved the branch but the reply was lost: recovered in the same sync.
    review(&a, &album.edition, "y");
    gh.fault(Method::Patch, "/git/refs", 1, FaultReply::LostReply);
    let patches = gh.count(Method::Patch, "/git/refs");
    synced(a.sync().await);
    assert_eq!(gh.count(Method::Patch, "/git/refs"), patches + 1);
    assert_eq!(a.sync.status(&a.state).unwrap().revision, gh.head());

    // Rate limits and server errors are retried with backoff.
    rate(&b, &album.tracks[0], Some(1));
    gh.fault(Method::Get, "/git/ref/heads", 1, FaultReply::RetryAfter(5));
    gh.fault(Method::Get, "/git/trees", 2, FaultReply::Status(502));
    synced(b.sync().await);
    // A wait longer than a minute is reported instead.
    gh.fault(Method::Get, "", 1, FaultReply::RetryAfter(3600));
    assert_eq!(a.sync().await.unwrap_err().code(), "rate_limited");
    synced(a.sync().await);
    assert_eq!(a.dump(), b.dump());
}

#[tokio::test(start_paused = true)]
async fn damaged_newer_unrelated_or_public_repositories_are_refused() {
    let (gh, _a, b, _album) = linked_pair().await;
    let before = b.dump();
    let original = gh.file(FORMAT_PATH).unwrap();
    let mut format: Value = serde_json::from_slice(&original).unwrap();

    format["formatVersion"] = json!(99);
    gh.commit_files(&[(FORMAT_PATH, Some(&serde_json::to_vec(&format).unwrap()))]);
    let err = b.sync().await.unwrap_err();
    assert_eq!(err.code(), "sync_setup");
    assert!(err.to_string().contains("newer"));

    format["formatVersion"] = json!(1);
    format["schemaVersion"] = json!(999);
    gh.commit_files(&[(FORMAT_PATH, Some(&serde_json::to_vec(&format).unwrap()))]);
    assert!(b.sync().await.unwrap_err().to_string().contains("newer"));

    gh.commit_files(&[(FORMAT_PATH, Some(&original))]);
    let mut album_file = gh.file("sync/state/album.json").unwrap();
    album_file.push(b' ');
    gh.commit_files(&[("sync/state/album.json", Some(&album_file))]);
    let err = b.sync().await.unwrap_err();
    assert_eq!(err.code(), "sync_setup");
    assert!(err.to_string().contains("damaged"), "{err}");
    assert_eq!(b.dump(), before, "nothing changed");

    // Data written at an older schema (before Listen List progress and Next Up slots
    // existed) is upgraded with the normal migrations.
    let good = gh.file("sync/state/album.json").unwrap();
    let progress = "sync/state/listen_list_track.json";
    let progress_file = gh.file(progress).unwrap();
    // Columns added after schema 11 are absent from files written then.
    let strip = |bytes: &[u8], columns: &[&str]| -> Vec<u8> {
        let mut data: Value = serde_json::from_slice(bytes).unwrap();
        for column in columns {
            let i = data["columns"]
                .as_array()
                .unwrap()
                .iter()
                .position(|c| c == column)
                .unwrap();
            data["columns"].as_array_mut().unwrap().remove(i);
            for row in data["rows"].as_array_mut().unwrap() {
                row.as_array_mut().unwrap().remove(i);
            }
        }
        serde_json::to_vec(&data).unwrap()
    };
    let sessions = "sync/state/selection_session.json";
    let sessions_file = gh.file(sessions).unwrap();
    let old_sessions = strip(&sessions_file, &["slot"]);
    let albums = "sync/state/album.json";
    let old_albums = strip(
        &good[..good.len() - 1],
        &["primary_type", "secondary_types"],
    );
    gh.commit_files(&[
        (albums, Some(&old_albums)),
        (progress, None),
        (sessions, Some(&old_sessions)),
    ]);
    let current = format.clone();
    format["schemaVersion"] = json!(11);
    format["files"]
        .as_array_mut()
        .unwrap()
        .retain(|f| f["path"] != progress);
    for f in format["files"].as_array_mut().unwrap() {
        for (path, bytes) in [(sessions, &old_sessions), (albums, &old_albums)] {
            if f["path"] == path {
                f["bytes"] = json!(bytes.len());
                f["sha256"] = json!(crate::profile::sha256_hex(bytes));
            }
        }
    }
    gh.commit_files(&[(FORMAT_PATH, Some(&serde_json::to_vec(&format).unwrap()))]);
    synced(b.sync().await);
    assert_eq!(b.dump(), before);

    // A table that didn't exist at the claimed schema is damage, reported in words.
    gh.commit_files(&[
        (progress, Some(&progress_file)),
        (sessions, Some(&sessions_file)),
    ]);
    let mut stale = current.clone();
    stale["schemaVersion"] = json!(11);
    for f in stale["files"].as_array_mut().unwrap() {
        if f["path"] == albums {
            f["bytes"] = json!(old_albums.len());
            f["sha256"] = json!(crate::profile::sha256_hex(&old_albums));
        }
    }
    gh.commit_files(&[(FORMAT_PATH, Some(&serde_json::to_vec(&stale).unwrap()))]);
    let err = b.sync().await.unwrap_err();
    assert!(
        err.to_string()
            .contains("doesn't exist at this schema version"),
        "{err}"
    );
    gh.commit_files(&[
        (FORMAT_PATH, Some(&serde_json::to_vec(&current).unwrap())),
        (albums, Some(&good[..good.len() - 1])),
    ]);

    // An unrelated repository (say, the app's source) is never written to.
    let unrelated = Arc::new(MockGitHub::new(TOKEN));
    unrelated.commit_files(&[
        ("README.md", Some(b"MuDraft")),
        ("src/main.rs", Some(b"fn main() {}")),
    ]);
    let c = Device::new(&unrelated).await;
    add_album(&c, "Mine");
    assert_eq!(
        c.sync.initialize(&c.state, true).await.unwrap_err().code(),
        "sync_setup"
    );
    assert_eq!(c.sync().await.unwrap_err().code(), "sync_setup");
    assert_eq!(unrelated.count(Method::Post, ""), 0);
    assert_eq!(unrelated.count(Method::Patch, ""), 0);

    // A public repository is refused.
    let public = new_repo_with_readme();
    public.repo().private = false;
    let d = Device::new(&public).await;
    let err = d.sync.initialize(&d.state, true).await.unwrap_err();
    assert!(err.to_string().contains("public"));
    assert_eq!(public.count(Method::Post, ""), 0);

    // An uninitialised repository is a setup error for Sync, never an empty library.
    let fresh = new_repo_with_readme();
    let e = Device::new(&fresh).await;
    add_album(&e, "Mine");
    assert_eq!(e.sync().await.unwrap_err().code(), "sync_setup");
    assert_eq!(e.int("SELECT COUNT(*) FROM album"), Some(1));
}

#[tokio::test(start_paused = true)]
async fn oversized_payloads_are_refused_before_transfer() {
    let small = Limits {
        max_file_bytes: 4096,
        ..Limits::default()
    };
    let gh = Arc::new(MockGitHub::new(TOKEN));
    let a = Device::with_limits(&gh, small).await;
    for n in 0..20 {
        add_album(&a, &format!("Album {n}"));
    }
    let err = a.sync.initialize(&a.state, true).await.unwrap_err();
    assert_eq!(err.code(), "validation");
    assert!(err.to_string().contains("limit"), "{err}");
    assert!(gh.head().is_none(), "nothing written");
    assert_eq!(gh.count(Method::Put, ""), 0);

    // A computer with tighter limits refuses to download the big state.
    let big = Device::new(&gh).await;
    for n in 0..20 {
        add_album(&big, &format!("Album {n}"));
    }
    big.init().await;
    let b = Device::with_limits(&gh, small).await;
    let blob_reads = gh.count(Method::Get, "/git/blobs/");
    let err = b.sync().await.unwrap_err();
    assert_eq!(err.code(), "sync_setup");
    assert!(err.to_string().contains("larger"), "{err}");
    assert_eq!(
        gh.count(Method::Get, "/git/blobs/"),
        blob_reads + 1,
        "only format.json was read"
    );
    assert_eq!(b.int("SELECT COUNT(*) FROM album"), Some(0));
}

fn files_containing(dir: &Path, needle: &[u8]) -> Vec<String> {
    let mut hits = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            hits.extend(files_containing(&path, needle));
        } else if std::fs::read(&path)
            .unwrap()
            .windows(needle.len())
            .any(|w| w == needle)
        {
            hits.push(path.display().to_string());
        }
    }
    hits
}

#[tokio::test(start_paused = true)]
async fn expired_or_missing_tokens_are_reported_and_never_stored() {
    let (gh, a, _b, album) = linked_pair().await;
    review(&a, &album.edition, "kept");
    gh.repo().token = "github_pat_rotated".into();
    let err = a.sync().await.unwrap_err();
    assert_eq!(err.code(), "auth_required");
    assert!(!err.to_string().contains(TOKEN));
    assert_eq!(review_of(&a, &album.edition).as_deref(), Some("kept"));
    let status = a.sync.status(&a.state).unwrap();
    assert_eq!(status.last_error.unwrap().code, "auth_required");
    assert_eq!(status.pending_local_changes, Some(true));

    a.sync.remove_token().unwrap();
    assert!(!a.sync.status(&a.state).unwrap().has_token);
    assert_eq!(a.sync().await.unwrap_err().code(), "auth_required");
    assert!(matches!(
        a.sync
            .set_token("ghp_classic_token_00000000000000000000000"),
        Err(AppError::Validation { .. })
    ));

    // The token only ever travelled as a request credential.
    assert!(
        gh.repo()
            .tokens_seen
            .iter()
            .all(|t| t == TOKEN || t.is_empty() || t == "github_pat_rotated" || t == TOKEN)
    );
    crate::commands::profile::export_to(&a.state, &a.dir.path().join("export.mudraft"), true)
        .unwrap();
    // Close the profile first: Windows locks byte ranges of open SQLite files.
    let Device { dir, state, .. } = a;
    drop(state);
    assert!(files_containing(dir.path(), TOKEN.as_bytes()).is_empty());
    assert!(gh.paths().iter().all(|p| {
        !gh.file(p)
            .unwrap()
            .windows(TOKEN.len())
            .any(|w| w == TOKEN.as_bytes())
    }));
}

#[tokio::test(start_paused = true)]
async fn a_failed_backup_or_transaction_changes_nothing() {
    let (gh, a, b, album) = linked_pair().await;
    review(&b, &album.edition, "remote");
    synced(b.sync().await);
    rate(&a, &album.tracks[0], Some(5));
    let before = a.dump();
    let head = gh.head();

    let backups = a.state.data_dir.join("backups");
    let _ = std::fs::remove_dir_all(&backups);
    std::fs::write(&backups, b"not a folder").unwrap();
    assert_eq!(a.sync().await.unwrap_err().code(), "backup_failed");
    assert_eq!(a.dump(), before);
    assert_eq!(gh.head(), head, "nothing published either");
    std::fs::remove_file(&backups).unwrap();

    a.sync.faults.fail_apply.store(true, Ordering::SeqCst);
    assert_eq!(a.sync().await.unwrap_err().code(), "storage_unavailable");
    assert_eq!(a.dump(), before, "the transaction rolled back");
    let patches = gh.count(Method::Patch, "/git/refs");
    synced(a.sync().await);
    assert_eq!(
        gh.count(Method::Patch, "/git/refs"),
        patches,
        "finished idempotently"
    );
    assert_eq!(review_of(&a, &album.edition).as_deref(), Some("remote"));
    synced(b.sync().await);
    assert_eq!(rating_of(&b, &album.tracks[0]), Some(5));
    assert_eq!(a.dump(), b.dump());
}

#[tokio::test(start_paused = true)]
async fn the_link_and_pending_changes_survive_a_restart() {
    let (gh, a, _b, album) = linked_pair().await;
    let a = a.restart();
    let status = a.sync.status(&a.state).unwrap();
    assert!(status.linked);
    assert_eq!(status.revision, gh.head());
    assert_eq!(status.pending_local_changes, Some(false));
    rate(&a, &album.tracks[0], Some(7));
    let a = a.restart();
    assert_eq!(
        a.sync.status(&a.state).unwrap().pending_local_changes,
        Some(true)
    );
    synced(a.sync().await);
    let a = a.restart();
    assert!(matches!(
        a.sync().await.unwrap(),
        SyncOutcome::UpToDate { .. }
    ));
}

#[tokio::test(start_paused = true)]
async fn deleting_an_edition_the_other_computer_listened_to_is_a_conflict() {
    let (_gh, a, b, album) = linked_pair().await;
    a.write(|tx| {
        tx.execute(
            "DELETE FROM listen_list_entry WHERE edition_id = ?1",
            [&album.edition],
        )?;
        tx.execute("DELETE FROM edition WHERE id = ?1", [&album.edition])?;
        Ok(())
    });
    listen(&b, &album.edition, "2026-10-07");
    synced(a.sync().await);
    let found = conflicts(b.sync().await);
    assert!(
        found
            .iter()
            .any(|c| c.kind == ConflictKind::Orphan && c.table == "edition")
    );
    assert_eq!(
        b.int("SELECT COUNT(*) FROM listen_event"),
        Some(1),
        "nothing changed yet"
    );
    // Keep the edition (it's this computer's side).
    synced(b.sync_with(None, choose(&found, Side::Local)).await);
    synced(a.sync().await);
    assert_eq!(
        a.int(&format!(
            "SELECT COUNT(*) FROM track WHERE edition_id = '{}'",
            album.edition
        )),
        Some(3)
    );
    assert_eq!(a.int("SELECT COUNT(*) FROM listen_event"), Some(1));
    assert_eq!(a.dump(), b.dump());
}

#[tokio::test(start_paused = true)]
async fn custom_artwork_syncs_by_content_and_a_missing_file_isnt_a_deletion() {
    let (gh, a, b, album) = linked_pair().await;
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    png.extend_from_slice(&[7; 64]);
    crate::artwork::replace_local(&a.state, &album.edition, &png).unwrap();
    synced(a.sync().await);
    let sha = crate::profile::sha256_hex(&png);
    assert!(gh.file(&format!("sync/assets/{sha}.png")).is_some());
    synced(b.sync().await);
    let name = b
        .text("SELECT file_name FROM artwork WHERE source = 'local'")
        .unwrap();
    assert_eq!(
        std::fs::read(b.state.data_dir.join("artwork").join(&name)).unwrap(),
        png
    );

    let lost = a
        .text("SELECT file_name FROM artwork WHERE source = 'local'")
        .unwrap();
    std::fs::remove_file(a.state.data_dir.join("artwork").join(&lost)).unwrap();
    synced(a.sync().await);
    let restored = a
        .text("SELECT file_name FROM artwork WHERE source = 'local'")
        .unwrap();
    assert_eq!(
        std::fs::read(a.state.data_dir.join("artwork").join(restored)).unwrap(),
        png
    );
    assert!(
        gh.file(&format!("sync/assets/{sha}.png")).is_some(),
        "not deleted remotely"
    );

    crate::artwork::remove(&b.state, &album.edition).unwrap();
    synced(b.sync().await);
    synced(a.sync().await);
    assert_eq!(
        a.text("SELECT source FROM artwork WHERE owner_type = 'edition'"),
        Some("removed".into())
    );
    assert!(
        gh.file(&format!("sync/assets/{sha}.png")).is_none(),
        "unreferenced images are pruned"
    );
    assert_eq!(a.dump(), b.dump());
}
