//! Stats overview (read-only; computed from stored evidence on every call).

use tauri::State;

use crate::error::AppResult;
use crate::library::stats::{self, Stats};
use crate::metadata::clock::utc_now;
use crate::state::AppState;

#[tauri::command]
pub async fn stats_overview(state: State<'_, AppState>) -> AppResult<Stats> {
    let now = utc_now();
    state.with_db(|db| db.read(|c| stats::overview(c, &now)))
}
