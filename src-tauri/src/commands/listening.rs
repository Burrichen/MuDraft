//! Listen logging, Collection membership, and the listening setting.

use rusqlite::params;
use tauri::State;

use crate::domain::ids::parse_uuid;
use crate::error::AppResult;
use crate::library::listening::{self, ListenFields, LogListen, Logged, UndoLog};
use crate::library::settings::{self, Settings};
use crate::library::{album_of_edition, personal};
use crate::state::AppState;

/// `request_id` is the idempotency key: repeating it (a double click, a retry) records
/// the listen once.
#[tauri::command]
pub async fn listen_log(
    state: State<'_, AppState>,
    request_id: String,
    listen: LogListen,
) -> AppResult<Logged> {
    state.with_db(|db| db.write(|tx| listening::log(tx, Some(&request_id), &listen)))
}

#[tauri::command]
pub async fn listen_undo_log(state: State<'_, AppState>, undo: UndoLog) -> AppResult<()> {
    state.with_db(|db| db.write(|tx| listening::undo_log(tx, &undo)))
}

/// Returns the previous values so the correction can be undone.
#[tauri::command]
pub async fn listen_correct(
    state: State<'_, AppState>,
    listen_id: String,
    fields: ListenFields,
) -> AppResult<ListenFields> {
    state.with_db(|db| db.write(|tx| listening::correct(tx, &listen_id, &fields)))
}

#[tauri::command]
pub async fn listen_delete(state: State<'_, AppState>, listen_id: String) -> AppResult<()> {
    state.with_db(|db| db.write(|tx| listening::delete(tx, &listen_id)))
}

#[tauri::command]
pub async fn listen_restore(state: State<'_, AppState>, listen_id: String) -> AppResult<()> {
    state.with_db(|db| db.write(|tx| listening::restore(tx, &listen_id)))
}

#[tauri::command]
pub async fn listen_confirm_coverage(
    state: State<'_, AppState>,
    listen_id: String,
) -> AppResult<u32> {
    state.with_db(|db| db.write(|tx| listening::confirm_coverage(tx, &listen_id)))
}

/// Explicit Collection membership, with or without any listen.
#[tauri::command]
pub async fn collection_add(state: State<'_, AppState>, edition_id: String) -> AppResult<()> {
    state.with_db(|db| {
        db.write(|tx| {
            personal::add_to_collection(
                tx,
                &parse_uuid("edition", &edition_id)?,
                personal::CollectionSource::Manual,
            )
        })
    })
}

/// Remove an edition from the Collection. Listens, ratings, and reviews are kept.
#[tauri::command]
pub async fn collection_remove(state: State<'_, AppState>, edition_id: String) -> AppResult<bool> {
    state.with_db(|db| {
        db.write(|tx| {
            let edition_id = parse_uuid("edition", &edition_id)?;
            album_of_edition(tx, &edition_id)?;
            Ok(tx.execute(
                "DELETE FROM collection_entry WHERE edition_id = ?1",
                params![edition_id],
            )? > 0)
        })
    })
}

#[tauri::command]
pub async fn listening_settings(state: State<'_, AppState>) -> AppResult<Settings> {
    state.with_db(|db| db.read(settings::load))
}

#[tauri::command]
pub async fn listening_settings_set(
    state: State<'_, AppState>,
    settings: Settings,
) -> AppResult<()> {
    state.with_db(|db| db.write(|tx| settings::save(tx, &settings)))
}
