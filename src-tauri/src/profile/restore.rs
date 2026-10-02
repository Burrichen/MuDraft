//! Validate and stage a `.mudraft` archive, then activate it under a crash-safe journal.
//!
//! Staging builds `restore-staging/` (database + artwork) from the archive; nothing live is
//! touched until everything validates. Activation moves the live profile to
//! `restore-previous/` and the staged one into place, recording each phase in
//! `profile-restore.json`, so [`recover`] can finish an interrupted swap at startup.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use rusqlite::types::Value;
use serde::{Deserialize, Serialize};
use zip::ZipArchive;

use super::export::{counts, insertable_columns};
use super::{
    ArtworkMode, Counts, FORMAT, FORMAT_VERSION, MAX_ARCHIVE_BYTES, MAX_ASSET_BYTES,
    MAX_COMPRESSION_RATIO, MAX_DATA_FILE_BYTES, MAX_ENTRIES, MAX_MANIFEST_BYTES,
    MAX_TOTAL_UNCOMPRESSED, Manifest, TABLES, TableData, corrupt, io, portable_path,
    safe_file_name, sha256_hex, sha256_reader,
};
use crate::db::{self, Database, SCHEMA_VERSION};
use crate::domain::ids::parse_uuid;
use crate::error::{AppError, AppResult};
use crate::library::catalogue::LISTEN_ASAP_TAG_ID;
use crate::paths::DB_FILE_NAME;

pub const STAGING_DIR: &str = "restore-staging";
pub const PREVIOUS_DIR: &str = "restore-previous";
pub const JOURNAL: &str = "profile-restore.json";
pub const ARTWORK_DIR: &str = "artwork";

/// What a profile consists of on disk, in the order it is moved.
fn items() -> [String; 4] {
    [
        DB_FILE_NAME.to_owned(),
        format!("{DB_FILE_NAME}-wal"),
        format!("{DB_FILE_NAME}-shm"),
        ARTWORK_DIR.to_owned(),
    ]
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub format_version: u32,
    /// Schema the archive was written at; upgraded on restore when older.
    pub schema_version: i64,
    pub current_schema_version: i64,
    pub app_version: String,
    pub created_at: String,
    pub artwork: ArtworkMode,
    /// Counted from the validated, staged profile (not taken on trust from the manifest).
    pub counts: Counts,
    pub archive_bytes: u64,
    /// Fingerprint of the archive; restore refuses a file that changed after preview.
    pub sha256: String,
}

fn read_bounded(mut r: impl Read, limit: u64, what: &str) -> AppResult<Vec<u8>> {
    let mut buf = Vec::new();
    r.by_ref()
        .take(limit + 1)
        .read_to_end(&mut buf)
        .map_err(|e| corrupt(format!("{what}: {e}")))?;
    if buf.len() as u64 > limit {
        return Err(corrupt(format!("{what} is larger than declared")));
    }
    Ok(buf)
}

struct Entries {
    /// Portable name → archive index.
    by_name: HashMap<String, usize>,
}

/// Structural checks on every entry before any content is trusted.
fn scan(zip: &mut ZipArchive<File>) -> AppResult<Entries> {
    if zip.len() > MAX_ENTRIES {
        return Err(corrupt("too many entries"));
    }
    let mut by_name = HashMap::new();
    let mut lower = HashSet::new();
    let mut total: u64 = 0;
    for i in 0..zip.len() {
        let entry = zip.by_index(i).map_err(|e| corrupt(e.to_string()))?;
        let raw = entry.name().to_owned();
        let is_link =
            entry.is_symlink() || entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000);
        if is_link {
            return Err(corrupt(format!("“{raw}” is a symbolic link")));
        }
        if entry.is_dir() {
            portable_path(raw.trim_end_matches(['/', '\\']))?;
            continue;
        }
        let name = portable_path(&raw)?;
        total = total.saturating_add(entry.size());
        if total > MAX_TOTAL_UNCOMPRESSED {
            return Err(corrupt("expands beyond the allowed size"));
        }
        let packed = entry.compressed_size().max(1);
        if entry.size() > 1024 * 1024 && entry.size() / packed > MAX_COMPRESSION_RATIO {
            return Err(corrupt(format!("“{name}” is compressed suspiciously well")));
        }
        // Case-insensitive file systems (macOS, Windows) would merge these.
        if !lower.insert(name.to_lowercase()) {
            return Err(corrupt(format!("“{name}” appears twice")));
        }
        by_name.insert(name, i);
    }
    Ok(Entries { by_name })
}

