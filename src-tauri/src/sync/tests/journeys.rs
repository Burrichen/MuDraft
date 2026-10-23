//! User journeys between two separate profiles. "mac" and "windows" are just the two
//! profiles' names: both run on whichever OS runs the tests.

use std::sync::{Arc, Mutex};

use super::*;
use crate::library::album::{self, AlbumEdit};
use crate::sync::summary::ChangeSummary;
use crate::sync::{Progress, RemoteKind};

fn retitle(dev: &Device, album_id: &str, title: &str) {
    dev.write(|tx| {
        album::edit(
            tx,
            album_id,
            &AlbumEdit {
                title: Some(title.into()),
                original_date: None,
                description: None,
            },
        )
    });
}

fn title_of(dev: &Device, album_id: &str) -> Option<String> {
    dev.text(&format!("SELECT title FROM album WHERE id = '{album_id}'"))
}

fn summary(outcome: SyncOutcome) -> (ChangeSummary, ChangeSummary) {
    match outcome {
        SyncOutcome::Synced { received, sent, .. } => (received, sent),
        other => panic!("expected changes, got {other:?}"),
    }
}

#[tokio::test(start_paused = true)]
async fn mac_edits_album_a_and_windows_downloads_it() {
    let (_gh, mac, windows, a) = linked_pair().await;
    retitle(&mac, &a.album, "OK Computer OKNOTOK");
    let (_, sent) = summary(mac.sync().await.unwrap());
    assert_eq!(
        sent,
        ChangeSummary {
            albums: 1,
            ..ChangeSummary::default()
        }
    );
    let outcome = windows.sync().await.unwrap();
    let SyncOutcome::Synced {
        backup_file_name, ..
    } = &outcome
    else {
        panic!()
    };
    assert!(
        backup_file_name.is_some(),
        "backed up before applying GitHub's data"
    );
    let (received, _) = summary(outcome);
    assert_eq!(
        received,
        ChangeSummary {
            albums: 1,
            ..ChangeSummary::default()
        }
    );
    assert_eq!(
        title_of(&windows, &a.album).as_deref(),
        Some("OK Computer OKNOTOK")
    );
    assert_eq!(
        windows
            .sync
            .status(&windows.state)
            .unwrap()
            .pending_local_changes,
        Some(false)
    );
}

#[tokio::test(start_paused = true)]
async fn windows_adds_album_b_and_mac_downloads_it() {
    let (_gh, mac, windows, _a) = linked_pair().await;
    let b = add_album(&windows, "Kid A");
    listen(&windows, &b.edition, "2026-10-10");
    rate(&windows, &b.tracks[0], Some(7));
    review(&windows, &b.edition, "Cold and lovely");
    windows.write(|tx| {
        let tag = tags::create(tx, "Winter", None)?;
        tags::set_album_tags(tx, std::slice::from_ref(&b.album), &[tag.id], &[])
    });
    let status = windows.sync.status(&windows.state).unwrap();
    let pending = status.pending.unwrap();
    assert_eq!(
        (
            pending.albums,
            pending.listens,
            pending.ratings,
            pending.reviews,
            pending.tags
        ),
        (1, 1, 1, 1, 2)
    );
    let (_, sent) = summary(windows.sync().await.unwrap());
    assert_eq!(sent, pending, "what was pending is exactly what was sent");
    let (received, _) = summary(mac.sync().await.unwrap());
    assert_eq!(received, pending);
    assert_eq!(title_of(&mac, &b.album).as_deref(), Some("Kid A"));
    assert_eq!(mac.dump(), windows.dump());
}

