//! Listen logging. A listen is evidence: when (a real local date, or unknown), which
//! edition, and exactly which known tracks it covered at that moment.
//!
//! - Logging is idempotent per request key, so a double click records one listen.
//! - A full-album listen snapshots the edition's tracks *as they are now*: tracks added
//!   later (e.g. Deluxe bonus tracks from a refresh) are never retroactively listened.
//!   If the edition had no tracklist, coverage is "unknown" until the user confirms it.
//! - Re-listening adds an event, never another album entry. Ratings are untouched.
//! - Deleting is a soft delete with immediate undo; corrections return the previous
//!   values so they can be undone too. Nothing else is erased.
//! - Dates are never invented: unknown stays unknown.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use super::{album_of_edition, idempotency, settings};
use crate::domain::dates::CalendarDate;
use crate::domain::ids::{new_id, parse_uuid};
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListenKind {
    First,
    Relisten,
    Unspecified,
}

impl ListenKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::First => "first",
            Self::Relisten => "relisten",
            Self::Unspecified => "unspecified",
        }
    }

    fn parse(raw: &str) -> AppResult<Self> {
        match raw {
            "first" => Ok(Self::First),
            "relisten" => Ok(Self::Relisten),
            "unspecified" => Ok(Self::Unspecified),
            other => Err(AppError::StorageCorrupt(format!(
                "unknown listen kind '{other}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LogListen {
    pub edition_id: String,
    /// Local calendar date `YYYY-MM-DD`; `None` means the date is unknown.
    pub listened_on: Option<String>,
    pub kind: ListenKind,
    /// The user also listened before, at unknown times (recorded once per edition).
    pub earlier_undated: bool,
    /// `None` = the whole album; otherwise exactly these tracks of the edition.
    pub track_ids: Option<Vec<String>>,
    /// The Next Up pick this listen completes, if any.
    pub attempt_id: Option<String>,
}

/// Everything needed to undo a log exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UndoLog {
    pub listen_id: String,
    pub prior_history_id: Option<String>,
    /// Listen List entry removed by this log (album, edition), to put back.
    pub restore_listen_list: Option<(String, String)>,
    /// The Collection entry this log created (edition), to remove again.
    pub created_collection_for: Option<String>,
    pub completed_attempt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Logged {
    pub listen_id: String,
    pub is_full: bool,
    /// "tracks" or "unknown".
    pub coverage: String,
    pub covered_tracks: u32,
    pub added_to_collection: bool,
    pub removed_from_listen_list: bool,
    pub completed_next_up: bool,
    pub undo: UndoLog,
}

fn edition_tracks(conn: &Connection, edition_id: &str) -> AppResult<Vec<String>> {
    let mut stmt =
        conn.prepare("SELECT id FROM track WHERE edition_id = ?1 ORDER BY disc_number, position")?;
    let ids = stmt
        .query_map(params![edition_id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(ids)
}

fn parse_date(raw: Option<&str>) -> AppResult<Option<String>> {
    raw.map(str::trim)
        .filter(|d| !d.is_empty())
        .map(|d| CalendarDate::parse("listening date", d).map(|c| c.to_string()))
        .transpose()
}

pub fn log(tx: &Transaction<'_>, key: Option<&str>, input: &LogListen) -> AppResult<Logged> {
    idempotency::once(tx, key, "log_listen", input, |tx| {
        let edition_id = parse_uuid("edition", &input.edition_id)?;
        let album_id = album_of_edition(tx, &edition_id)?;
        let listened_on = parse_date(input.listened_on.as_deref())?;
        let all_tracks = edition_tracks(tx, &edition_id)?;

        let (is_full, covered, coverage) = match &input.track_ids {
            None => {
                let coverage = if all_tracks.is_empty() {
                    "unknown"
                } else {
                    "tracks"
                };
                (true, all_tracks.clone(), coverage)
            }
            Some(ids) => {
                let mut picked = Vec::new();
                for raw in ids {
                    let id = parse_uuid("track", raw)?;
                    if !all_tracks.contains(&id) {
                        return Err(AppError::validation(
                            "tracks",
                            format!("track {id} isn’t on this edition"),
                        ));
                    }
                    if !picked.contains(&id) {
                        picked.push(id);
                    }
                }
                if picked.is_empty() {
                    return Err(AppError::validation("tracks", "choose at least one track"));
                }
                (false, picked, "tracks")
            }
        };

        let attempt = match &input.attempt_id {
            Some(raw) => {
                let id = parse_uuid("Next Up pick", raw)?;
                let attempt_album: String = tx
                    .query_row(
                        "SELECT album_id FROM selection_attempt WHERE id = ?1",
                        params![id],
                        |r| r.get(0),
                    )
                    .optional()?
                    .ok_or_else(|| AppError::not_found("Next Up pick", &id))?;
                if attempt_album != album_id {
                    return Err(AppError::validation(
                        "Next Up pick",
                        "that pick was for a different album",
                    ));
                }
                Some(id)
            }
            None => None,
        };

        let listen_id = new_id();
        let (date_value, date_status) = match &listened_on {
            Some(d) => (Some(d.clone()), "known"),
            None => (None, "undated"),
        };
        tx.execute(
            "INSERT INTO listen_event (id, edition_id, album_id, listened_on, date_status, is_full, kind, coverage, selection_attempt_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![listen_id, edition_id, album_id, date_value, date_status, is_full, input.kind.as_str(), coverage, attempt],
        )?;
        for track in &covered {
            tx.execute(
                "INSERT INTO listen_event_track (listen_event_id, edition_id, track_id) VALUES (?1, ?2, ?3)",
                params![listen_id, edition_id, track],
            )?;
        }

        let prior_history_id = if input.earlier_undated {
            let exists: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM listen_event WHERE edition_id = ?1 AND kind = 'prior_history' AND deleted_at IS NULL)",
                params![edition_id],
                |r| r.get(0),
            )?;
            if exists {
                None
            } else {
                let id = new_id();
                tx.execute(
                    "INSERT INTO listen_event (id, edition_id, album_id, listened_on, date_status, is_full, kind, coverage)
                     VALUES (?1, ?2, ?3, NULL, 'undated', 1, 'prior_history', 'unknown')",
                    params![id, edition_id, album_id],
                )?;
                Some(id)
            }
        } else {
            None
        };

        let added_to_collection = tx.execute(
            "INSERT INTO collection_entry (edition_id, album_id, source) VALUES (?1, ?2, 'listen') ON CONFLICT DO NOTHING",
            params![edition_id, album_id],
        )? > 0;

        let mut restore_listen_list = None;
        if is_full && settings::load(tx)?.full_listen_removes_from_listen_list {
            let listed: Option<String> = tx
                .query_row(
                    "SELECT edition_id FROM listen_list_entry WHERE album_id = ?1",
                    params![album_id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(listed_edition) = listed {
                tx.execute(
                    "DELETE FROM listen_list_entry WHERE album_id = ?1",
                    params![album_id],
                )?;
                restore_listen_list = Some((album_id.clone(), listed_edition));
            }
        }

        let mut completed_attempt = None;
        if let Some(id) = &attempt {
            tx.execute(
                "UPDATE selection_attempt SET status = 'completed',
                     resolved_at = COALESCE(resolved_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                 WHERE id = ?1",
                params![id],
            )?;
            // Done with this pick: clear it, and do not choose another automatically.
            tx.execute(
                "DELETE FROM current_selection WHERE attempt_id = ?1",
                params![id],
            )?;
            completed_attempt = Some(id.clone());
        }

        Ok(Logged {
            is_full,
            coverage: coverage.into(),
            covered_tracks: covered.len() as u32,
            added_to_collection,
            removed_from_listen_list: restore_listen_list.is_some(),
            completed_next_up: completed_attempt.is_some(),
            undo: UndoLog {
                listen_id: listen_id.clone(),
                prior_history_id,
                restore_listen_list,
                created_collection_for: added_to_collection.then(|| edition_id.clone()),
                completed_attempt,
            },
            listen_id,
        })
    })
}

fn soft_delete(tx: &Transaction<'_>, id: &str) -> AppResult<()> {
    tx.execute(
        "UPDATE listen_event SET deleted_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1 AND deleted_at IS NULL",
        params![id],
    )?;
    Ok(())
}

/// Reverse a log: the listen (and any prior-history note it added) is withdrawn, the
/// Listen List entry comes back, a Collection entry created only by this log goes away,
/// and a completed Next Up pick becomes current again if nothing replaced it.
pub fn undo_log(tx: &Transaction<'_>, undo: &UndoLog) -> AppResult<()> {
    let listen_id = parse_uuid("listen", &undo.listen_id)?;
    soft_delete(tx, &listen_id)?;
    if let Some(id) = &undo.prior_history_id {
        soft_delete(tx, &parse_uuid("listen", id)?)?;
    }
    if let Some((album, edition)) = &undo.restore_listen_list {
        tx.execute(
            "INSERT INTO listen_list_entry (album_id, edition_id) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
            params![parse_uuid("album", album)?, parse_uuid("edition", edition)?],
        )?;
    }
    if let Some(edition) = &undo.created_collection_for {
        let edition = parse_uuid("edition", edition)?;
        let other_listens: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM listen_event WHERE edition_id = ?1 AND deleted_at IS NULL)",
            params![edition],
            |r| r.get(0),
        )?;
        if !other_listens {
            tx.execute(
                "DELETE FROM collection_entry WHERE edition_id = ?1 AND source = 'listen'",
                params![edition],
            )?;
        }
    }
    if let Some(attempt) = &undo.completed_attempt {
        let attempt = parse_uuid("Next Up pick", attempt)?;
        tx.execute(
            "UPDATE selection_attempt SET status = 'shown', resolved_at = NULL WHERE id = ?1 AND status = 'completed'",
            params![attempt],
        )?;
        tx.execute(
            "INSERT INTO current_selection (slot, attempt_id) VALUES (1, ?1) ON CONFLICT (slot) DO NOTHING",
            params![attempt],
        )?;
    }
    Ok(())
}

/// Editable fields of a listen; also returned as the "before" state for undo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListenFields {
    /// `None` = date unknown.
    pub listened_on: Option<String>,
    pub kind: ListenKind,
}

/// Correct a listen's date or kind. Returns the previous values for undo.
pub fn correct(
    tx: &Transaction<'_>,
    listen_id: &str,
    fields: &ListenFields,
) -> AppResult<ListenFields> {
    let listen_id = parse_uuid("listen", listen_id)?;
    let (date, kind): (Option<String>, String) = tx
        .query_row(
            "SELECT listened_on, kind FROM listen_event WHERE id = ?1 AND deleted_at IS NULL AND kind != 'prior_history'",
            params![listen_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("listen", &listen_id))?;
    let previous = ListenFields {
        listened_on: date,
        kind: ListenKind::parse(&kind)?,
    };
    let new_date = parse_date(fields.listened_on.as_deref())?;
    tx.execute(
        "UPDATE listen_event SET listened_on = ?2, date_status = ?3, kind = ?4,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
        params![
            listen_id,
            new_date,
            if new_date.is_some() {
                "known"
            } else {
                "undated"
            },
            fields.kind.as_str()
        ],
    )?;
    Ok(previous)
}

/// Delete a listen (soft; undo with [`restore`]). Collection membership, ratings, and
/// other listens are untouched.
pub fn delete(tx: &Transaction<'_>, listen_id: &str) -> AppResult<()> {
    let listen_id = parse_uuid("listen", listen_id)?;
    let changed = tx.execute(
        "UPDATE listen_event SET deleted_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1 AND deleted_at IS NULL",
        params![listen_id],
    )?;
    if changed == 0 {
        return Err(AppError::not_found("listen", &listen_id));
    }
    Ok(())
}

pub fn restore(tx: &Transaction<'_>, listen_id: &str) -> AppResult<()> {
    let listen_id = parse_uuid("listen", listen_id)?;
    tx.execute(
        "UPDATE listen_event SET deleted_at = NULL WHERE id = ?1",
        params![listen_id],
    )?;
    Ok(())
}

/// For a listen logged before the tracklist existed: mark the edition's current tracks
/// as covered. Only ever runs when the user explicitly confirms.
pub fn confirm_coverage(tx: &Transaction<'_>, listen_id: &str) -> AppResult<u32> {
    let listen_id = parse_uuid("listen", listen_id)?;
    let (edition_id, coverage): (String, String) = tx
        .query_row(
            "SELECT edition_id, coverage FROM listen_event WHERE id = ?1 AND deleted_at IS NULL AND kind != 'prior_history'",
            params![listen_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("listen", &listen_id))?;
    if coverage != "unknown" {
        return Err(AppError::validation(
            "listen",
            "this listen already records which tracks it covered",
        ));
    }
    let tracks = edition_tracks(tx, &edition_id)?;
    if tracks.is_empty() {
        return Err(AppError::validation(
            "listen",
            "this edition still has no tracklist",
        ));
    }
    for t in &tracks {
        tx.execute(
            "INSERT INTO listen_event_track (listen_event_id, edition_id, track_id) VALUES (?1, ?2, ?3) ON CONFLICT DO NOTHING",
            params![listen_id, edition_id, t],
        )?;
    }
    tx.execute(
        "UPDATE listen_event SET coverage = 'tracks' WHERE id = ?1",
        params![listen_id],
    )?;
    Ok(tracks.len() as u32)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListenView {
    pub id: String,
    pub edition_id: String,
    pub edition_name: String,
    pub listened_on: Option<String>,
    pub kind: ListenKind,
    pub is_full: bool,
    pub coverage: String,
    pub covered_tracks: u32,
    pub edition_tracks: u32,
    pub from_next_up: bool,
}

/// Album-level listening facts shared by album pages and Collection cards.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListeningSummary {
    /// Latest known listening date.
    pub last_listened: Option<String>,
    /// Logged listens (dated or not); excludes the "listened before" note.
    pub listen_count: u32,
    pub undated_listens: u32,
    /// The user said they listened before, at unknown times.
    pub earlier_undated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumListening {
    pub summary: ListeningSummary,
    /// Newest first; undated listens after dated ones.
    pub listens: Vec<ListenView>,
}

pub fn summary(conn: &Connection, album_id: &str) -> AppResult<ListeningSummary> {
    Ok(conn.query_row(
        "SELECT MAX(listened_on),
                COUNT(*) FILTER (WHERE kind != 'prior_history'),
                COUNT(*) FILTER (WHERE kind != 'prior_history' AND listened_on IS NULL),
                COUNT(*) FILTER (WHERE kind = 'prior_history') > 0
         FROM listen_event WHERE album_id = ?1 AND deleted_at IS NULL",
        params![album_id],
        |r| {
            Ok(ListeningSummary {
                last_listened: r.get(0)?,
                listen_count: r.get(1)?,
                undated_listens: r.get(2)?,
                earlier_undated: r.get(3)?,
            })
        },
    )?)
}

pub fn history(conn: &Connection, album_id: &str) -> AppResult<AlbumListening> {
    let album_id = parse_uuid("album", album_id)?;
    let mut stmt = conn.prepare(
        "SELECT l.id, l.edition_id, e.name, l.listened_on, l.kind, l.is_full, l.coverage,
                (SELECT COUNT(*) FROM listen_event_track t WHERE t.listen_event_id = l.id),
                (SELECT COUNT(*) FROM track t WHERE t.edition_id = l.edition_id),
                l.selection_attempt_id IS NOT NULL
         FROM listen_event l JOIN edition e ON e.id = l.edition_id
         WHERE l.album_id = ?1 AND l.deleted_at IS NULL AND l.kind != 'prior_history'
         ORDER BY l.listened_on IS NULL, l.listened_on DESC, l.created_at DESC",
    )?;
    let listens = stmt
        .query_map(params![album_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, bool>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, u32>(7)?,
                r.get::<_, u32>(8)?,
                r.get::<_, bool>(9)?,
            ))
        })?
        .map(|row| {
            let (
                id,
                edition_id,
                edition_name,
                listened_on,
                kind,
                is_full,
                coverage,
                covered_tracks,
                edition_tracks,
                from_next_up,
            ) = row?;
            Ok(ListenView {
                id,
                edition_id,
                edition_name,
                listened_on,
                kind: ListenKind::parse(&kind)?,
                is_full,
                coverage,
                covered_tracks,
                edition_tracks,
                from_next_up,
            })
        })
        .collect::<AppResult<Vec<_>>>()?;
    Ok(AlbumListening {
        summary: summary(conn, &album_id)?,
        listens,
    })
}
