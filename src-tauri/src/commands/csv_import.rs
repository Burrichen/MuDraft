//! CSV import commands. Files are chosen with native dialogs and read here in Rust; the
//! renderer never receives a path or file-system access.

use tauri::{AppHandle, State};

use super::files;
use super::metadata::MetadataService;
use crate::csv_import::commit::{self, CommitReport, TagDecisions};
use crate::csv_import::enrich::{self, EnrichProgress};
use crate::csv_import::mapping::{self, ColumnMapping};
use crate::csv_import::staging::{
    self, DecisionInput, RowFilter, RowPage, SessionSummary, StagedRow,
};
use crate::csv_import::{MAX_FILE_BYTES, parse, template_csv};
use crate::domain::ids::parse_uuid;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

const CSV_FILTER: (&str, &[&str]) = ("CSV", &["csv"]);

/// Save the header-only template. Returns the saved file name, or None if cancelled.
#[tauri::command]
pub async fn csv_import_save_template(app: AppHandle) -> AppResult<Option<String>> {
    let Some(target) =
        files::pick_save(&app, "template", "mudraft-import-template.csv", CSV_FILTER).await?
    else {
        return Ok(None);
    };
    std::fs::write(target.path(), template_csv())
        .map_err(|e| AppError::validation("file", format!("couldn't save the template: {e}")))?;
    let name = target.name().to_owned();
    target.finish()?;
    Ok(Some(name))
}

/// Choose and stage a CSV file. Nothing in the library changes.
#[tauri::command]
pub async fn csv_import_open(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Option<SessionSummary>> {
    let Some(file) = files::pick_open(
        &app,
        "csv",
        "Import albums from CSV",
        CSV_FILTER,
        MAX_FILE_BYTES as u64,
        "import.csv",
    )
    .await?
    else {
        return Ok(None);
    };
    let path = file.path().to_path_buf();
    let size = std::fs::metadata(&path)
        .map_err(|e| AppError::validation("file", e.to_string()))?
        .len();
    if size as usize > MAX_FILE_BYTES {
        return Err(AppError::validation(
            "CSV file",
            format!(
                "larger than {} MB; split it into smaller files",
                MAX_FILE_BYTES / (1024 * 1024)
            ),
        ));
    }
    let bytes = std::fs::read(&path)
        .map_err(|e| AppError::validation("file", format!("couldn't read it: {e}")))?;
    let parsed = tokio::task::spawn_blocking(move || parse::parse(&bytes))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
    let name = file.name().to_owned();
    let summary = state.with_db(|db| {
        db.write(|tx| {
            let id = staging::create_session(tx, &name, &parsed)?;
            let suggested = mapping::suggest(&parsed.headers);
            if suggested.validate(parsed.headers.len()).is_ok() {
                staging::apply_mapping(tx, &id, &suggested)?;
            }
            staging::summary(tx, &id)
        })
    })?;
    Ok(Some(summary))
}

#[tauri::command]
pub async fn csv_import_latest(state: State<'_, AppState>) -> AppResult<Option<SessionSummary>> {
    state.with_db(|db| {
        db.read(|c| {
            staging::latest_open(c)?
                .map(|id| staging::summary(c, &id))
                .transpose()
        })
    })
}

#[tauri::command]
pub async fn csv_import_summary(
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<SessionSummary> {
    let id = parse_uuid("import", &session_id)?;
    state.with_db(|db| db.read(|c| staging::summary(c, &id)))
}

#[tauri::command]
pub async fn csv_import_apply_mapping(
    state: State<'_, AppState>,
    session_id: String,
    mapping: ColumnMapping,
) -> AppResult<SessionSummary> {
    let id = parse_uuid("import", &session_id)?;
    state.with_db(|db| {
        db.write(|tx| {
            staging::apply_mapping(tx, &id, &mapping)?;
            staging::summary(tx, &id)
        })
    })
}

#[tauri::command]
pub async fn csv_import_rows(
    state: State<'_, AppState>,
    session_id: String,
    offset: u32,
    limit: u32,
    filter: RowFilter,
) -> AppResult<RowPage> {
    let id = parse_uuid("import", &session_id)?;
    state.with_db(|db| db.read(|c| staging::rows(c, &id, offset, limit, filter)))
}

/// Look up the next few rows online (bounded batch; call again to continue).
#[tauri::command]
pub async fn csv_import_enrich(
    service: State<'_, MetadataService>,
    state: State<'_, AppState>,
    session_id: String,
    request_id: String,
    max_rows: u32,
) -> AppResult<EnrichProgress> {
    let id = parse_uuid("import", &session_id)?;
    let guard = service.requests().begin(&request_id)?;
    enrich::enrich(service.provider(), &state, &id, max_rows, &guard.token).await
}

/// Change a row's decision. A match fetches its details now so commit can run offline.
#[tauri::command]
pub async fn csv_import_decide(
    service: State<'_, MetadataService>,
    state: State<'_, AppState>,
    session_id: String,
    request_id: String,
    row_number: u32,
    decision: DecisionInput,
) -> AppResult<StagedRow> {
    let id = parse_uuid("import", &session_id)?;
    let details = match &decision {
        DecisionInput::Match {
            release_group_id,
            release_id,
            ..
        } => {
            let rg = parse_uuid("release group", release_group_id)?;
            let rel = parse_uuid("release", release_id)?;
            let guard = service.requests().begin(&request_id)?;
            Some(enrich::fetch_match(service.provider(), &rg, &rel, &guard.token).await?)
        }
        _ => None,
    };
    state.with_db(|db| {
        db.write(|tx| staging::set_decision(tx, &id, row_number, &decision, details.as_ref()))
    })
}

#[tauri::command]
pub async fn csv_import_commit(
    state: State<'_, AppState>,
    session_id: String,
    tags: TagDecisions,
) -> AppResult<CommitReport> {
    let id = parse_uuid("import", &session_id)?;
    state.with_db(|db| db.write(|tx| commit::commit(tx, &id, &tags)))
}

/// Cancel an import before commit: deletes the staged rows only.
#[tauri::command]
pub async fn csv_import_discard(state: State<'_, AppState>, session_id: String) -> AppResult<()> {
    let id = parse_uuid("import", &session_id)?;
    state.with_db(|db| db.write(|tx| staging::discard(tx, &id)))
}
