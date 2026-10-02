//! Persistence contracts for the music library. Every write takes a `Transaction` so
//! multi-record operations are atomic; every query is parameterized; inputs are validated
//! here, at the native boundary, before SQLite constraints act as the last line of defence.

pub mod album;
pub mod catalogue;
pub mod discography;
pub mod editions;
pub mod idempotency;
pub mod listening;
pub mod listing;
pub mod metadata_import;
pub mod next_up;
pub mod personal;
pub mod preferences;
pub mod provenance;
pub mod ratings;
pub mod selection;
pub mod settings;
pub mod stats;
pub mod tags;

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{AppError, AppResult};

const MAX_TEXT: usize = 2_000;
const MAX_LONG_TEXT: usize = 100_000;

/// Trim and require non-empty, bounded text.
pub(crate) fn required_text(field: &'static str, raw: &str) -> AppResult<String> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(AppError::validation(field, "must not be empty"));
    }
    bounded(field, value, MAX_TEXT)
}

/// Trim; empty becomes `None`.
pub(crate) fn optional_text(field: &'static str, raw: Option<&str>) -> AppResult<Option<String>> {
    raw.map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| bounded(field, v, MAX_TEXT))
        .transpose()
}

/// Long free text (reviews). Kept verbatim apart from surrounding whitespace.
pub(crate) fn optional_long_text(
    field: &'static str,
    raw: Option<&str>,
) -> AppResult<Option<String>> {
    raw.map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| bounded(field, v, MAX_LONG_TEXT))
        .transpose()
}

fn bounded(field: &'static str, value: &str, max: usize) -> AppResult<String> {
    if value.chars().count() > max {
        return Err(AppError::validation(
            field,
            format!("longer than {max} characters"),
        ));
    }
    Ok(value.to_owned())
}

/// Tables that may be probed by `require`. A closed set keeps table names out of user input.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Table {
    Artist,
    Album,
    Edition,
    Track,
    Recording,
    Tag,
    SelectionSession,
    SelectionAttempt,
}

impl Table {
    fn name(self) -> &'static str {
        match self {
            Self::Artist => "artist",
            Self::Album => "album",
            Self::Edition => "edition",
            Self::Track => "track",
            Self::Recording => "recording",
            Self::Tag => "tag",
            Self::SelectionSession => "selection_session",
            Self::SelectionAttempt => "selection_attempt",
        }
    }
}

/// Fail with `not_found` unless a row with this id exists.
pub(crate) fn require(conn: &Connection, table: Table, id: &str) -> AppResult<()> {
    let sql = format!("SELECT 1 FROM {} WHERE id = ?1", table.name());
    let found = conn
        .query_row(&sql, params![id], |_| Ok(()))
        .optional()?
        .is_some();
    if found {
        Ok(())
    } else {
        Err(AppError::not_found(table.name(), id))
    }
}

/// The canonical album an edition belongs to.
pub(crate) fn album_of_edition(conn: &Connection, edition_id: &str) -> AppResult<String> {
    conn.query_row(
        "SELECT album_id FROM edition WHERE id = ?1",
        params![edition_id],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| AppError::not_found("edition", edition_id))
}
