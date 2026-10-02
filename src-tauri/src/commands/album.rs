//! Album detail, edits, editions, and artwork commands.

use std::collections::HashSet;

use rusqlite::params;
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use super::metadata::MetadataService;
use crate::artwork::http::MAX_IMAGE_BYTES;
use crate::artwork::{self, ArtworkService, ArtworkState, CacheCleared, CacheSize};
use crate::domain::ids::parse_uuid;
use crate::error::{AppError, AppResult};
use crate::library::album::{self, AlbumDetail, AlbumEdit};
use crate::library::editions::{self, SwitchPreview, SwitchResult};
use crate::library::metadata_import::{self, ImportOutcome, ImportRequest};
use crate::library::settings;
use crate::metadata::{EditionCandidate, Fetched};
use crate::state::AppState;

#[tauri::command]
pub async fn album_detail(
    state: State<'_, AppState>,
    album_id: String,
    edition_id: Option<String>,
) -> AppResult<AlbumDetail> {
    state.with_db(|db| db.read(|c| album::detail(c, &album_id, edition_id.as_deref())))
}

/// Manual edits; edited fields are locked against MusicBrainz refreshes.
#[tauri::command]
pub async fn album_edit(
    state: State<'_, AppState>,
    album_id: String,
    edit: AlbumEdit,
) -> AppResult<()> {
    state.with_db(|db| db.write(|tx| album::edit(tx, &album_id, &edit)))
}

#[tauri::command]
pub async fn album_unlock(
    state: State<'_, AppState>,
    album_id: String,
    field: String,
) -> AppResult<()> {
    state.with_db(|db| db.write(|tx| album::unlock(tx, &album_id, &field)))
}

#[tauri::command]
pub async fn edition_rename(
    state: State<'_, AppState>,
    edition_id: String,
    name: String,
) -> AppResult<()> {
    state.with_db(|db| db.write(|tx| album::rename_edition(tx, &edition_id, &name)))
}

fn mb_ids(
    state: &AppState,
    album_id: &str,
    edition_id: Option<&str>,
) -> AppResult<(String, Option<String>)> {
    state.with_db(|db| {
        db.read(|c| {
            let rgid: Option<String> = c
                .query_row("SELECT musicbrainz_release_group_id FROM album WHERE id = ?1", params![album_id], |r| r.get(0))
                .map_err(|_| AppError::not_found("album", album_id))?;
            let rgid = rgid.ok_or_else(|| AppError::validation("album", "this album isn’t linked to MusicBrainz"))?;
            let release = match edition_id {
                Some(e) => c
                    .query_row(
                        "SELECT musicbrainz_release_id FROM edition WHERE id = ?1 AND album_id = ?2",
                        params![e, album_id],
                        |r| r.get(0),
                    )
                    .map_err(|_| AppError::validation("edition", "that edition belongs to a different album"))?,
                None => None,
            };
            Ok((rgid, release))
        })
    })
}

