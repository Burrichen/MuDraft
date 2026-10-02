//! Listen List / Collection reads, removal, and global tag management.

use tauri::State;

use crate::error::AppResult;
use crate::library::listing::{self, ListQuery, ListResult, Source};
use crate::library::tags::{self, BulkTagResult, TagInfo};
use crate::state::AppState;

#[tauri::command]
pub async fn library_list(
    state: State<'_, AppState>,
    source: Source,
    query: ListQuery,
) -> AppResult<ListResult> {
    state.with_db(|db| db.read(|c| listing::query(c, source, &query)))
}

/// Remove albums from the Listen List only; reviews, ratings, listens and tags stay.
#[tauri::command]
pub async fn listen_list_remove(
    state: State<'_, AppState>,
    album_ids: Vec<String>,
) -> AppResult<u32> {
    state.with_db(|db| db.write(|tx| listing::remove_from_listen_list(tx, &album_ids)))
}

#[tauri::command]
pub async fn tags_list(state: State<'_, AppState>) -> AppResult<Vec<TagInfo>> {
    state.with_db(|db| db.read(tags::list))
}

#[tauri::command]
pub async fn tag_create(
    state: State<'_, AppState>,
    name: String,
    color: Option<String>,
) -> AppResult<TagInfo> {
    state.with_db(|db| db.write(|tx| tags::create(tx, &name, color.as_deref())))
}

#[tauri::command]
pub async fn tag_update(
    state: State<'_, AppState>,
    tag_id: String,
    name: Option<String>,
    color: Option<String>,
) -> AppResult<TagInfo> {
    state.with_db(|db| db.write(|tx| tags::update(tx, &tag_id, name.as_deref(), color.as_deref())))
}

/// Returns how many albums lost the tag.
#[tauri::command]
pub async fn tag_delete(state: State<'_, AppState>, tag_id: String) -> AppResult<u32> {
    state.with_db(|db| db.write(|tx| tags::delete(tx, &tag_id)))
}

#[tauri::command]
pub async fn album_tags_update(
    state: State<'_, AppState>,
    album_ids: Vec<String>,
    add: Vec<String>,
    remove: Vec<String>,
) -> AppResult<BulkTagResult> {
    state.with_db(|db| db.write(|tx| tags::set_album_tags(tx, &album_ids, &add, &remove)))
}
