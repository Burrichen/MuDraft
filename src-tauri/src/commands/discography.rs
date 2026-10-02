//! Artist catalogue retrieval. Every request goes through the shared MusicBrainz queue,
//! cache, and request registry (`metadata_cancel` stops a run). Each page and each album's
//! tracklist is committed as it arrives, so a cancelled or failed run keeps its progress
//! and the next run resumes from there. The renderer shows progress by re-reading
//! `artist_catalogue`, whose coverage counts reflect what has been stored so far.

use serde::Serialize;
use tauri::State;

use super::metadata::MetadataService;
use crate::domain::ids::parse_uuid;
use crate::error::{AppError, AppResult};
use crate::library::discography::{self, ArtistCatalogue, ManualEntry, Scope};
use crate::library::metadata_import::{self, ImportRequest};
use crate::library::personal;
use crate::metadata::FetchSource;
use crate::state::AppState;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchRun {
    pub pages_fetched: u32,
    pub complete: bool,
    /// Some pages came from the offline cache because MusicBrainz was unreachable.
    pub used_offline_cache: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TracklistFailure {
    pub release_group_id: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TracklistRun {
    pub loaded: u32,
    pub failed: Vec<TracklistFailure>,
    /// Still without a tracklist after this run (resume to continue).
    pub remaining: u32,
    /// Error code that stopped the run early (offline, timeout, rate limited).
    pub stopped: Option<String>,
    pub used_offline_cache: bool,
}

fn stops_run(e: &AppError) -> bool {
    matches!(
        e,
        AppError::Offline(_) | AppError::Timeout(_) | AppError::RateLimited(_)
    )
}

/// Fetch (or resume fetching) the artist's official catalogue, page by page.
pub async fn fetch_catalogue(
    svc: &MetadataService,
    state: &AppState,
    request_id: &str,
    artist_id: &str,
) -> AppResult<FetchRun> {
    let artist_id = parse_uuid("artist", artist_id)?;
    let mbid = state
        .with_db(|db| db.read(|c| discography::artist_musicbrainz_id(c, &artist_id)))?
        .ok_or_else(|| {
            AppError::validation(
                "artist",
                "this artist isn't linked to MusicBrainz, so there is no online catalogue to fetch",
            )
        })?;
    let guard = svc.requests().begin(request_id)?;
    let mut offset =
        state.with_db(|db| db.write(|tx| discography::begin_pass(tx, &artist_id, &mbid)))?;
    let mut run = FetchRun {
        pages_fetched: 0,
        complete: false,
        used_offline_cache: false,
    };
    loop {
        let page = match svc
            .provider()
            .artist_release_groups(&mbid, offset, &guard.token)
            .await
        {
            Ok(p) => p,
            Err(AppError::Cancelled) => return Err(AppError::Cancelled),
            Err(e) => {
                let message = e.to_string();
                state.with_db(|db| {
                    db.write(|tx| discography::record_pass_error(tx, &artist_id, &message))
                })?;
                return Err(e);
            }
        };
        if guard.token.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        run.used_offline_cache |= page.source == FetchSource::StaleCache;
        let done = state.with_db(|db| {
            db.write(|tx| discography::record_page(tx, &artist_id, &page.value, &page.fetched_at))
        })?;
        run.pages_fetched += 1;
        if done {
            run.complete = true;
            return Ok(run);
        }
        offset = page.value.offset + page.value.received;
    }
}

/// Load reference tracklists for counted albums that lack one, one album at a time.
pub async fn load_tracklists(
    svc: &MetadataService,
    state: &AppState,
    request_id: &str,
    artist_id: &str,
) -> AppResult<TracklistRun> {
    let artist_id = parse_uuid("artist", artist_id)?;
    let targets = state.with_db(|db| db.read(|c| discography::tracklist_targets(c, &artist_id)))?;
    let guard = svc.requests().begin(request_id)?;
    let mut run = TracklistRun {
        loaded: 0,
        failed: vec![],
        remaining: targets.len() as u32,
        stopped: None,
        used_offline_cache: false,
    };
    for group in targets {
        if guard.token.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        let result: AppResult<()> = async {
            let editions = svc.provider().editions(&group, &guard.token).await?;
            run.used_offline_cache |= editions.source == FetchSource::StaleCache;
            let chosen = state.with_db(|db| {
                db.write(|tx| {
                    discography::record_representative(
                        tx,
                        &group,
                        &editions.value.editions,
                        &editions.fetched_at,
                    )
                })
            })?;
            let Some(release_id) = chosen else {
                return Err(AppError::Provider(
                    "MusicBrainz lists no release with a tracklist for this album".into(),
                ));
            };
            let release = svc.provider().release(&release_id, &guard.token).await?;
            run.used_offline_cache |= release.source == FetchSource::StaleCache;
            if guard.token.is_cancelled() {
                return Err(AppError::Cancelled);
            }
            state.with_db(|db| {
                db.write(|tx| {
                    discography::record_tracklist(tx, &group, &release.value, &release.fetched_at)
                })
            })
        }
        .await;
        match result {
            Ok(()) => {
                run.loaded += 1;
                run.remaining -= 1;
            }
            Err(AppError::Cancelled) => return Err(AppError::Cancelled),
            Err(e) => {
                let message = e.to_string();
                state.with_db(|db| {
                    db.write(|tx| discography::record_reference_error(tx, &group, &message))
                })?;
                let stop = stops_run(&e);
                run.failed.push(TracklistFailure {
                    release_group_id: group,
                    code: e.code().into(),
                    message,
                });
                if stop {
                    run.stopped = Some(e.code().into());
                    break;
                }
            }
        }
    }
    Ok(run)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddedToListenList {
    pub album_id: String,
    pub edition_id: String,
    /// False when an edition already in the library was used.
    pub imported: bool,
}

/// Explicitly add a discovered album to the Listen List: an edition already in the
/// library if there is one, otherwise the album's reference (representative) release.
pub async fn add_to_listen_list(
    svc: &MetadataService,
    state: &AppState,
    request_id: &str,
    entry_id: &str,
) -> AppResult<AddedToListenList> {
    let group = state.with_db(|db| db.read(|c| discography::entry_release_group(c, entry_id)))?;
    if let Some(edition) =
        state.with_db(|db| db.read(|c| discography::library_edition_for_group(c, &group)))?
    {
        return state.with_db(|db| {
            db.write(|tx| {
                personal::add_to_listen_list(tx, &edition)?;
                Ok(AddedToListenList {
                    album_id: crate::library::album_of_edition(tx, &edition)?,
                    edition_id: edition.clone(),
                    imported: false,
                })
            })
        });
    }
    let guard = svc.requests().begin(request_id)?;
    let release_id =
        match state.with_db(|db| db.read(|c| discography::stored_representative(c, &group)))? {
            Some(r) => r,
            None => {
                let editions = svc.provider().editions(&group, &guard.token).await?;
                state
                    .with_db(|db| {
                        db.write(|tx| {
                            discography::record_representative(
                                tx,
                                &group,
                                &editions.value.editions,
                                &editions.fetched_at,
                            )
                        })
                    })?
                    .ok_or_else(|| {
                        AppError::Provider(
                            "MusicBrainz lists no release with a tracklist for this album".into(),
                        )
                    })?
            }
        };
    let detail = svc.provider().release_group(&group, &guard.token).await?;
    let release = svc.provider().release(&release_id, &guard.token).await?;
    if guard.token.is_cancelled() {
        return Err(AppError::Cancelled);
    }
    state.with_db(|db| {
        db.write(|tx| {
            let out = metadata_import::import_release(
                tx,
                &ImportRequest {
                    release_group: &detail.value,
                    release: &release.value,
                    edition_name: None,
                    group_fetched_at: &detail.fetched_at,
                    release_fetched_at: &release.fetched_at,
                },
            )?;
            personal::add_to_listen_list(tx, &out.edition_id)?;
            Ok(AddedToListenList {
                album_id: out.album_id,
                edition_id: out.edition_id,
                imported: true,
            })
        })
    })
}

#[tauri::command]
pub async fn artist_catalogue_add_to_listen_list(
    service: State<'_, MetadataService>,
    state: State<'_, AppState>,
    request_id: String,
    entry_id: String,
) -> AppResult<AddedToListenList> {
    add_to_listen_list(&service, &state, &request_id, &entry_id).await
}

#[tauri::command]
pub async fn artist_catalogue(
    state: State<'_, AppState>,
    artist_id: String,
) -> AppResult<ArtistCatalogue> {
    state.with_db(|db| db.read(|c| discography::catalogue(c, &artist_id)))
}

/// Cancel with `metadata_cancel(request_id)`; call again to resume.
#[tauri::command]
pub async fn artist_catalogue_fetch(
    service: State<'_, MetadataService>,
    state: State<'_, AppState>,
    request_id: String,
    artist_id: String,
) -> AppResult<FetchRun> {
    fetch_catalogue(&service, &state, &request_id, &artist_id).await
}

/// Only on user request: one editions browse and one release lookup per album.
#[tauri::command]
pub async fn artist_tracklists_load(
    service: State<'_, MetadataService>,
    state: State<'_, AppState>,
    request_id: String,
    artist_id: String,
) -> AppResult<TracklistRun> {
    load_tracklists(&service, &state, &request_id, &artist_id).await
}

#[tauri::command]
pub async fn artist_catalogue_scope_set(
    state: State<'_, AppState>,
    artist_id: String,
    scope: Scope,
) -> AppResult<Scope> {
    state.with_db(|db| db.write(|tx| discography::set_scope(tx, &artist_id, &scope)))
}

#[tauri::command]
pub async fn artist_catalogue_add_manual(
    state: State<'_, AppState>,
    artist_id: String,
    entry: ManualEntry,
) -> AppResult<String> {
    state.with_db(|db| db.write(|tx| discography::add_manual(tx, &artist_id, &entry)))
}

#[tauri::command]
pub async fn artist_catalogue_remove_manual(
    state: State<'_, AppState>,
    entry_id: String,
) -> AppResult<()> {
    state.with_db(|db| db.write(|tx| discography::remove_manual(tx, &entry_id)))
}

#[tauri::command]
pub async fn artist_catalogue_exclude(
    state: State<'_, AppState>,
    entry_id: String,
    excluded: bool,
) -> AppResult<()> {
    state.with_db(|db| db.write(|tx| discography::set_excluded(tx, &entry_id, excluded)))
}

#[cfg(test)]
mod tests;
