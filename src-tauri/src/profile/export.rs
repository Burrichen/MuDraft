//! Write a `.mudraft` archive from one consistent read snapshot and publish it atomically.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use rusqlite::types::ValueRef;
use serde::Serialize;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::{
    ArtworkMode, AssetEntry, Counts, FORMAT, FORMAT_VERSION, Manifest, SETTING_PREFIXES, TABLES,
    TableData, TableEntry, corrupt, io, safe_file_name, sha256_hex,
};
use crate::domain::ids::new_id;
use crate::error::{AppError, AppResult};
use crate::metadata::clock::utc_now;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOutcome {
    pub file_name: String,
    pub bytes: u64,
    pub artwork: ArtworkMode,
    pub counts: Counts,
    /// Artwork rows whose cached file was already missing on disk (left out).
    pub artwork_missing: u64,
}

/// Columns that can be written back (generated columns are recomputed on restore).
pub(crate) fn insertable_columns(conn: &Connection, table: &str) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))?;
    let cols = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if cols.is_empty() {
        return Err(AppError::Internal(format!("table {table} has no columns")));
    }
    Ok(cols)
}

fn to_json(v: ValueRef<'_>, table: &str) -> AppResult<serde_json::Value> {
    Ok(match v {
        ValueRef::Null => serde_json::Value::Null,
        ValueRef::Integer(i) => i.into(),
        ValueRef::Real(f) => serde_json::Number::from_f64(f)
            .map(serde_json::Value::Number)
            .ok_or_else(|| AppError::Internal(format!("non-finite number in {table}")))?,
        ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned().into(),
        ValueRef::Blob(_) => {
            return Err(AppError::Internal(format!(
                "unexpected binary value in {table}"
            )));
        }
    })
}

pub(crate) fn dump_table(conn: &Connection, table: &str) -> AppResult<TableData> {
    let columns = insertable_columns(conn, table)?;
    let list = columns
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let mut stmt = conn.prepare(&format!("SELECT {list} FROM \"{table}\" ORDER BY rowid"))?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let mut values = Vec::with_capacity(columns.len());
        for i in 0..columns.len() {
            values.push(to_json(row.get_ref(i)?, table)?);
        }
        out.push(values);
    }
    Ok(TableData {
        table: table.into(),
        columns,
        rows: out,
    })
}

fn column(data: &TableData, name: &str) -> Option<usize> {
    data.columns.iter().position(|c| c == name)
}

pub(crate) fn counts(conn: &Connection) -> AppResult<Counts> {
    let n = |sql: &str| -> AppResult<u64> {
        Ok(conn.query_row(sql, [], |r| r.get::<_, i64>(0))? as u64)
    };
    Ok(Counts {
        artists: n("SELECT COUNT(*) FROM artist")?,
        albums: n("SELECT COUNT(*) FROM album")?,
        editions: n("SELECT COUNT(*) FROM edition")?,
        tracks: n("SELECT COUNT(*) FROM track")?,
        listen_list: n("SELECT COUNT(*) FROM listen_list_entry")?,
        collection: n("SELECT COUNT(*) FROM collection_entry")?,
        listens: n("SELECT COUNT(*) FROM listen_event")?,
        rated_tracks: n("SELECT COUNT(*) FROM track_rating WHERE rating IS NOT NULL")?,
        album_reviews: n("SELECT COUNT(*) FROM album_review")?,
        tags: n("SELECT COUNT(*) FROM tag")?,
        selection_attempts: n("SELECT COUNT(*) FROM selection_attempt")?,
        catalogue_entries: n("SELECT COUNT(*) FROM catalogue_entry")?,
        artwork_files: n("SELECT COUNT(*) FROM artwork WHERE file_name IS NOT NULL")?,
    })
}

struct Snapshot {
    schema_version: i64,
    tables: Vec<TableData>,
    /// Artwork file names to stream from `artwork_dir`.
    assets: Vec<String>,
    artwork_missing: u64,
    counts: Counts,
}

