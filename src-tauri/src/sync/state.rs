//! The portable sync state: rows of every synced table keyed by primary key, plus
//! tombstones (keys that were deliberately deleted). It is encoded as deterministic JSON
//! files: the same state always produces byte-identical files, so unchanged data is never
//! uploaded again.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::schema::{Schema, TableSchema, portable_setting};
use crate::db::{self, Database, SCHEMA_VERSION};
use crate::domain::ids::parse_uuid;
use crate::error::{AppError, AppResult};
use crate::library::catalogue::LISTEN_ASAP_TAG_ID;
use crate::profile::export::to_json;
use crate::profile::restore::sql_value;
use crate::profile::{TableData, sha256_hex};

pub const FORMAT: &str = "mudraft-sync";
pub const FORMAT_VERSION: u32 = 1;

pub const README_PATH: &str = "sync/README.md";
pub const FORMAT_PATH: &str = "sync/format.json";
pub const STATE_DIR: &str = "sync/state/";
pub const TOMBSTONES_PATH: &str = "sync/state/tombstones.json";
pub const ASSET_DIR: &str = "sync/assets/";

pub const README: &str = "# MuDraft sync data\n\nWritten by MuDraft's GitHub sync. Don't edit these files by hand: \
MuDraft validates them and refuses damaged data.\n\nKeep this repository private. Private GitHub storage is \
not end-to-end encrypted: GitHub can read it.\n";

pub type Row = Vec<Value>;
type Rows = BTreeMap<String, Row>;

static NO_ROWS: Rows = BTreeMap::new();

#[derive(Debug, Clone, Default, PartialEq)]
pub struct State {
    /// Table name → primary-key JSON → row (values in schema column order).
    pub tables: BTreeMap<String, Rows>,
    /// Table name → keys deleted at some point and not live now.
    pub tombstones: BTreeMap<String, BTreeSet<String>>,
}

impl State {
    pub fn empty(schema: &Schema) -> Self {
        Self {
            tables: schema
                .tables
                .iter()
                .map(|t| (t.name.to_owned(), Rows::new()))
                .collect(),
            tombstones: BTreeMap::new(),
        }
    }

    pub fn rows(&self, table: &str) -> &Rows {
        self.tables.get(table).unwrap_or(&NO_ROWS)
    }

    pub fn rows_mut(&mut self, table: &str) -> &mut Rows {
        self.tables.entry(table.to_owned()).or_default()
    }

    pub fn row(&self, table: &str, key: &str) -> Option<&Row> {
        self.rows(table).get(key)
    }

    pub fn row_count(&self) -> usize {
        self.tables.values().map(BTreeMap::len).sum()
    }
}

pub fn key_of(ts: &TableSchema, row: &[Value]) -> String {
    let values: Vec<&Value> = ts.pk.iter().map(|&i| &row[i]).collect();
    serde_json::to_string(&values).unwrap_or_default()
}

fn bad(reason: impl Into<String>) -> AppError {
    AppError::validation("sync data", reason)
}

pub fn mime_extension(mime: &str) -> Option<&'static str> {
    match mime {
        "image/jpeg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        _ => None,
    }
}

pub fn asset_path(sha256: &str, mime: &str) -> String {
    format!(
        "{ASSET_DIR}{sha256}.{}",
        mime_extension(mime).unwrap_or("bin")
    )
}

fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

// ---------------------------------------------------------------- local snapshot

pub struct Snapshot {
    pub state: State,
    /// SHA-256 → local artwork file name, for every custom image in `state`.
    pub assets: HashMap<String, String>,
    /// Rows that couldn't be read (a custom image missing on disk). The merge treats them
    /// as unchanged, so a damaged file is never mistaken for a deletion.
    pub unknown: BTreeSet<(String, String)>,
}

enum ArtworkFiles<'a> {
    /// Hash the real files in this folder.
    Dir(&'a Path),
    /// Scratch databases store `<sha256>.<ext>` placeholders instead of files.
    Placeholders,
}

