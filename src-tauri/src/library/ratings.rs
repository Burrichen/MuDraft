//! The rating service: every place that shows or sorts by an album rating reads it here,
//! and every rating write goes through here. Ratings belong to an edition's tracks and to
//! the edition itself, so Standard and Deluxe are rated separately.
//!
//! Rating, favouriting, or reviewing never records a listen, marks anything as listened,
//! or invents a date.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::personal;
use super::{Table, require};
use crate::domain::ids::parse_uuid;
use serde::Serialize;

use crate::domain::rating::{HalfStars, RatingSource, RatingSummary, mean_stars, summarize};
use crate::error::{AppError, AppResult};

/// Parse a rating sent as half-stars (0–10). `None` clears.
pub fn half_stars(raw: Option<i64>) -> AppResult<Option<HalfStars>> {
    raw.map(HalfStars::new).transpose()
}

pub fn summary(conn: &Connection, edition_id: &str) -> AppResult<RatingSummary> {
    let edition_id = parse_uuid("edition", edition_id)?;
    require(conn, Table::Edition, &edition_id)?;
    Ok(summaries(conn, std::slice::from_ref(&edition_id))?
        .remove(&edition_id)
        .unwrap_or_else(|| summarize(None, [])))
}

/// Summaries for many editions at once (cards, sorting).
pub fn summaries(
    conn: &Connection,
    edition_ids: &[String],
) -> AppResult<HashMap<String, RatingSummary>> {
    let mut explicit_stmt =
        conn.prepare("SELECT rating FROM album_review WHERE edition_id = ?1")?;
    let mut tracks_stmt = conn.prepare(
        "SELECT r.rating FROM track t LEFT JOIN track_rating r ON r.track_id = t.id WHERE t.edition_id = ?1",
    )?;
    let mut out = HashMap::with_capacity(edition_ids.len());
    for id in edition_ids {
        let explicit: Option<i64> = explicit_stmt
            .query_row(params![id], |r| r.get(0))
            .optional()?
            .flatten();
        let tracks = tracks_stmt
            .query_map(params![id], |r| r.get::<_, Option<i64>>(0))?
            .map(|v| v?.map(HalfStars::new).transpose())
            .collect::<AppResult<Vec<_>>>()?;
        out.insert(id.clone(), summarize(half_stars(explicit)?, tracks));
    }
    Ok(out)
}

fn edition_of_track(conn: &Connection, track_id: &str) -> AppResult<String> {
    conn.query_row(
        "SELECT edition_id FROM track WHERE id = ?1",
        params![track_id],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| AppError::not_found("track", track_id))
}

/// Set (0–10 half-stars, zero included) or clear (`None`) a track rating.
pub fn set_track(
    tx: &Transaction<'_>,
    track_id: &str,
    rating: Option<i64>,
) -> AppResult<RatingSummary> {
    let track_id = parse_uuid("track", track_id)?;
    personal::set_track_rating(tx, &track_id, half_stars(rating)?)?;
    summary(tx, &edition_of_track(tx, &track_id)?)
}

pub fn set_favourite(tx: &Transaction<'_>, track_id: &str, favourite: bool) -> AppResult<()> {
    personal::set_track_favourite(tx, &parse_uuid("track", track_id)?, favourite)
}

/// A track the user liked (the heart), wherever its album is in the library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LikedTrack {
    pub track_id: String,
    pub title: String,
    pub length_ms: Option<i64>,
    pub album_id: String,
    pub album_title: String,
    pub credit: String,
    pub edition_name: String,
}

/// Every liked track, by artist credit, album, edition, then tracklist order.
pub fn liked_tracks(conn: &Connection) -> AppResult<Vec<LikedTrack>> {
    let mut credits: HashMap<String, String> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT c.album_id, COALESCE(c.credited_name, ar.name), c.join_phrase
         FROM album_artist_credit c JOIN artist ar ON ar.id = c.artist_id
         WHERE c.album_id IN (SELECT e.album_id FROM track_rating tr
             JOIN track t ON t.id = tr.track_id JOIN edition e ON e.id = t.edition_id
             WHERE tr.is_favourite = 1)
         ORDER BY c.album_id, c.position",
    )?;
    for row in stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })? {
        let (album, name, join) = row?;
        credits
            .entry(album)
            .or_default()
            .push_str(&format!("{name}{join}"));
    }

    let mut stmt = conn.prepare(
        "SELECT t.id, t.title, t.length_ms, e.album_id, a.title, e.name FROM track_rating tr
         JOIN track t ON t.id = tr.track_id JOIN edition e ON e.id = t.edition_id
         JOIN album a ON a.id = e.album_id
         WHERE tr.is_favourite = 1
         ORDER BY a.title COLLATE NOCASE, e.name, t.disc_number, t.position",
    )?;
    let mut out: Vec<LikedTrack> = stmt
        .query_map([], |r| {
            let album_id: String = r.get(3)?;
            Ok(LikedTrack {
                track_id: r.get(0)?,
                title: r.get(1)?,
                length_ms: r.get(2)?,
                credit: credits.get(&album_id).cloned().unwrap_or_default(),
                album_id,
                album_title: r.get(4)?,
                edition_name: r.get(5)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    // Stable: keeps album and tracklist order within each artist.
    out.sort_by_key(|t| t.credit.to_lowercase());
    Ok(out)
}

/// Set or clear the explicit album rating for an edition. Track ratings are untouched;
/// clearing restores the calculated value.
pub fn set_album(
    tx: &Transaction<'_>,
    edition_id: &str,
    rating: Option<i64>,
) -> AppResult<RatingSummary> {
    let edition_id = parse_uuid("edition", edition_id)?;
    personal::set_album_rating(tx, &edition_id, half_stars(rating)?)?;
    summary(tx, &edition_id)
}

/// Persist the album review/notes for an edition (`None` or blank clears).
pub fn set_review(tx: &Transaction<'_>, edition_id: &str, review: Option<&str>) -> AppResult<()> {
    personal::set_album_review(tx, &parse_uuid("edition", edition_id)?, review)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditionScore {
    pub edition_id: String,
    pub name: String,
    pub rating: HalfStars,
    pub source: RatingSource,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
/// The canonical album score shared by Artist pages and Stats.
pub struct AlbumScore {
    /// Stars (may be fractional): the mean of the rated editions' effective ratings.
    pub stars: f64,
    pub editions: Vec<EditionScore>,
}

/// Mean of the effective ratings of the album's rated editions (zero is a rating).
pub fn album_score(conn: &Connection, album_id: &str) -> AppResult<Option<AlbumScore>> {
    let mut stmt =
        conn.prepare("SELECT id, name FROM edition WHERE album_id = ?1 ORDER BY created_at, id")?;
    let editions: Vec<(String, String)> = stmt
        .query_map(params![album_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let ids: Vec<String> = editions.iter().map(|(id, _)| id.clone()).collect();
    let summaries = summaries(conn, &ids)?;
    let rated: Vec<EditionScore> = editions
        .into_iter()
        .filter_map(|(id, name)| {
            let s = summaries.get(&id)?;
            Some(EditionScore {
                rating: s.effective?,
                source: s.source,
                edition_id: id,
                name,
            })
        })
        .collect();
    Ok(
        mean_stars(rated.iter().map(|e| e.rating)).map(|stars| AlbumScore {
            stars,
            editions: rated,
        }),
    )
}
