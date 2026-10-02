//! Narrow IPC surface. Each command validates its own inputs and returns `AppError` on failure.

pub mod album;
pub mod csv_import;
pub mod discography;
pub mod library;
pub mod listening;
pub mod metadata;
pub mod next_up;
pub mod stats;

use serde::Serialize;
use tauri::State;

use crate::db;
use crate::error::AppResult;
use crate::library::preferences::{self, UiPreferences, UiPreferencesPatch};
use crate::paths::StorageProfile;
use crate::state::AppState;

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HealthReport {
    pub app_version: &'static str,
    pub profile: StorageProfile,
    pub data_dir: String,
    pub schema_version: i64,
    pub sqlite_version: &'static str,
}

pub fn health_report(state: &AppState) -> AppResult<HealthReport> {
    let schema_version = state.with_db(|db| {
        db.quick_check()?;
        db.schema_version()
    })?;
    Ok(HealthReport {
        app_version: env!("CARGO_PKG_VERSION"),
        profile: state.profile,
        data_dir: state.data_dir.display().to_string(),
        schema_version,
        sqlite_version: db::sqlite_version(),
    })
}

#[tauri::command]
pub async fn health_check(state: State<'_, AppState>) -> AppResult<HealthReport> {
    let result = health_report(&state);
    trace("health_check", &result);
    result
}

#[tauri::command]
pub async fn get_ui_preferences(state: State<'_, AppState>) -> AppResult<UiPreferences> {
    let result = state.with_db(|db| db.read(preferences::load));
    trace("get_ui_preferences", &result);
    result
}

/// Validated partial update; returns the full stored preferences.
#[tauri::command]
pub async fn update_ui_preferences(
    state: State<'_, AppState>,
    patch: UiPreferencesPatch,
) -> AppResult<UiPreferences> {
    let result = state.with_db(|db| db.write(|tx| preferences::update(tx, &patch)));
    trace("update_ui_preferences", &result);
    result
}

/// Debug-only trace so a desktop launch can confirm the renderer reached Rust.
fn trace<T: std::fmt::Debug>(command: &str, result: &AppResult<T>) {
    #[cfg(debug_assertions)]
    match result {
        Ok(v) => eprintln!("[mudraft] {command} ok {v:?}"),
        Err(e) => eprintln!("[mudraft] {command} error [{}]", e.code()),
    }
    #[cfg(not(debug_assertions))]
    let _ = (command, result);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppError;

    #[test]
    fn reports_healthy_storage() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::open(StorageProfile::Development, dir.path().join("data"));
        let report = health_report(&state).unwrap();
        assert_eq!(report.schema_version, db::SCHEMA_VERSION);
        assert_eq!(report.profile, StorageProfile::Development);
        assert!(dir.path().join("data").join("mudraft.sqlite3").exists());
        let json = serde_json::to_value(&report).unwrap();
        assert!(json.get("sqliteVersion").is_some());
    }

    #[test]
    fn reports_startup_failure_on_every_call() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::failed(
            StorageProfile::Development,
            dir.path().to_path_buf(),
            AppError::StorageUnavailable("disk".into()),
        );
        for _ in 0..2 {
            assert_eq!(
                health_report(&state).unwrap_err().code(),
                "storage_unavailable"
            );
        }
    }
}