#[tokio::test(start_paused = true)]
async fn different_fields_of_one_album_merge_safely() {
    let (_gh, mac, windows, a) = linked_pair().await;
    retitle(&mac, &a.album, "OK Computer (1997)");
    windows.write(|tx| {
        album::edit(
            tx,
            &a.album,
            &AlbumEdit {
                title: None,
                original_date: Some("1997-05-21".into()),
                description: Some("Third album".into()),
            },
        )
    });
    synced(mac.sync().await);
    synced(windows.sync().await);
    synced(mac.sync().await);
    for dev in [&mac, &windows] {
        assert_eq!(
            title_of(dev, &a.album).as_deref(),
            Some("OK Computer (1997)")
        );
        assert_eq!(
            dev.text(&format!(
                "SELECT original_date || ' ' || description FROM album WHERE id = '{}'",
                a.album
            ))
            .as_deref(),
            Some("1997-05-21 Third album")
        );
    }
    assert_eq!(mac.dump(), windows.dump());
}

#[tokio::test(start_paused = true)]
async fn the_same_field_prompts_a_conflict_and_other_changes_wait() {
    let (gh, mac, windows, a) = linked_pair().await;
    retitle(&mac, &a.album, "Mac title");
    retitle(&windows, &a.album, "Windows title");
    rate(&windows, &a.tracks[0], Some(5)); // independent, waits for the choice
    synced(mac.sync().await);
    let head = gh.head();
    let found = conflicts(windows.sync().await);
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].record, "Windows title",
        "named from this computer's data"
    );
    assert_eq!(found[0].columns, vec!["title".to_owned()]);
    assert_eq!(found[0].local, json!({ "title": "Windows title" }));
    assert_eq!(found[0].remote, json!({ "title": "Mac title" }));
    assert_eq!(gh.head(), head, "nothing published while undecided");

    // Cancelling (closing the app, even) keeps everything as it was.
    let windows = windows.restart();
    let status = windows.sync.status(&windows.state).unwrap();
    assert_eq!(status.unresolved_conflicts, 1);
    assert_eq!(
        status.pending.unwrap().ratings,
        1,
        "the rating is still pending"
    );
    assert_eq!(
        title_of(&windows, &a.album).as_deref(),
        Some("Windows title")
    );

    let (received, sent) = summary(
        windows
            .sync_with(None, choose(&found, Side::Remote))
            .await
            .unwrap(),
    );
    assert_eq!(received.albums, 1);
    assert_eq!(sent.ratings, 1);
    assert_eq!(
        windows
            .sync
            .status(&windows.state)
            .unwrap()
            .unresolved_conflicts,
        0
    );
    synced(mac.sync().await);
    assert_eq!(title_of(&mac, &a.album).as_deref(), Some("Mac title"));
    assert_eq!(rating_of(&mac, &a.tracks[0]), Some(5));
    assert_eq!(mac.dump(), windows.dump());
}

#[tokio::test(start_paused = true)]
async fn a_deleted_listen_stays_deleted_on_both_computers() {
    let (_gh, mac, windows, a) = linked_pair().await;
    let id = listen(&mac, &a.edition, "2026-10-11");
    synced(mac.sync().await);
    synced(windows.sync().await);
    windows.write(|tx| listening::delete(tx, &id));
    let (_, sent) = summary(windows.sync().await.unwrap());
    assert_eq!(sent.listens, 1);
    synced(mac.sync().await);
    let deleted = format!("SELECT deleted_at IS NOT NULL FROM listen_event WHERE id = '{id}'");
    assert_eq!(mac.int(&deleted), Some(1));
    let mac = mac.restart();
    assert!(matches!(
        mac.sync().await.unwrap(),
        SyncOutcome::UpToDate { .. }
    ));
    assert_eq!(mac.int(&deleted), Some(1));
}

