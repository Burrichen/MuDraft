//! Which tables, rows, and columns sync, with the relationships the merge needs. Columns,
//! keys, foreign keys, and unique indexes are read from the live schema, so the portable
//! form always matches this build's migrations.

use std::collections::HashMap;

use rusqlite::Connection;

use crate::error::{AppError, AppResult};
use crate::profile::export::insertable_columns;

pub struct TableSpec {
    pub name: &'static str,
    /// SQL condition selecting the rows that sync; other rows stay on this computer.
    pub filter: Option<&'static str>,
    /// Child rows merged as one unit per parent (ordered lists and wholesale-replaced sets).
    pub collection: &'static [&'static str],
    /// Columns that only change together (a date and its precision, a status and its time).
    pub groups: &'static [&'static [&'static str]],
}

const fn table(name: &'static str) -> TableSpec {
    TableSpec {
        name,
        filter: None,
        collection: &[],
        groups: &[],
    }
}

/// Portable preferences. Window/device state (sidebar, last route) and the artwork
/// download consent stay per computer.
pub const SETTING_FILTER: &str =
    "key LIKE 'listen.%' OR key IN ('ui.album_layout', 'ui.start_page', 'ui.show_artwork')";

pub fn portable_setting(key: &str) -> bool {
    key.starts_with("listen.")
        || matches!(key, "ui.album_layout" | "ui.start_page" | "ui.show_artwork")
}

/// Synced tables, parents before children. Excluded: caches (`metadata_cache`, downloaded
/// artwork, discovered discographies), CSV staging, idempotency keys, sync bookkeeping.
pub const SPECS: &[TableSpec] = &[
    table("artist"),
    table("genre"),
    TableSpec {
        groups: &[&["original_date", "original_date_precision"]],
        ..table("album")
    },
    TableSpec {
        collection: &["album_id"],
        ..table("album_artist_credit")
    },
    TableSpec {
        collection: &["album_id"],
        ..table("album_genre")
    },
    TableSpec {
        groups: &[&["release_date", "release_date_precision"]],
        ..table("edition")
    },
    table("recording"),
    table("track"),
    TableSpec {
        collection: &["track_id"],
        ..table("track_artist_credit")
    },
    table("tag"),
    table("album_tag"),
    TableSpec {
        groups: &[&["source", "source_ref", "provider_value", "is_override"]],
        ..table("metadata_provenance")
    },
    TableSpec {
        collection: &["album_id"],
        ..table("album_source_tag")
    },
    table("listen_list_entry"),
    table("collection_entry"),
    table("selection_session"),
    TableSpec {
        groups: &[&["status", "resolved_at"]],
        ..table("selection_attempt")
    },
    table("current_selection"),
    TableSpec {
        groups: &[&["listened_on", "date_status"]],
        ..table("listen_event")
    },
    TableSpec {
        collection: &["listen_event_id"],
        ..table("listen_event_track")
    },
    table("track_rating"),
    table("album_review"),
    TableSpec {
        filter: Some(SETTING_FILTER),
        ..table("setting")
    },
    TableSpec {
        filter: Some("source IN ('local', 'removed')"),
        groups: &[&["source", "asset", "mime", "bytes"]],
        ..table("artwork")
    },
    TableSpec {
        filter: Some("source = 'manual'"),
        ..table("catalogue_entry")
    },
    table("imported_row"),
];

/// Portable artwork columns: the image is referenced by content (`asset` = SHA-256), never
/// by a local file name.
pub const ARTWORK_COLUMNS: &[&str] =
    &["owner_type", "owner_id", "source", "asset", "mime", "bytes"];

/// Audit timestamps. They never conflict: when both sides changed one, the larger value
/// is kept (they record when, not what, so no user decision rides on them).
pub const META_COLUMNS: &[&str] = &[
    "created_at",
    "updated_at",
    "added_at",
    "set_at",
    "imported_at",
    "fetched_at",
    "checked_at",
    "started_at",
    "shown_at",
];

