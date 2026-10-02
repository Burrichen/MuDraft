//! Profile export and restore. Files are chosen with native dialogs here in Rust; the
//! renderer only sees file names, previews, and a fingerprint to confirm.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use super::metadata::MetadataService;
use crate::domain::ids::new_id;
use crate::error::{AppError, AppResult};
use crate::metadata::clock::utc_now;
use crate::profile::EXTENSION;
use crate::profile::export::{self, ExportOutcome};
use crate::profile::restore::{self, Preview};
use crate::state::AppState;

/// The archive chosen for restore, waiting for confirmation.
#[derive(Default)]
pub struct ProfileImports {
    pending: Mutex<Option<(PathBuf, String)>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreOutcome {
    /// Backup of the replaced profile, in the data folder's `backups/`.
    pub backup_file_name: String,
    pub preview: Preview,
}

async fn pick(app: &AppHandle, save_as: Option<String>) -> AppResult<Option<PathBuf>> {
    #[cfg(feature = "e2e")]
    if let Some(p) = crate::e2e::dialog_path(if save_as.is_some() {
        "export"
    } else {
        "restore"
    }) {
        return Ok(Some(p));
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    let dialog = app
        .dialog()
        .file()
        .add_filter("MuDraft profile", &[EXTENSION]);
    let done = move |p: Option<tauri_plugin_dialog::FilePath>| {
        let _ = tx.send(p);
    };
    match save_as {
        Some(name) => dialog.set_file_name(name).save_file(done),
        None => dialog
            .set_title("Restore a MuDraft profile")
            .pick_file(done),
    }
    rx.await
        .map_err(|_| AppError::Internal("file dialog closed unexpectedly".into()))?
        .map(|p| {
            p.into_path()
                .map_err(|e| AppError::validation("file", e.to_string()))
        })
        .transpose()
}

fn with_extension(path: PathBuf) -> PathBuf {
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION))
    {
        path
    } else {
        let mut p = path.into_os_string();
        p.push(format!(".{EXTENSION}"));
        p.into()
    }
}

fn artwork_dir(state: &AppState) -> PathBuf {
    crate::artwork::artwork_dir(state)
}

/// Export to `dest` from a consistent snapshot (the database lock is held throughout).
pub fn export_to(state: &AppState, dest: &Path, include_artwork: bool) -> AppResult<ExportOutcome> {
    let art = artwork_dir(state);
    state.with_db(|db| db.read(|c| export::export(c, &art, dest, include_artwork)))
}

/// Validate an archive by staging it somewhere disposable.
pub fn inspect(state: &AppState, archive: &Path) -> AppResult<Preview> {
    let scratch = state
        .data_dir
        .join(format!(".profile-inspect-{}", new_id()));
    let result = restore::stage(archive, &scratch);
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

/// Back up the current profile, stage the archive, and swap it in. `expected_sha256`
/// must match the previewed file, so a file changed after preview is refused.
pub fn restore_from(
    svc: Option<&MetadataService>,
    state: &AppState,
    archive: &Path,
    expected_sha256: &str,
) -> AppResult<RestoreOutcome> {
    let backups = state.data_dir.join("backups");
    std::fs::create_dir_all(&backups).map_err(|e| AppError::StorageUnavailable(e.to_string()))?;
    let stamp = utc_now().replace([':', '.'], "-");
    let backup = backups.join(format!("before-restore-{stamp}.{EXTENSION}"));
    export_to(state, &backup, true)?;

    let staging = restore::staging_dir(&state.data_dir);
    if staging.exists() {
        std::fs::remove_dir_all(&staging)
            .map_err(|e| AppError::StorageUnavailable(e.to_string()))?;
    }
    let preview = match restore::stage(archive, &staging) {
        Ok(p) if p.sha256 == expected_sha256 => p,
        Ok(_) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(AppError::Conflict(
                "the file changed after it was previewed; choose it again".into(),
            ));
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }
    };
    if let Some(s) = svc {
        s.suspend_cache();
    }
    let activated = state.activate_staged_profile();
    if let Some(s) = svc {
        s.resume_cache();
    }
    activated?;
    Ok(RestoreOutcome {
        backup_file_name: backup
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        preview,
    })
}

/// Choose where to save, then export. `None` when cancelled.
#[tauri::command]
pub async fn profile_export(
    app: AppHandle,
    state: State<'_, AppState>,
    include_artwork: bool,
) -> AppResult<Option<ExportOutcome>> {
    let default = format!("mudraft-profile-{}.{EXTENSION}", &utc_now()[..10]);
    let Some(path) = pick(&app, Some(default)).await? else {
        return Ok(None);
    };
    export_to(&state, &with_extension(path), include_artwork).map(Some)
}

/// Choose an archive and preview it (fully validated). Nothing changes yet.
#[tauri::command]
pub async fn profile_import_choose(
    app: AppHandle,
    state: State<'_, AppState>,
    imports: State<'_, ProfileImports>,
) -> AppResult<Option<Preview>> {
    let Some(path) = pick(&app, None).await? else {
        return Ok(None);
    };
    let preview = inspect(&state, &path)?;
    if let Ok(mut pending) = imports.pending.lock() {
        *pending = Some((path, preview.sha256.clone()));
    }
    Ok(Some(preview))
}

/// Replace the current profile with the previewed archive (after backing it up).
#[tauri::command]
pub async fn profile_import_confirm(
    service: State<'_, MetadataService>,
    state: State<'_, AppState>,
    imports: State<'_, ProfileImports>,
    sha256: String,
) -> AppResult<RestoreOutcome> {
    let (path, expected) = imports
        .pending
        .lock()
        .map_err(|_| AppError::Internal("restore state poisoned".into()))?
        .clone()
        .ok_or_else(|| AppError::validation("restore", "choose a profile to restore first"))?;
    if expected != sha256 {
        return Err(AppError::Conflict(
            "that isn't the profile you previewed".into(),
        ));
    }
    let outcome = restore_from(Some(&service), &state, &path, &expected)?;
    if let Ok(mut pending) = imports.pending.lock() {
        *pending = None;
    }
    Ok(outcome)
}

#[tauri::command]
pub fn profile_import_cancel(imports: State<'_, ProfileImports>) -> AppResult<()> {
    if let Ok(mut pending) = imports.pending.lock() {
        *pending = None;
    }
    Ok(())
}
