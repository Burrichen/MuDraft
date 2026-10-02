//! Metadata search/match commands. The renderer supplies a request ID (UUID) per call so
//! it can cancel work it no longer needs. Nothing is imported until the renderer names an
//! explicit release group *and* release the user chose.

use std::path::Path;
use std::sync::Arc;

use tauri::State;

use crate::domain::ids::parse_uuid;
use crate::error::{AppError, AppResult};
use crate::library::metadata_import::{
    self, ImportOutcome, ImportRequest, ManualAlbum, ManualAlbumCreated,
};
use crate::library::personal;
use crate::metadata::cache::{MemoryCache, ResponseCache, SqliteCache};
use crate::metadata::genres::BROAD_GENRES;
use crate::metadata::http::{ReqwestTransport, Transport};
use crate::metadata::musicbrainz::MusicBrainz;
use crate::metadata::requests::RequestRegistry;
use crate::metadata::{EditionList, Fetched, MetadataProvider, SearchPage, SearchQuery};
use crate::state::AppState;

/// Where a newly added album goes. Collection membership here is explicit (no listen).
#[derive(Debug, Clone, Copy, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Destination {
    pub listen_list: bool,
    pub collection: bool,
}

impl Destination {
    fn apply(self, tx: &rusqlite::Transaction<'_>, edition_id: &str) -> AppResult<()> {
        if self.listen_list {
            personal::add_to_listen_list(tx, edition_id)?;
        }
        if self.collection {
            personal::add_to_collection(tx, edition_id, personal::CollectionSource::Manual)?;
        }
        Ok(())
    }
}

pub struct MetadataService {
    provider: Arc<dyn MetadataProvider>,
    requests: RequestRegistry,
}

impl MetadataService {
    pub fn new(provider: Arc<dyn MetadataProvider>) -> Self {
        Self {
            provider,
            requests: RequestRegistry::default(),
        }
    }

    /// Production service: native HTTPS, one shared MusicBrainz queue, and the response
    /// cache in the app database (falling back to memory if storage is unavailable).
    pub fn musicbrainz(db_path: Option<&Path>) -> Result<Self, String> {
        let transport: Arc<dyn Transport> = Arc::new(ReqwestTransport::new()?);
        let cache: Arc<dyn ResponseCache> = match db_path.map(SqliteCache::open) {
            Some(Ok(c)) => Arc::new(c),
            Some(Err(e)) => {
                eprintln!("MuDraft: metadata cache unavailable, using memory: {e}");
                Arc::new(MemoryCache::default())
            }
            None => Arc::new(MemoryCache::default()),
        };
        Ok(Self::new(Arc::new(MusicBrainz::new(transport, cache))))
    }

    pub fn provider(&self) -> &dyn MetadataProvider {
        self.provider.as_ref()
    }

    pub fn requests(&self) -> &RequestRegistry {
        &self.requests
    }

    pub async fn search(
        &self,
        request_id: &str,
        query: &SearchQuery,
        offset: u32,
    ) -> AppResult<Fetched<SearchPage>> {
        let guard = self.requests.begin(request_id)?;
        self.provider.search(query, offset, &guard.token).await
    }

    pub async fn editions(
        &self,
        request_id: &str,
        release_group_id: &str,
    ) -> AppResult<Fetched<EditionList>> {
        let guard = self.requests.begin(request_id)?;
        self.provider.editions(release_group_id, &guard.token).await
    }

    pub fn cancel(&self, request_id: &str) -> AppResult<bool> {
        self.requests.cancel(request_id)
    }

    /// Fetch the chosen release group and release, then persist them in one transaction.
    pub async fn import(
        &self,
        state: &AppState,
        request_id: &str,
        release_group_id: &str,
        release_id: &str,
        edition_name: Option<&str>,
        dest: Destination,
    ) -> AppResult<ImportOutcome> {
        let rgid = parse_uuid("release group", release_group_id)?;
        let rid = parse_uuid("release", release_id)?;
        let guard = self.requests.begin(request_id)?;
        let group = self.provider.release_group(&rgid, &guard.token).await?;
        let release = self.provider.release(&rid, &guard.token).await?;
        if guard.token.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        state.with_db(|db| {
            db.write(|tx| {
                let outcome = metadata_import::import_release(
                    tx,
                    &ImportRequest {
                        release_group: &group.value,
                        release: &release.value,
                        edition_name,
                        group_fetched_at: &group.fetched_at,
                        release_fetched_at: &release.fetched_at,
                    },
                )?;
                dest.apply(tx, &outcome.edition_id)?;
                Ok(outcome)
            })
        })
    }
}

#[tauri::command]
pub async fn metadata_search(
    service: State<'_, MetadataService>,
    request_id: String,
    query: SearchQuery,
    offset: Option<u32>,
) -> AppResult<Fetched<SearchPage>> {
    service
        .search(&request_id, &query, offset.unwrap_or(0))
        .await
}

#[tauri::command]
pub async fn metadata_editions(
    service: State<'_, MetadataService>,
    request_id: String,
    release_group_id: String,
) -> AppResult<Fetched<EditionList>> {
    service.editions(&request_id, &release_group_id).await
}