/// Read the synced rows. Call inside a transaction so every table is one point in time.
pub fn snapshot(conn: &Connection, schema: &Schema, artwork_dir: &Path) -> AppResult<Snapshot> {
    let mut snap = Snapshot {
        state: State::empty(schema),
        assets: HashMap::new(),
        unknown: BTreeSet::new(),
    };
    for ts in &schema.tables {
        read_table(conn, ts, ArtworkFiles::Dir(artwork_dir), &mut snap)?;
    }
    Ok(snap)
}

fn read_table(
    conn: &Connection,
    ts: &TableSchema,
    files: ArtworkFiles<'_>,
    snap: &mut Snapshot,
) -> AppResult<()> {
    let filter = ts.filter.map(|f| format!(" WHERE {f}")).unwrap_or_default();
    if ts.name == "artwork" {
        let mut stmt = conn.prepare(&format!(
            "SELECT owner_type, owner_id, source, file_name, mime, bytes FROM artwork{filter}"
        ))?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<i64>>(5)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for (owner_type, owner_id, source, file, mime, stored_bytes) in rows {
            let mut row: Row = vec![
                owner_type.into(),
                owner_id.into(),
                source.clone().into(),
                Value::Null,
                Value::Null,
                Value::Null,
            ];
            if source == "local" {
                let read = match (&files, &file) {
                    (ArtworkFiles::Dir(dir), Some(name)) => read_image(dir, name),
                    (ArtworkFiles::Placeholders, Some(name)) => name
                        .split_once('.')
                        .filter(|(sha, _)| is_sha256(sha))
                        .zip(mime.clone())
                        .map(|((sha, _), m)| {
                            (sha.to_owned(), m, stored_bytes.unwrap_or_default() as u64)
                        }),
                    _ => None,
                };
                let key = key_of(ts, &row);
                let Some((sha, mime, bytes)) = read else {
                    snap.unknown.insert((ts.name.to_owned(), key));
                    continue;
                };
                if let Some(name) = file {
                    snap.assets.insert(sha.clone(), name);
                }
                row[3] = sha.into();
                row[4] = mime.into();
                row[5] = bytes.into();
            }
            snap.state.rows_mut(ts.name).insert(key_of(ts, &row), row);
        }
        return Ok(());
    }
    let list = ts
        .columns
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let mut stmt = conn.prepare(&format!("SELECT {list} FROM \"{}\"{filter}", ts.name))?;
    let mut rows = stmt.query([])?;
    let out = snap.state.rows_mut(ts.name);
    while let Some(r) = rows.next()? {
        let mut row = Vec::with_capacity(ts.columns.len());
        for i in 0..ts.columns.len() {
            row.push(to_json(r.get_ref(i)?, ts.name)?);
        }
        out.insert(key_of(ts, &row), row);
    }
    Ok(())
}

/// (sha256, mime, bytes) of a valid local image, or `None` when missing or damaged.
fn read_image(dir: &Path, name: &str) -> Option<(String, String, u64)> {
    if !crate::profile::safe_file_name(name) {
        return None;
    }
    let bytes = std::fs::read(dir.join(name)).ok()?;
    let mime = crate::artwork::validate_image(&bytes).ok()?;
    Some((sha256_hex(&bytes), mime.to_owned(), bytes.len() as u64))
}

