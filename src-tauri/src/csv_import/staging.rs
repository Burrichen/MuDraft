//! Staging sessions in SQLite. Everything here reads the library but never writes it.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use super::commit::CommitReport;
use super::mapping::{self, ColumnMapping, RowFields, RowIssue, norm};
use super::parse::ParsedFile;
use crate::domain::ids::new_id;
use crate::error::{AppError, AppResult};
use crate::metadata::{EditionCandidate, ReleaseDetail, ReleaseGroupCandidate, ReleaseGroupDetail};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChosenBy {
    /// IDs written in the CSV itself.
    CsvIds,
    /// Picked by the user in the match dialog.
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExistingReason {
    /// Same row content was imported before.
    PreviousImport,
    /// The edition's MusicBrainz release ID is already in the library.
    ReleaseId,
    /// Same release group, and the edition name (or only edition) matches.
    ReleaseGroupEdition,
    /// Same title, artist, and edition name as an existing album. Shown for review.
    SameName,
}

/// How a row will be committed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum Decision {
    Match {
        release_group_id: Option<String>,
        release_id: Option<String>,
        edition_name: Option<String>,
        chosen_by: ChosenBy,
    },
    Manual,
    Skip,
    UseExisting {
        album_id: String,
        edition_id: String,
        reason: ExistingReason,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum Candidates {
    Groups {
        candidates: Vec<ReleaseGroupCandidate>,
    },
    Editions {
        editions: Vec<EditionCandidate>,
    },
    Failed {
        message: String,
    },
}

/// Provider data fetched for a match, so commit is purely local.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchDetails {
    pub release_group: ReleaseGroupDetail,
    pub release: ReleaseDetail,
    pub group_fetched_at: String,
    pub release_fetched_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Readiness {
    Ready,
    /// Matched by ID but the details haven't been fetched (needs online lookup).
    NeedsLookup,
    /// Release group known, edition not chosen yet.
    NeedsEdition,
    Error,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExistingSummary {
    pub album_title: String,
    pub edition_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedRow {
    pub row_number: u32,
    pub raw: Vec<String>,
    pub fields: Option<RowFields>,
    pub issues: Vec<RowIssue>,
    pub duplicate_of: Option<u32>,
    pub decision: Decision,
    pub enrichment: String,
    pub candidates: Option<Candidates>,
    pub has_details: bool,
    pub readiness: Readiness,
    pub existing: Option<ExistingSummary>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub rows: u32,
    pub ready: u32,
    pub errors: u32,
    pub duplicates: u32,
    pub existing: u32,
    pub matched: u32,
    pub manual: u32,
    pub skipped: u32,
    pub needs_attention: u32,
    pub with_candidates: u32,
    pub lookup_pending: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagCount {
    pub name: String,
    pub rows: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub session_id: String,
    pub file_name: String,
    pub headers: Vec<String>,
    pub mapping: Option<ColumnMapping>,
    pub suggested_mapping: ColumnMapping,
    pub sample: Vec<Vec<String>>,
    pub counts: Counts,
    /// Tags that don't exist yet and must be confirmed before commit.
    pub unknown_tags: Vec<TagCount>,
    pub status: String,
    pub report: Option<CommitReport>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowFilter {
    All,
    Attention,
    Errors,
    Duplicates,
    Existing,
    Candidates,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RowPage {
    pub rows: Vec<StagedRow>,
    pub total: u32,
}

fn json<T: Serialize>(v: &T) -> AppResult<String> {
    serde_json::to_string(v).map_err(|e| AppError::Internal(e.to_string()))
}

fn from_json<T: serde::de::DeserializeOwned>(raw: &str) -> AppResult<T> {
    serde_json::from_str(raw)
        .map_err(|e| AppError::StorageCorrupt(format!("staged import data is unreadable: {e}")))
}

pub fn require_open(conn: &Connection, session_id: &str) -> AppResult<()> {
    let status: Option<String> = conn
        .query_row(
            "SELECT status FROM import_session WHERE id = ?1",
            params![session_id],
            |r| r.get(0),
        )
        .optional()?;
    match status.as_deref() {
        None => Err(AppError::not_found("import", session_id)),
        Some("open") => Ok(()),
        Some(_) => Err(AppError::Conflict(
            "this import was already committed".into(),
        )),
    }
}

pub fn create_session(
    tx: &Transaction<'_>,
    file_name: &str,
    parsed: &ParsedFile,
) -> AppResult<String> {
    let id = new_id();
    tx.execute(
        "INSERT INTO import_session (id, file_name, headers) VALUES (?1, ?2, ?3)",
        params![id, file_name, json(&parsed.headers)?],
    )?;
    let mut insert = tx.prepare(
        "INSERT INTO import_row (session_id, row_number, raw, errors) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for r in &parsed.records {
        let errors: Vec<RowIssue> = r
            .problem
            .iter()
            .map(|p| RowIssue {
                field: "row".into(),
                message: format!("Row {p}"),
            })
            .collect();
        insert.execute(params![id, r.row_number, json(&r.cells)?, json(&errors)?])?;
    }
    Ok(id)
}

/// Existing library records this row refers to, checked in order of certainty.
pub fn resolve_existing(
    conn: &Connection,
    fields: &RowFields,
    fingerprint: &str,
) -> AppResult<Option<(String, String, ExistingReason)>> {
    if let Some((a, e)) = conn
        .query_row(
            "SELECT i.album_id, i.edition_id FROM imported_row i
             JOIN edition e ON e.id = i.edition_id AND e.album_id = i.album_id WHERE i.fingerprint = ?1",
            params![fingerprint],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
    {
        return Ok(Some((a, e, ExistingReason::PreviousImport)));
    }
    if let Some(rid) = &fields.release_id
        && let Some((e, a)) = conn
            .query_row(
                "SELECT id, album_id FROM edition WHERE musicbrainz_release_id = ?1",
                params![rid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
    {
        return Ok(Some((a, e, ExistingReason::ReleaseId)));
    }
    if fields.release_id.is_none()
        && let Some(rgid) = &fields.release_group_id
        && let Some(album_id) = conn
            .query_row(
                "SELECT id FROM album WHERE musicbrainz_release_group_id = ?1",
                params![rgid],
                |r| r.get::<_, String>(0),
            )
            .optional()?
        && let Some(edition_id) = pick_edition(conn, &album_id, fields.edition.as_deref())?
    {
        return Ok(Some((
            album_id,
            edition_id,
            ExistingReason::ReleaseGroupEdition,
        )));
    }
    if fields.release_group_id.is_none() && fields.release_id.is_none() {
        let mut stmt = conn.prepare(
            "SELECT a.id, ar.name FROM album a
             JOIN album_artist_credit c ON c.album_id = a.id AND c.position = 0
             JOIN artist ar ON ar.id = c.artist_id
             WHERE a.title = ?1 COLLATE NOCASE ORDER BY a.created_at",
        )?;
        let candidates: Vec<(String, String)> = stmt
            .query_map(params![fields.album], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        for (album_id, artist) in candidates {
            if norm(&artist) == norm(&fields.artist)
                && let Some(edition_id) = pick_edition(conn, &album_id, fields.edition.as_deref())?
            {
                return Ok(Some((album_id, edition_id, ExistingReason::SameName)));
            }
        }
    }
    Ok(None)
}

/// Edition by name (default "Standard"), or the only edition when no name was given.
fn pick_edition(
    conn: &Connection,
    album_id: &str,
    name: Option<&str>,
) -> AppResult<Option<String>> {
    let mut stmt = conn.prepare("SELECT id, name FROM edition WHERE album_id = ?1")?;
    let editions: Vec<(String, String)> = stmt
        .query_map(params![album_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let wanted = norm(name.unwrap_or("Standard"));
    if let Some((id, _)) = editions.iter().find(|(_, n)| norm(n) == wanted) {
        return Ok(Some(id.clone()));
    }
    Ok(match (name, editions.as_slice()) {
        (None, [(id, _)]) => Some(id.clone()),
        _ => None,
    })
}

/// Apply a column mapping: validate every row, find duplicates and existing records, and
/// reset decisions and enrichment to safe defaults.
pub fn apply_mapping(
    tx: &Transaction<'_>,
    session_id: &str,
    mapping: &ColumnMapping,
) -> AppResult<()> {
    require_open(tx, session_id)?;
    let headers: Vec<String> = from_json(&tx.query_row(
        "SELECT headers FROM import_session WHERE id = ?1",
        params![session_id],
        |r| r.get::<_, String>(0),
    )?)?;
    mapping.validate(headers.len())?;
    tx.execute(
        "UPDATE import_session SET mapping = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
        params![session_id, json(mapping)?],
    )?;

    let rows: Vec<(u32, String, String)> = {
        let mut stmt = tx.prepare(
            "SELECT row_number, raw, errors FROM import_row WHERE session_id = ?1 ORDER BY row_number",
        )?;
        stmt.query_map(params![session_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .collect::<Result<_, _>>()?
    };
    let mut first_seen: HashMap<String, u32> = HashMap::new();
    let mut update = tx.prepare(
        "UPDATE import_row SET fields = ?3, errors = ?4, fingerprint = ?5, duplicate_of = ?6, decision = ?7,
             enrichment = ?8, candidates = NULL, details = NULL
         WHERE session_id = ?1 AND row_number = ?2",
    )?;
    for (row_number, raw, errors) in rows {
        let cells: Vec<String> = from_json(&raw)?;
        let structural: Vec<RowIssue> = from_json::<Vec<RowIssue>>(&errors)?
            .into_iter()
            .filter(|i| i.field == "row")
            .collect();
        let (fields, mut issues) = mapping::normalize_row(&cells, mapping);
        let fields = if structural.is_empty() { fields } else { None };
        issues.splice(0..0, structural);

        let mut fingerprint = None;
        let mut duplicate_of = None;
        let (decision, enrichment) = match &fields {
            None => (Decision::Skip, "not_needed"),
            Some(f) => {
                let fp = mapping::fingerprint(f);
                let key = mapping::duplicate_key(f);
                fingerprint = Some(fp.clone());
                if let Some(first) = first_seen.get(&key) {
                    duplicate_of = Some(*first);
                    (Decision::Skip, "not_needed")
                } else {
                    first_seen.insert(key, row_number);
                    default_decision(tx, f, &fp)?
                }
            }
        };
        update.execute(params![
            session_id,
            row_number,
            fields.as_ref().map(json).transpose()?,
            json(&issues)?,
            fingerprint,
            duplicate_of,
            json(&decision)?,
            enrichment,
        ])?;
    }
    Ok(())
}

fn default_decision(
    conn: &Connection,
    f: &RowFields,
    fp: &str,
) -> AppResult<(Decision, &'static str)> {
    if let Some((album_id, edition_id, reason)) = resolve_existing(conn, f, fp)? {
        return Ok((
            Decision::UseExisting {
                album_id,
                edition_id,
                reason,
            },
            "not_needed",
        ));
    }
    if f.release_id.is_some() || f.release_group_id.is_some() {
        return Ok((
            Decision::Match {
                release_group_id: f.release_group_id.clone(),
                release_id: f.release_id.clone(),
                edition_name: f.edition.clone(),
                chosen_by: ChosenBy::CsvIds,
            },
            "not_started",
        ));
    }
    // Rows without IDs import as manual entries unless the user picks a match.
    Ok((Decision::Manual, "not_started"))
}

fn readiness(issues: &[RowIssue], decision: &Decision, has_details: bool) -> Readiness {
    if !issues.is_empty() {
        return Readiness::Error;
    }
    match decision {
        Decision::Skip => Readiness::Skipped,
        Decision::Match {
            release_id: None, ..
        } => Readiness::NeedsEdition,
        Decision::Match { .. } if !has_details => Readiness::NeedsLookup,
        _ => Readiness::Ready,
    }
}

const ROW_COLUMNS: &str = "row_number, raw, fields, errors, duplicate_of, decision, enrichment, candidates, details IS NOT NULL";

fn read_row(conn: &Connection, r: &rusqlite::Row<'_>) -> AppResult<StagedRow> {
    let raw: String = r.get(1)?;
    let fields: Option<String> = r.get(2)?;
    let errors: String = r.get(3)?;
    let decision: Option<String> = r.get(5)?;
    let candidates: Option<String> = r.get(7)?;
    let has_details: bool = r.get(8)?;
    let issues: Vec<RowIssue> = from_json(&errors)?;
    let decision: Decision = decision
        .as_deref()
        .map(from_json)
        .transpose()?
        .unwrap_or(Decision::Skip);
    let existing = match &decision {
        Decision::UseExisting { album_id, edition_id, .. } => conn
            .query_row(
                "SELECT a.title, e.name FROM album a JOIN edition e ON e.album_id = a.id WHERE a.id = ?1 AND e.id = ?2",
                params![album_id, edition_id],
                |r| Ok(ExistingSummary { album_title: r.get(0)?, edition_name: r.get(1)? }),
            )
            .optional()?,
        _ => None,
    };
    Ok(StagedRow {
        row_number: r.get(0)?,
        raw: from_json(&raw)?,
        fields: fields.as_deref().map(from_json).transpose()?,
        readiness: readiness(&issues, &decision, has_details),
        issues,
        duplicate_of: r.get(4)?,
        decision,
        enrichment: r.get(6)?,
        candidates: candidates.as_deref().map(from_json).transpose()?,
        has_details,
        existing,
    })
}

pub fn row(conn: &Connection, session_id: &str, row_number: u32) -> AppResult<StagedRow> {
    let sql =
        format!("SELECT {ROW_COLUMNS} FROM import_row WHERE session_id = ?1 AND row_number = ?2");
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(params![session_id, row_number])?;
    match rows.next()? {
        Some(r) => read_row(conn, r),
        None => Err(AppError::not_found("import row", row_number.to_string())),
    }
}

pub fn all_rows(conn: &Connection, session_id: &str) -> AppResult<Vec<StagedRow>> {
    let sql =
        format!("SELECT {ROW_COLUMNS} FROM import_row WHERE session_id = ?1 ORDER BY row_number");
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(params![session_id])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(read_row(conn, r)?);
    }
    Ok(out)
}

pub fn rows(
    conn: &Connection,
    session_id: &str,
    offset: u32,
    limit: u32,
    filter: RowFilter,
) -> AppResult<RowPage> {
    let limit = limit.clamp(1, 500) as usize;
    let matching: Vec<StagedRow> = all_rows(conn, session_id)?
        .into_iter()
        .filter(|r| match filter {
            RowFilter::All => true,
            RowFilter::Errors => r.readiness == Readiness::Error,
            RowFilter::Attention => {
                matches!(
                    r.readiness,
                    Readiness::NeedsLookup | Readiness::NeedsEdition
                ) || matches!(
                    r.decision,
                    Decision::UseExisting {
                        reason: ExistingReason::SameName,
                        ..
                    }
                ) || (matches!(r.decision, Decision::Manual)
                    && matches!(r.candidates, Some(Candidates::Groups { .. })))
            }
            RowFilter::Duplicates => r.duplicate_of.is_some(),
            RowFilter::Existing => matches!(r.decision, Decision::UseExisting { .. }),
            RowFilter::Candidates => matches!(
                r.candidates,
                Some(Candidates::Groups { .. } | Candidates::Editions { .. })
            ),
            RowFilter::Skipped => r.readiness == Readiness::Skipped,
        })
        .collect();
    let total = matching.len() as u32;
    Ok(RowPage {
        rows: matching
            .into_iter()
            .skip(offset as usize)
            .take(limit)
            .collect(),
        total,
    })
}

/// Tags on importable rows that don't exist yet (case-insensitive), with row counts.
pub fn unknown_tags(conn: &Connection, rows: &[StagedRow]) -> AppResult<Vec<TagCount>> {
    let mut existing: HashSet<String> = HashSet::new();
    {
        let mut stmt = conn.prepare("SELECT name FROM tag")?;
        for name in stmt.query_map([], |r| r.get::<_, String>(0))? {
            existing.insert(norm(&name?));
        }
    }
    let mut counts: Vec<TagCount> = Vec::new();
    for r in rows
        .iter()
        .filter(|r| !matches!(r.readiness, Readiness::Skipped | Readiness::Error))
    {
        for tag in r.fields.iter().flat_map(|f| &f.tags) {
            if existing.contains(&norm(tag)) {
                continue;
            }
            match counts.iter_mut().find(|c| norm(&c.name) == norm(tag)) {
                Some(c) => c.rows += 1,
                None => counts.push(TagCount {
                    name: tag.clone(),
                    rows: 1,
                }),
            }
        }
    }
    counts.sort_by_key(|c| c.name.to_lowercase());
    Ok(counts)
}

pub fn summary(conn: &Connection, session_id: &str) -> AppResult<SessionSummary> {
    let (file_name, headers, mapping, status, report): (
        String,
        String,
        Option<String>,
        String,
        Option<String>,
    ) = conn
        .query_row(
            "SELECT file_name, headers, mapping, status, report FROM import_session WHERE id = ?1",
            params![session_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("import", session_id))?;
    let headers: Vec<String> = from_json(&headers)?;
    let rows = all_rows(conn, session_id)?;
    let mut c = Counts {
        rows: rows.len() as u32,
        ..Default::default()
    };
    for r in &rows {
        match r.readiness {
            Readiness::Ready => c.ready += 1,
            Readiness::Error => c.errors += 1,
            Readiness::Skipped => c.skipped += 1,
            Readiness::NeedsLookup | Readiness::NeedsEdition => c.needs_attention += 1,
        }
        if r.duplicate_of.is_some() {
            c.duplicates += 1;
        }
        match &r.decision {
            Decision::UseExisting { .. } => c.existing += 1,
            Decision::Match { .. } => c.matched += 1,
            Decision::Manual => c.manual += 1,
            Decision::Skip => {}
        }
        if matches!(
            r.candidates,
            Some(Candidates::Groups { .. } | Candidates::Editions { .. })
        ) {
            c.with_candidates += 1;
        }
        if r.enrichment == "not_started" {
            c.lookup_pending += 1;
        }
    }
    Ok(SessionSummary {
        session_id: session_id.to_owned(),
        file_name,
        suggested_mapping: mapping::suggest(&headers),
        sample: rows.iter().take(5).map(|r| r.raw.clone()).collect(),
        unknown_tags: if mapping.is_some() {
            unknown_tags(conn, &rows)?
        } else {
            Vec::new()
        },
        mapping: mapping.as_deref().map(from_json).transpose()?,
        headers,
        counts: c,
        status,
        report: report.as_deref().map(from_json).transpose()?,
    })
}

pub fn latest_open(conn: &Connection) -> AppResult<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT id FROM import_session WHERE status = 'open' ORDER BY created_at DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?)
}

/// What the user may ask for; `Match` details are fetched by the caller first.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum DecisionInput {
    Match {
        release_group_id: String,
        release_id: String,
        edition_name: Option<String>,
    },
    Manual,
    Skip,
    UseExisting,
}

pub fn set_decision(
    tx: &Transaction<'_>,
    session_id: &str,
    row_number: u32,
    input: &DecisionInput,
    details: Option<&MatchDetails>,
) -> AppResult<StagedRow> {
    require_open(tx, session_id)?;
    let current = row(tx, session_id, row_number)?;
    if !current.issues.is_empty() && *input != DecisionInput::Skip {
        return Err(AppError::validation(
            "row",
            "this row has problems; fix it in the file and import again, or skip it",
        ));
    }
    let fields = current.fields.clone();
    let (decision, details_json) = match input {
        DecisionInput::Skip => (Decision::Skip, None),
        DecisionInput::Manual => (Decision::Manual, None),
        DecisionInput::UseExisting => {
            let f = fields
                .as_ref()
                .ok_or_else(|| AppError::validation("row", "row has no values"))?;
            let (album_id, edition_id, reason) = resolve_existing(tx, f, &mapping::fingerprint(f))?
                .ok_or_else(|| AppError::validation("row", "no existing album matches this row"))?;
            (
                Decision::UseExisting {
                    album_id,
                    edition_id,
                    reason,
                },
                None,
            )
        }
        DecisionInput::Match {
            release_group_id,
            release_id,
            edition_name,
        } => {
            let d = details.ok_or_else(|| AppError::Internal("match details missing".into()))?;
            if d.release.id != *release_id
                || d.release_group.id != *release_group_id
                || d.release.release_group_id.as_deref() != Some(release_group_id.as_str())
            {
                return Err(AppError::validation(
                    "match",
                    "the chosen edition does not belong to the chosen album",
                ));
            }
            let edition_name = edition_name
                .as_deref()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_owned)
                .or_else(|| fields.as_ref().and_then(|f| f.edition.clone()));
            (
                Decision::Match {
                    release_group_id: Some(release_group_id.clone()),
                    release_id: Some(release_id.clone()),
                    edition_name,
                    chosen_by: ChosenBy::User,
                },
                Some(json(d)?),
            )
        }
    };
    let enrichment = match input {
        DecisionInput::Match { .. } => Some("resolved"),
        _ => None,
    };
    tx.execute(
        "UPDATE import_row SET decision = ?3,
             details = CASE WHEN ?4 IS NULL THEN details ELSE ?4 END,
             enrichment = COALESCE(?5, enrichment)
         WHERE session_id = ?1 AND row_number = ?2",
        params![
            session_id,
            row_number,
            json(&decision)?,
            details_json,
            enrichment
        ],
    )?;
    row(tx, session_id, row_number)
}

/// Record the outcome of an online lookup for one row.
pub fn store_enrichment(
    tx: &Transaction<'_>,
    session_id: &str,
    row_number: u32,
    state: &str,
    candidates: Option<&Candidates>,
    resolved: Option<(&Decision, &MatchDetails)>,
) -> AppResult<()> {
    require_open(tx, session_id)?;
    tx.execute(
        "UPDATE import_row SET enrichment = ?3, candidates = ?4 WHERE session_id = ?1 AND row_number = ?2",
        params![session_id, row_number, state, candidates.map(json).transpose()?],
    )?;
    if let Some((decision, details)) = resolved {
        tx.execute(
            "UPDATE import_row SET decision = ?3, details = ?4 WHERE session_id = ?1 AND row_number = ?2",
            params![session_id, row_number, json(decision)?, json(details)?],
        )?;
    }
    Ok(())
}

pub fn details(
    conn: &Connection,
    session_id: &str,
    row_number: u32,
) -> AppResult<Option<MatchDetails>> {
    let raw: Option<String> = conn.query_row(
        "SELECT details FROM import_row WHERE session_id = ?1 AND row_number = ?2",
        params![session_id, row_number],
        |r| r.get(0),
    )?;
    raw.as_deref().map(from_json).transpose()
}

/// Delete a session and its staged rows. Library data is untouched.
pub fn discard(tx: &Transaction<'_>, session_id: &str) -> AppResult<()> {
    tx.execute(
        "DELETE FROM import_session WHERE id = ?1",
        params![session_id],
    )?;
    Ok(())
}

pub fn mark_committed(
    tx: &Transaction<'_>,
    session_id: &str,
    report: &CommitReport,
) -> AppResult<()> {
    tx.execute(
        "UPDATE import_session SET status = 'committed', report = ?2,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
        params![session_id, json(report)?],
    )?;
    tx.execute(
        "DELETE FROM import_row WHERE session_id = ?1",
        params![session_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipc_shapes_are_camel_case() {
        let d = Decision::Match {
            release_group_id: Some("g".into()),
            release_id: None,
            edition_name: None,
            chosen_by: ChosenBy::CsvIds,
        };
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["kind"], "match");
        assert_eq!(v["releaseGroupId"], "g");
        assert_eq!(v["chosenBy"], "csv_ids");
        let input: DecisionInput = serde_json::from_str(
            r#"{"kind":"match","releaseGroupId":"g","releaseId":"r","editionName":null}"#,
        )
        .unwrap();
        assert!(matches!(input, DecisionInput::Match { .. }));
        assert!(serde_json::from_str::<DecisionInput>(r#"{"kind":"use_existing"}"#).is_ok());
        assert!(serde_json::from_str::<DecisionInput>(r#"{"kind":"merge"}"#).is_err());
    }
}
