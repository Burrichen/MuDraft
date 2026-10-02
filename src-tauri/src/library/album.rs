//! Album detail read model and manual edits.
//!
//! Every edition keeps its own tracks: a Deluxe edition's tracks are separate rows from the
//! Standard edition's, linked only through shared recording identities when known.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use super::listening::{self as listening_mod, AlbumListening};
use super::listing::TagRef;
use super::metadata_import::{current_genres, is_locked};
use super::{Table, optional_long_text, require, required_text};
use crate::artwork::{self, ArtworkState};
use crate::domain::dates::PartialDate;
use crate::domain::ids::parse_uuid;
use crate::domain::rating::RatingSummary;
use crate::domain::text::norm;
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditView {
    pub artist_id: String,
    pub name: String,
    pub credited_name: Option<String>,
    pub join_phrase: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DateView {
    pub value: Option<String>,
    pub precision: String,
    pub year: Option<u16>,
}

fn date_view(value: Option<String>, precision: String) -> DateView {
    let year = value
        .as_deref()
        .and_then(|v| v.get(..4))
        .and_then(|y| y.parse().ok());
    DateView {
        value,
        precision,
        year,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditionView {
    pub id: String,
    pub name: String,
    pub release_date: DateView,
    pub musicbrainz_release_id: Option<String>,
    pub track_count: u32,
    pub on_listen_list: bool,
    pub in_collection: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackView {
    pub id: String,
    pub position: u32,
    pub title: String,
    /// Milliseconds; `None` means unknown (never treated as zero).
    pub length_ms: Option<i64>,
    /// Shown only when the track credit differs from the album credit.
    pub credit: Option<String>,
    pub recording_id: Option<String>,
    pub rating: Option<u8>,
    pub favourite: bool,
    pub listened: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Disc {
    pub number: u32,
    pub tracks: Vec<TrackView>,
}

/// Total of known lengths; `unknown_tracks` > 0 means the real total is longer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Runtime {
    pub known_ms: i64,
    pub unknown_tracks: u32,
    pub track_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Description {
    pub text: String,
    /// "musicbrainz" (annotation) or "manual".
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumDetail {
    pub album_id: String,
    pub title: String,
    pub credits: Vec<CreditView>,
    pub original_date: DateView,
    pub musicbrainz_release_group_id: Option<String>,
    pub genres: Vec<String>,
    pub tags: Vec<TagRef>,
    pub description: Option<Description>,
    /// Facts from stored metadata only, for when there is no description.
    pub summary: String,
    /// Fields the user edited; refreshes leave them alone.
    pub locked_fields: Vec<String>,
    pub editions: Vec<EditionView>,
    /// The edition being shown.
    pub edition_id: String,
    pub discs: Vec<Disc>,
    pub runtime: Runtime,
    pub artwork: ArtworkState,
    /// Shared rating calculation for the shown edition.
    pub rating: RatingSummary,
    /// The user's review/notes for the shown edition.
    pub review: Option<String>,
    /// Listens for the whole album (all editions), newest first.
    pub listening: AlbumListening,
}

pub fn format_duration(ms: i64) -> String {
    let secs = ms / 1000;
    let (h, m, s) = (secs / 3600, secs % 3600 / 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn credit_text(credits: &[CreditView]) -> String {
    credits
        .iter()
        .map(|c| {
            format!(
                "{}{}",
                c.credited_name.as_deref().unwrap_or(&c.name),
                c.join_phrase
            )
        })
        .collect()
}

/// Plain facts, no opinions: who, when, how long, which genres.
pub fn factual_summary(
    title: &str,
    credits: &[CreditView],
    date: &DateView,
    edition_name: &str,
    discs: usize,
    runtime: &Runtime,
    genres: &[String],
) -> String {
    let mut parts = Vec::new();
    let by = credit_text(credits);
    let mut first = if by.is_empty() {
        format!("“{title}”")
    } else {
        format!("“{title}” by {by}")
    };
    match (&date.value, date.precision.as_str()) {
        (Some(v), "day" | "month") => first.push_str(&format!(", first released {v}")),
        (Some(v), _) => first.push_str(&format!(", first released in {v}")),
        _ => {}
    }
    parts.push(format!("{first}."));
    if runtime.track_count > 0 {
        let length = if runtime.known_ms == 0 {
            "length unknown".to_owned()
        } else if runtime.unknown_tracks > 0 {
            format!("at least {}", format_duration(runtime.known_ms))
        } else {
            format_duration(runtime.known_ms)
        };
        let disc_text = if discs > 1 {
            format!(" on {discs} discs")
        } else {
            String::new()
        };
        parts.push(format!(
            "The {edition_name} edition has {} track{}{disc_text} ({length}).",
            runtime.track_count,
            if runtime.track_count == 1 { "" } else { "s" }
        ));
    }
    if !genres.is_empty() {
        parts.push(format!("Genres: {}.", genres.join(", ")));
    }
    parts.join(" ")
}

/// Default edition to show: the Listen List's, then a collected one, then the earliest.
fn default_edition(conn: &Connection, album_id: &str) -> AppResult<String> {
    let pick: Option<String> = conn
        .query_row(
            "SELECT edition_id FROM listen_list_entry WHERE album_id = ?1
             UNION ALL SELECT edition_id FROM (SELECT edition_id FROM collection_entry WHERE album_id = ?1 ORDER BY added_at)
             UNION ALL SELECT id FROM (SELECT id FROM edition WHERE album_id = ?1 ORDER BY release_date IS NULL, release_date, created_at)
             LIMIT 1",
            params![album_id],
            |r| r.get(0),
        )
        .optional()?;
    pick.ok_or_else(|| AppError::not_found("edition of album", album_id))
}

pub fn detail(
    conn: &Connection,
    album_id: &str,
    edition_id: Option<&str>,
) -> AppResult<AlbumDetail> {
    let album_id = parse_uuid("album", album_id)?;
    let (title, date, precision, rgid, description): (String, Option<String>, String, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT title, original_date, original_date_precision, musicbrainz_release_group_id, description
             FROM album WHERE id = ?1",
            params![album_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("album", &album_id))?;

    let credits: Vec<CreditView> = {
        let mut stmt = conn.prepare(
            "SELECT ar.id, ar.name, c.credited_name, c.join_phrase FROM album_artist_credit c
             JOIN artist ar ON ar.id = c.artist_id WHERE c.album_id = ?1 ORDER BY c.position",
        )?;
        stmt.query_map(params![album_id], |r| {
            Ok(CreditView {
                artist_id: r.get(0)?,
                name: r.get(1)?,
                credited_name: r.get(2)?,
                join_phrase: r.get(3)?,
            })
        })?
        .collect::<Result<_, _>>()?
    };
    let album_artists: Vec<String> = credits.iter().map(|c| c.artist_id.clone()).collect();

    let tags: Vec<TagRef> = {
        let mut stmt = conn.prepare(
            "SELECT t.id, t.name, t.color, t.builtin_key IS NOT NULL FROM album_tag at JOIN tag t ON t.id = at.tag_id
             WHERE at.album_id = ?1",
        )?;
        let mut tags: Vec<TagRef> = stmt
            .query_map(params![album_id], |r| {
                Ok(TagRef {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    color: r.get(2)?,
                    builtin: r.get(3)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        tags.sort_by(|a, b| {
            b.builtin
                .cmp(&a.builtin)
                .then_with(|| norm(&a.name).cmp(&norm(&b.name)))
        });
        tags
    };

    let editions: Vec<EditionView> = {
        let mut stmt = conn.prepare(
            "SELECT e.id, e.name, e.release_date, e.release_date_precision, e.musicbrainz_release_id,
                    (SELECT COUNT(*) FROM track t WHERE t.edition_id = e.id),
                    EXISTS (SELECT 1 FROM listen_list_entry l WHERE l.edition_id = e.id),
                    EXISTS (SELECT 1 FROM collection_entry c WHERE c.edition_id = e.id)
             FROM edition e WHERE e.album_id = ?1 ORDER BY e.release_date IS NULL, e.release_date, e.created_at",
        )?;
        stmt.query_map(params![album_id], |r| {
            Ok(EditionView {
                id: r.get(0)?,
                name: r.get(1)?,
                release_date: date_view(r.get(2)?, r.get(3)?),
                musicbrainz_release_id: r.get(4)?,
                track_count: r.get(5)?,
                on_listen_list: r.get(6)?,
                in_collection: r.get(7)?,
            })
        })?
        .collect::<Result<_, _>>()?
    };

    let edition_id = match edition_id {
        Some(e) => {
            let e = parse_uuid("edition", e)?;
            if !editions.iter().any(|x| x.id == e) {
                return Err(AppError::validation(
                    "edition",
                    "that edition belongs to a different album",
                ));
            }
            e
        }
        None => default_edition(conn, &album_id)?,
    };
    let edition_name = editions
        .iter()
        .find(|e| e.id == edition_id)
        .map(|e| e.name.clone())
        .unwrap_or_default();

    let mut discs: Vec<Disc> = Vec::new();
    let mut runtime = Runtime {
        known_ms: 0,
        unknown_tracks: 0,
        track_count: 0,
    };
    {
        let mut stmt = conn.prepare(
            "SELECT t.id, t.disc_number, t.position, t.title, t.length_ms, t.recording_id,
                    r.rating, COALESCE(r.is_favourite, 0),
                    EXISTS (SELECT 1 FROM listen_event_track l JOIN listen_event ev ON ev.id = l.listen_event_id
                            WHERE l.track_id = t.id AND ev.deleted_at IS NULL)
             FROM track t LEFT JOIN track_rating r ON r.track_id = t.id
             WHERE t.edition_id = ?1 ORDER BY t.disc_number, t.position",
        )?;
        let mut credit_stmt = conn.prepare(
            "SELECT ar.id, ar.name, c.credited_name, c.join_phrase FROM track_artist_credit c
             JOIN artist ar ON ar.id = c.artist_id WHERE c.track_id = ?1 ORDER BY c.position",
        )?;
        let rows = stmt.query_map(params![edition_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, u32>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<u8>>(6)?,
                r.get::<_, bool>(7)?,
                r.get::<_, bool>(8)?,
            ))
        })?;
        for row in rows {
            let (id, disc, position, title, length_ms, recording_id, rating, favourite, listened) =
                row?;
            let track_credits: Vec<CreditView> = credit_stmt
                .query_map(params![id], |r| {
                    Ok(CreditView {
                        artist_id: r.get(0)?,
                        name: r.get(1)?,
                        credited_name: r.get(2)?,
                        join_phrase: r.get(3)?,
                    })
                })?
                .collect::<Result<_, _>>()?;
            let differs = !track_credits.is_empty()
                && track_credits
                    .iter()
                    .map(|c| c.artist_id.clone())
                    .collect::<Vec<_>>()
                    != album_artists;
            runtime.track_count += 1;
            match length_ms {
                Some(ms) => runtime.known_ms += ms,
                None => runtime.unknown_tracks += 1,
            }
            let track = TrackView {
                id,
                position,
                title,
                length_ms,
                credit: differs.then(|| credit_text(&track_credits)),
                recording_id,
                rating,
                favourite,
                listened,
            };
            match discs.last_mut() {
                Some(d) if d.number == disc => d.tracks.push(track),
                _ => discs.push(Disc {
                    number: disc,
                    tracks: vec![track],
                }),
            }
        }
    }

    let genres = current_genres(conn, &album_id)?;
    let original_date = date_view(date, precision);
    let description = match description {
        Some(text) => {
            let source: Option<String> = conn
                .query_row(
                    "SELECT source FROM metadata_provenance
                     WHERE entity_type = 'album' AND entity_id = ?1 AND field = 'description'",
                    params![album_id],
                    |r| r.get(0),
                )
                .optional()?;
            Some(Description {
                text,
                source: source.unwrap_or_else(|| "manual".into()),
            })
        }
        None => None,
    };
    let locked_fields: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT field FROM metadata_provenance WHERE entity_type = 'album' AND entity_id = ?1 AND is_override = 1 ORDER BY field",
        )?;
        stmt.query_map(params![album_id], |r| r.get(0))?
            .collect::<Result<_, _>>()?
    };
    let summary = factual_summary(
        &title,
        &credits,
        &original_date,
        &edition_name,
        discs.len(),
        &runtime,
        &genres,
    );

    let artwork = artwork::lookup(conn, &edition_id)?;
    let rating = super::ratings::summary(conn, &edition_id)?;
    let review = super::personal::album_review(conn, &edition_id)?.review;
    let listening = listening_mod::history(conn, &album_id)?;
    Ok(AlbumDetail {
        listening,
        rating,
        review,
        artwork,
        album_id,
        title,
        credits,
        original_date,
        musicbrainz_release_group_id: rgid,
        genres,
        tags,
        description,
        summary,
        locked_fields,
        editions,
        edition_id,
        discs,
        runtime,
    })
}

/// Fields the user may edit. `Some("")` clears the description or date.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AlbumEdit {
    pub title: Option<String>,
    pub original_date: Option<String>,
    pub description: Option<String>,
}

fn lock(tx: &Transaction<'_>, album_id: &str, field: &str) -> AppResult<()> {
    tx.execute(
        "INSERT INTO metadata_provenance (entity_type, entity_id, field, source, is_override)
         VALUES ('album', ?1, ?2, 'manual', 1)
         ON CONFLICT (entity_type, entity_id, field) DO UPDATE SET source = 'manual', is_override = 1,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        params![album_id, field],
    )?;
    Ok(())
}

/// Apply manual edits. Each edited field is locked so MusicBrainz refreshes keep it.
pub fn edit(tx: &Transaction<'_>, album_id: &str, e: &AlbumEdit) -> AppResult<()> {
    let album_id = parse_uuid("album", album_id)?;
    require(tx, Table::Album, &album_id)?;
    if let Some(title) = &e.title {
        tx.execute(
            "UPDATE album SET title = ?2 WHERE id = ?1",
            params![album_id, required_text("album title", title)?],
        )?;
        lock(tx, &album_id, "title")?;
    }
    if let Some(raw) = &e.original_date {
        let (value, precision) = PartialDate::parse("original date", Some(raw))?.to_db();
        tx.execute(
            "UPDATE album SET original_date = ?2, original_date_precision = ?3 WHERE id = ?1",
            params![album_id, value, precision],
        )?;
        lock(tx, &album_id, "original_date")?;
    }
    if let Some(text) = &e.description {
        tx.execute(
            "UPDATE album SET description = ?2 WHERE id = ?1",
            params![album_id, optional_long_text("description", Some(text))?],
        )?;
        lock(tx, &album_id, "description")?;
    }
    if e.title.is_some() || e.original_date.is_some() || e.description.is_some() {
        tx.execute(
            "UPDATE album SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
            params![album_id],
        )?;
    }
    Ok(())
}

/// Hand a field back to MusicBrainz: the next refresh may replace it again.
pub fn unlock(tx: &Transaction<'_>, album_id: &str, field: &str) -> AppResult<()> {
    if !["title", "original_date", "description", "genres"].contains(&field) {
        return Err(AppError::validation(
            "field",
            format!("“{field}” can’t be unlocked"),
        ));
    }
    tx.execute(
        "UPDATE metadata_provenance SET is_override = 0 WHERE entity_type = 'album' AND entity_id = ?1 AND field = ?2",
        params![parse_uuid("album", album_id)?, field],
    )?;
    Ok(())
}

pub fn rename_edition(tx: &Transaction<'_>, edition_id: &str, name: &str) -> AppResult<()> {
    let edition_id = parse_uuid("edition", edition_id)?;
    let name = required_text("edition name", name)?;
    let album_id: String = tx
        .query_row(
            "SELECT album_id FROM edition WHERE id = ?1",
            params![edition_id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("edition", &edition_id))?;
    let mut stmt = tx.prepare("SELECT name FROM edition WHERE album_id = ?1 AND id != ?2")?;
    let taken = stmt
        .query_map(params![album_id, edition_id], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .any(|n| norm(n) == norm(&name));
    if taken {
        return Err(AppError::Conflict(format!(
            "this album already has an edition named “{name}”"
        )));
    }
    tx.execute(
        "UPDATE edition SET name = ?2 WHERE id = ?1",
        params![edition_id, name],
    )?;
    Ok(())
}

pub fn album_is_locked(conn: &Connection, album_id: &str, field: &str) -> AppResult<bool> {
    is_locked(conn, "album", album_id, field)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_and_summaries_state_only_facts() {
        assert_eq!(format_duration(53_000), "0:53");
        assert_eq!(format_duration(3_201_000), "53:21");
        assert_eq!(format_duration(4_500_000), "1:15:00");
        let credits = vec![CreditView {
            artist_id: "a".into(),
            name: "Radiohead".into(),
            credited_name: None,
            join_phrase: String::new(),
        }];
        let date = DateView {
            value: Some("1997-05-21".into()),
            precision: "day".into(),
            year: Some(1997),
        };
        let rt = Runtime {
            known_ms: 3_201_000,
            unknown_tracks: 1,
            track_count: 12,
        };
        let s = factual_summary(
            "OK Computer",
            &credits,
            &date,
            "Standard",
            1,
            &rt,
            &["Rock".into()],
        );
        assert_eq!(
            s,
            "“OK Computer” by Radiohead, first released 1997-05-21. The Standard edition has 12 tracks (at least 53:21). Genres: Rock."
        );
        let none = Runtime {
            known_ms: 0,
            unknown_tracks: 3,
            track_count: 3,
        };
        let unknown = DateView {
            value: None,
            precision: "unknown".into(),
            year: None,
        };
        assert_eq!(
            factual_summary("Demo", &[], &unknown, "Standard", 2, &none, &[]),
            "“Demo”. The Standard edition has 3 tracks on 2 discs (length unknown)."
        );
    }
}