/// Read every exported table (and the artwork files they name) inside one read
/// transaction, so the archive is a single point-in-time view.
fn snapshot(conn: &Connection, artwork_dir: &Path, include_artwork: bool) -> AppResult<Snapshot> {
    let tx = conn.unchecked_transaction()?;
    let schema_version: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let mut tables = Vec::with_capacity(TABLES.len());
    let mut assets = Vec::new();
    let mut artwork_missing = 0;
    for table in TABLES {
        let mut data = dump_table(&tx, table)?;
        match *table {
            "setting" => {
                let key = column(&data, "key").ok_or_else(|| corrupt("setting without key"))?;
                data.rows.retain(|r| {
                    r[key]
                        .as_str()
                        .is_some_and(|k| SETTING_PREFIXES.iter().any(|p| k.starts_with(p)))
                });
            }
            "artwork" => {
                let file =
                    column(&data, "file_name").ok_or_else(|| corrupt("artwork without file"))?;
                let mut kept = Vec::with_capacity(data.rows.len());
                for row in data.rows {
                    let Some(name) = row[file].as_str().map(str::to_owned) else {
                        kept.push(row); // "removed"/"none" markers carry no file
                        continue;
                    };
                    if !include_artwork {
                        continue;
                    }
                    let present = safe_file_name(&name)
                        && std::fs::metadata(artwork_dir.join(&name)).is_ok_and(|m| m.is_file());
                    if present {
                        assets.push(name);
                        kept.push(row);
                    } else {
                        artwork_missing += 1;
                    }
                }
                data.rows = kept;
            }
            _ => {}
        }
        tables.push(data);
    }
    let mut counts = counts(&tx)?;
    counts.artwork_files = assets.len() as u64;
    Ok(Snapshot {
        schema_version,
        tables,
        assets,
        artwork_missing,
        counts,
    })
}

/// Export the profile to `dest`. The archive is written beside `dest` under a temporary
/// name, flushed to disk, and only then renamed into place, so `dest` is never partial.
pub fn export(
    conn: &Connection,
    artwork_dir: &Path,
    dest: &Path,
    include_artwork: bool,
) -> AppResult<ExportOutcome> {
    let file_name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| AppError::validation("file", "choose a file name"))?;
    let snap = snapshot(conn, artwork_dir, include_artwork)?;
    let tmp: PathBuf = dest.with_file_name(format!(".{file_name}.partial-{}", new_id()));
    let result = write_archive(&tmp, &snap, artwork_dir, include_artwork).and_then(|bytes| {
        std::fs::rename(&tmp, dest).map_err(io)?;
        Ok(bytes)
    });
    match result {
        Ok(bytes) => Ok(ExportOutcome {
            file_name,
            bytes,
            artwork: if include_artwork {
                ArtworkMode::Included
            } else {
                ArtworkMode::Omitted
            },
            counts: snap.counts,
            artwork_missing: snap.artwork_missing,
        }),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

fn write_archive(
    path: &Path,
    snap: &Snapshot,
    artwork_dir: &Path,
    include_artwork: bool,
) -> AppResult<u64> {
    let file = File::create_new(path).map_err(io)?;
    let mut zip = ZipWriter::new(BufWriter::new(file));
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .large_file(true);
    let zip_err = |e: zip::result::ZipError| AppError::StorageUnavailable(e.to_string());

    let mut tables = Vec::with_capacity(snap.tables.len());
    for data in &snap.tables {
        let bytes = serde_json::to_vec(data)
            .map_err(|e| AppError::Internal(format!("cannot encode {}: {e}", data.table)))?;
        let entry = format!("data/{}.json", data.table);
        zip.start_file(entry.as_str(), options).map_err(zip_err)?;
        zip.write_all(&bytes).map_err(io)?;
        tables.push(TableEntry {
            name: data.table.clone(),
            path: entry,
            rows: data.rows.len() as u64,
            bytes: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
        });
    }
    let mut assets = Vec::with_capacity(snap.assets.len());
    for name in &snap.assets {
        // Artwork files are never rewritten in place (new content gets a new name), and
        // deletions happen under the database lock the caller holds.
        let bytes = std::fs::read(artwork_dir.join(name)).map_err(|e| {
            AppError::Conflict(format!("artwork changed during export ({e}); try again"))
        })?;
        let bytes = bytes.as_slice();
        let entry = format!("artwork/{name}");
        // Images are already compressed; storing them avoids wasted work.
        zip.start_file(
            entry.as_str(),
            options.compression_method(CompressionMethod::Stored),
        )
        .map_err(zip_err)?;
        zip.write_all(bytes).map_err(io)?;
        assets.push(AssetEntry {
            path: entry,
            bytes: bytes.len() as u64,
            sha256: sha256_hex(bytes),
        });
    }
    let manifest = Manifest {
        format: FORMAT.into(),
        format_version: FORMAT_VERSION,
        schema_version: snap.schema_version,
        app_version: env!("CARGO_PKG_VERSION").into(),
        created_at: utc_now(),
        artwork: if include_artwork {
            ArtworkMode::Included
        } else {
            ArtworkMode::Omitted
        },
        counts: snap.counts.clone(),
        tables,
        assets,
    };
    zip.start_file("manifest.json", options).map_err(zip_err)?;
    let manifest = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| AppError::Internal(format!("cannot encode manifest: {e}")))?;
    zip.write_all(&manifest).map_err(io)?;
    let writer = zip.finish().map_err(zip_err)?;
    let file = writer
        .into_inner()
        .map_err(|e| AppError::StorageUnavailable(e.to_string()))?;
    file.sync_all().map_err(io)?;
    Ok(file.metadata().map_err(io)?.len())
}
