//! Persisting reviewed provider metadata. The user has already chosen a release group
//! (canonical album) and a release (edition); nothing here guesses a match.
//!
//! Identity rules:
//! - Artists, albums, editions, and recordings are matched **only** by provider ID.
//!   A same-named artist without that ID is a different artist and is never merged.
//! - Re-importing the same IDs refreshes in place (no duplicates).
//!
//! Refresh rules:
//! - A field whose provenance is a user override (`is_override`) is never overwritten;
//!   the provider's new value is still recorded in `provider_value` for reference.
//! - Tracks are matched by provider track ID. Tracks with ratings, favourites, or listen
//!   evidence are never deleted or merged into another track: if the provider dropped
//!   them, they are kept (moved after the provider's tracks on the same disc).

use std::collections::{HashMap, HashSet};

use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use super::{Table, idempotency, require, required_text};
use crate::domain::dates::PartialDate;
use crate::domain::ids::new_id;
use crate::error::{AppError, AppResult};
use crate::metadata::genres;
use crate::metadata::{CreditPart, ReleaseDetail, ReleaseGroupDetail, TrackDetail, credit_text};

const SOURCE: &str = "musicbrainz";
/// Temporary offset so tracks can be renumbered without violating (disc, position) uniqueness.
const RENUMBER_OFFSET: i64 = 1_000_000;

pub struct ImportRequest<'a> {
    pub release_group: &'a ReleaseGroupDetail,
    pub release: &'a ReleaseDetail,
    /// User-chosen edition name for a new edition; a default is derived when absent.
    pub edition_name: Option<&'a str>,
    pub group_fetched_at: &'a str,
    pub release_fetched_at: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOutcome {
    pub album_id: String,
    pub edition_id: String,
    pub album_created: bool,
    pub edition_created: bool,
    /// `entity.field` names left unchanged because the user locked them.
    pub locked_fields_kept: Vec<String>,
    /// Rated or listened tracks kept although the provider no longer lists them.
    pub retained_tracks: u32,
    pub genres: Vec<String>,
}

pub fn import_release(tx: &Transaction<'_>, req: &ImportRequest<'_>) -> AppResult<ImportOutcome> {
    let rg = req.release_group;
    let rel = req.release;
    if rel.release_group_id.as_deref() != Some(rg.id.as_str()) {
        return Err(AppError::validation(
            "edition",
            "the selected release does not belong to the selected album",
        ));
    }
    let mut locked = Vec::new();

    let (album_id, album_created) = upsert_album(tx, rg, req.group_fetched_at, &mut locked)?;
    replace_source_tags(tx, &album_id, rg, req.group_fetched_at)?;
    let genres = if is_locked(tx, "album", &album_id, "genres")? {
        locked.push("album.genres".into());
        current_genres(tx, &album_id)?
    } else {
        let normalized = genres::normalize(&rg.genres, &rg.tags);
        set_genres(tx, &album_id, &normalized)?;
        let raw: Vec<String> = rg
            .genres
            .iter()
            .chain(&rg.tags)
            .map(|t| t.name.clone())
            .collect();
        provenance(
            tx,
            "album",
            &album_id,
            "genres",
            &rg.id,
            Some(&raw.join("; ")),
            Some(req.group_fetched_at),
        )?;
        normalized.iter().map(|g| (*g).to_owned()).collect()
    };

    let (edition_id, edition_created) = upsert_edition(
        tx,
        &album_id,
        rel,
        req.edition_name,
        req.release_fetched_at,
        &mut locked,
    )?;
    let retained = sync_tracks(tx, &edition_id, rel, req.release_fetched_at, &mut locked)?;

    locked.sort();
    locked.dedup();
    Ok(ImportOutcome {
        album_id,
        edition_id,
        album_created,
        edition_created,
        locked_fields_kept: locked,
        retained_tracks: retained,
        genres,
    })
}

// ---------------------------------------------------------------- artists

