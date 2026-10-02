//! Personal data. Every row identifies the selected edition (and, through it, the album).

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use super::{Table, album_of_edition, idempotency, optional_long_text, require, settings};
use crate::domain::dates::{CalendarDate, ListenDate};
use crate::domain::ids::new_id;
use crate::domain::rating::HalfStars;
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CollectionSource {
    Manual,
    Listen,
}

impl CollectionSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Listen => "listen",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewListen {
    pub edition_id: String,
    /// Local calendar date `YYYY-MM-DD`; `None` records undated prior history.
    pub listened_on: Option<String>,
    pub is_full: bool,
    /// Tracks heard; each must belong to `edition_id`.
    pub track_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListenRecorded {
    pub listen_id: String,
    pub removed_from_listen_list: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListenListEntry {
    pub album_id: String,
    pub edition_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackRating {
    pub rating: Option<HalfStars>,
    pub is_favourite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumReview {
    pub rating: Option<HalfStars>,
    pub review: Option<String>,
}

/// Put an album on the Listen List via the chosen edition. Re-adding is a no-op;
/// adding a different edition of the same album switches the chosen edition.
pub fn add_to_listen_list(tx: &Transaction<'_>, edition_id: &str) -> AppResult<()> {
    let album_id = album_of_edition(tx, edition_id)?;
    tx.execute(
        "INSERT INTO listen_list_entry (album_id, edition_id) VALUES (?1, ?2)
         ON CONFLICT (album_id) DO UPDATE SET edition_id = excluded.edition_id
         WHERE edition_id IS NOT excluded.edition_id",
        params![album_id, edition_id],
    )?;
    Ok(())
}

pub fn remove_from_listen_list(tx: &Transaction<'_>, album_id: &str) -> AppResult<bool> {
    Ok(tx.execute(
        "DELETE FROM listen_list_entry WHERE album_id = ?1",
        params![album_id],
    )? > 0)
}

pub fn listen_list(conn: &Connection) -> AppResult<Vec<ListenListEntry>> {
    let mut stmt = conn.prepare(
        "SELECT album_id, edition_id FROM listen_list_entry ORDER BY added_at, album_id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(ListenListEntry {
            album_id: r.get(0)?,
            edition_id: r.get(1)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Add an edition to the Collection; repeating it never creates a duplicate.
pub fn add_to_collection(
    tx: &Transaction<'_>,
    edition_id: &str,
    source: CollectionSource,
) -> AppResult<()> {
    let album_id = album_of_edition(tx, edition_id)?;
    tx.execute(
        "INSERT INTO collection_entry (edition_id, album_id, source) VALUES (?1, ?2, ?3)
         ON CONFLICT (edition_id) DO NOTHING",
        params![edition_id, album_id, source.as_str()],
    )?;
    Ok(())
}

pub fn collection_edition_ids(conn: &Connection) -> AppResult<Vec<String>> {
    let mut stmt =
        conn.prepare("SELECT edition_id FROM collection_entry ORDER BY added_at, edition_id")?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Record a listen with its track evidence, add the edition to the Collection, and —
/// for a full listen when the setting allows — remove the album from the Listen List.
/// All of it commits together. Pass a mutation key to make retries safe.
pub fn record_listen(
    tx: &Transaction<'_>,
    key: Option<&str>,
    input: &NewListen,
) -> AppResult<ListenRecorded> {
    idempotency::once(tx, key, "record_listen", input, |tx| {
        let album_id = album_of_edition(tx, &input.edition_id)?;
        let date = match input
            .listened_on
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
        {
            Some(raw) => ListenDate::Known(CalendarDate::parse("listening date", raw)?),
            None => ListenDate::Undated,
        };
        let (listened_on, date_status) = date.to_db();

        let listen_id = new_id();
        tx.execute(
            "INSERT INTO listen_event (id, edition_id, album_id, listened_on, date_status, is_full)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                listen_id,
                input.edition_id,
                album_id,
                listened_on,
                date_status,
                input.is_full
            ],
        )?;

        let mut seen = HashSet::new();
        for track_id in &input.track_ids {
            if !seen.insert(track_id.as_str()) {
                continue;
            }
            let track_edition: Option<String> = tx
                .query_row(
                    "SELECT edition_id FROM track WHERE id = ?1",
                    params![track_id],
                    |r| r.get(0),
                )
                .optional()?;
            match track_edition {
                None => return Err(AppError::not_found("track", track_id)),
                Some(e) if e != input.edition_id => {
                    return Err(AppError::validation(
                        "listened tracks",
                        format!("track {track_id} belongs to a different edition"),
                    ));
                }
                Some(_) => {}
            }
            tx.execute(
                "INSERT INTO listen_event_track (listen_event_id, edition_id, track_id) VALUES (?1, ?2, ?3)",
                params![listen_id, input.edition_id, track_id],
            )?;
        }

        add_to_collection(tx, &input.edition_id, CollectionSource::Listen)?;
        let removed_from_listen_list = input.is_full
            && settings::load(tx)?.full_listen_removes_from_listen_list
            && remove_from_listen_list(tx, &album_id)?;
        Ok(ListenRecorded {
            listen_id,
            removed_from_listen_list,
        })
    })
}

/// Set or clear (`None`) a track rating. Zero is a valid rating.
pub fn set_track_rating(
    tx: &Transaction<'_>,
    track_id: &str,
    rating: Option<HalfStars>,
) -> AppResult<()> {
    require(tx, Table::Track, track_id)?;
    tx.execute(
        "INSERT INTO track_rating (track_id, rating) VALUES (?1, ?2)
         ON CONFLICT (track_id) DO UPDATE SET rating = excluded.rating,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        params![track_id, rating.map(HalfStars::get)],
    )?;
    Ok(())
}

pub fn set_track_favourite(
    tx: &Transaction<'_>,
    track_id: &str,
    is_favourite: bool,
) -> AppResult<()> {
    require(tx, Table::Track, track_id)?;
    tx.execute(
        "INSERT INTO track_rating (track_id, is_favourite) VALUES (?1, ?2)
         ON CONFLICT (track_id) DO UPDATE SET is_favourite = excluded.is_favourite,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        params![track_id, is_favourite],
    )?;
    Ok(())
}

pub fn track_rating(conn: &Connection, track_id: &str) -> AppResult<TrackRating> {
    let row: Option<(Option<i64>, bool)> = conn
        .query_row(
            "SELECT rating, is_favourite FROM track_rating WHERE track_id = ?1",
            params![track_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (rating, is_favourite) = row.unwrap_or((None, false));
    Ok(TrackRating {
        rating: rating.map(HalfStars::new).transpose()?,
        is_favourite,
    })
}

/// Set or clear the explicit album rating for an edition. Never called with a computed value.
pub fn set_album_rating(
    tx: &Transaction<'_>,
    edition_id: &str,
    rating: Option<HalfStars>,
) -> AppResult<()> {
    let album_id = album_of_edition(tx, edition_id)?;
    tx.execute(
        "INSERT INTO album_review (edition_id, album_id, rating) VALUES (?1, ?2, ?3)
         ON CONFLICT (edition_id) DO UPDATE SET rating = excluded.rating,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        params![edition_id, album_id, rating.map(HalfStars::get)],
    )?;
    Ok(())
}

pub fn set_album_review(
    tx: &Transaction<'_>,
    edition_id: &str,
    review: Option<&str>,
) -> AppResult<()> {
    let album_id = album_of_edition(tx, edition_id)?;
    tx.execute(
        "INSERT INTO album_review (edition_id, album_id, review) VALUES (?1, ?2, ?3)
         ON CONFLICT (edition_id) DO UPDATE SET review = excluded.review,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        params![edition_id, album_id, optional_long_text("review", review)?],
    )?;
    Ok(())
}

pub fn album_review(conn: &Connection, edition_id: &str) -> AppResult<AlbumReview> {
    let row: Option<(Option<i64>, Option<String>)> = conn
        .query_row(
            "SELECT rating, review FROM album_review WHERE edition_id = ?1",
            params![edition_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (rating, review) = row.unwrap_or((None, None));
    Ok(AlbumReview {
        rating: rating.map(HalfStars::new).transpose()?,
        review,
    })
}

/// Explicit rating if set, else the rounded mean of the edition's rated tracks.
/// Thin wrapper over the shared rating service.
pub fn effective_rating(conn: &Connection, edition_id: &str) -> AppResult<Option<HalfStars>> {
    Ok(super::ratings::summary(conn, edition_id)?.effective)
}
