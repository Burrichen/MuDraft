//! Typed settings over the `setting` key/value table. Absent keys take defaults; a stored
//! value of the wrong type is reported, never silently replaced by the default.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

const FULL_LISTEN_REMOVES: &str = "listen.full_listen_removes_from_listen_list";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub full_listen_removes_from_listen_list: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            full_listen_removes_from_listen_list: true,
        }
    }
}

pub fn load(conn: &Connection) -> AppResult<Settings> {
    let defaults = Settings::default();
    Ok(Settings {
        full_listen_removes_from_listen_list: read_value(conn, FULL_LISTEN_REMOVES, "true/false")?
            .unwrap_or(defaults.full_listen_removes_from_listen_list),
    })
}

/// Save every setting; unknown keys already in the table are left untouched.
pub fn save(tx: &Transaction<'_>, settings: &Settings) -> AppResult<()> {
    write_value(
        tx,
        FULL_LISTEN_REMOVES,
        settings.full_listen_removes_from_listen_list,
    )
}

/// Read one JSON setting; `expected` describes the type for the error message.
pub(super) fn read_value<T: DeserializeOwned>(
    conn: &Connection,
    key: &'static str,
    expected: &str,
) -> AppResult<Option<T>> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM setting WHERE key = ?1",
            params![key],
            |r| r.get(0),
        )
        .optional()?;
    raw.map(|v| {
        serde_json::from_str::<T>(&v).map_err(|_| {
            AppError::validation("setting", format!("{key} holds {v}, expected {expected}"))
        })
    })
    .transpose()
}

pub(super) fn write_value(tx: &Transaction<'_>, key: &str, value: impl Serialize) -> AppResult<()> {
    let json = serde_json::to_string(&value).map_err(|e| AppError::Internal(e.to_string()))?;
    tx.execute(
        "INSERT INTO setting (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        params![key, json],
    )?;
    Ok(())
}

const ARTWORK_DOWNLOAD: &str = "artwork.download";

/// `None` until the user has answered the artwork consent prompt.
pub fn artwork_download(conn: &Connection) -> AppResult<Option<bool>> {
    read_value(conn, ARTWORK_DOWNLOAD, "true/false")
}

pub fn set_artwork_download(tx: &Transaction<'_>, allowed: bool) -> AppResult<()> {
    write_value(tx, ARTWORK_DOWNLOAD, allowed)
}