fn ensure_artist(tx: &Transaction<'_>, part: &CreditPart, fetched_at: &str) -> AppResult<String> {
    let existing: Option<String> = tx
        .query_row(
            "SELECT id FROM artist WHERE musicbrainz_id = ?1",
            params![part.artist_id],
            |r| r.get(0),
        )
        .optional()?;
    let name =
        required_text("artist name", &part.artist_name).unwrap_or_else(|_| "Unknown artist".into());
    let id = match existing {
        Some(id) => {
            if !is_locked(tx, "artist", &id, "name")? {
                tx.execute(
                    "UPDATE artist SET name = ?2, sort_name = ?3, disambiguation = ?4,
                         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
                    params![id, name, part.sort_name, part.disambiguation],
                )?;
            }
            id
        }
        None => {
            let id = new_id();
            tx.execute(
                "INSERT INTO artist (id, name, sort_name, musicbrainz_id, disambiguation) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, name, part.sort_name, part.artist_id, part.disambiguation],
            )?;
            id
        }
    };
    provenance(
        tx,
        "artist",
        &id,
        "name",
        &part.artist_id,
        Some(&part.artist_name),
        Some(fetched_at),
    )?;
    Ok(id)
}

/// Resolve credit parts to local artist IDs, dropping repeated artists (schema: one
/// credit per artist per owner).
fn credit_rows<'p>(
    tx: &Transaction<'_>,
    parts: &'p [CreditPart],
    fetched_at: &str,
) -> AppResult<Vec<(String, &'p CreditPart)>> {
    let mut seen = HashSet::new();
    let mut rows = Vec::new();
    for part in parts {
        let id = ensure_artist(tx, part, fetched_at)?;
        if seen.insert(id.clone()) {
            rows.push((id, part));
        }
    }
    Ok(rows)
}

fn write_credits(
    tx: &Transaction<'_>,
    table: &'static str,
    owner_column: &'static str,
    owner_id: &str,
    rows: &[(String, &CreditPart)],
) -> AppResult<()> {
    tx.execute(
        &format!("DELETE FROM {table} WHERE {owner_column} = ?1"),
        params![owner_id],
    )?;
    let sql = format!(
        "INSERT INTO {table} ({owner_column}, position, artist_id, credited_name, join_phrase) VALUES (?1, ?2, ?3, ?4, ?5)"
    );
    for (position, (artist_id, part)) in rows.iter().enumerate() {
        tx.execute(
            &sql,
            params![
                owner_id,
                position as i64,
                artist_id,
                part.credited_name,
                part.join_phrase
            ],
        )?;
    }
    Ok(())
}

// ---------------------------------------------------------------- album

fn upsert_album(
    tx: &Transaction<'_>,
    rg: &ReleaseGroupDetail,
    fetched_at: &str,
    locked: &mut Vec<String>,
) -> AppResult<(String, bool)> {
    let title = required_text("album title", &rg.title).unwrap_or_else(|_| "Untitled album".into());
    let date = PartialDate::parse("original date", rg.original_date.value.as_deref())?;
    let (date_value, precision) = date.to_db();
    let existing: Option<String> = tx
        .query_row(
            "SELECT id FROM album WHERE musicbrainz_release_group_id = ?1",
            params![rg.id],
            |r| r.get(0),
        )
        .optional()?;
    let (id, created) = match existing {
        Some(id) => {
            if is_locked(tx, "album", &id, "title")? {
                locked.push("album.title".into());
            } else {
                tx.execute(
                    "UPDATE album SET title = ?2 WHERE id = ?1",
                    params![id, title],
                )?;
            }
            if is_locked(tx, "album", &id, "original_date")? {
                locked.push("album.original_date".into());
            } else {
                tx.execute(
                    "UPDATE album SET original_date = ?2, original_date_precision = ?3 WHERE id = ?1",
                    params![id, date_value, precision],
                )?;
            }
            tx.execute(
                "UPDATE album SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
                params![id],
            )?;
            (id, false)
        }
        None => {
            let id = new_id();
            tx.execute(
                "INSERT INTO album (id, title, original_date, original_date_precision, musicbrainz_release_group_id)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, title, date_value, precision, rg.id],
            )?;
            (id, true)
        }
    };
    provenance(
        tx,
        "album",
        &id,
        "title",
        &rg.id,
        Some(&rg.title),
        Some(fetched_at),
    )?;
    provenance(
        tx,
        "album",
        &id,
        "original_date",
        &rg.id,
        date_value.as_deref(),
        Some(fetched_at),
    )?;

    // Description: the MusicBrainz annotation (sourced text), unless the user wrote their own.
    let description_locked = is_locked(tx, "album", &id, "description")?;
    if description_locked {
        locked.push("album.description".into());
    } else {
        tx.execute(
            "UPDATE album SET description = ?2 WHERE id = ?1",
            params![id, rg.annotation],
        )?;
    }
    if rg.annotation.is_some() || description_locked {
        provenance(
            tx,
            "album",
            &id,
            "description",
            &rg.id,
            rg.annotation.as_deref(),
            Some(fetched_at),
        )?;
    }

    if !rg.artist_credit.is_empty() {
        if is_locked(tx, "album", &id, "credits")? {
            locked.push("album.credits".into());
            // Still make sure the provider's artists exist (by ID) for later review.
            credit_rows(tx, &rg.artist_credit, fetched_at)?;
        } else {
            let rows = credit_rows(tx, &rg.artist_credit, fetched_at)?;
            write_credits(tx, "album_artist_credit", "album_id", &id, &rows)?;
        }
        provenance(
            tx,
            "album",
            &id,
            "credits",
            &rg.id,
            Some(&credit_text(&rg.artist_credit)),
            Some(fetched_at),
        )?;
    }
    Ok((id, created))
}

