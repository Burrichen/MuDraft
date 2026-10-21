//! GitHub sync commands. The token goes in once (`sync_set_token`) and never comes back
//! out: status reports only whether one is stored.

use tauri::{AppHandle, Emitter, State};

use crate::error::AppResult;
use crate::state::AppState;
use crate::sync::{Progress, RemoteCheck, SyncOutcome, SyncRequest, SyncService, SyncStatus};

/// Progress of a running sync, for the Settings page.
pub const PROGRESS_EVENT: &str = "sync-progress";

fn reporter(app: AppHandle) -> impl Fn(Progress) + Send + Sync {
    move |p| {
        let _ = app.emit(PROGRESS_EVENT, &p);
    }
}

#[tauri::command]
pub async fn sync_status(
    service: State<'_, SyncService>,
    state: State<'_, AppState>,
) -> AppResult<SyncStatus> {
    service.status(&state)
}

#[tauri::command]
pub async fn sync_configure(
    service: State<'_, SyncService>,
    state: State<'_, AppState>,
    owner: String,
    repo: String,
) -> AppResult<SyncStatus> {
    service.configure(&state, &owner, &repo).await
}

#[tauri::command]
pub async fn sync_disconnect(
    service: State<'_, SyncService>,
    state: State<'_, AppState>,
) -> AppResult<SyncStatus> {
    service.disconnect(&state).await
}

#[tauri::command]
pub async fn sync_set_token(
    service: State<'_, SyncService>,
    state: State<'_, AppState>,
    token: String,
) -> AppResult<SyncStatus> {
    service.set_token(&token)?;
    service.status(&state)
}

#[tauri::command]
pub async fn sync_remove_token(
    service: State<'_, SyncService>,
    state: State<'_, AppState>,
) -> AppResult<SyncStatus> {
    service.remove_token()?;
    service.status(&state)
}

/// Read-only look at the repository (kind, whether it's this computer's sync, counts).
#[tauri::command]
pub async fn sync_check(
    service: State<'_, SyncService>,
    state: State<'_, AppState>,
) -> AppResult<RemoteCheck> {
    service.check(&state).await
}

/// Publish this computer's library to an empty repository. `confirm` is required when the
/// library isn't empty.
#[tauri::command]
pub async fn sync_initialize(
    app: AppHandle,
    service: State<'_, SyncService>,
    state: State<'_, AppState>,
    confirm: bool,
) -> AppResult<SyncOutcome> {
    service
        .initialize_reporting(&state, confirm, &reporter(app))
        .await
}

#[tauri::command]
pub async fn sync_now(
    app: AppHandle,
    service: State<'_, SyncService>,
    state: State<'_, AppState>,
    request: SyncRequest,
) -> AppResult<SyncOutcome> {
    service
        .sync_reporting(&state, &request, &reporter(app))
        .await
}
