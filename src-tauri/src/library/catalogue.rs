//! Catalogue metadata: artists, canonical albums, editions, tracks, recordings, genres, tags.
//! Nothing here creates Listen List or Collection membership.

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use super::{Table, idempotency, optional_text, require, required_text};
use crate::domain::dates::PartialDate;
use crate::domain::ids::{new_id, parse_optional_uuid};
use crate::error::{AppError, AppResult};

/// Seeded by migration v2; drives Weighted Random.
pub const LISTEN_ASAP_TAG_ID: &str = "00000000-0000-7000-8000-000000000001";
const MAX_GENRES: usize = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewArtist {
    pub name: String,
    pub sort_name: Option<String>,
    pub musicbrainz_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditInput {
    pub artist_id: String,
    pub credited_name: Option<String>,
    pub join_phrase: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewAlbum {
    pub title: String,
    /// `YYYY`, `YYYY-MM`, `YYYY-MM-DD`, or absent for unknown.
    pub original_date: Option<String>,
    pub musicbrainz_release_group_id: Option<String>,
    pub credits: Vec<CreditInput>,
    /// Broad genre names, most relevant first; at most three.
    pub genres: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewTrack {
    pub disc_number: Option<i64>,
    pub position: i64,
    pub title: String,
    pub length_ms: Option<i64>,
    pub recording_id: Option<String>,
    pub musicbrainz_track_id: Option<String>,
    /// Empty means the track is credited like its album.
    pub credits: Vec<CreditInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewEdition {
    pub album_id: String,
    pub name: String,
    pub release_date: Option<String>,
    pub musicbrainz_release_id: Option<String>,
    pub tracks: Vec<NewTrack>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditionCreated {
    pub edition_id: String,
    /// In the order the tracks were supplied.
    pub track_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewRecording {
    pub title: String,
    pub length_ms: Option<i64>,
    pub musicbrainz_recording_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub id: String,
    pub name: String,
    pub builtin_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditionSummary {
    pub id: String,
    pub name: String,
    pub release_date: Option<String>,
    pub release_date_precision: String,
}

pub fn create_artist(
    tx: &Transaction<'_>,
    key: Option<&str>,
    input: &NewArtist,
) -> AppResult<String> {
    idempotency::once(tx, key, "create_artist", input, |tx| {
        let id = new_id();
        tx.execute(
            "INSERT INTO artist (id, name, sort_name, musicbrainz_id) VALUES (?1, ?2, ?3, ?4)",
            params![
                id,
                required_text("artist name", &input.name)?,
                optional_text("artist sort name", input.sort_name.as_deref())?,
                parse_optional_uuid("MusicBrainz artist ID", input.musicbrainz_id.as_deref())?,
            ],
        )?;
        Ok(id)
    })
}

pub fn create_album(
    tx: &Transaction<'_>,
    key: Option<&str>,
    input: &NewAlbum,
) -> AppResult<String> {
    idempotency::once(tx, key, "create_album", input, |tx| {
        let title = required_text("album title", &input.title)?;
        let (date, precision) =
            PartialDate::parse("original date", input.original_date.as_deref())?.to_db();
        let mbid = parse_optional_uuid(
            "MusicBrainz release group ID",
            input.musicbrainz_release_group_id.as_deref(),
        )?;
        if input.credits.is_empty() {
            return Err(AppError::validation(
                "album credits",
                "at least one artist is required",
            ));
        }
        let genres = normalize_genre_names(&input.genres)?;

        let id = new_id();
        tx.execute(
            "INSERT INTO album (id, title, original_date, original_date_precision, musicbrainz_release_group_id)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, title, date, precision, mbid],
        )?;
        insert_credits(tx, "album_artist_credit", "album_id", &id, &input.credits)?;
        for (position, name) in genres.iter().enumerate() {
            let genre_id = genre_id_for(tx, name)?;
            tx.execute(
                "INSERT INTO album_genre (album_id, genre_id, position) VALUES (?1, ?2, ?3)",
                params![id, genre_id, position as i64],
            )?;
        }
        Ok(id)
    })
}

/// Create an edition and its whole tracklist atomically.
pub fn create_edition(
    tx: &Transaction<'_>,
    key: Option<&str>,
    input: &NewEdition,
) -> AppResult<EditionCreated> {
    idempotency::once(tx, key, "create_edition", input, |tx| {
        require(tx, Table::Album, &input.album_id)?;
        let name = required_text("edition name", &input.name)?;
        let (date, precision) =
            PartialDate::parse("edition release date", input.release_date.as_deref())?.to_db();
        let mbid = parse_optional_uuid(
            "MusicBrainz release ID",
            input.musicbrainz_release_id.as_deref(),
        )?;

        let edition_id = new_id();
        tx.execute(
            "INSERT INTO edition (id, album_id, name, release_date, release_date_precision, musicbrainz_release_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![edition_id, input.album_id, name, date, precision, mbid],
        )?;

        let mut seen = HashSet::new();
        let mut track_ids = Vec::with_capacity(input.tracks.len());
        for track in &input.tracks {
            let disc = track.disc_number.unwrap_or(1);
            if disc < 1 {
                return Err(AppError::validation(
                    "disc number",
                    format!("{disc} must be at least 1"),
                ));
            }
            if track.position < 1 {
                return Err(AppError::validation(
                    "track position",
                    format!("{} must be at least 1", track.position),
                ));
            }
            if !seen.insert((disc, track.position)) {
                return Err(AppError::validation(
                    "tracklist",
                    format!("disc {disc} position {} appears twice", track.position),
                ));
            }
            let recording_id = parse_optional_uuid("recording ID", track.recording_id.as_deref())?;
            if let Some(rid) = &recording_id {
                require(tx, Table::Recording, rid)?;
            }
            let track_id = new_id();
            tx.execute(
                "INSERT INTO track (id, edition_id, disc_number, position, title, length_ms, recording_id, musicbrainz_track_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    track_id,
                    edition_id,
                    disc,
                    track.position,
                    required_text("track title", &track.title)?,
                    positive_length(track.length_ms)?,
                    recording_id,
                    parse_optional_uuid("MusicBrainz track ID", track.musicbrainz_track_id.as_deref())?,
                ],
            )?;
            insert_credits(
                tx,
                "track_artist_credit",
                "track_id",
                &track_id,
                &track.credits,
            )?;
            track_ids.push(track_id);
        }
        Ok(EditionCreated {
            edition_id,
            track_ids,
        })
    })
}

pub fn create_recording(
    tx: &Transaction<'_>,
    key: Option<&str>,
    input: &NewRecording,
) -> AppResult<String> {
    idempotency::once(tx, key, "create_recording", input, |tx| {
        let id = new_id();
        tx.execute(
            "INSERT INTO recording (id, title, length_ms, musicbrainz_recording_id) VALUES (?1, ?2, ?3, ?4)",
            params![
                id,
                required_text("recording title", &input.title)?,
                positive_length(input.length_ms)?,
                parse_optional_uuid("MusicBrainz recording ID", input.musicbrainz_recording_id.as_deref())?,
            ],
        )?;
        Ok(id)
    })
}

/// Attach a tag to a canonical album; repeating it is a no-op.
pub fn tag_album(tx: &Transaction<'_>, album_id: &str, tag_id: &str) -> AppResult<()> {
    require(tx, Table::Album, album_id)?;
    require(tx, Table::Tag, tag_id)?;
    tx.execute(
        "INSERT INTO album_tag (album_id, tag_id) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
        params![album_id, tag_id],
    )?;
    Ok(())
}

pub fn untag_album(tx: &Transaction<'_>, album_id: &str, tag_id: &str) -> AppResult<()> {
    tx.execute(
        "DELETE FROM album_tag WHERE album_id = ?1 AND tag_id = ?2",
        params![album_id, tag_id],
    )?;
    Ok(())
}

pub fn album_tags(conn: &Connection, album_id: &str) -> AppResult<Vec<Tag>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.name, t.builtin_key FROM album_tag at JOIN tag t ON t.id = at.tag_id
         WHERE at.album_id = ?1 ORDER BY t.name COLLATE NOCASE",
    )?;
    let rows = stmt.query_map(params![album_id], |r| {
        Ok(Tag {
            id: r.get(0)?,
            name: r.get(1)?,
            builtin_key: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Album IDs an artist is credited on, by original year then title.
pub fn albums_for_artist(conn: &Connection, artist_id: &str) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT a.id FROM album_artist_credit c JOIN album a ON a.id = c.album_id
         WHERE c.artist_id = ?1 ORDER BY a.original_year IS NULL, a.original_year, a.title COLLATE NOCASE",
    )?;
    let rows = stmt.query_map(params![artist_id], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Credited artist IDs in credit order.
pub fn album_credit_artists(conn: &Connection, album_id: &str) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT artist_id FROM album_artist_credit WHERE album_id = ?1 ORDER BY position",
    )?;
    let rows = stmt.query_map(params![album_id], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn editions_of_album(conn: &Connection, album_id: &str) -> AppResult<Vec<EditionSummary>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, release_date, release_date_precision FROM edition
         WHERE album_id = ?1 ORDER BY release_date IS NULL, release_date, name COLLATE NOCASE",
    )?;
    let rows = stmt.query_map(params![album_id], |r| {
        Ok(EditionSummary {
            id: r.get(0)?,
            name: r.get(1)?,
            release_date: r.get(2)?,
            release_date_precision: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn insert_credits(
    tx: &Transaction<'_>,
    table: &'static str,
    owner_column: &'static str,
    owner_id: &str,
    credits: &[CreditInput],
) -> AppResult<()> {
    let sql = format!(
        "INSERT INTO {table} ({owner_column}, position, artist_id, credited_name, join_phrase)
         VALUES (?1, ?2, ?3, ?4, ?5)"
    );
    let mut seen = HashSet::new();
    for (position, credit) in credits.iter().enumerate() {
        require(tx, Table::Artist, &credit.artist_id)?;
        if !seen.insert(credit.artist_id.as_str()) {
            return Err(AppError::validation(
                "artist credits",
                format!("artist {} is credited twice", credit.artist_id),
            ));
        }
        tx.execute(
            &sql,
            params![
                owner_id,
                position as i64,
                credit.artist_id,
                optional_text("credited name", credit.credited_name.as_deref())?,
                // Join phrases are significant whitespace (" & ", " feat. "), so not trimmed.
                credit.join_phrase.as_deref().unwrap_or(""),
            ],
        )?;
    }
    Ok(())
}

fn normalize_genre_names(raw: &[String]) -> AppResult<Vec<String>> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for name in raw {
        let name = required_text("genre", name)?;
        if seen.insert(name.to_lowercase()) {
            out.push(name);
        }
    }
    if out.len() > MAX_GENRES {
        return Err(AppError::validation(
            "genres",
            format!("at most {MAX_GENRES} broad genres per album"),
        ));
    }
    Ok(out)
}

fn genre_id_for(tx: &Transaction<'_>, name: &str) -> AppResult<String> {
    if let Some(id) = tx
        .query_row(
            "SELECT id FROM genre WHERE name = ?1 COLLATE NOCASE",
            params![name],
            |r| r.get(0),
        )
        .optional()?
    {
        return Ok(id);
    }
    let id = new_id();
    tx.execute(
        "INSERT INTO genre (id, name) VALUES (?1, ?2)",
        params![id, name],
    )?;
    Ok(id)
}

fn positive_length(length_ms: Option<i64>) -> AppResult<Option<i64>> {
    match length_ms {
        Some(ms) if ms <= 0 => Err(AppError::validation(
            "length",
            format!("{ms} ms must be positive"),
        )),
        other => Ok(other),
    }
}