// ---------------------------------------------------------------- encoding

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileEntry {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssetEntry {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
    pub mime: String,
}

/// `sync/format.json`: the format marker and the checksum of every other file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FormatFile {
    pub format: String,
    pub format_version: u32,
    /// Random identity chosen when the repository was initialised. A computer linked to
    /// one sync never merges with a different one.
    pub sync_id: String,
    pub schema_version: i64,
    pub app_version: String,
    pub files: Vec<FileEntry>,
    pub assets: Vec<AssetEntry>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TombstonesFile {
    tables: BTreeMap<String, Vec<Value>>,
}

pub struct Encoded {
    /// Path → content for README, format marker, and state files.
    pub files: BTreeMap<String, Vec<u8>>,
    /// Path → custom images the state references (content addressed).
    pub assets: BTreeMap<String, AssetEntry>,
}

impl Encoded {
    pub fn total_bytes(&self) -> u64 {
        self.files.values().map(|f| f.len() as u64).sum()
    }
}

fn to_vec<T: Serialize>(value: &T) -> AppResult<Vec<u8>> {
    serde_json::to_vec(value)
        .map_err(|e| AppError::Internal(format!("cannot encode sync data: {e}")))
}

pub fn table_path(table: &str) -> String {
    format!("{STATE_DIR}{table}.json")
}

/// Encode `state` deterministically (rows in key order, no timestamps).
pub fn encode(schema: &Schema, state: &State, sync_id: &str) -> AppResult<Encoded> {
    let mut files = BTreeMap::new();
    let mut entries = Vec::new();
    for ts in &schema.tables {
        let data = TableData {
            table: ts.name.to_owned(),
            columns: ts.columns.clone(),
            rows: state.rows(ts.name).values().cloned().collect(),
        };
        let bytes = to_vec(&data)?;
        let path = table_path(ts.name);
        entries.push(FileEntry {
            path: path.clone(),
            bytes: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
        });
        files.insert(path, bytes);
    }
    let tomb = TombstonesFile {
        tables: state
            .tombstones
            .iter()
            .filter(|(_, keys)| !keys.is_empty())
            .map(|(t, keys)| {
                let parsed = keys
                    .iter()
                    .map(|k| serde_json::from_str(k).unwrap_or(Value::Null))
                    .collect();
                (t.clone(), parsed)
            })
            .collect(),
    };
    let bytes = to_vec(&tomb)?;
    entries.push(FileEntry {
        path: TOMBSTONES_PATH.to_owned(),
        bytes: bytes.len() as u64,
        sha256: sha256_hex(&bytes),
    });
    files.insert(TOMBSTONES_PATH.to_owned(), bytes);

    let mut assets = BTreeMap::new();
    for row in state.rows("artwork").values() {
        if let (Some(sha), Some(mime), Some(bytes)) =
            (row[3].as_str(), row[4].as_str(), row[5].as_u64())
        {
            let path = asset_path(sha, mime);
            assets.insert(
                path.clone(),
                AssetEntry {
                    path,
                    bytes,
                    sha256: sha.to_owned(),
                    mime: mime.to_owned(),
                },
            );
        }
    }
    let format = FormatFile {
        format: FORMAT.to_owned(),
        format_version: FORMAT_VERSION,
        sync_id: sync_id.to_owned(),
        schema_version: SCHEMA_VERSION,
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        files: entries,
        assets: assets.values().cloned().collect(),
    };
    files.insert(
        FORMAT_PATH.to_owned(),
        serde_json::to_vec_pretty(&format)
            .map_err(|e| AppError::Internal(format!("cannot encode sync format: {e}")))?,
    );
    files.insert(README_PATH.to_owned(), README.as_bytes().to_vec());
    Ok(Encoded { files, assets })
}

// ---------------------------------------------------------------- decoding

/// Parse `sync/format.json`, checking the marker and version before the strict shape so
/// unrelated or newer data gets a clear message.
pub fn parse_format(bytes: &[u8]) -> AppResult<FormatFile> {
    let probe: Value = serde_json::from_slice(bytes)
        .map_err(|e| AppError::SyncSetup(format!("sync/format.json isn't valid ({e})")))?;
    if probe.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(AppError::SyncSetup(
            "the repository's sync folder wasn't written by MuDraft".into(),
        ));
    }
    let version = probe.get("formatVersion").and_then(Value::as_u64);
    if version.is_none_or(|v| v > u64::from(FORMAT_VERSION)) {
        return Err(AppError::SyncSetup(
            "the sync data was written by a newer MuDraft; update MuDraft on this computer".into(),
        ));
    }
    let schema = probe.get("schemaVersion").and_then(Value::as_i64);
    if schema.is_none_or(|v| v > SCHEMA_VERSION) {
        return Err(AppError::SyncSetup(
            "the sync data comes from a newer MuDraft; update MuDraft on this computer".into(),
        ));
    }
    let format: FormatFile = serde_json::from_slice(bytes)
        .map_err(|e| AppError::SyncSetup(format!("sync/format.json is damaged ({e})")))?;
    if format.schema_version < 1
        || parse_uuid("syncId", &format.sync_id).ok() != Some(format.sync_id.clone())
    {
        return Err(AppError::SyncSetup("sync/format.json is damaged".into()));
    }
    Ok(format)
}

