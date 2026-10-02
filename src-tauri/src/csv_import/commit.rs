//! Commit approved rows in one transaction. Any failure rolls back everything, and the
//! error names the row. Personal data (ratings, reviews, listens) is never written, and an
//! album already on the Listen List keeps the edition the user chose.

use std::collections::{HashMap, HashSet};

use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use super::mapping::{RowFields, fingerprint, norm};
use super::staging::{self, Decision, Readiness, StagedRow};
use crate::domain::ids::new_id;
use crate::error::{AppError, AppResult};
use crate::library::metadata_import::{self, ImportRequest};
use crate::library::tags;

/// Every tag not yet in the library must be listed in exactly one of these.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TagDecisions {
    pub create: Vec<String>,
    pub skip: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Created,
    AddedEdition,
    Reused,
    Skipped,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowOutcome {
    pub row_number: u32,
    pub outcome: Outcome,
    pub album_id: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitReport {
    pub albums_created: u32,
    pub editions_created: u32,
    pub reused_existing: u32,
    pub added_to_listen_list: u32,
    pub already_on_listen_list: u32,
    pub skipped: u32,
    pub errors: u32,
    pub tags_created: Vec<String>,
    pub tags_skipped: Vec<String>,
    pub tag_links_added: u32,
    pub rows: Vec<RowOutcome>,
}

fn at_row(row: u32, e: AppError) -> AppError {
    AppError::validation("CSV row", format!("row {row}: {e}"))
}

fn list(rows: &[u32]) -> String {
    let shown: Vec<String> = rows.iter().take(10).map(u32::to_string).collect();
    let more = rows.len().saturating_sub(10);
    if more > 0 {
        format!("{} and {more} more", shown.join(", "))
    } else {
        shown.join(", ")
    }
}

pub fn commit(
    tx: &Transaction<'_>,
    session_id: &str,
    tags: &TagDecisions,
) -> AppResult<CommitReport> {
    staging::require_open(tx, session_id)?;
    let mapped: bool = tx.query_row(
        "SELECT mapping IS NOT NULL FROM import_session WHERE id = ?1",
        params![session_id],
        |r| r.get(0),
    )?;
    if !mapped {
        return Err(AppError::validation(
            "import",
            "choose the column mapping first",
        ));
    }
    let rows = staging::all_rows(tx, session_id)?;

    let blocked: Vec<u32> = rows
        .iter()
        .filter(|r| {
            matches!(
                r.readiness,
                Readiness::NeedsLookup | Readiness::NeedsEdition
            )
        })
        .map(|r| r.row_number)
        .collect();
    if !blocked.is_empty() {
        return Err(AppError::validation(
            "import",
            format!(
                "rows {} need a MusicBrainz lookup or an edition choice. Look them up, keep them as manual entries, or skip them",
                list(&blocked)
            ),
        ));
    }

    let unknown = staging::unknown_tags(tx, &rows)?;
    let create: HashSet<String> = tags.create.iter().map(|t| norm(t)).collect();
    let skip: HashSet<String> = tags.skip.iter().map(|t| norm(t)).collect();
    let unconfirmed: Vec<&str> = unknown
        .iter()
        .filter(|t| !create.contains(&norm(&t.name)) && !skip.contains(&norm(&t.name)))
        .map(|t| t.name.as_str())
        .collect();
    if !unconfirmed.is_empty() {
        return Err(AppError::validation(
            "tags",
            format!(
                "confirm whether to create these new tags: {}",
                unconfirmed.join(", ")
            ),
        ));
    }

    let mut report = CommitReport::default();
    let mut ctx = Ctx {
        artists: HashMap::new(),
        albums: HashMap::new(),
        tag_ids: HashMap::new(),
        create,
    };
    for row in &rows {
        let outcome = commit_row(tx, session_id, row, &mut ctx, &mut report)
            .map_err(|e| at_row(row.row_number, e))?;
        report.rows.push(outcome);
    }
    for t in &unknown {
        let bucket = if ctx.create.contains(&norm(&t.name)) {
            &mut report.tags_created
        } else {
            &mut report.tags_skipped
        };
        bucket.push(t.name.clone());
    }
    staging::mark_committed(tx, session_id, &report)?;
    Ok(report)
}

struct Ctx {
    /// Manual artists created in this import, by the exact (normalized) name written.
    artists: HashMap<String, String>,
    /// Manual albums created in this import, by (title, artist ID), so Standard and
    /// Deluxe rows of one album share it.
    albums: HashMap<(String, String), String>,
    tag_ids: HashMap<String, Option<String>>,
    create: HashSet<String>,
}

fn commit_row(
    tx: &Transaction<'_>,
    session_id: &str,
    row: &StagedRow,
    ctx: &mut Ctx,
    report: &mut CommitReport,
) -> AppResult<RowOutcome> {
    let done = |outcome, album_id: Option<String>, detail: Option<String>| RowOutcome {
        row_number: row.row_number,
        outcome,
        album_id,
        detail,
    };
    match row.readiness {
        Readiness::Error => {
            report.errors += 1;
            let detail = row
                .issues
                .iter()
                .map(|i| i.message.clone())
                .collect::<Vec<_>>()
                .join("; ");
            return Ok(done(Outcome::Error, None, Some(detail)));
        }
        Readiness::Skipped => {
            report.skipped += 1;
            let detail = row.duplicate_of.map(|n| format!("duplicate of row {n}"));
            return Ok(done(Outcome::Skipped, None, detail));
        }
        _ => {}
    }
    let f = row
        .fields
        .as_ref()
        .ok_or_else(|| AppError::Internal("importable row without fields".into()))?;
    let fp = fingerprint(f);

    let previous: Option<(String, String)> = tx
        .query_row(
            "SELECT i.album_id, i.edition_id FROM imported_row i
             JOIN edition e ON e.id = i.edition_id WHERE i.fingerprint = ?1",
            params![fp],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;

    let (album_id, edition_id, outcome) = if let Some((a, e)) = previous {
        (a, e, Outcome::Reused)
    } else {
        match &row.decision {
            Decision::UseExisting { .. } => {
                let (a, e, _) = staging::resolve_existing(tx, f, &fp)?.ok_or_else(|| {
                    AppError::Conflict(
                        "the existing album this row matched was removed; review the import again"
                            .into(),
                    )
                })?;
                (a, e, Outcome::Reused)
            }
            Decision::Match { edition_name, .. } => {
                let details = staging::details(tx, session_id, row.row_number)?
                    .ok_or_else(|| AppError::validation("match", "details were not fetched"))?;
                let out = metadata_import::import_release(
                    tx,
                    &ImportRequest {
                        release_group: &details.release_group,
                        release: &details.release,
                        edition_name: edition_name.as_deref(),
                        group_fetched_at: &details.group_fetched_at,
                        release_fetched_at: &details.release_fetched_at,
                    },
                )?;
                let outcome = match (out.album_created, out.edition_created) {
                    (true, _) => Outcome::Created,
                    (false, true) => Outcome::AddedEdition,
                    (false, false) => Outcome::Reused,
                };
                (out.album_id, out.edition_id, outcome)
            }
            Decision::Manual => create_manual(tx, f, ctx)?,
            Decision::Skip => unreachable!("handled by readiness"),
        }
    };
    match outcome {
        Outcome::Created => {
            report.albums_created += 1;
            report.editions_created += 1;
        }
        Outcome::AddedEdition => report.editions_created += 1,
        Outcome::Reused => report.reused_existing += 1,
        _ => {}
    }

    // Keep the user's chosen edition if the album is already listed.
    let added = tx.execute(
        "INSERT INTO listen_list_entry (album_id, edition_id) VALUES (?1, ?2) ON CONFLICT (album_id) DO NOTHING",
        params![album_id, edition_id],
    )?;
    if added > 0 {
        report.added_to_listen_list += 1;
    } else {
        report.already_on_listen_list += 1;
    }

    for tag in &f.tags {
        if let Some(tag_id) = tag_id(tx, tag, ctx)? {
            report.tag_links_added += tx.execute(
                "INSERT INTO album_tag (album_id, tag_id) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
                params![album_id, tag_id],
            )? as u32;
        }
    }

    tx.execute(
        "INSERT INTO imported_row (fingerprint, album_id, edition_id) VALUES (?1, ?2, ?3) ON CONFLICT DO NOTHING",
        params![fp, album_id, edition_id],
    )?;
    Ok(done(outcome, Some(album_id), None))
}

fn tag_id(tx: &Transaction<'_>, name: &str, ctx: &mut Ctx) -> AppResult<Option<String>> {
    let key = norm(name);
    if let Some(cached) = ctx.tag_ids.get(&key) {
        return Ok(cached.clone());
    }
    let id = match tags::find_by_name(tx, name, None)? {
        Some((id, _)) => Some(id),
        None if ctx.create.contains(&key) => Some(tags::ensure(tx, name)?.id),
        None => None,
    };
    ctx.tag_ids.insert(key, id.clone());
    Ok(id)
}

fn create_manual(
    tx: &Transaction<'_>,
    f: &RowFields,
    ctx: &mut Ctx,
) -> AppResult<(String, String, Outcome)> {
    let artist_id = match ctx.artists.get(&norm(&f.artist)) {
        Some(id) => id.clone(),
        None => {
            let id = new_id();
            tx.execute(
                "INSERT INTO artist (id, name) VALUES (?1, ?2)",
                params![id, f.artist],
            )?;
            ctx.artists.insert(norm(&f.artist), id.clone());
            id
        }
    };
    let album_key = (norm(&f.album), artist_id.clone());
    let (album_id, album_created) = match ctx.albums.get(&album_key) {
        Some(id) => (id.clone(), false),
        None => {
            let id = new_id();
            let (date, precision) = match f.year {
                Some(y) => (Some(format!("{y:04}")), "year"),
                None => (None, "unknown"),
            };
            tx.execute(
                "INSERT INTO album (id, title, original_date, original_date_precision) VALUES (?1, ?2, ?3, ?4)",
                params![id, f.album, date, precision],
            )?;
            tx.execute(
                "INSERT INTO album_artist_credit (album_id, position, artist_id) VALUES (?1, 0, ?2)",
                params![id, artist_id],
            )?;
            for field in ["title", "original_date", "credits"] {
                tx.execute(
                    "INSERT INTO metadata_provenance (entity_type, entity_id, field, source) VALUES ('album', ?1, ?2, 'csv')",
                    params![id, field],
                )?;
            }
            ctx.albums.insert(album_key, id.clone());
            (id, true)
        }
    };
    let edition_name = f.edition.clone().unwrap_or_else(|| "Standard".into());
    let existing: Option<String> = tx
        .query_row(
            "SELECT id FROM edition WHERE album_id = ?1 AND name = ?2 COLLATE NOCASE",
            params![album_id, edition_name],
            |r| r.get(0),
        )
        .optional()?;
    let (edition_id, edition_created) = match existing {
        Some(id) => (id, false),
        None => {
            let id = new_id();
            tx.execute(
                "INSERT INTO edition (id, album_id, name) VALUES (?1, ?2, ?3)",
                params![id, album_id, edition_name],
            )?;
            (id, true)
        }
    };
    let outcome = match (album_created, edition_created) {
        (true, _) => Outcome::Created,
        (false, true) => Outcome::AddedEdition,
        (false, false) => Outcome::Reused,
    };
    Ok((album_id, edition_id, outcome))
}
