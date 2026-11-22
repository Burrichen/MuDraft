//! Adding editions deliberately and switching the Listen List edition safely.
//!
//! - A second edition is allowed only when it is genuinely different (e.g. Deluxe with
//!   extra tracks). The same release, or a release with exactly the same recordings in
//!   the same order (an ordinary pressing), is refused.
//! - Track data never moves by position. When switching editions, ratings and favourites
//!   can be copied only between tracks that share a recording identity; different mixes
//!   or live versions have different recordings and are never matched. Listens stay on
//!   the edition that was actually heard. Nothing on the old edition is deleted.
//! - An edition can be deleted only when that loses none of the user's own record of it:
//!   never the album's last edition, and never one with listens, ratings, favourites or a
//!   review.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Serialize;

use crate::domain::ids::parse_uuid;
use crate::error::{AppError, AppResult};
use crate::metadata::ReleaseDetail;

/// Refuse duplicates before importing a release as an extra edition of `album_id`.
pub fn check_new_edition(
    conn: &Connection,
    album_id: &str,
    release: &ReleaseDetail,
) -> AppResult<()> {
    let rgid: Option<String> = conn
        .query_row(
            "SELECT musicbrainz_release_group_id FROM album WHERE id = ?1",
            params![album_id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("album", album_id))?;
    if rgid.is_none() || rgid != release.release_group_id {
        return Err(AppError::validation(
            "edition",
            "that release belongs to a different album in the online metadata",
        ));
    }
    if let Some(name) = conn
        .query_row(
            "SELECT name FROM edition WHERE musicbrainz_release_id = ?1",
            params![release.id],
            |r| r.get::<_, String>(0),
        )
        .optional()?
    {
        return Err(AppError::Conflict(format!(
            "that release is already in your library as the “{name}” edition"
        )));
    }
    let incoming: Vec<Option<&str>> = release
        .tracks
        .iter()
        .map(|t| t.recording_id.as_deref())
        .collect();
    if incoming.is_empty() || incoming.iter().any(Option::is_none) {
        return Ok(()); // can't prove it's identical, so don't block a deliberate choice
    }
    let mut stmt = conn.prepare("SELECT id, name FROM edition WHERE album_id = ?1")?;
    let editions: Vec<(String, String)> = stmt
        .query_map(params![album_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    for (id, name) in editions {
        let existing = recordings(conn, &id)?;
        if existing.len() == incoming.len()
            && existing
                .iter()
                .zip(&incoming)
                .all(|(a, b)| a.as_deref() == *b)
        {
            return Err(AppError::Conflict(format!(
                "it has exactly the same recordings as your “{name}” edition — an ordinary pressing isn’t added again. Choose a release with different tracks, such as a Deluxe edition"
            )));
        }
    }
    Ok(())
}

/// MusicBrainz recording IDs of an edition's tracks, in order (comparable with a release).
fn recordings(conn: &Connection, edition_id: &str) -> AppResult<Vec<Option<String>>> {
    let mut stmt = conn.prepare(
        "SELECT r.musicbrainz_recording_id FROM track t LEFT JOIN recording r ON r.id = t.recording_id
         WHERE t.edition_id = ?1 ORDER BY t.disc_number, t.position",
    )?;
    let rows = stmt
        .query_map(params![edition_id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditionRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackRef {
    pub track_id: String,
    pub title: String,
    pub disc: u32,
    pub position: u32,
}

/// A rated/favourited track whose exact recording also exists on the target edition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Carry {
    pub from: TrackRef,
    pub to: TrackRef,
    pub rating: Option<u8>,
    pub favourite: bool,
    /// The target track already has its own rating or favourite; it is never overwritten.
    pub target_has_data: bool,
}

/// Personal data that stays on the old edition (no exact recording match, or listens).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stay {
    pub track: TrackRef,
    pub rating: Option<u8>,
    pub favourite: bool,
    pub listened: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchPreview {
    pub from: EditionRef,
    pub to: EditionRef,
    pub carry: Vec<Carry>,
    pub stay: Vec<Stay>,
    /// Listen events on the old edition. They stay there: they record what was heard.
    pub listens_kept: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchResult {
    pub copied: u32,
}

struct Row {
    track: TrackRef,
    recording: Option<String>,
    rating: Option<u8>,
    favourite: bool,
    listened: bool,
}

fn tracks(conn: &Connection, edition_id: &str) -> AppResult<Vec<Row>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.title, t.disc_number, t.position, t.recording_id, r.rating, COALESCE(r.is_favourite, 0),
                EXISTS (SELECT 1 FROM listen_event_track l JOIN listen_event ev ON ev.id = l.listen_event_id
                            WHERE l.track_id = t.id AND ev.deleted_at IS NULL)
         FROM track t LEFT JOIN track_rating r ON r.track_id = t.id
         WHERE t.edition_id = ?1 ORDER BY t.disc_number, t.position",
    )?;
    let rows = stmt
        .query_map(params![edition_id], |r| {
            Ok(Row {
                track: TrackRef {
                    track_id: r.get(0)?,
                    title: r.get(1)?,
                    disc: r.get(2)?,
                    position: r.get(3)?,
                },
                recording: r.get(4)?,
                rating: r.get(5)?,
                favourite: r.get(6)?,
                listened: r.get(7)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

fn edition_ref(conn: &Connection, album_id: &str, edition_id: &str) -> AppResult<EditionRef> {
    conn.query_row(
        "SELECT id, name FROM edition WHERE id = ?1 AND album_id = ?2",
        params![edition_id, album_id],
        |r| {
            Ok(EditionRef {
                id: r.get(0)?,
                name: r.get(1)?,
            })
        },
    )
    .optional()?
    .ok_or_else(|| AppError::validation("edition", "that edition belongs to a different album"))
}

pub fn preview_switch(
    conn: &Connection,
    album_id: &str,
    to_edition: &str,
) -> AppResult<SwitchPreview> {
    let album_id = parse_uuid("album", album_id)?;
    let to_edition = parse_uuid("edition", to_edition)?;
    let from_id: String = conn
        .query_row(
            "SELECT edition_id FROM listen_list_entry WHERE album_id = ?1",
            params![album_id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| AppError::validation("album", "this album isn’t on your Listen List"))?;
    let from = edition_ref(conn, &album_id, &from_id)?;
    let to = edition_ref(conn, &album_id, &to_edition)?;
    if from.id == to.id {
        return Err(AppError::validation(
            "edition",
            "the Listen List already uses that edition",
        ));
    }

    let target = tracks(conn, &to.id)?;
    // Only recordings that appear exactly once on the target can be matched unambiguously.
    let mut by_recording: HashMap<&str, Vec<&Row>> = HashMap::new();
    for t in &target {
        if let Some(r) = &t.recording {
            by_recording.entry(r.as_str()).or_default().push(t);
        }
    }
    let mut carry = Vec::new();
    let mut stay = Vec::new();
    for t in tracks(conn, &from.id)? {
        let has_rating_data = t.rating.is_some() || t.favourite;
        if !has_rating_data && !t.listened {
            continue;
        }
        let matched = t
            .recording
            .as_deref()
            .and_then(|r| by_recording.get(r))
            .filter(|m| m.len() == 1)
            .and_then(|m| m.first().copied());
        match matched {
            Some(target) if has_rating_data => carry.push(Carry {
                from: t.track.clone(),
                to: target.track.clone(),
                rating: t.rating,
                favourite: t.favourite,
                target_has_data: target.rating.is_some() || target.favourite,
            }),
            _ => {}
        }
        if matched.is_none() || t.listened {
            stay.push(Stay {
                track: t.track,
                rating: t.rating,
                favourite: t.favourite,
                listened: t.listened,
            });
        }
    }
    let listens_kept: u32 = conn.query_row(
        "SELECT COUNT(*) FROM listen_event WHERE edition_id = ?1 AND deleted_at IS NULL AND kind != 'prior_history'",
        params![from.id],
        |r| r.get(0),
    )?;
    Ok(SwitchPreview {
        from,
        to,
        carry,
        stay,
        listens_kept,
    })
}

/// Point the Listen List at another edition. With `copy_ratings`, ratings and favourites
/// are copied to exactly matching recordings that have none of their own. The old
/// edition keeps everything.
pub fn switch(
    tx: &Transaction<'_>,
    album_id: &str,
    to_edition: &str,
    copy_ratings: bool,
) -> AppResult<SwitchResult> {
    let preview = preview_switch(tx, album_id, to_edition)?;
    let mut copied = 0;
    if copy_ratings {
        for c in preview.carry.iter().filter(|c| !c.target_has_data) {
            copied += tx.execute(
                "INSERT INTO track_rating (track_id, rating, is_favourite) VALUES (?1, ?2, ?3)
                 ON CONFLICT (track_id) DO NOTHING",
                params![c.to.track_id, c.rating, c.favourite],
            )? as u32;
        }
    }
    tx.execute(
        "UPDATE listen_list_entry SET edition_id = ?2 WHERE album_id = ?1",
        params![parse_uuid("album", album_id)?, preview.to.id],
    )?;
    Ok(SwitchResult { copied })
}

/// Delete an extra edition of an album.
///
/// Refused when it is the album's only edition, or when it holds listens, rated or
/// favourite tracks, or a rating or review: that is the user's own record and is never
/// discarded silently. Otherwise its tracks, artwork row and CSV-import markers go; the
/// Listen List entry and Next Up history move to the earliest remaining edition, and its
/// Collection membership (the only thing lost) is removed. Returns the cached artwork
/// file to delete from disk once this commits.
pub fn delete(tx: &Transaction<'_>, album_id: &str, edition_id: &str) -> AppResult<Option<String>> {
    let album_id = parse_uuid("album", album_id)?;
    let edition_id = parse_uuid("edition", edition_id)?;
    let edition = edition_ref(tx, &album_id, &edition_id)?;
    let keep: Option<String> = tx
        .query_row(
            "SELECT id FROM edition WHERE album_id = ?1 AND id != ?2 ORDER BY created_at, id LIMIT 1",
            params![album_id, edition_id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(keep) = keep else {
        return Err(AppError::Conflict(format!(
            "“{}” is this album’s only edition, and an album needs at least one",
            edition.name
        )));
    };

    let count =
        |sql: &str| -> AppResult<u32> { Ok(tx.query_row(sql, params![edition_id], |r| r.get(0))?) };
    let listens =
        count("SELECT COUNT(*) FROM listen_event WHERE edition_id = ?1 AND deleted_at IS NULL")?;
    let rated_tracks = count(
        "SELECT COUNT(*) FROM track_rating tr JOIN track t ON t.id = tr.track_id
         WHERE t.edition_id = ?1 AND (tr.rating IS NOT NULL OR tr.is_favourite = 1)",
    )?;
    let reviewed = count(
        "SELECT COUNT(*) FROM album_review WHERE edition_id = ?1
         AND (rating IS NOT NULL OR review IS NOT NULL)",
    )?;
    let plural = |n: u32| if n == 1 { "" } else { "s" };
    let mut kept = Vec::new();
    if listens > 0 {
        kept.push(format!("{listens} listen{}", plural(listens)));
    }
    if rated_tracks > 0 {
        kept.push(format!(
            "{rated_tracks} rated or liked track{}",
            plural(rated_tracks)
        ));
    }
    if reviewed > 0 {
        kept.push("an album rating or review".to_owned());
    }
    if !kept.is_empty() {
        return Err(AppError::Conflict(format!(
            "the “{}” edition has {}. Deleting it would lose them, so it was kept",
            edition.name,
            kept.join(", ")
        )));
    }

    let artwork_file: Option<String> = tx
        .query_row(
            "SELECT file_name FROM artwork WHERE owner_type = 'edition' AND owner_id = ?1",
            params![edition_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    // Listens already deleted (and undoable) go with it; ratings and reviews left empty
    // are only placeholder rows.
    tx.execute(
        "DELETE FROM listen_event WHERE edition_id = ?1",
        params![edition_id],
    )?;
    tx.execute(
        "DELETE FROM track_rating WHERE track_id IN (SELECT id FROM track WHERE edition_id = ?1)",
        params![edition_id],
    )?;
    tx.execute(
        "DELETE FROM album_review WHERE edition_id = ?1",
        params![edition_id],
    )?;
    tx.execute(
        "DELETE FROM collection_entry WHERE edition_id = ?1",
        params![edition_id],
    )?;
    tx.execute(
        "UPDATE listen_list_entry SET edition_id = ?2 WHERE edition_id = ?1",
        params![edition_id, keep],
    )?;
    tx.execute(
        "UPDATE selection_attempt SET edition_id = ?2 WHERE edition_id = ?1",
        params![edition_id, keep],
    )?;
    tx.execute("DELETE FROM edition WHERE id = ?1", params![edition_id])?;
    Ok(artwork_file)
}