fn replace_source_tags(
    tx: &Transaction<'_>,
    album_id: &str,
    rg: &ReleaseGroupDetail,
    fetched_at: &str,
) -> AppResult<()> {
    tx.execute(
        "DELETE FROM album_source_tag WHERE album_id = ?1 AND source = ?2",
        params![album_id, SOURCE],
    )?;
    for (kind, list) in [("genre", &rg.genres), ("tag", &rg.tags)] {
        for t in list {
            tx.execute(
                "INSERT INTO album_source_tag (album_id, source, kind, name, votes, fetched_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT DO NOTHING",
                params![album_id, SOURCE, kind, t.name, t.votes, fetched_at],
            )?;
        }
    }
    Ok(())
}

pub(crate) fn set_genres(tx: &Transaction<'_>, album_id: &str, names: &[&str]) -> AppResult<()> {
    tx.execute(
        "DELETE FROM album_genre WHERE album_id = ?1",
        params![album_id],
    )?;
    for (position, name) in names.iter().enumerate() {
        let genre_id: String = match tx
            .query_row(
                "SELECT id FROM genre WHERE name = ?1 COLLATE NOCASE",
                params![name],
                |r| r.get(0),
            )
            .optional()?
        {
            Some(id) => id,
            None => {
                let id = new_id();
                tx.execute(
                    "INSERT INTO genre (id, name) VALUES (?1, ?2)",
                    params![id, name],
                )?;
                id
            }
        };
        tx.execute(
            "INSERT INTO album_genre (album_id, genre_id, position) VALUES (?1, ?2, ?3)",
            params![album_id, genre_id, position as i64],
        )?;
    }
    Ok(())
}