/// Re-fetch the album and the shown edition from MusicBrainz. Locked fields, ratings,
/// and listened tracks are preserved (see `metadata_import`).
#[tauri::command]
pub async fn album_refresh(
    service: State<'_, MetadataService>,
    state: State<'_, AppState>,
    request_id: String,
    album_id: String,
    edition_id: String,
) -> AppResult<ImportOutcome> {
    let album_id = parse_uuid("album", &album_id)?;
    let edition_id = parse_uuid("edition", &edition_id)?;
    let (rgid, release) = mb_ids(&state, &album_id, Some(&edition_id))?;
    let release = release.ok_or_else(|| {
        AppError::validation(
            "edition",
            "this edition isn’t linked to a MusicBrainz release",
        )
    })?;
    let guard = service.requests().begin(&request_id)?;
    let group = service
        .provider()
        .release_group(&rgid, &guard.token)
        .await?;
    let rel = service.provider().release(&release, &guard.token).await?;
    state.with_db(|db| {
        db.write(|tx| {
            metadata_import::import_release(
                tx,
                &ImportRequest {
                    release_group: &group.value,
                    release: &rel.value,
                    edition_name: None,
                    group_fetched_at: &group.fetched_at,
                    release_fetched_at: &rel.fetched_at,
                },
            )
        })
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditionChoice {
    #[serde(flatten)]
    pub edition: EditionCandidate,
    /// Name of the library edition already linked to this release, if any.
    pub in_library_as: Option<String>,
}

/// MusicBrainz releases of this album, marking those already in the library.
#[tauri::command]
pub async fn edition_candidates(
    service: State<'_, MetadataService>,
    state: State<'_, AppState>,
    request_id: String,
    album_id: String,
) -> AppResult<Fetched<Vec<EditionChoice>>> {
    let album_id = parse_uuid("album", &album_id)?;
    let (rgid, _) = mb_ids(&state, &album_id, None)?;
    let guard = service.requests().begin(&request_id)?;
    let list = service.provider().editions(&rgid, &guard.token).await?;
    let linked: Vec<(String, String)> = state.with_db(|db| {
        db.read(|c| {
            let mut stmt = c.prepare("SELECT musicbrainz_release_id, name FROM edition WHERE album_id = ?1 AND musicbrainz_release_id IS NOT NULL")?;
            Ok(stmt.query_map(params![album_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?)
        })
    })?;
    let value = list
        .value
        .editions
        .into_iter()
        .map(|e| {
            let in_library_as = linked
                .iter()
                .find(|(id, _)| *id == e.id)
                .map(|(_, n)| n.clone());
            EditionChoice {
                edition: e,
                in_library_as,
            }
        })
        .collect();
    Ok(Fetched {
        value,
        fetched_at: list.fetched_at,
        source: list.source,
    })
}

/// Deliberately add another edition (e.g. Deluxe). Ordinary pressings are refused.
#[tauri::command]
pub async fn edition_add(
    service: State<'_, MetadataService>,
    state: State<'_, AppState>,
    request_id: String,
    album_id: String,
    release_id: String,
    edition_name: String,
) -> AppResult<ImportOutcome> {
    let album_id = parse_uuid("album", &album_id)?;
    let release_id = parse_uuid("release", &release_id)?;
    let (rgid, _) = mb_ids(&state, &album_id, None)?;
    let guard = service.requests().begin(&request_id)?;
    let rel = service
        .provider()
        .release(&release_id, &guard.token)
        .await?;
    state.with_db(|db| db.read(|c| editions::check_new_edition(c, &album_id, &rel.value)))?;
    let group = service
        .provider()
        .release_group(&rgid, &guard.token)
        .await?;
    state.with_db(|db| {
        db.write(|tx| {
            editions::check_new_edition(tx, &album_id, &rel.value)?;
            metadata_import::import_release(
                tx,
                &ImportRequest {
                    release_group: &group.value,
                    release: &rel.value,
                    edition_name: Some(&edition_name),
                    group_fetched_at: &group.fetched_at,
                    release_fetched_at: &rel.fetched_at,
                },
            )
        })
    })
}

#[tauri::command]
pub async fn edition_switch_preview(
    state: State<'_, AppState>,
    album_id: String,
    edition_id: String,
) -> AppResult<SwitchPreview> {
    state.with_db(|db| db.read(|c| editions::preview_switch(c, &album_id, &edition_id)))
}

#[tauri::command]
pub async fn edition_switch(
    state: State<'_, AppState>,
    album_id: String,
    edition_id: String,
    copy_ratings: bool,
) -> AppResult<SwitchResult> {
    state.with_db(|db| db.write(|tx| editions::switch(tx, &album_id, &edition_id, copy_ratings)))
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtworkPreference {
    /// `None` until the user answers the consent prompt.
    pub allowed: Option<bool>,
    pub cache: CacheSize,
}

#[tauri::command]
pub async fn artwork_preference(state: State<'_, AppState>) -> AppResult<ArtworkPreference> {
    state.with_db(|db| {
        db.read(|c| {
            Ok(ArtworkPreference {
                allowed: settings::artwork_download(c)?,
                cache: artwork::cache_size(c)?,
            })
        })
    })
}

#[tauri::command]
pub async fn artwork_set_preference(state: State<'_, AppState>, allowed: bool) -> AppResult<()> {
    state.with_db(|db| db.write(|tx| settings::set_artwork_download(tx, allowed)))
}

#[tauri::command]
pub async fn artwork_fetch(
    service: State<'_, MetadataService>,
    art: State<'_, ArtworkService>,
    state: State<'_, AppState>,
    request_id: String,
    edition_id: String,
) -> AppResult<ArtworkState> {
    let guard = service.requests().begin(&request_id)?;
    art.fetch(&state, &edition_id, &guard.token).await
}

pub const MAX_FETCH_BATCH: usize = 24;

/// Fetch artwork for several editions in turn (e.g. the cards on screen). Stops early on
/// network problems; returns how many editions were processed.
#[tauri::command]
pub async fn artwork_fetch_many(
    service: State<'_, MetadataService>,
    art: State<'_, ArtworkService>,
    state: State<'_, AppState>,
    request_id: String,
    edition_ids: Vec<String>,
) -> AppResult<u32> {
    let guard = service.requests().begin(&request_id)?;
    let mut seen = HashSet::new();
    let mut done = 0;
    for id in edition_ids
        .iter()
        .filter(|id| seen.insert(id.as_str()))
        .take(MAX_FETCH_BATCH)
    {
        match art.fetch(&state, id, &guard.token).await {
            Ok(_) => done += 1,
            Err(AppError::NotFound { .. }) => {}
            Err(e) => return if done > 0 { Ok(done) } else { Err(e) },
        }
    }
    Ok(done)
}

/// Pick a local image with the native dialog and use it for this edition.
#[tauri::command]
pub async fn artwork_replace(
    app: AppHandle,
    state: State<'_, AppState>,
    edition_id: String,
) -> AppResult<Option<ArtworkState>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Choose album artwork")
        .add_filter("Images", &["jpg", "jpeg", "png", "gif", "webp"])
        .pick_file(move |p| {
            let _ = tx.send(p);
        });
    let Some(picked) = rx
        .await
        .map_err(|_| AppError::Internal("file dialog closed unexpectedly".into()))?
    else {
        return Ok(None);
    };
    let path = picked
        .into_path()
        .map_err(|e| AppError::validation("file", e.to_string()))?;
    let size = std::fs::metadata(&path)
        .map_err(|e| AppError::validation("file", e.to_string()))?
        .len();
    if size as usize > MAX_IMAGE_BYTES {
        return Err(AppError::validation(
            "image",
            format!("larger than {} MB", MAX_IMAGE_BYTES / (1024 * 1024)),
        ));
    }
    let bytes = std::fs::read(&path).map_err(|e| AppError::validation("file", e.to_string()))?;
    artwork::replace_local(&state, &edition_id, &bytes).map(Some)
}

#[tauri::command]
pub async fn artwork_remove(
    state: State<'_, AppState>,
    edition_id: String,
) -> AppResult<ArtworkState> {
    artwork::remove(&state, &edition_id)
}

#[tauri::command]
pub async fn artwork_restore(
    state: State<'_, AppState>,
    edition_id: String,
) -> AppResult<ArtworkState> {
    artwork::restore(&state, &edition_id)
}

#[tauri::command]
pub async fn artwork_clear_cache(state: State<'_, AppState>) -> AppResult<CacheCleared> {
    artwork::clear_cache(&state)
}

/// Ratings are sent as half-stars (0–10, zero included); `None` clears.
#[tauri::command]
pub async fn rating_set_track(
    state: State<'_, AppState>,
    track_id: String,
    rating: Option<i64>,
) -> AppResult<crate::domain::rating::RatingSummary> {
    state.with_db(|db| db.write(|tx| crate::library::ratings::set_track(tx, &track_id, rating)))
}

#[tauri::command]
pub async fn rating_set_album(
    state: State<'_, AppState>,
    edition_id: String,
    rating: Option<i64>,
) -> AppResult<crate::domain::rating::RatingSummary> {
    state.with_db(|db| db.write(|tx| crate::library::ratings::set_album(tx, &edition_id, rating)))
}

#[tauri::command]
pub async fn track_set_favourite(
    state: State<'_, AppState>,
    track_id: String,
    favourite: bool,
) -> AppResult<()> {
    state.with_db(|db| {
        db.write(|tx| crate::library::ratings::set_favourite(tx, &track_id, favourite))
    })
}

#[tauri::command]
pub async fn album_review_set(
    state: State<'_, AppState>,
    edition_id: String,
    review: Option<String>,
) -> AppResult<()> {
    state.with_db(|db| {
        db.write(|tx| crate::library::ratings::set_review(tx, &edition_id, review.as_deref()))
    })
}
