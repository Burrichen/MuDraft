//! Next Up rolls, pool resets, manual picks, and state. The current year always comes from
//! the local calendar and randomness from the OS; tests inject both at the library layer.

use tauri::State;

use crate::domain::picker::{GuidedCriteria, Method};
use crate::error::AppResult;
use crate::library::next_up::{
    self, GuidedOptions, NextUpState, OsRandom, RollOutcome, Shown, local_year,
};
use crate::state::AppState;

#[tauri::command]
pub async fn next_up_state(state: State<'_, AppState>) -> AppResult<NextUpState> {
    state.with_db(|db| db.read(|c| next_up::state(c, local_year())))
}

/// Choices with live counts for the given (default: all Agnostic) guided criteria.
#[tauri::command]
pub async fn next_up_options(
    state: State<'_, AppState>,
    criteria: Option<GuidedCriteria>,
) -> AppResult<GuidedOptions> {
    state.with_db(|db| db.read(|c| next_up::options(c, criteria.unwrap_or_default(), local_year())))
}

/// Clears the current pick without logging a listen; nothing replaces it.
#[tauri::command]
pub async fn next_up_clear(state: State<'_, AppState>) -> AppResult<bool> {
    state.with_db(|db| db.write(next_up::clear))
}

/// `request_id` makes a double-clicked reroll roll once.
#[tauri::command]
pub async fn next_up_roll(
    state: State<'_, AppState>,
    request_id: String,
    method: Method,
) -> AppResult<RollOutcome> {
    state.with_db(|db| {
        db.write(|tx| next_up::roll(tx, Some(&request_id), method, local_year(), &mut OsRandom))
    })
}

/// Returns the new pool cycle.
#[tauri::command]
pub async fn next_up_reset_pool(state: State<'_, AppState>) -> AppResult<i64> {
    state.with_db(|db| db.write(next_up::reset_pool))
}

#[tauri::command]
pub async fn next_up_choose(
    state: State<'_, AppState>,
    request_id: String,
    edition_id: String,
) -> AppResult<Shown> {
    state.with_db(|db| db.write(|tx| next_up::choose(tx, Some(&request_id), &edition_id)))
}