fn read_entry(
    zip: &mut ZipArchive<File>,
    entries: &Entries,
    name: &str,
    declared: u64,
    limit: u64,
) -> AppResult<Vec<u8>> {
    let index = *entries
        .by_name
        .get(name)
        .ok_or_else(|| corrupt(format!("“{name}” is missing")))?;
    if declared > limit {
        return Err(corrupt(format!("“{name}” is too large")));
    }
    let entry = zip.by_index(index).map_err(|e| corrupt(e.to_string()))?;
    if entry.size() != declared {
        return Err(corrupt(format!("“{name}” has the wrong size")));
    }
    let bytes = read_bounded(entry, declared, name)?;
    if bytes.len() as u64 != declared {
        return Err(corrupt(format!("“{name}” is truncated")));
    }
    Ok(bytes)
}

fn read_manifest(zip: &mut ZipArchive<File>, entries: &Entries) -> AppResult<Manifest> {
    let index = *entries
        .by_name
        .get("manifest.json")
        .ok_or_else(|| corrupt("this isn't a MuDraft profile (no manifest)"))?;
    let entry = zip.by_index(index).map_err(|e| corrupt(e.to_string()))?;
    let bytes = read_bounded(entry, MAX_MANIFEST_BYTES, "manifest")?;
    // Check the version before the strict shape, so newer archives get a clear message.
    let probe: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| corrupt(format!("manifest: {e}")))?;
    if probe.get("format").and_then(|f| f.as_str()) != Some(FORMAT) {
        return Err(corrupt("this isn't a MuDraft profile"));
    }
    let format_version = probe
        .get("formatVersion")
        .and_then(serde_json::Value::as_u64);
    if format_version.is_none_or(|v| v > u64::from(FORMAT_VERSION)) {
        return Err(AppError::validation(
            "profile archive",
            "it was made by a newer version of MuDraft; update MuDraft to restore it",
        ));
    }
    let schema = probe
        .get("schemaVersion")
        .and_then(serde_json::Value::as_i64);
    if schema.is_none_or(|v| v > SCHEMA_VERSION) {
        return Err(AppError::validation(
            "profile archive",
            "its data comes from a newer version of MuDraft; update MuDraft to restore it",
        ));
    }
    serde_json::from_slice(&bytes).map_err(|e| corrupt(format!("manifest: {e}")))
}

fn sql_value(v: &serde_json::Value, table: &str) -> AppResult<Value> {
    Ok(match v {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Integer(i64::from(*b)),
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(i) => Value::Integer(i),
            None => Value::Real(
                n.as_f64()
                    .ok_or_else(|| corrupt(format!("bad number in {table}")))?,
            ),
        },
        serde_json::Value::String(s) => Value::Text(s.clone()),
        _ => return Err(corrupt(format!("nested value in {table}"))),
    })
}

