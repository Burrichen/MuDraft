//! Next Up rolls, pool resets, manual picks, and state, per slot. The current year always
//! comes from the local calendar and randomness from the OS; tests inject both at the
//! library layer.

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

/// Removes a slot without logging a listen: its pick and picking method go; nothing
/// replaces them.
#[tauri::command]
pub async fn next_up_clear(state: State<'_, AppState>, slot: i64) -> AppResult<bool> {
    state.with_db(|db| db.write(|tx| next_up::clear(tx, slot)))
}

/// `request_id` makes a double-clicked reroll roll once.
#[tauri::command]
pub async fn next_up_roll(
    state: State<'_, AppState>,
    request_id: String,
    slot: i64,
    method: Method,
) -> AppResult<RollOutcome> {
    state.with_db(|db| {
        db.write(|tx| {
            next_up::roll(
                tx,
                Some(&request_id),
                slot,
                method,
                local_year(),
                &mut OsRandom,
            )
        })
    })
}

/// Returns the slot's new pool cycle.
#[tauri::command]
pub async fn next_up_reset_pool(state: State<'_, AppState>, slot: i64) -> AppResult<i64> {
    state.with_db(|db| db.write(|tx| next_up::reset_pool(tx, slot)))
}

/// Without `slot`, the pick goes into the lowest free slot.
#[tauri::command]
pub async fn next_up_choose(
    state: State<'_, AppState>,
    request_id: String,
    edition_id: String,
    slot: Option<i64>,
) -> AppResult<Shown> {
    state.with_db(|db| db.write(|tx| next_up::choose(tx, Some(&request_id), &edition_id, slot)))
}