pub fn parse_table(bytes: &[u8]) -> AppResult<TableData> {
    serde_json::from_slice(bytes).map_err(|e| bad(format!("a state file is damaged ({e})")))
}

pub fn parse_tombstones(bytes: &[u8]) -> AppResult<BTreeMap<String, Vec<Value>>> {
    let file: TombstonesFile =
        serde_json::from_slice(bytes).map_err(|e| bad(format!("tombstones are damaged ({e})")))?;
    Ok(file.tables)
}

/// Validate decoded tables (already at this build's schema) and build a [`State`].
/// Everything a database constraint would catch is checked again by [`validate`].
pub fn build_state(
    schema: &Schema,
    tables: Vec<TableData>,
    tombstones: BTreeMap<String, Vec<Value>>,
) -> AppResult<State> {
    let mut state = State::empty(schema);
    let mut seen = BTreeSet::new();
    for data in tables {
        let ts = schema
            .table(&data.table)
            .ok_or_else(|| bad(format!("unknown table {}", data.table)))?;
        if !seen.insert(ts.name) {
            return Err(bad(format!("{} appears twice", ts.name)));
        }
        if data.columns != ts.columns {
            return Err(bad(format!("{} has unexpected columns", ts.name)));
        }
        let id_col = ts.col("id");
        let rows = state.rows_mut(ts.name);
        for row in data.rows {
            if row.len() != ts.columns.len() {
                return Err(bad(format!("a row in {} has the wrong length", ts.name)));
            }
            if row.iter().any(|v| v.is_array() || v.is_object()) {
                return Err(bad(format!("nested value in {}", ts.name)));
            }
            if let Some(i) = id_col {
                let raw = row[i]
                    .as_str()
                    .ok_or_else(|| bad(format!("bad id in {}", ts.name)))?;
                if parse_uuid("id", raw)? != raw {
                    return Err(bad(format!("non-canonical id in {}", ts.name)));
                }
            }
            check_row(ts, &row)?;
            if rows.insert(key_of(ts, &row), row).is_some() {
                return Err(bad(format!("duplicate key in {}", ts.name)));
            }
        }
    }
    if let Some(missing) = schema.tables.iter().find(|t| !seen.contains(t.name)) {
        return Err(bad(format!("{} is missing", missing.name)));
    }
    let tag = schema.table("tag").ok_or_else(|| bad("no tag table"))?;
    let builtin_col = tag
        .col("builtin_key")
        .ok_or_else(|| bad("tag without builtin_key"))?;
    let builtins: Vec<&Row> = state
        .rows("tag")
        .values()
        .filter(|r| !r[builtin_col].is_null())
        .collect();
    let builtin_ok = builtins.len() == 1
        && builtins[0][builtin_col] == "listen_asap"
        && tag
            .col("id")
            .is_some_and(|i| builtins[0][i] == LISTEN_ASAP_TAG_ID);
    if !builtin_ok {
        return Err(bad("the built-in Listen ASAP tag is missing or changed"));
    }
    for (table, keys) in tombstones {
        let ts = schema
            .table(&table)
            .ok_or_else(|| bad(format!("tombstones for unknown table {table}")))?;
        let set = state.tombstones.entry(ts.name.to_owned()).or_default();
        for key in keys {
            if key.as_array().is_none_or(|k| k.len() != ts.pk.len()) {
                return Err(bad(format!("bad tombstone in {table}")));
            }
            set.insert(key.to_string());
        }
    }
    Ok(state)
}