/// Load one table's rows into the staging database, validating shape and IDs. The
/// built-in Listen ASAP tag already exists there, so it is updated, never duplicated.
fn load_table(
    tx: &rusqlite::Transaction<'_>,
    data: &TableData,
    expected_rows: u64,
    tag_remap: &mut HashMap<String, String>,
) -> AppResult<()> {
    let allowed = insertable_columns(tx, &data.table).map_err(|_| {
        corrupt(format!(
            "{} doesn't exist at this schema version",
            data.table
        ))
    })?;
    let mut seen = HashSet::new();
    for c in &data.columns {
        if !allowed.contains(c) || !seen.insert(c) {
            return Err(corrupt(format!("unexpected column {c} in {}", data.table)));
        }
    }
    if data.rows.len() as u64 != expected_rows {
        return Err(corrupt(format!(
            "{} row count doesn't match the manifest",
            data.table
        )));
    }
    let id_col = data.columns.iter().position(|c| c == "id");
    let pos = |name: &str| data.columns.iter().position(|c| c == name);
    let list = data
        .columns
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let marks = vec!["?"; data.columns.len()].join(", ");
    let mut insert = tx.prepare(&format!(
        "INSERT INTO \"{}\" ({list}) VALUES ({marks})",
        data.table
    ))?;
    for row in &data.rows {
        if row.len() != data.columns.len() {
            return Err(corrupt(format!(
                "a row in {} has the wrong length",
                data.table
            )));
        }
        if let Some(i) = id_col {
            let raw = row[i]
                .as_str()
                .ok_or_else(|| corrupt(format!("bad id in {}", data.table)))?;
            if parse_uuid("id", raw)? != raw {
                return Err(corrupt(format!("non-canonical id {raw} in {}", data.table)));
            }
        }
        let mut values: Vec<Value> = row
            .iter()
            .map(|v| sql_value(v, &data.table))
            .collect::<AppResult<_>>()?;
        if data.table == "tag"
            && let Some(k) = pos("builtin_key")
            && !row[k].is_null()
        {
            if row[k].as_str() != Some("listen_asap") {
                return Err(corrupt("unknown built-in tag"));
            }
            let id = row[id_col.ok_or_else(|| corrupt("tag without id"))?]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            if !tag_remap.is_empty() {
                return Err(corrupt("the built-in tag appears twice"));
            }
            tag_remap.insert(id, LISTEN_ASAP_TAG_ID.to_owned());
            // The staging database already has this tag; carry over its editable fields.
            for col in ["color", "created_at"] {
                if let Some(c) = pos(col) {
                    tx.execute(
                        &format!("UPDATE tag SET {col} = ?1 WHERE builtin_key = 'listen_asap'"),
                        [sql_value(&row[c], &data.table)?],
                    )
                    .map_err(|e| corrupt(format!("tag: {e}")))?;
                }
            }
            continue;
        }
        if data.table == "album_tag"
            && let Some(t) = pos("tag_id")
            && let Some(mapped) = row[t].as_str().and_then(|id| tag_remap.get(id))
        {
            values[t] = Value::Text(mapped.clone());
        }
        insert
            .execute(rusqlite::params_from_iter(values))
            .map_err(|e| corrupt(format!("{}: {e}", data.table)))?;
    }
    Ok(())
}

