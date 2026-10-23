//! Apply a merged, validated state to the live database. Runs inside one transaction with
//! foreign-key actions off ([`crate::db::Database::write_unordered`]), so rows can change
//! in any order without cascades; every reference is verified before commit.

use std::collections::HashMap;

use rusqlite::{Transaction, params_from_iter};

use super::schema::{Schema, TableSchema};
use super::state::{Row, State, upsert_sql};
use crate::error::{AppError, AppResult};
use crate::profile::restore::sql_value;

pub struct Applied {
    /// Rows inserted, updated, or deleted.
    pub changes: u64,
    /// Artwork files nothing references any more; delete them after commit.
    pub stale_files: Vec<String>,
}

fn pk_names(ts: &TableSchema) -> Vec<String> {
    ts.pk.iter().map(|&i| ts.columns[i].clone()).collect()
}

fn pk_values(ts: &TableSchema, row: &Row) -> AppResult<Vec<rusqlite::types::Value>> {
    ts.pk.iter().map(|&i| sql_value(&row[i], ts.name)).collect()
}

fn where_pk(ts: &TableSchema) -> String {
    ts.pk
        .iter()
        .enumerate()
        .map(|(n, &i)| format!("\"{}\" = ?{}", ts.columns[i], n + 1))
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn is_unique_violation(e: &rusqlite::Error) -> bool {
    matches!(
        e.sqlite_error().map(|x| x.extended_code),
        Some(rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE | rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY)
    )
}

/// `current` must be a snapshot taken inside `tx`. `files` maps each image hash in
/// `merged` to its file in the artwork folder.
pub fn apply(
    tx: &Transaction<'_>,
    schema: &Schema,
    current: &State,
    merged: &State,
    files: &HashMap<String, String>,
) -> AppResult<Applied> {
    let mut changes = 0_u64;
    let mut stale = Vec::new();
    for ts in schema.tables.iter().rev() {
        let new = merged.rows(ts.name);
        for (k, row) in current.rows(ts.name) {
            if !new.contains_key(k) {
                delete(tx, ts, row, &mut stale)?;
                changes += 1;
            }
        }
    }
    let mut pending: Vec<(&TableSchema, &Row)> = Vec::new();
    for ts in &schema.tables {
        let cur = current.rows(ts.name);
        for (k, row) in merged.rows(ts.name) {
            if cur.get(k) != Some(row) {
                pending.push((ts, row));
            }
        }
    }
    changes += pending.len() as u64;
    // A row may clash with another that hasn't moved yet (tracks renumbered, names
    // swapped); retry until every row lands or no progress is possible.
    while !pending.is_empty() {
        let before = pending.len();
        let mut retry = Vec::new();
        let mut last = None;
        for (ts, row) in pending {
            match upsert(tx, ts, row, files, &mut stale) {
                Ok(()) => {}
                Err(Step::Sql(e)) if is_unique_violation(&e) => {
                    last = Some(format!("{}: {e}", ts.name));
                    retry.push((ts, row));
                }
                Err(Step::Sql(e)) => {
                    return Err(AppError::validation(
                        "sync data",
                        format!("{}: {e}", ts.name),
                    ));
                }
                Err(Step::App(e)) => return Err(e),
            }
        }
        if retry.len() == before {
            return Err(AppError::validation(
                "sync data",
                format!(
                    "records couldn't be reordered safely ({})",
                    last.unwrap_or_default()
                ),
            ));
        }
        pending = retry;
    }
    // Discovered discographies are local caches keyed to artists; drop any left behind.
    tx.execute(
        "DELETE FROM artist_catalogue WHERE artist_id NOT IN (SELECT id FROM artist)",
        [],
    )?;
    tx.execute(
        "DELETE FROM catalogue_entry WHERE source != 'manual' AND artist_id NOT IN (SELECT id FROM artist)",
        [],
    )?;
    let mut unreferenced = Vec::new();
    for name in stale {
        let used: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM artwork WHERE file_name = ?1)",
            [&name],
            |r| r.get(0),
        )?;
        if !used && !unreferenced.contains(&name) {
            unreferenced.push(name);
        }
    }
    Ok(Applied {
        changes,
        stale_files: unreferenced,
    })
}

