//! Next Up selection history: sessions, the attempts shown in them, and the current
//! selection of each slot (up to [`MAX_SLOTS`]). Choosing *which* album to show is
//! `domain::picker` (driven by `library::next_up`); this module records outcomes and
//! enforces the no-repeat-per-pool and one-slot-per-album rules. A session's pool cycle
//! advances only through an explicit `reset_pool`.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use super::{Table, album_of_edition, require};
use crate::domain::ids::new_id;
use crate::domain::picker::GuidedCriteria;
use crate::error::{AppError, AppResult};

/// How many Next Up slots may hold a pick or a picking method at once.
pub const MAX_SLOTS: i64 = 5;

pub fn check_slot(slot: i64) -> AppResult<i64> {
    if (1..=MAX_SLOTS).contains(&slot) {
        Ok(slot)
    } else {
        Err(AppError::validation(
            "Next Up slot",
            format!("choose a slot from 1 to {MAX_SLOTS}"),
        ))
    }
}

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
    pub slot: i64,
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

/// Start a session in `slot`. Reroll sessions close the slot's open one (changing method
/// or criteria begins a new pool); manual sessions leave it untouched. Other slots are
/// never affected. `criteria` must already be validated (`next_up::validate`); required
/// only for guided.
pub fn start_session(
    tx: &Transaction<'_>,
    slot: i64,
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
    let slot = check_slot(slot)?;
    if mode != SelectionMode::Manual {
        end_open_session(tx, slot)?;
    }
    let id = new_id();
    tx.execute(
        "INSERT INTO selection_session (id, slot, mode, criteria, ended_at)
         VALUES (?1, ?2, ?3, ?4, CASE WHEN ?3 = 'manual' THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now') END)",
        params![id, slot, enum_str(mode), criteria],
    )?;
    Ok(id)
}

/// Close `slot`'s open reroll session, if any. Returns whether one was open.
pub fn end_open_session(tx: &Transaction<'_>, slot: i64) -> AppResult<bool> {
    Ok(tx.execute(
        "UPDATE selection_session SET ended_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE ended_at IS NULL AND slot = ?1",
        params![slot],
    )? > 0)
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

/// Record that an edition was shown and make it the current selection of the session's
/// slot. The slot's previous pick, if still shown, becomes skipped (a reroll). Within the
/// session's pool cycle an album never repeats, and an album already picked in another
/// slot is refused.
pub fn record_attempt(
    tx: &Transaction<'_>,
    session_id: &str,
    edition_id: &str,
    source: SelectionSource,
) -> AppResult<String> {
    require(tx, Table::SelectionSession, session_id)?;
    let album_id = album_of_edition(tx, edition_id)?;
    let (last_seq, cycle, slot): (i64, i64, i64) = tx.query_row(
        "SELECT (SELECT COALESCE(MAX(sequence), 0) FROM selection_attempt WHERE session_id = s.id),
                s.pool_cycle, s.slot
         FROM selection_session s WHERE s.id = ?1",
        params![session_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
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
    let elsewhere = tx
        .query_row(
            "SELECT c.slot FROM current_selection c JOIN selection_attempt a ON a.id = c.attempt_id
             WHERE c.slot != ?1 AND a.album_id = ?2",
            params![slot, album_id],
            |r| r.get::<_, i64>(0),
        )
        .optional()?;
    if let Some(other) = elsewhere {
        return Err(AppError::Conflict(format!(
            "album {album_id} is already the pick of Next Up slot {other}"
        )));
    }

    tx.execute(
        "UPDATE selection_attempt SET status = 'skipped', resolved_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE status = 'shown'
           AND (session_id = ?1 OR id IN (SELECT attempt_id FROM current_selection WHERE slot = ?2))",
        params![session_id, slot],
    )?;
    let attempt_id = new_id();
    tx.execute(
        "INSERT INTO selection_attempt (id, session_id, sequence, pool_cycle, album_id, edition_id, source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![attempt_id, session_id, last_seq + 1, cycle, album_id, edition_id, enum_str(source)],
    )?;
    tx.execute(
        "INSERT INTO current_selection (slot, attempt_id) VALUES (?1, ?2)
         ON CONFLICT (slot) DO UPDATE SET attempt_id = excluded.attempt_id,
             set_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        params![slot, attempt_id],
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

/// Clear `slot`'s pick without listening: a still-shown pick counts as skipped. Returns
/// whether the slot had a pick.
pub fn clear_current_selection(tx: &Transaction<'_>, slot: i64) -> AppResult<bool> {
    tx.execute(
        "UPDATE selection_attempt SET status = 'skipped',
             resolved_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE status = 'shown' AND id IN (SELECT attempt_id FROM current_selection WHERE slot = ?1)",
        params![slot],
    )?;
    Ok(tx.execute(
        "DELETE FROM current_selection WHERE slot = ?1",
        params![slot],
    )? > 0)
}

/// Every slot's current selection, in slot order.
pub fn current_selections(conn: &Connection) -> AppResult<Vec<CurrentSelection>> {
    let mut stmt = conn.prepare(
        "SELECT c.slot, a.id, a.session_id, a.album_id, a.edition_id, a.source, a.status
         FROM current_selection c JOIN selection_attempt a ON a.id = c.attempt_id
         ORDER BY c.slot",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, String>(6)?,
        ))
    })?;
    rows.map(|row| {
        let (slot, attempt_id, session_id, album_id, edition_id, source, status) = row?;
        Ok(CurrentSelection {
            slot,
            attempt_id,
            session_id,
            album_id,
            edition_id,
            source: enum_from("selection source", source)?,
            status: enum_from("attempt status", status)?,
        })
    })
    .collect()
}