/// Validate `archive` completely and build the profile in `staging` (which must not exist).
pub fn stage(archive: &Path, staging: &Path) -> AppResult<Preview> {
    let meta = std::fs::symlink_metadata(archive).map_err(io)?;
    if !meta.is_file() {
        return Err(corrupt("choose a .mudraft file"));
    }
    if meta.len() > MAX_ARCHIVE_BYTES {
        return Err(corrupt("the file is too large"));
    }
    let sha256 = sha256_reader(File::open(archive).map_err(io)?)?;
    let mut zip = ZipArchive::new(File::open(archive).map_err(io)?)
        .map_err(|e| corrupt(format!("not a readable archive ({e})")))?;
    let entries = scan(&mut zip)?;
    let manifest = read_manifest(&mut zip, &entries)?;

    // Every entry must be accounted for, and every listed file must be well-formed.
    let mut listed: HashSet<String> = HashSet::from(["manifest.json".to_owned()]);
    for t in &manifest.tables {
        if !TABLES.contains(&t.name.as_str()) {
            return Err(corrupt(format!("unknown table {}", t.name)));
        }
        let path = portable_path(&t.path)?;
        if path != format!("data/{}.json", t.name) || !listed.insert(path) {
            return Err(corrupt(format!("bad path for {}", t.name)));
        }
    }
    for a in &manifest.assets {
        let path = portable_path(&a.path)?;
        let name = path.strip_prefix("artwork/").unwrap_or_default();
        if !safe_file_name(name) || !listed.insert(path.clone()) {
            return Err(corrupt(format!("bad artwork path {}", a.path)));
        }
    }
    if let Some(extra) = entries.by_name.keys().find(|n| !listed.contains(*n)) {
        return Err(corrupt(format!("unexpected file “{extra}”")));
    }
    if manifest.artwork == ArtworkMode::Omitted && !manifest.assets.is_empty() {
        return Err(corrupt("artwork listed in a lightweight export"));
    }

    std::fs::create_dir(staging).map_err(io)?;
    let db_path = staging.join(DB_FILE_NAME);
    {
        let mut conn = Database::create_at_version(&db_path, manifest.schema_version)?;
        let tx = conn.transaction()?;
        let mut tag_remap = HashMap::new();
        // Load in dependency order regardless of manifest order.
        for table in TABLES {
            let Some(entry) = manifest.tables.iter().find(|t| t.name == *table) else {
                continue;
            };
            let path = portable_path(&entry.path)?;
            let bytes = read_entry(&mut zip, &entries, &path, entry.bytes, MAX_DATA_FILE_BYTES)?;
            if sha256_hex(&bytes) != entry.sha256 {
                return Err(corrupt(format!("{} failed its checksum", entry.name)));
            }
            let data: TableData = serde_json::from_slice(&bytes)
                .map_err(|e| corrupt(format!("{}: {e}", entry.name)))?;
            if data.table != entry.name {
                return Err(corrupt(format!("{} holds the wrong table", entry.path)));
            }
            load_table(&tx, &data, entry.rows, &mut tag_remap)?;
        }
        tx.commit()?;
        validate_staged(&conn, &manifest)?;
    }

    let art_dir = staging.join(ARTWORK_DIR);
    std::fs::create_dir(&art_dir).map_err(io)?;
    for a in &manifest.assets {
        let path = portable_path(&a.path)?;
        let bytes = read_entry(&mut zip, &entries, &path, a.bytes, MAX_ASSET_BYTES)?;
        if sha256_hex(&bytes) != a.sha256 {
            return Err(corrupt(format!("{} failed its checksum", a.path)));
        }
        let name = path.strip_prefix("artwork/").unwrap_or_default();
        if crate::artwork::sniff_image(&bytes).is_none() {
            return Err(corrupt(format!("{} isn't a supported image", a.path)));
        }
        let mut f = File::create_new(art_dir.join(name)).map_err(io)?;
        f.write_all(&bytes).map_err(io)?;
        f.sync_all().map_err(io)?;
    }

    // Upgrade older archives with the normal migrations, then re-check the result.
    let db = Database::open(&db_path)?;
    db.quick_check()?;
    let counts = db.read(counts)?;
    drop(db);
    Ok(Preview {
        format_version: manifest.format_version,
        schema_version: manifest.schema_version,
        current_schema_version: SCHEMA_VERSION,
        app_version: manifest.app_version,
        created_at: manifest.created_at,
        artwork: manifest.artwork,
        counts,
        archive_bytes: meta.len(),
        sha256,
    })
}

