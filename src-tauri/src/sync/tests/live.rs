//! Two separate profiles against a real, dedicated PRIVATE GitHub repository. Ignored by
//! default; run with
//!
//! ```text
//! MUDRAFT_SYNC_TEST_OWNER=… MUDRAFT_SYNC_TEST_REPO=… MUDRAFT_SYNC_TEST_TOKEN=github_pat_… \
//!   cargo test --lib live_ -- --ignored
//! ```
//!
//! The repository must be empty, freshly created with a README, or already used only by
//! this test. Each run adds one uniquely named album to it.

use std::sync::Arc;

use super::*;
use crate::sync::github::ReqwestGitHubTransport;
use crate::sync::{RemoteKind, SyncService};

struct Live {
    _dir: tempfile::TempDir,
    state: AppState,
    sync: SyncService,
}

async fn live_profile(owner: &str, repo: &str, token: &str) -> Live {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::open(StorageProfile::Development, dir.path().join("data"));
    let secrets = Arc::new(MemorySecretStore::default());
    secrets.write(token).unwrap();
    let sync = SyncService::new(Arc::new(ReqwestGitHubTransport::new().unwrap()), secrets);
    sync.configure(&state, owner, repo).await.unwrap();
    Live {
        _dir: dir,
        state,
        sync,
    }
}

#[tokio::test]
#[ignore = "needs a dedicated private GitHub repository and token (see module docs)"]
async fn live_two_profiles_sync_through_a_private_repository() {
    let vars = [
        "MUDRAFT_SYNC_TEST_OWNER",
        "MUDRAFT_SYNC_TEST_REPO",
        "MUDRAFT_SYNC_TEST_TOKEN",
    ]
    .map(|v| std::env::var(v).unwrap_or_default());
    let [owner, repo, token] = &vars;
    if vars.iter().any(String::is_empty) {
        eprintln!("skipped: MUDRAFT_SYNC_TEST_OWNER/REPO/TOKEN aren't set");
        return;
    }
    let first = live_profile(owner, repo, token).await;
    let check = first.sync.check(&first.state).await.unwrap();
    match check.kind {
        RemoteKind::Empty | RemoteKind::Uninitialised => {
            assert!(matches!(
                first.sync.initialize(&first.state, true).await.unwrap(),
                SyncOutcome::Synced { .. }
            ));
        }
        RemoteKind::Ready => {
            first
                .sync
                .sync(&first.state, &SyncRequest::default())
                .await
                .unwrap();
        }
        RemoteKind::Unrelated => {
            panic!("{owner}/{repo} holds other files; use a dedicated repository")
        }
    }

    let title = format!("Live test {}", crate::domain::ids::new_id());
    first
        .state
        .with_db(|db| {
            db.write(|tx| {
                let artist = catalogue::create_artist(
                    tx,
                    None,
                    &NewArtist {
                        name: "Live".into(),
                        sort_name: None,
                        musicbrainz_id: None,
                    },
                )?;
                catalogue::create_album(
                    tx,
                    None,
                    &NewAlbum {
                        title: title.clone(),
                        original_date: None,
                        musicbrainz_release_group_id: None,
                        credits: credit(artist),
                        genres: vec![],
                    },
                )
            })
        })
        .unwrap();
    let outcome = first
        .sync
        .sync(&first.state, &SyncRequest::default())
        .await
        .unwrap();
    assert!(matches!(outcome, SyncOutcome::Synced { .. }), "{outcome:?}");

    let second = live_profile(owner, repo, token).await;
    let outcome = second
        .sync
        .sync(&second.state, &SyncRequest::default())
        .await
        .unwrap();
    assert!(matches!(outcome, SyncOutcome::Synced { .. }), "{outcome:?}");
    let found: i64 = second
        .state
        .with_db(|db| {
            db.read(|c| {
                Ok(c.query_row(
                    "SELECT COUNT(*) FROM album WHERE title = ?1",
                    [&title],
                    |r| r.get(0),
                )?)
            })
        })
        .unwrap();
    assert_eq!(found, 1);
}