fn check_row(ts: &TableSchema, row: &[Value]) -> AppResult<()> {
    match ts.name {
        "setting" => {
            let ok = ts
                .col("key")
                .and_then(|i| row[i].as_str())
                .is_some_and(portable_setting);
            if !ok {
                return Err(bad("a setting that doesn't sync"));
            }
        }
        "catalogue_entry" => {
            if ts.col("source").map(|i| &row[i]) != Some(&Value::from("manual")) {
                return Err(bad("a discovered catalogue entry"));
            }
        }
        "artwork" => {
            let ok = match row[2].as_str() {
                Some("local") => {
                    row[3].as_str().is_some_and(is_sha256)
                        && row[4].as_str().and_then(mime_extension).is_some()
                        && row[5].as_u64().is_some_and(|b| {
                            b > 0 && b <= crate::artwork::http::MAX_IMAGE_BYTES as u64
                        })
                }
                Some("removed") => row[3].is_null() && row[4].is_null() && row[5].is_null(),
                _ => false,
            };
            if !ok || !matches!(row[0].as_str(), Some("album" | "edition")) {
                return Err(bad("an invalid artwork record"));
            }
        }
        _ => {}
    }
    Ok(())
}

/// The locally stored common base: the state as of the last successful sync.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoredBase {
    pub schema_version: i64,
    pub tables: Vec<TableData>,
    pub tombstones: BTreeMap<String, Vec<Value>>,
}

pub fn store_base(schema: &Schema, state: &State) -> AppResult<String> {
    let base = StoredBase {
        schema_version: SCHEMA_VERSION,
        tables: schema
            .tables
            .iter()
            .map(|ts| TableData {
                table: ts.name.to_owned(),
                columns: ts.columns.clone(),
                rows: state.rows(ts.name).values().cloned().collect(),
            })
            .collect(),
        tombstones: state
            .tombstones
            .iter()
            .map(|(t, keys)| {
                (
                    t.clone(),
                    keys.iter()
                        .map(|k| serde_json::from_str(k).unwrap_or(Value::Null))
                        .collect(),
                )
            })
            .collect(),
    };
    String::from_utf8(to_vec(&base)?).map_err(|e| AppError::Internal(e.to_string()))
}

// ---------------------------------------------------------------- database round trips

