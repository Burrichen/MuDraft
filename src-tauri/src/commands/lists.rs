//! Lists: reading, editing, importing from a web page, and adding to the Listen List.

use serde::Serialize;
use tauri::State;

use crate::csv_import::staging::SessionSummary;
use crate::error::AppResult;
use crate::library::lists::{
    self, EntriesAdded, ListDetail, ListSummary, ListenListAdded, NewEntry,
};
use crate::list_import::{ListImport, ListImporter};
use crate::state::AppState;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListCreated {
    pub id: String,
}

#[tauri::command]
pub async fn lists_overview(state: State<'_, AppState>) -> AppResult<Vec<ListSummary>> {
    state.with_db(|db| db.read(lists::overview))
}

#[tauri::command]
pub async fn list_detail(state: State<'_, AppState>, list_id: String) -> AppResult<ListDetail> {
    state.with_db(|db| db.read(|c| lists::detail(c, &list_id)))
}

/// Create a list, with the albums of a reviewed import when `entries` is given.
#[tauri::command]
pub async fn list_create(
    state: State<'_, AppState>,
    name: String,
    source_url: Option<String>,
    entries: Vec<NewEntry>,
) -> AppResult<ListCreated> {
    let id = state
        .with_db(|db| db.write(|tx| lists::create(tx, &name, source_url.as_deref(), &entries)))?;
    Ok(ListCreated { id })
}

#[tauri::command]
pub async fn list_rename(
    state: State<'_, AppState>,
    list_id: String,
    name: String,
) -> AppResult<()> {
    state.with_db(|db| db.write(|tx| lists::rename(tx, &list_id, &name)))
}

/// Delete a list only; albums, the Collection and the Listen List are untouched.
#[tauri::command]
pub async fn list_delete(state: State<'_, AppState>, list_id: String) -> AppResult<()> {
    state.with_db(|db| db.write(|tx| lists::delete(tx, &list_id)))
}

#[tauri::command]
pub async fn list_entries_add(
    state: State<'_, AppState>,
    list_id: String,
    entries: Vec<NewEntry>,
) -> AppResult<EntriesAdded> {
    state.with_db(|db| db.write(|tx| lists::add_entries(tx, &list_id, &entries)))
}

#[tauri::command]
pub async fn list_entry_remove(state: State<'_, AppState>, entry_id: String) -> AppResult<()> {
    state.with_db(|db| db.write(|tx| lists::remove_entry(tx, &entry_id)))
}

/// Read the list on a web page for review. Nothing is saved.
#[tauri::command]
pub async fn list_import_preview(
    importer: State<'_, ListImporter>,
    url: String,
) -> AppResult<ListImport> {
    importer.preview(&url).await
}

#[tauri::command]
pub async fn list_entry_add_to_listen_list(
    state: State<'_, AppState>,
    entry_id: String,
) -> AppResult<ListenListAdded> {
    state.with_db(|db| db.write(|tx| lists::add_entry_to_listen_list(tx, &entry_id)))
}

/// Link an entry to the album the user chose for it in a reviewed MusicBrainz match.
#[tauri::command]
pub async fn list_entry_link(
    state: State<'_, AppState>,
    entry_id: String,
    album_id: String,
) -> AppResult<()> {
    state.with_db(|db| db.write(|tx| lists::link_entry(tx, &entry_id, &album_id)))
}

/// Stage the albums that aren't in the library (all of a list's, or one entry's) as an
/// import to review against MusicBrainz. Nothing is added until the user confirms it.
#[tauri::command]
pub async fn list_review_missing(
    state: State<'_, AppState>,
    list_id: String,
    entry_id: Option<String>,
) -> AppResult<SessionSummary> {
    state.with_db(|db| db.write(|tx| lists::review_session(tx, &list_id, entry_id.as_deref())))
}

/// Every entry that isn't listened to yet, in one transaction.
#[tauri::command]
pub async fn list_add_unlistened_to_listen_list(
    state: State<'_, AppState>,
    list_id: String,
) -> AppResult<ListenListAdded> {
    state.with_db(|db| db.write(|tx| lists::add_unlistened_to_listen_list(tx, &list_id)))
}