/// References, integrity, and artwork consistency of the freshly loaded data.
fn validate_staged(conn: &Connection, manifest: &Manifest) -> AppResult<()> {
    db::check_foreign_keys(conn).map_err(|e| corrupt(format!("broken reference: {e}")))?;
    db::quick_check(conn)?;
    let has = |table: &str| -> AppResult<bool> {
        Ok(conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            [table],
            |r| r.get(0),
        )?)
    };
    if has("tag")? {
        let builtin: i64 = conn.query_row(
            "SELECT COUNT(*) FROM tag WHERE builtin_key IS NOT NULL",
            [],
            |r| r.get(0),
        )?;
        if builtin != 1 {
            return Err(corrupt(
                "the built-in Listen ASAP tag is missing or duplicated",
            ));
        }
    }
    let assets: HashMap<String, u64> = manifest
        .assets
        .iter()
        .map(|a| {
            let p = portable_path(&a.path).unwrap_or_default();
            (p.trim_start_matches("artwork/").to_owned(), a.bytes)
        })
        .collect();
    if has("artwork")? {
        let mut stmt =
            conn.prepare("SELECT file_name, bytes FROM artwork WHERE file_name IS NOT NULL")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        for row in rows {
            let (name, bytes) = row?;
            if assets.get(&name) != Some(&(bytes as u64)) {
                return Err(corrupt(format!(
                    "artwork {name} is missing or the wrong size"
                )));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- activation

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Phase {
    /// Moving the live profile into `restore-previous/`.
    MovingOut,
    /// Moving the staged profile into place.
    MovingIn,
}

#[derive(Serialize, Deserialize)]
struct Journal {
    phase: Phase,
}

pub(crate) fn write_journal(data_dir: &Path, phase: Phase) -> AppResult<()> {
    let tmp = data_dir.join(format!("{JOURNAL}.partial"));
    let mut f = File::create(&tmp).map_err(io)?;
    f.write_all(&serde_json::to_vec(&Journal { phase }).unwrap_or_default())
        .map_err(io)?;
    f.sync_all().map_err(io)?;
    std::fs::rename(&tmp, data_dir.join(JOURNAL)).map_err(io)
}

pub(crate) fn move_out(data_dir: &Path) -> AppResult<()> {
    let previous = data_dir.join(PREVIOUS_DIR);
    if !previous.exists() {
        std::fs::create_dir(&previous).map_err(io)?;
    }
    for item in items() {
        let live = data_dir.join(&item);
        if live.exists() && !previous.join(&item).exists() {
            std::fs::rename(&live, previous.join(&item)).map_err(io)?;
        }
    }
    Ok(())
}

fn move_in(data_dir: &Path) -> AppResult<()> {
    let staging = data_dir.join(STAGING_DIR);
    for item in items() {
        let staged = staging.join(&item);
        if staged.exists() {
            std::fs::rename(&staged, data_dir.join(&item)).map_err(io)?;
        }
    }
    Ok(())
}

/// Swap the staged profile in. The live database must already be closed.
pub fn activate(data_dir: &Path) -> AppResult<()> {
    if !data_dir.join(STAGING_DIR).join(DB_FILE_NAME).is_file() {
        return Err(AppError::Internal("no staged profile to activate".into()));
    }
    write_journal(data_dir, Phase::MovingOut)?;
    move_out(data_dir)?;
    write_journal(data_dir, Phase::MovingIn)?;
    move_in(data_dir)
}

/// Undo an activation that failed before it was finished: the previous profile returns.
pub fn rollback(data_dir: &Path) -> AppResult<()> {
    let previous = data_dir.join(PREVIOUS_DIR);
    if previous.exists() {
        for item in items() {
            let old = previous.join(&item);
            let live = data_dir.join(&item);
            if old.exists() {
                remove_path(&live)?;
                std::fs::rename(&old, &live).map_err(io)?;
            }
        }
    }
    clean(data_dir)
}

/// Remove the journal and leftovers once the new profile is open.
pub fn finish(data_dir: &Path) -> AppResult<()> {
    clean(data_dir)
}

fn clean(data_dir: &Path) -> AppResult<()> {
    remove_path(&data_dir.join(JOURNAL))?;
    remove_path(&data_dir.join(STAGING_DIR))?;
    remove_path(&data_dir.join(PREVIOUS_DIR))
}

fn remove_path(p: &Path) -> AppResult<()> {
    match std::fs::symlink_metadata(p) {
        Err(_) => Ok(()),
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(p).map_err(io),
        Ok(_) => std::fs::remove_file(p).map_err(io),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recovery {
    Nothing,
    /// A staged profile that never started activating was discarded.
    DiscardedStaging,
    /// An interrupted activation was completed.
    Completed,
}

/// Run at startup, before the database opens. A validated staged profile whose activation
/// began is always completed; anything earlier is discarded. Never resets data.
pub fn recover(data_dir: &Path) -> AppResult<Recovery> {
    let journal = data_dir.join(JOURNAL);
    if !journal.exists() {
        if data_dir.join(STAGING_DIR).exists() {
            remove_path(&data_dir.join(STAGING_DIR))?;
            return Ok(Recovery::DiscardedStaging);
        }
        return Ok(Recovery::Nothing);
    }
    let raw = std::fs::read(&journal).map_err(io)?;
    let phase = serde_json::from_slice::<Journal>(&raw)
        .map_err(|e| AppError::StorageCorrupt(format!("restore journal: {e}")))?
        .phase;
    if phase == Phase::MovingOut {
        move_out(data_dir)?;
        write_journal(data_dir, Phase::MovingIn)?;
    }
    move_in(data_dir)?;
    finish(data_dir)?;
    Ok(Recovery::Completed)
}

/// Where restore keeps its staging (inside the data folder, on the same volume, so the
/// final moves are renames).
pub fn staging_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(STAGING_DIR)
}