#[tauri::command]
pub fn metadata_cancel(service: State<'_, MetadataService>, request_id: String) -> AppResult<bool> {
    service.cancel(&request_id)
}

#[tauri::command]
pub async fn metadata_import(
    service: State<'_, MetadataService>,
    state: State<'_, AppState>,
    request_id: String,
    release_group_id: String,
    release_id: String,
    edition_name: Option<String>,
    destination: Destination,
) -> AppResult<ImportOutcome> {
    service
        .import(
            &state,
            &request_id,
            &release_group_id,
            &release_id,
            edition_name.as_deref(),
            destination,
        )
        .await
}

/// Unmatched album entered by hand. `request_id` doubles as the idempotency key.
#[tauri::command]
pub async fn library_add_manual_album(
    state: State<'_, AppState>,
    request_id: String,
    album: ManualAlbum,
    destination: Destination,
) -> AppResult<ManualAlbumCreated> {
    state.with_db(|db| {
        db.write(|tx| {
            let created = metadata_import::add_manual_album(tx, Some(&request_id), &album)?;
            destination.apply(tx, &created.edition_id)?;
            Ok(created)
        })
    })
}

#[tauri::command]
pub async fn library_set_album_genres(
    state: State<'_, AppState>,
    album_id: String,
    genres: Vec<String>,
) -> AppResult<Vec<String>> {
    state.with_db(|db| {
        db.write(|tx| metadata_import::set_album_genres_manually(tx, &album_id, &genres))
    })
}

#[tauri::command]
pub fn genre_vocabulary() -> Vec<&'static str> {
    BROAD_GENRES.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::new_id;
    use crate::metadata::testing::{MockTransport, Reply};
    use crate::paths::StorageProfile;

    const RG: &str = "b1392450-e666-3926-a536-22c65f834433";
    const REL: &str = "11111111-1111-4111-8111-111111111111";

    fn service(transport: Arc<MockTransport>) -> MetadataService {
        MetadataService::new(Arc::new(MusicBrainz::new(
            transport,
            Arc::new(MemoryCache::default()),
        )))
    }

    fn responder(url: &str, _: usize) -> Reply {
        if url.contains("/release-group/") {
            Reply::Json(
                200,
                format!(
                    r#"{{"id": "{RG}", "title": "OK Computer", "first-release-date": "1997-05-21",
                "artist-credit": [{{"name": "Radiohead", "artist": {{"id": "a74b1b7f-71a5-4011-9441-d0b5e4122711", "name": "Radiohead"}}}}],
                "genres": [{{"name": "alternative rock", "count": 5}}], "tags": []}}"#
                ),
            )
        } else {
            Reply::Json(
                200,
                format!(
                    r#"{{"id": "{REL}", "title": "OK Computer", "release-group": {{"id": "{RG}"}},
                "media": [{{"position": 1, "tracks": [{{"id": "22222222-2222-4222-8222-222222222221", "position": 1, "title": "Airbag"}}]}}]}}"#
                ),
            )
        }
    }

    #[tokio::test(start_paused = true)]
    async fn import_fetches_the_chosen_ids_and_can_add_to_listen_list() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::open(StorageProfile::Development, dir.path().to_path_buf());
        let transport = Arc::new(MockTransport::new(responder));
        let svc = service(transport.clone());
        let out = svc
            .import(
                &state,
                &new_id(),
                RG,
                REL,
                None,
                Destination {
                    listen_list: true,
                    collection: false,
                },
            )
            .await
            .unwrap();
        assert!(out.album_created);
        assert_eq!(out.genres, vec!["Rock"]);
        assert_eq!(transport.call_count(), 2);
        let listed = state.with_db(|db| db.read(personal::listen_list)).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].edition_id, out.edition_id);

        let err = svc
            .import(
                &state,
                &new_id(),
                "fuzzy match",
                REL,
                None,
                Destination::default(),
            )
            .await
            .unwrap_err();
        assert_eq!(
            err.code(),
            "validation",
            "only explicit provider IDs are accepted"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn cancelled_import_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::open(StorageProfile::Development, dir.path().to_path_buf());
        let transport = Arc::new(MockTransport::new(|url, n| {
            if n == 0 {
                responder(url, n)
            } else {
                Reply::Hang
            }
        }));
        let svc = Arc::new(service(transport));
        let id = new_id();
        let task = {
            let svc = svc.clone();
            let id = id.clone();
            let state_dir = dir.path().to_path_buf();
            tokio::spawn(async move {
                let state = AppState::open(StorageProfile::Development, state_dir);
                svc.import(
                    &state,
                    &id,
                    RG,
                    REL,
                    None,
                    Destination {
                        listen_list: true,
                        collection: false,
                    },
                )
                .await
            })
        };
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        assert!(svc.cancel(&id).unwrap());
        assert_eq!(task.await.unwrap().unwrap_err().code(), "cancelled");
        let albums: i64 = state
            .with_db(|db| {
                db.read(|c| Ok(c.query_row("SELECT COUNT(*) FROM album", [], |r| r.get(0))?))
            })
            .unwrap();
        assert_eq!(albums, 0);
    }
}