/// References whose parent table is named by a column (cleaned up by triggers locally).
pub const POLY_REFS: &[(&str, &str, &str)] = &[
    ("metadata_provenance", "entity_type", "entity_id"),
    ("artwork", "owner_type", "owner_id"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnDelete {
    Cascade,
    SetNull,
    Restrict,
}

#[derive(Debug, Clone)]
pub struct ForeignKey {
    pub parent: String,
    pub from: Vec<usize>,
    pub to: Vec<String>,
    pub on_delete: OnDelete,
}

#[derive(Debug, Clone)]
pub struct Unique {
    pub columns: Vec<usize>,
    pub nocase: Vec<bool>,
}

#[derive(Debug, Clone)]
pub struct TableSchema {
    pub name: &'static str,
    pub filter: Option<&'static str>,
    pub columns: Vec<String>,
    pub pk: Vec<usize>,
    pub meta: Vec<bool>,
    /// Merge units over the non-meta columns: each group, then every other column alone.
    pub units: Vec<Vec<usize>>,
    pub collection: Vec<usize>,
    pub fks: Vec<ForeignKey>,
    pub uniques: Vec<Unique>,
}

impl TableSchema {
    pub fn col(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c == name)
    }
}

#[derive(Debug, Clone)]
pub struct Schema {
    pub tables: Vec<TableSchema>,
    index: HashMap<&'static str, usize>,
}

impl Schema {
    pub fn load(conn: &Connection) -> AppResult<Self> {
        let mut tables = Vec::with_capacity(SPECS.len());
        for spec in SPECS {
            tables.push(load_table(conn, spec)?);
        }
        let index = tables
            .iter()
            .enumerate()
            .map(|(i, t)| (t.name, i))
            .collect();
        Ok(Self { tables, index })
    }

    pub fn table(&self, name: &str) -> Option<&TableSchema> {
        self.index.get(name).map(|&i| &self.tables[i])
    }
}

fn load_table(conn: &Connection, spec: &TableSpec) -> AppResult<TableSchema> {
    let name = spec.name;
    let columns: Vec<String> = if name == "artwork" {
        ARTWORK_COLUMNS.iter().map(|c| (*c).to_owned()).collect()
    } else {
        insertable_columns(conn, name)?
    };
    let pos = |c: &str| -> AppResult<usize> {
        columns
            .iter()
            .position(|x| x == c)
            .ok_or_else(|| AppError::Internal(format!("{name} has no column {c}")))
    };
    let mut stmt =
        conn.prepare("SELECT name FROM pragma_table_info(?1) WHERE pk > 0 ORDER BY pk")?;
    let pk_names = stmt
        .query_map([name], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if pk_names.is_empty() {
        return Err(AppError::Internal(format!("{name} has no primary key")));
    }
    let pk = pk_names
        .iter()
        .map(|c| pos(c))
        .collect::<AppResult<Vec<_>>>()?;
    let meta: Vec<bool> = columns
        .iter()
        .map(|c| META_COLUMNS.contains(&c.as_str()))
        .collect();
    let mut grouped = vec![false; columns.len()];
    let mut units = Vec::new();
    for group in spec.groups {
        let unit = group
            .iter()
            .map(|c| pos(c))
            .collect::<AppResult<Vec<_>>>()?;
        for &i in &unit {
            grouped[i] = true;
        }
        units.push(unit);
    }
    for i in 0..columns.len() {
        if !grouped[i] && !meta[i] {
            units.push(vec![i]);
        }
    }
    let collection = spec
        .collection
        .iter()
        .map(|c| pos(c))
        .collect::<AppResult<Vec<_>>>()?;

    let mut fks: Vec<ForeignKey> = Vec::new();
    if name != "artwork" {
        let mut stmt = conn.prepare(
            "SELECT id, \"table\", \"from\", \"to\", on_delete FROM pragma_foreign_key_list(?1) ORDER BY id, seq",
        )?;
        let rows = stmt
            .query_map([name], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut last_id = None;
        for (id, parent, from, to, on_delete) in rows {
            if last_id != Some(id) {
                let on_delete = match on_delete.as_str() {
                    "CASCADE" => OnDelete::Cascade,
                    "SET NULL" => OnDelete::SetNull,
                    _ => OnDelete::Restrict,
                };
                fks.push(ForeignKey {
                    parent: parent.clone(),
                    from: Vec::new(),
                    to: Vec::new(),
                    on_delete,
                });
                last_id = Some(id);
            }
            let fk = fks.last_mut().expect("pushed above");
            fk.from.push(pos(&from)?);
            fk.to.push(to.unwrap_or_else(|| "id".to_owned()));
        }
        if let Some(fk) = fks
            .iter()
            .find(|f| !SPECS.iter().any(|s| s.name == f.parent))
        {
            return Err(AppError::Internal(format!(
                "{name} references unsynced table {}",
                fk.parent
            )));
        }
    }

    let mut uniques = Vec::new();
    if name != "artwork" {
        let mut stmt = conn.prepare(
            "SELECT name FROM pragma_index_list(?1) WHERE \"unique\" = 1 AND origin != 'pk' AND partial = 0",
        )?;
        let indexes = stmt
            .query_map([name], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        for index in indexes {
            let mut stmt = conn.prepare(
                "SELECT cid, name, coll FROM pragma_index_xinfo(?1) WHERE key = 1 ORDER BY seqno",
            )?;
            let cols = stmt
                .query_map([&index], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            // Expression indexes can't be evaluated here; validation still enforces them.
            if cols.iter().any(|(cid, n, _)| *cid < 0 || n.is_none()) {
                continue;
            }
            let mut u = Unique {
                columns: Vec::new(),
                nocase: Vec::new(),
            };
            for (_, n, coll) in cols {
                u.columns.push(pos(&n.unwrap_or_default())?);
                u.nocase
                    .push(coll.is_some_and(|c| c.eq_ignore_ascii_case("NOCASE")));
            }
            uniques.push(u);
        }
    }
    Ok(TableSchema {
        name,
        filter: spec.filter,
        columns,
        pk,
        meta,
        units,
        collection,
        fks,
        uniques,
    })
}
