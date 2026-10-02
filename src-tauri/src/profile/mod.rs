//! Profile export and restore: a portable `.mudraft` archive, never a raw database copy.
//!
//! # Format (version 1)
//! A ZIP archive (DEFLATE) with forward-slash, relative entry names only:
//! - `manifest.json` — [`Manifest`]: format name and version, the database schema version
//!   the data was written at, app version, creation time, whether artwork is included,
//!   summary counts, and the size and SHA-256 of every other entry.
//! - `data/<table>.json` — one file per exported table: `{"table", "columns", "rows"}`,
//!   rows as arrays of JSON scalars in column order. Stable IDs and relationships are kept
//!   as stored.
//! - `artwork/<file name>` — cached and user-added images (unless exported "lightweight").
//!
//! Exported tables are [`TABLES`]; everything else is listed in [`EXCLUDED`] with the
//! reason (caches, idempotency keys, CSV staging). Settings are limited to
//! [`SETTING_PREFIXES`]. No absolute paths, credentials, or development flags are written.
//!
//! # Versions and migrations
//! `formatVersion` changes only if the archive layout changes; readers reject newer ones.
//! `schemaVersion` is the database schema of the data. An archive from an older MuDraft is
//! loaded into a database created at *its* schema version, then upgraded by the app's
//! normal, tested migrations — the same path an older installation takes. Archives from
//! a newer schema are rejected without touching anything. FDraft profiles are not
//! compatible and are rejected as malformed.
//!
//! # Restore
//! Replace-only (no merging): validate and stage into `restore-staging/`, back up the
//! current profile as a `.mudraft` archive, then activate under a journal so a crash at
//! any point either leaves the old profile in place or completes the new one
//! ([`restore::recover`] runs at startup).

pub mod export;
pub mod restore;
#[cfg(test)]
mod tests;

use std::io::Read;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};

pub const FORMAT: &str = "mudraft-profile";
pub const FORMAT_VERSION: u32 = 1;
pub const EXTENSION: &str = "mudraft";

/// Exported tables, parents before children.
pub const TABLES: &[&str] = &[
    "artist",
    "genre",
    "album",
    "album_artist_credit",
    "album_genre",
    "edition",
    "recording",
    "track",
    "track_artist_credit",
    "tag",
    "album_tag",
    "metadata_provenance",
    "album_source_tag",
    "listen_list_entry",
    "collection_entry",
    "listen_event",
    "listen_event_track",
    "track_rating",
    "album_review",
    "setting",
    "selection_session",
    "selection_attempt",
    "current_selection",
    "artwork",
    "artist_catalogue",
    "catalogue_entry",
    "catalogue_reference",
    "catalogue_track",
    "imported_row",
];

/// Tables deliberately left out, with why.
pub const EXCLUDED: &[(&str, &str)] = &[
    ("app_meta", "reserved; holds nothing user-owned"),
    (
        "applied_mutation",
        "idempotency keys for in-flight requests (transient)",
    ),
    ("metadata_cache", "disposable MusicBrainz response cache"),
    ("import_session", "CSV import staging (transient)"),
    ("import_row", "CSV import staging (transient)"),
];

/// Only user preferences travel; anything else in `setting` stays behind.
pub const SETTING_PREFIXES: &[&str] = &["ui.", "listen.", "artwork."];

// Limits checked before and while reading an archive.
pub const MAX_ARCHIVE_BYTES: u64 = 8 * 1024 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 200_000;
pub const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_DATA_FILE_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_ASSET_BYTES: u64 = crate::artwork::http::MAX_IMAGE_BYTES as u64;
pub const MAX_TOTAL_UNCOMPRESSED: u64 = 16 * 1024 * 1024 * 1024;
/// Larger expansion than this is treated as a decompression bomb.
pub const MAX_COMPRESSION_RATIO: u64 = 250;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtworkMode {
    Included,
    /// Lightweight export: no images. Removed/none markers are still kept.
    Omitted,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Counts {
    pub artists: u64,
    pub albums: u64,
    pub editions: u64,
    pub tracks: u64,
    pub listen_list: u64,
    pub collection: u64,
    pub listens: u64,
    pub rated_tracks: u64,
    pub album_reviews: u64,
    pub tags: u64,
    pub selection_attempts: u64,
    pub catalogue_entries: u64,
    pub artwork_files: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TableEntry {
    pub name: String,
    pub path: String,
    pub rows: u64,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssetEntry {
    /// Relative, forward-slash path inside the archive, e.g. `artwork/x.jpg`.
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub format: String,
    pub format_version: u32,
    pub schema_version: i64,
    pub app_version: String,
    pub created_at: String,
    pub artwork: ArtworkMode,
    pub counts: Counts,
    pub tables: Vec<TableEntry>,
    pub assets: Vec<AssetEntry>,
}

/// One table's rows as written in `data/<table>.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TableData {
    pub table: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256_reader(mut r: impl Read) -> AppResult<String> {
    let mut hasher = Sha256::new();
    let mut buf = [0_u8; 64 * 1024];
    loop {
        let n = r.read(&mut buf).map_err(io)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

pub(crate) fn io(e: std::io::Error) -> AppError {
    AppError::StorageUnavailable(e.to_string())
}

pub(crate) fn corrupt(reason: impl Into<String>) -> AppError {
    AppError::validation("profile archive", reason)
}

/// A file name MuDraft itself would write: letters, digits, `.`, `-`, `_`; no separators.
pub fn safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// Normalize an archive entry name to a portable relative path. Backslashes (written by
/// some Windows tools) count as separators; absolute paths, drive letters, `.`/`..`, and
/// empty components are rejected rather than repaired.
pub fn portable_path(raw: &str) -> AppResult<String> {
    let unified = raw.replace('\\', "/");
    if unified.starts_with('/') || unified.contains(':') || unified.contains('\0') {
        return Err(corrupt(format!("unsafe path “{raw}”")));
    }
    let parts: Vec<&str> = unified.split('/').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == "." || *p == ".." || p.chars().any(char::is_control))
    {
        return Err(corrupt(format!("unsafe path “{raw}”")));
    }
    Ok(parts.join("/"))
}