enum Step {
    Sql(rusqlite::Error),
    App(AppError),
}

impl From<rusqlite::Error> for Step {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sql(e)
    }
}

impl From<AppError> for Step {
    fn from(e: AppError) -> Self {
        Self::App(e)
    }
}

fn artwork_file(
    tx: &Transaction<'_>,
    owner_type: &str,
    owner_id: &str,
) -> rusqlite::Result<Option<String>> {
    tx.query_row(
        "SELECT file_name FROM artwork WHERE owner_type = ?1 AND owner_id = ?2",
        [owner_type, owner_id],
        |r| r.get(0),
    )
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        e => Err(e),
    })
}

fn delete(
    tx: &Transaction<'_>,
    ts: &TableSchema,
    row: &Row,
    stale: &mut Vec<String>,
) -> AppResult<()> {
    if ts.name == "artwork" {
        let (ot, oid) = (
            row[0].as_str().unwrap_or_default(),
            row[1].as_str().unwrap_or_default(),
        );
        stale.extend(artwork_file(tx, ot, oid)?);
        tx.execute(
            "DELETE FROM artwork WHERE owner_type = ?1 AND owner_id = ?2 AND source IN ('local', 'removed')",
            [ot, oid],
        )?;
        return Ok(());
    }
    if matches!(ts.name, "album" | "edition")
        && let Some(id) = ts.col("id").and_then(|i| row[i].as_str())
    {
        // Their artwork rows go with them (trigger); remember the files.
        stale.extend(artwork_file(tx, ts.name, id)?);
    }
    tx.execute(
        &format!("DELETE FROM \"{}\" WHERE {}", ts.name, where_pk(ts)),
        params_from_iter(pk_values(ts, row)?),
    )?;
    Ok(())
}

fn upsert(
    tx: &Transaction<'_>,
    ts: &TableSchema,
    row: &Row,
    files: &HashMap<String, String>,
    stale: &mut Vec<String>,
) -> Result<(), Step> {
    if ts.name == "artwork" {
        let (ot, oid) = (
            row[0].as_str().unwrap_or_default(),
            row[1].as_str().unwrap_or_default(),
        );
        let file = match row[3].as_str() {
            Some(sha) => Some(files.get(sha).cloned().ok_or_else(|| {
                AppError::Internal(format!("artwork {sha} wasn't prepared before applying"))
            })?),
            None => None,
        };
        if let Some(old) = artwork_file(tx, ot, oid)?
            && Some(&old) != file.as_ref()
        {
            stale.push(old);
        }
        tx.execute(
            "INSERT INTO artwork (owner_type, owner_id, source, file_name, mime, bytes, source_url)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)
             ON CONFLICT (owner_type, owner_id) DO UPDATE SET source = excluded.source,
                 file_name = excluded.file_name, mime = excluded.mime, bytes = excluded.bytes,
                 source_url = NULL, checked_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            params_from_iter([
                sql_value(&row[0], ts.name)?,
                sql_value(&row[1], ts.name)?,
                sql_value(&row[2], ts.name)?,
                file.map_or(rusqlite::types::Value::Null, rusqlite::types::Value::Text),
                sql_value(&row[4], ts.name)?,
                sql_value(&row[5], ts.name)?,
            ]),
        )?;
        return Ok(());
    }
    let values: Vec<_> = row
        .iter()
        .map(|v| sql_value(v, ts.name))
        .collect::<AppResult<_>>()?;
    tx.execute(
        &upsert_sql(ts.name, &ts.columns, &pk_names(ts)),
        params_from_iter(values),
    )?;
    Ok(())
}