fn pk_names(conn: &Connection, table: &str) -> AppResult<Vec<String>> {
    let mut stmt =
        conn.prepare("SELECT name FROM pragma_table_info(?1) WHERE pk > 0 ORDER BY pk")?;
    Ok(stmt
        .query_map([table], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?)
}

pub(crate) fn upsert_sql(table: &str, columns: &[String], pk: &[String]) -> String {
    let quoted = |c: &String| format!("\"{c}\"");
    let list = columns.iter().map(quoted).collect::<Vec<_>>().join(", ");
    let marks = vec!["?"; columns.len()].join(", ");
    let conflict = pk.iter().map(quoted).collect::<Vec<_>>().join(", ");
    let updates: Vec<String> = columns
        .iter()
        .filter(|c| !pk.contains(c))
        .map(|c| format!("\"{c}\" = excluded.\"{c}\""))
        .collect();
    let action = if updates.is_empty() {
        "NOTHING".to_owned()
    } else {
        format!("UPDATE SET {}", updates.join(", "))
    };
    format!(
        "INSERT INTO \"{table}\" ({list}) VALUES ({marks}) ON CONFLICT ({conflict}) DO {action}"
    )
}

/// Artwork rows as stored in a scratch database: the asset becomes a placeholder file name.
fn artwork_db_row(row: &[Value]) -> (Vec<String>, Vec<Value>) {
    let file = match (row[3].as_str(), row[4].as_str()) {
        (Some(sha), Some(mime)) => {
            format!("{sha}.{}", mime_extension(mime).unwrap_or("bin")).into()
        }
        _ => Value::Null,
    };
    let cols = [
        "owner_type",
        "owner_id",
        "source",
        "file_name",
        "mime",
        "bytes",
    ]
    .map(str::to_owned)
    .to_vec();
    (
        cols,
        vec![
            row[0].clone(),
            row[1].clone(),
            row[2].clone(),
            file,
            row[4].clone(),
            row[5].clone(),
        ],
    )
}

fn load_rows(tx: &Connection, table: &str, columns: &[String], rows: &[Row]) -> AppResult<()> {
    let pk = pk_names(tx, table)?;
    if table == "artwork" {
        for row in rows {
            let (cols, values) = artwork_db_row(row);
            let values: Vec<_> = values
                .iter()
                .map(|v| sql_value(v, table))
                .collect::<AppResult<_>>()?;
            tx.execute(
                &upsert_sql(table, &cols, &pk),
                rusqlite::params_from_iter(values),
            )
            .map_err(|e| bad(format!("{table}: {e}")))?;
        }
        return Ok(());
    }
    let mut stmt = tx
        .prepare(&upsert_sql(table, columns, &pk))
        .map_err(|e| bad(format!("{table}: {e}")))?;
    for row in rows {
        let values: Vec<_> = row
            .iter()
            .map(|v| sql_value(v, table))
            .collect::<AppResult<_>>()?;
        stmt.execute(rusqlite::params_from_iter(values))
            .map_err(|e| bad(format!("{table}: {e}")))?;
    }
    Ok(())
}

/// Load `state` into a fresh in-memory database at this schema and verify every
/// constraint, reference, and index. Nothing is applied or uploaded unless this passes.
pub fn validate(schema: &Schema, state: &State) -> AppResult<()> {
    let mut conn = Database::create_at_version(Path::new(":memory:"), SCHEMA_VERSION)?;
    let tx = conn.transaction()?;
    for ts in &schema.tables {
        let rows: Vec<Row> = state.rows(ts.name).values().cloned().collect();
        load_rows(&tx, ts.name, &ts.columns, &rows)?;
    }
    db::check_foreign_keys(&tx).map_err(|e| bad(format!("broken reference: {e}")))?;
    tx.commit()?;
    db::quick_check(&conn)
}

/// Bring tables written at an older schema up to this one using the app's normal
/// migrations, in a scratch database under `scratch` (removed afterwards).
pub fn upgrade(tables: Vec<TableData>, from: i64, scratch: &Path) -> AppResult<Vec<TableData>> {
    if from == SCHEMA_VERSION {
        return Ok(tables);
    }
    if !(1..SCHEMA_VERSION).contains(&from) {
        return Err(bad(format!("schema version {from} can't be read")));
    }
    std::fs::create_dir_all(scratch).map_err(|e| AppError::StorageUnavailable(e.to_string()))?;
    let result = (|| {
        let path = scratch.join("upgrade.sqlite3");
        {
            let mut conn = Database::create_at_version(&path, from)?;
            let tx = conn.transaction()?;
            for data in &tables {
                load_rows(&tx, &data.table, &data.columns, &data.rows)?;
            }
            db::check_foreign_keys(&tx).map_err(|e| bad(format!("broken reference: {e}")))?;
            tx.commit()?;
        }
        let db = Database::open(&path)?;
        db.read(|c| {
            let schema = Schema::load(c)?;
            let mut snap = Snapshot {
                state: State::empty(&schema),
                assets: HashMap::new(),
                unknown: BTreeSet::new(),
            };
            for ts in &schema.tables {
                read_table(c, ts, ArtworkFiles::Placeholders, &mut snap)?;
            }
            Ok(schema
                .tables
                .iter()
                .map(|ts| TableData {
                    table: ts.name.to_owned(),
                    columns: ts.columns.clone(),
                    rows: snap.state.rows(ts.name).values().cloned().collect(),
                })
                .collect())
        })
    })();
    let _ = std::fs::remove_dir_all(scratch);
    result
}