pub fn current_genres(conn: &rusqlite::Connection, album_id: &str) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT g.name FROM album_genre ag JOIN genre g ON g.id = ag.genre_id WHERE ag.album_id = ?1 ORDER BY ag.position",
    )?;
    let rows = stmt.query_map(params![album_id], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

// ---------------------------------------------------------------- edition

fn upsert_edition(
    tx: &Transaction<'_>,
    album_id: &str,
    rel: &ReleaseDetail,
    edition_name: Option<&str>,
    fetched_at: &str,
    locked: &mut Vec<String>,
) -> AppResult<(String, bool)> {
    let date = PartialDate::parse("edition date", rel.date.value.as_deref())?;
    let (date_value, precision) = date.to_db();
    let existing: Option<(String, String)> = tx
        .query_row(
            "SELECT id, album_id FROM edition WHERE musicbrainz_release_id = ?1",
            params![rel.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (id, created) = match existing {
        Some((_, owner)) if owner != album_id => {
            return Err(AppError::Conflict(format!(
                "release {} is already linked to a different album in your library",
                rel.id
            )));
        }
        Some((id, _)) => {
            // The name is the user's; only provider facts refresh.
            if is_locked(tx, "edition", &id, "release_date")? {
                locked.push("edition.release_date".into());
            } else {
                tx.execute(
                    "UPDATE edition SET release_date = ?2, release_date_precision = ?3,
                         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
                    params![id, date_value, precision],
                )?;
            }
            (id, false)
        }
        None => {
            let name = match edition_name.map(str::trim).filter(|n| !n.is_empty()) {
                Some(n) => required_text("edition name", n)?,
                None => default_edition_name(tx, album_id, rel)?,
            };
            let taken: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM edition WHERE album_id = ?1 AND name = ?2 COLLATE NOCASE)",
                params![album_id, name],
                |r| r.get(0),
            )?;
            if taken {
                return Err(AppError::Conflict(format!(
                    "this album already has an edition named \"{name}\"; choose another name"
                )));
            }
            let id = new_id();
            tx.execute(
                "INSERT INTO edition (id, album_id, name, release_date, release_date_precision, musicbrainz_release_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, album_id, name, date_value, precision, rel.id],
            )?;
            (id, true)
        }
    };
    provenance(
        tx,
        "edition",
        &id,
        "release_date",
        &rel.id,
        date_value.as_deref(),
        Some(fetched_at),
    )?;
    Ok((id, created))
}

/// "Standard" for an album's first edition; otherwise the release's disambiguation or a
/// format · country · year summary, made unique.
fn default_edition_name(
    tx: &Transaction<'_>,
    album_id: &str,
    rel: &ReleaseDetail,
) -> AppResult<String> {
    let mut stmt = tx.prepare("SELECT lower(name) FROM edition WHERE album_id = ?1")?;
    let taken: HashSet<String> = stmt
        .query_map(params![album_id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let base = if taken.is_empty() {
        "Standard".to_owned()
    } else if let Some(d) = &rel.disambiguation {
        d.clone()
    } else {
        let parts: Vec<String> = [
            (!rel.formats.is_empty()).then(|| rel.formats.join(" + ")),
            rel.country.clone(),
            rel.date.year.map(|y| y.to_string()),
        ]
        .into_iter()
        .flatten()
        .collect();
        if parts.is_empty() {
            "Edition".into()
        } else {
            parts.join(" · ")
        }
    };
    let mut name = base.clone();
    let mut n = 2;
    while taken.contains(&name.to_lowercase()) {
        name = format!("{base} ({n})");
        n += 1;
    }
    Ok(name)
}

// ---------------------------------------------------------------- tracks

struct ExistingTrack {
    id: String,
    provider_id: Option<String>,
    disc: i64,
    has_personal_data: bool,
}

fn sync_tracks(
    tx: &Transaction<'_>,
    edition_id: &str,
    rel: &ReleaseDetail,
    fetched_at: &str,
    locked: &mut Vec<String>,
) -> AppResult<u32> {
    let mut slots = HashSet::new();
    for t in &rel.tracks {
        if !slots.insert((t.disc, t.position)) {
            return Err(AppError::Provider(format!(
                "release lists disc {} track {} twice",
                t.disc, t.position
            )));
        }
    }

    let existing: Vec<ExistingTrack> = {
        let mut stmt = tx.prepare(
            "SELECT t.id, t.musicbrainz_track_id, t.disc_number,
                    EXISTS (SELECT 1 FROM track_rating r WHERE r.track_id = t.id)
                    OR EXISTS (SELECT 1 FROM listen_event_track l WHERE l.track_id = t.id)
             FROM track t WHERE t.edition_id = ?1 ORDER BY t.disc_number, t.position",
        )?;
        stmt.query_map(params![edition_id], |r| {
            Ok(ExistingTrack {
                id: r.get(0)?,
                provider_id: r.get(1)?,
                disc: r.get(2)?,
                has_personal_data: r.get(3)?,
            })
        })?
        .collect::<Result<_, _>>()?
    };
    let incoming_ids: HashSet<&str> = rel.tracks.iter().map(|t| t.id.as_str()).collect();
    let by_provider: HashMap<&str, &ExistingTrack> = existing
        .iter()
        .filter_map(|e| e.provider_id.as_deref().map(|p| (p, e)))
        .collect();

    // Park every existing track out of the way, then place tracks at their final slots.
    tx.execute(
        "UPDATE track SET position = position + ?2 WHERE edition_id = ?1",
        params![edition_id, RENUMBER_OFFSET],
    )?;

    let mut retained = Vec::new();
    for e in &existing {
        let still_listed = e
            .provider_id
            .as_deref()
            .is_some_and(|p| incoming_ids.contains(p));
        if still_listed {
            continue;
        }
        if e.has_personal_data {
            retained.push(e);
        } else {
            tx.execute("DELETE FROM track WHERE id = ?1", params![e.id])?;
        }
    }

    let release_artists: Vec<&str> = rel
        .artist_credit
        .iter()
        .map(|c| c.artist_id.as_str())
        .collect();
    for t in &rel.tracks {
        let recording_id = t
            .recording_id
            .as_deref()
            .map(|r| ensure_recording(tx, r, t))
            .transpose()?;
        let title =
            required_text("track title", &t.title).unwrap_or_else(|_| "Untitled track".into());
        let track_id = match by_provider.get(t.id.as_str()) {
            Some(e) => {
                tx.execute(
                    "UPDATE track SET disc_number = ?2, position = ?3, recording_id = ?4 WHERE id = ?1",
                    params![e.id, t.disc, t.position, recording_id],
                )?;
                if is_locked(tx, "track", &e.id, "title")? {
                    locked.push("track.title".into());
                } else {
                    tx.execute(
                        "UPDATE track SET title = ?2 WHERE id = ?1",
                        params![e.id, title],
                    )?;
                }
                if is_locked(tx, "track", &e.id, "length_ms")? {
                    locked.push("track.length_ms".into());
                } else {
                    tx.execute(
                        "UPDATE track SET length_ms = ?2 WHERE id = ?1",
                        params![e.id, t.length_ms],
                    )?;
                }
                e.id.clone()
            }
            None => {
                let id = new_id();
                tx.execute(
                    "INSERT INTO track (id, edition_id, disc_number, position, title, length_ms, recording_id, musicbrainz_track_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![id, edition_id, t.disc, t.position, title, t.length_ms, recording_id, t.id],
                )?;
                id
            }
        };
        provenance(
            tx,
            "track",
            &track_id,
            "title",
            &t.id,
            Some(&t.title),
            Some(fetched_at),
        )?;

        // Store a track credit only when it differs from the release credit.
        let track_artists: Vec<&str> = t
            .artist_credit
            .iter()
            .map(|c| c.artist_id.as_str())
            .collect();
        if is_locked(tx, "track", &track_id, "credits")? {
            locked.push("track.credits".into());
        } else if !track_artists.is_empty() && track_artists != release_artists {
            let rows = credit_rows(tx, &t.artist_credit, fetched_at)?;
            write_credits(tx, "track_artist_credit", "track_id", &track_id, &rows)?;
        } else {
            tx.execute(
                "DELETE FROM track_artist_credit WHERE track_id = ?1",
                params![track_id],
            )?;
        }
    }

    // Kept tracks go after the provider's tracks on their own disc, preserving their order.
    for e in &retained {
        let next: i64 = tx.query_row(
            "SELECT COALESCE(MAX(position), 0) + 1 FROM track
             WHERE edition_id = ?1 AND disc_number = ?2 AND position < ?3",
            params![edition_id, e.disc, RENUMBER_OFFSET],
            |r| r.get(0),
        )?;
        tx.execute(
            "UPDATE track SET position = ?2 WHERE id = ?1",
            params![e.id, next],
        )?;
    }

    provenance(
        tx,
        "edition",
        edition_id,
        "tracklist",
        &rel.id,
        None,
        Some(fetched_at),
    )?;
    Ok(retained.len() as u32)
}

fn ensure_recording(tx: &Transaction<'_>, provider_id: &str, t: &TrackDetail) -> AppResult<String> {
    if let Some(id) = tx
        .query_row(
            "SELECT id FROM recording WHERE musicbrainz_recording_id = ?1",
            params![provider_id],
            |r| r.get(0),
        )
        .optional()?
    {
        return Ok(id);
    }
    let id = new_id();
    let title = t.recording_title.as_deref().unwrap_or(&t.title);
    tx.execute(
        "INSERT INTO recording (id, title, length_ms, musicbrainz_recording_id) VALUES (?1, ?2, ?3, ?4)",
        params![id, required_text("recording title", title).unwrap_or_else(|_| "Untitled".into()), t.length_ms, provider_id],
    )?;
    Ok(id)
}

// ---------------------------------------------------------------- provenance

pub(crate) fn is_locked(
    tx: &rusqlite::Connection,
    entity: &str,
    id: &str,
    field: &str,
) -> AppResult<bool> {
    Ok(tx
        .query_row(
            "SELECT is_override FROM metadata_provenance WHERE entity_type = ?1 AND entity_id = ?2 AND field = ?3",
            params![entity, id, field],
            |r| r.get::<_, bool>(0),
        )
        .optional()?
        .unwrap_or(false))
}

/// Record provider provenance. A user override keeps its source and lock; only the
/// provider's latest value and fetch time are updated alongside it.
fn provenance(
    tx: &Transaction<'_>,
    entity: &str,
    id: &str,
    field: &str,
    source_ref: &str,
    provider_value: Option<&str>,
    fetched_at: Option<&str>,
) -> AppResult<()> {
    tx.execute(
        "INSERT INTO metadata_provenance
             (entity_type, entity_id, field, source, source_ref, provider_value, is_override, fetched_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)
         ON CONFLICT (entity_type, entity_id, field) DO UPDATE SET
             source = CASE WHEN is_override = 1 THEN source ELSE excluded.source END,
             source_ref = excluded.source_ref,
             provider_value = excluded.provider_value,
             fetched_at = excluded.fetched_at,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        params![entity, id, field, SOURCE, source_ref, provider_value, fetched_at],
    )?;
    Ok(())
}

// ---------------------------------------------------------------- manual entry

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManualAlbum {
    pub title: String,
    /// Creates a new artist. Never matched to an existing artist by name.
    pub artist_name: Option<String>,
    /// Use an existing artist the user explicitly picked instead.
    pub artist_id: Option<String>,
    pub year: Option<i64>,
    pub edition_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualAlbumCreated {
    pub album_id: String,
    pub edition_id: String,
    pub artist_id: String,
}

/// Unmatched/manual album with one edition and no tracklist. Pass a mutation key to make
/// retries safe.
pub fn add_manual_album(
    tx: &Transaction<'_>,
    key: Option<&str>,
    input: &ManualAlbum,
) -> AppResult<ManualAlbumCreated> {
    idempotency::once(tx, key, "add_manual_album", input, |tx| {
        let title = required_text("album title", &input.title)?;
        let date = match input.year {
            Some(y) => PartialDate::parse("year", Some(&format!("{y:04}")))?,
            None => PartialDate::Unknown,
        };
        let (date_value, precision) = date.to_db();
        let artist_id = match (
            &input.artist_id,
            input
                .artist_name
                .as_deref()
                .map(str::trim)
                .filter(|n| !n.is_empty()),
        ) {
            (Some(id), _) => {
                require(tx, Table::Artist, id)?;
                id.clone()
            }
            (None, Some(name)) => {
                let id = new_id();
                tx.execute(
                    "INSERT INTO artist (id, name) VALUES (?1, ?2)",
                    params![id, required_text("artist name", name)?],
                )?;
                id
            }
            (None, None) => return Err(AppError::validation("artist", "enter an artist name")),
        };
        let album_id = new_id();
        tx.execute(
            "INSERT INTO album (id, title, original_date, original_date_precision) VALUES (?1, ?2, ?3, ?4)",
            params![album_id, title, date_value, precision],
        )?;
        tx.execute(
            "INSERT INTO album_artist_credit (album_id, position, artist_id) VALUES (?1, 0, ?2)",
            params![album_id, artist_id],
        )?;
        let edition_name = input
            .edition_name
            .as_deref()
            .map(|n| required_text("edition name", n))
            .transpose()?
            .unwrap_or_else(|| "Standard".into());
        let edition_id = new_id();
        tx.execute(
            "INSERT INTO edition (id, album_id, name) VALUES (?1, ?2, ?3)",
            params![edition_id, album_id, edition_name],
        )?;
        for field in ["title", "original_date", "credits"] {
            tx.execute(
                "INSERT INTO metadata_provenance (entity_type, entity_id, field, source, is_override)
                 VALUES ('album', ?1, ?2, 'manual', 0)",
                params![album_id, field],
            )?;
        }
        Ok(ManualAlbumCreated {
            album_id,
            edition_id,
            artist_id,
        })
    })
}

/// Manual genre correction: validated against the broad vocabulary and locked so
/// provider refreshes leave it alone.
pub fn set_album_genres_manually(
    tx: &Transaction<'_>,
    album_id: &str,
    names: &[String],
) -> AppResult<Vec<String>> {
    require(tx, Table::Album, album_id)?;
    let genres = genres::validate_manual(names)?;
    set_genres(tx, album_id, &genres)?;
    tx.execute(
        "INSERT INTO metadata_provenance (entity_type, entity_id, field, source, is_override)
         VALUES ('album', ?1, 'genres', 'manual', 1)
         ON CONFLICT (entity_type, entity_id, field) DO UPDATE SET source = 'manual', is_override = 1,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        params![album_id],
    )?;
    Ok(genres.iter().map(|g| (*g).to_owned()).collect())
}
