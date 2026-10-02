//! Next Up selection history: sessions, the attempts shown in them, and the single
//! current selection. Choosing *which* album to show is `domain::picker` (driven by
//! `library::next_up`); this module records outcomes and enforces the no-repeat-per-pool
//! rule. A session's pool cycle advances only through an explicit `reset_pool`.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use super::{Table, album_of_edition, require};
use crate::domain::ids::new_id;
use crate::domain::picker::GuidedCriteria;
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionMode {
    CompletelyRandom,
    WeightedRandom,
    Guided,
    /// A single hand-picked album; never the open reroll session.
    Manual,
}

/// How an attempt's album was chosen: by the session's mode, or picked manually.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionSource {
    CompletelyRandom,
    WeightedRandom,
    Guided,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptStatus {
    Shown,
    Skipped,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentSelection {
    pub attempt_id: String,
    pub session_id: String,
    pub album_id: String,
    pub edition_id: String,
    pub source: SelectionSource,
    pub status: AttemptStatus,
}

pub(crate) fn enum_str<T: Serialize>(value: T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        _ => unreachable!("unit enums serialize as strings"),
    }
}

pub(crate) fn enum_from<T: serde::de::DeserializeOwned>(what: &str, raw: String) -> AppResult<T> {
    serde_json::from_value(serde_json::Value::String(raw.clone()))
        .map_err(|_| AppError::StorageCorrupt(format!("unknown {what} '{raw}'")))
}

/// Start a session. Reroll sessions close any other open one (changing method or criteria
/// begins a new pool); manual sessions leave the open reroll session untouched.
/// `criteria` must already be validated (`next_up::validate`); required only for guided.
pub fn start_session(
    tx: &Transaction<'_>,
    mode: SelectionMode,
    criteria: Option<&GuidedCriteria>,
) -> AppResult<String> {
    let criteria = match (mode, criteria) {
        (SelectionMode::Guided, Some(c)) => Some(
            serde_json::to_string(c)
                .map_err(|e| AppError::Internal(format!("cannot encode criteria: {e}")))?,
        ),
        (SelectionMode::Guided, None) => {
            return Err(AppError::validation(
                "guided criteria",
                "required for guided sessions",
            ));
        }
        _ => None,
    };
    if mode != SelectionMode::Manual {
        end_open_session(tx)?;
    }
    let id = new_id();
    tx.execute(
        "INSERT INTO selection_session (id, mode, criteria, ended_at)
         VALUES (?1, ?2, ?3, CASE WHEN ?2 = 'manual' THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now') END)",
        params![id, enum_str(mode), criteria],
    )?;
    Ok(id)
}

pub fn end_open_session(tx: &Transaction<'_>) -> AppResult<()> {
    tx.execute(
        "UPDATE selection_session SET ended_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE ended_at IS NULL",
        [],
    )?;
    Ok(())
}

/// Begin the session's next pool cycle so every eligible album may be shown again.
/// A no-op while the current cycle has shown nothing, so repeated clicks reset once.
pub fn reset_pool(tx: &Transaction<'_>, session_id: &str) -> AppResult<i64> {
    require(tx, Table::SelectionSession, session_id)?;
    tx.execute(
        "UPDATE selection_session SET pool_cycle = pool_cycle + 1
         WHERE id = ?1 AND EXISTS (
             SELECT 1 FROM selection_attempt a
             WHERE a.session_id = ?1 AND a.pool_cycle = selection_session.pool_cycle)",
        params![session_id],
    )?;
    Ok(tx.query_row(
        "SELECT pool_cycle FROM selection_session WHERE id = ?1",
        params![session_id],
        |r| r.get(0),
    )?)
}

/// Record that an edition was shown and make it the current selection. The previous
/// current pick, if still shown, becomes skipped (a reroll). Within the session's pool
/// cycle an album never repeats.
pub fn record_attempt(
    tx: &Transaction<'_>,
    session_id: &str,
    edition_id: &str,
    source: SelectionSource,
) -> AppResult<String> {
    require(tx, Table::SelectionSession, session_id)?;
    let album_id = album_of_edition(tx, edition_id)?;
    let (last_seq, cycle): (i64, i64) = tx.query_row(
        "SELECT (SELECT COALESCE(MAX(sequence), 0) FROM selection_attempt WHERE session_id = s.id),
                s.pool_cycle
         FROM selection_session s WHERE s.id = ?1",
        params![session_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let repeated = tx
        .query_row(
            "SELECT 1 FROM selection_attempt WHERE session_id = ?1 AND pool_cycle = ?2 AND album_id = ?3",
            params![session_id, cycle, album_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if repeated {
        return Err(AppError::Conflict(format!(
            "album {album_id} was already shown in this session's current pool"
        )));
    }

    tx.execute(
        "UPDATE selection_attempt SET status = 'skipped', resolved_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE status = 'shown'
           AND (session_id = ?1 OR id IN (SELECT attempt_id FROM current_selection))",
        params![session_id],
    )?;
    let attempt_id = new_id();
    tx.execute(
        "INSERT INTO selection_attempt (id, session_id, sequence, pool_cycle, album_id, edition_id, source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![attempt_id, session_id, last_seq + 1, cycle, album_id, edition_id, enum_str(source)],
    )?;
    tx.execute(
        "INSERT INTO current_selection (slot, attempt_id) VALUES (1, ?1)
         ON CONFLICT (slot) DO UPDATE SET attempt_id = excluded.attempt_id,
             set_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        params![attempt_id],
    )?;
    Ok(attempt_id)
}

/// Mark an attempt skipped or completed. Repeating the same resolution is a no-op;
/// changing a resolved attempt is a conflict. The current selection is kept.
pub fn resolve_attempt(
    tx: &Transaction<'_>,
    attempt_id: &str,
    status: AttemptStatus,
) -> AppResult<()> {
    if status == AttemptStatus::Shown {
        return Err(AppError::validation(
            "attempt status",
            "resolve to skipped or completed",
        ));
    }
    require(tx, Table::SelectionAttempt, attempt_id)?;
    let current: String = tx.query_row(
        "SELECT status FROM selection_attempt WHERE id = ?1",
        params![attempt_id],
        |r| r.get(0),
    )?;
    let current: AttemptStatus = enum_from("attempt status", current)?;
    match current {
        AttemptStatus::Shown => {
            tx.execute(
                "UPDATE selection_attempt SET status = ?2, resolved_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE id = ?1",
                params![attempt_id, enum_str(status)],
            )?;
            Ok(())
        }
        s if s == status => Ok(()),
        s => Err(AppError::Conflict(format!(
            "attempt {attempt_id} is already {}",
            enum_str(s)
        ))),
    }
}

pub fn clear_current_selection(tx: &Transaction<'_>) -> AppResult<()> {
    tx.execute("DELETE FROM current_selection", [])?;
    Ok(())
}

pub fn current_selection(conn: &Connection) -> AppResult<Option<CurrentSelection>> {
    let row: Option<(String, String, String, String, String, String)> = conn
        .query_row(
            "SELECT a.id, a.session_id, a.album_id, a.edition_id, a.source, a.status
             FROM current_selection c JOIN selection_attempt a ON a.id = c.attempt_id",
            [],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()?;
    row.map(
        |(attempt_id, session_id, album_id, edition_id, source, status)| {
            Ok(CurrentSelection {
                attempt_id,
                session_id,
                album_id,
                edition_id,
                source: enum_from("selection source", source)?,
                status: enum_from("attempt status", status)?,
            })
        },
    )
    .transpose()
}