#[tokio::test(start_paused = true)]
async fn first_sync_previews_and_never_silently_replaces_a_collection() {
    let gh = new_repo_with_readme();
    let mac = Device::new(&gh).await;
    add_album(&mac, "OK Computer");
    mac.init().await;

    let windows = Device::new(&gh).await;
    let mine = add_album(&windows, "Mine");
    windows.write(|tx| personal::add_to_collection(tx, &mine.edition, CollectionSource::Manual));
    let check = windows.sync.check(&windows.state).await.unwrap();
    assert_eq!(check.kind, RemoteKind::Ready);
    assert_eq!(check.same_sync, None);
    assert_eq!((check.local.albums, check.local.collection), (1, 1));
    assert_eq!(check.remote.as_ref().unwrap().albums, 1);

    let before = windows.dump();
    match windows.sync().await.unwrap() {
        SyncOutcome::NeedsConfirmation {
            reason,
            local,
            remote,
        } => {
            assert_eq!(reason, ConfirmReason::MergeIntoPopulated);
            assert_eq!(local.collection, 1);
            assert_eq!(remote.unwrap().albums, 1);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(windows.dump(), before);
    assert_eq!(windows.sync_backups(), 0);

    // "Download remote library": explicit, and backed up first.
    let SyncOutcome::Synced {
        backup_file_name, ..
    } = synced(windows.sync_with(Some(FirstSync::Download), vec![]).await)
    else {
        panic!()
    };
    assert_eq!(
        windows.int("SELECT COUNT(*) FROM album WHERE title = 'Mine'"),
        Some(0)
    );
    let backup = windows
        .state
        .data_dir
        .join("backups")
        .join(backup_file_name.unwrap());
    let kept: i64 = rusqlite::Connection::open(&backup)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM album WHERE title = 'Mine'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(kept, 1, "the replaced library is in the backup");
    assert_eq!(mac.dump(), windows.dump());

    // "Merge both" keeps both libraries.
    let linux = Device::new(&gh).await;
    add_album(&linux, "Theirs");
    synced(linux.sync_with(Some(FirstSync::Merge), vec![]).await);
    synced(mac.sync().await);
    assert_eq!(mac.int("SELECT COUNT(*) FROM album"), Some(2));
    assert_eq!(mac.dump(), linux.dump());
}

#[tokio::test(start_paused = true)]
async fn too_little_disk_space_is_reported_before_anything_changes() {
    let (gh, mac, windows, a) = linked_pair().await;
    retitle(&mac, &a.album, "New");
    synced(mac.sync().await);
    *windows.sync.faults.free_space.lock().unwrap() = Some(1024);
    let before = windows.dump();
    let backups = windows.sync_backups();
    let err = windows.sync().await.unwrap_err();
    assert_eq!(err.code(), "disk_full");
    assert!(err.to_string().contains("Nothing was changed"));
    assert_eq!(windows.dump(), before);
    assert_eq!(windows.sync_backups(), backups);
    assert_eq!(gh.head(), mac.sync.status(&mac.state).unwrap().revision);
    *windows.sync.faults.free_space.lock().unwrap() = None;
    synced(windows.sync().await);
    assert_eq!(title_of(&windows, &a.album).as_deref(), Some("New"));
}

#[tokio::test(start_paused = true)]
async fn progress_is_reported_in_order() {
    let (_gh, mac, windows, a) = linked_pair().await;
    retitle(&mac, &a.album, "From Mac");
    synced(mac.sync().await);
    rate(&windows, &a.tracks[0], Some(4));
    let seen: Arc<Mutex<Vec<Progress>>> = Arc::default();
    let sink = seen.clone();
    let report = move |p: Progress| sink.lock().unwrap().push(p);
    synced(
        windows
            .sync
            .sync_reporting(&windows.state, &SyncRequest::default(), &report)
            .await,
    );
    let phases: Vec<&'static str> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|p| match p {
            Progress::Reading => "reading",
            Progress::Downloading { .. } => "downloading",
            Progress::Merging => "merging",
            Progress::BackingUp => "backing up",
            Progress::Uploading { .. } => "uploading",
            Progress::Publishing => "publishing",
            Progress::Applying => "applying",
        })
        .collect();
    let mut order: Vec<&str> = phases.clone();
    order.dedup();
    assert_eq!(
        order,
        [
            "reading",
            "downloading",
            "merging",
            "backing up",
            "uploading",
            "publishing",
            "applying"
        ],
        "{phases:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn test_connection_reports_remote_changes_without_applying_them() {
    let (_gh, mac, windows, a) = linked_pair().await;
    listen(&mac, &a.edition, "2026-10-12");
    synced(mac.sync().await);
    let check = windows.sync.check(&windows.state).await.unwrap();
    assert_eq!(check.same_sync, Some(true));
    assert_eq!(check.remote_changes.unwrap().listens, 1);
    assert_eq!(
        windows.int("SELECT COUNT(*) FROM listen_event"),
        Some(0),
        "only looked"
    );
}

/// The release acceptance scenario (docs/RELEASE_ACCEPTANCE.md) end to end, with profile
/// export/import still working as the recovery path afterwards.
#[tokio::test(start_paused = true)]
async fn release_acceptance_two_devices() {
    let (_gh, mac, windows, a) = linked_pair().await;

    // 1. Listen, rate, and tag on the Mac → Sync Now → Windows → Sync Now.
    let heard = listen(&mac, &a.edition, "2026-10-20");
    rate(&mac, &a.tracks[0], Some(9));
    rate(&mac, &a.tracks[1], Some(0));
    mac.write(|tx| {
        let tag = tags::create(tx, "Release check", None)?;
        tags::set_album_tags(tx, std::slice::from_ref(&a.album), &[tag.id], &[])
    });
    let (_, sent) = summary(mac.sync().await.unwrap());
    assert_eq!((sent.listens, sent.ratings, sent.tags), (1, 2, 2));
    let (received, _) = summary(windows.sync().await.unwrap());
    assert_eq!(received.listens, 1);
    assert_eq!(
        windows.int(&format!(
            "SELECT COUNT(*) FROM listen_event WHERE id = '{heard}'"
        )),
        Some(1)
    );
    assert_eq!(rating_of(&windows, &a.tracks[0]), Some(9));
    assert_eq!(
        rating_of(&windows, &a.tracks[1]),
        Some(0),
        "zero stays a rating"
    );
    assert_eq!(
        windows.text("SELECT name FROM tag WHERE builtin_key IS NULL"),
        Some("Release check".into())
    );

    // 2. Independent Windows changes → Sync Now → received on the Mac.
    let b = add_album(&windows, "Amnesiac");
    review(&windows, &a.edition, "Better every time");
    synced(windows.sync().await);
    synced(mac.sync().await);
    assert_eq!(title_of(&mac, &b.album).as_deref(), Some("Amnesiac"));
    assert_eq!(
        review_of(&mac, &a.edition).as_deref(),
        Some("Better every time")
    );

    // 3. Conflicting offline edits → a safe conflict prompt → resolved.
    review(&mac, &a.edition, "Mac's take");
    review(&windows, &a.edition, "Windows' take");
    synced(mac.sync().await);
    let found = conflicts(windows.sync().await);
    assert_eq!(found.len(), 1);
    assert_eq!(
        review_of(&windows, &a.edition).as_deref(),
        Some("Windows' take"),
        "nothing applied yet"
    );
    synced(windows.sync_with(None, choose(&found, Side::Local)).await);
    synced(mac.sync().await);
    assert_eq!(
        review_of(&mac, &a.edition).as_deref(),
        Some("Windows' take")
    );
    assert_eq!(mac.dump(), windows.dump());

    // 4. Recovery path: export on Windows, restore into a fresh profile.
    let archive = windows.dir.path().join("recovery.mudraft");
    crate::commands::profile::export_to(&windows.state, &archive, true).unwrap();
    let spare = tempfile::tempdir().unwrap();
    let fresh = AppState::open(StorageProfile::Development, spare.path().join("data"));
    let preview = crate::commands::profile::inspect(&fresh, &archive).unwrap();
    crate::commands::profile::restore_from(None, &fresh, &archive, &preview.sha256).unwrap();
    let schema = fresh
        .with_db(|db| db.read(crate::sync::schema::Schema::load))
        .unwrap();
    let restored = crate::sync::read_snapshot(&fresh, &schema).unwrap().state;
    assert_eq!(
        restored,
        windows.dump(),
        "the exported profile restores the whole library"
    );
}
