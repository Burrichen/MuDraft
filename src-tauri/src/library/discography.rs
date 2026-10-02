//! Artist discography catalogues: canonical albums discovered online (MusicBrainz release
//! groups) or added by hand, kept apart from the library — nothing here touches the Listen
//! List or Collection. Progress counts one *reference edition* per canonical album, never
//! the union of every pressing, and recognizes tracks across editions only by identity
//! (same track, MusicBrainz track ID, or MusicBrainz recording ID) — never by title.
//! Unknown is reported as unknown: a missing tracklist is not zero tracks.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use super::ratings::{self, AlbumScore};
use super::{Table, require, required_text};
use crate::domain::ids::{new_id, parse_uuid};
use crate::error::{AppError, AppResult};
use crate::metadata::clock::utc_now;
use crate::metadata::{EditionCandidate, ReleaseDetail, ReleaseGroupPage, credit_text};

pub const PRIMARY_TYPES: &[&str] = &["Album", "EP", "Single", "Broadcast", "Other"];
pub const SECONDARY_TYPES: &[&str] = &[
    "Compilation",
    "Soundtrack",
    "Spokenword",
    "Interview",
    "Audiobook",
    "Audio drama",
    "Live",
    "Remix",
    "DJ-mix",
    "Mixtape/Street",
    "Demo",
    "Field recording",
];

/// Which release-group types count toward progress. An entry counts when its primary type
/// is listed and every one of its secondary types is listed too.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scope {
    pub primary_types: Vec<String>,
    pub secondary_types: Vec<String>,
}

impl Default for Scope {
    /// Official studio albums only: no singles, EPs, live albums, compilations, etc.
    fn default() -> Self {
        Self {
            primary_types: vec!["Album".into()],
            secondary_types: vec![],
        }
    }
}

fn known_type(field: &'static str, allowed: &[&str], raw: &str) -> AppResult<String> {
    allowed
        .iter()
        .find(|t| t.eq_ignore_ascii_case(raw.trim()))
        .map(|t| (*t).to_owned())
        .ok_or_else(|| AppError::validation(field, format!("“{raw}” isn't a MusicBrainz type")))
}

impl Scope {
    pub fn validated(&self) -> AppResult<Self> {
        let mut primary = self
            .primary_types
            .iter()
            .map(|t| known_type("primary type", PRIMARY_TYPES, t))
            .collect::<AppResult<Vec<_>>>()?;
        if primary.is_empty() {
            return Err(AppError::validation(
                "primary type",
                "count at least one type",
            ));
        }
        let mut secondary = self
            .secondary_types
            .iter()
            .map(|t| known_type("secondary type", SECONDARY_TYPES, t))
            .collect::<AppResult<Vec<_>>>()?;
        primary.sort_unstable();
        primary.dedup();
        secondary.sort_unstable();
        secondary.dedup();
        Ok(Self {
            primary_types: primary,
            secondary_types: secondary,
        })
    }

    fn counts(&self, primary: Option<&str>, secondary: &[String]) -> bool {
        primary.is_some_and(|p| self.primary_types.iter().any(|t| t == p))
            && secondary
                .iter()
                .all(|s| self.secondary_types.iter().any(|t| t == s))
    }

    /// The denominator in words.
    pub fn describe(&self) -> String {
        let primary = self.primary_types.join(", ");
        let secondary = if self.secondary_types.is_empty() {
            "with no secondary type (so no live albums, compilations, soundtracks, remixes, or demos)"
                .to_owned()
        } else {
            format!(
                "with no secondary type, or only these: {}",
                self.secondary_types.join(", ")
            )
        };
        format!(
            "Counting MusicBrainz release groups of type {primary} {secondary}, from the official \
             catalogue MusicBrainz shows on its artist page, including collaborations. Each album \
             counts once however many editions exist. Albums in your library that aren't in this \
             catalogue are counted too; entries you exclude are not."
        )
    }
}

pub fn type_label(primary: Option<&str>, secondary: &[String]) -> String {
    let primary = primary.unwrap_or("Unknown type");
    if secondary.is_empty() {
        primary.to_owned()
    } else {
        format!("{primary} · {}", secondary.join(", "))
    }
}

/// A credited artist as stored with a catalogue entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredCredit {
    musicbrainz_artist_id: String,
    name: String,
    join_phrase: String,
}

// ---------------------------------------------------------------- fetch state

pub fn artist_musicbrainz_id(conn: &Connection, artist_id: &str) -> AppResult<Option<String>> {
    let artist_id = parse_uuid("artist", artist_id)?;
    conn.query_row(
        "SELECT musicbrainz_id FROM artist WHERE id = ?1",
        params![artist_id],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| AppError::not_found("artist", &artist_id))
}

/// Start or resume a pass. Returns the offset to request next.
pub fn begin_pass(tx: &Transaction<'_>, artist_id: &str, mbid: &str) -> AppResult<u32> {
    let row: Option<(Option<String>, i64)> = tx
        .query_row(
            "SELECT musicbrainz_artist_id, next_offset FROM artist_catalogue WHERE artist_id = ?1",
            params![artist_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match row {
        Some((Some(prev), offset)) if prev == mbid => {
            if offset == 0 {
                tx.execute(
                    "UPDATE artist_catalogue SET started_at = ?2 WHERE artist_id = ?1",
                    params![artist_id, utc_now()],
                )?;
            }
            Ok(u32::try_from(offset).unwrap_or(0))
        }
        Some(_) => {
            // Relinked to a different MusicBrainz artist: the old discoveries don't apply.
            tx.execute(
                "DELETE FROM catalogue_entry WHERE artist_id = ?1 AND source = 'musicbrainz'",
                params![artist_id],
            )?;
            tx.execute(
                "UPDATE artist_catalogue SET musicbrainz_artist_id = ?2, provider_total = NULL,
                     next_offset = 0, started_at = ?3, page_fetched_at = NULL,
                     completed_at = NULL, last_error = NULL
                 WHERE artist_id = ?1",
                params![artist_id, mbid, utc_now()],
            )?;
            Ok(0)
        }
        None => {
            tx.execute(
                "INSERT INTO artist_catalogue (artist_id, musicbrainz_artist_id, started_at)
                 VALUES (?1, ?2, ?3)",
                params![artist_id, mbid, utc_now()],
            )?;
            Ok(0)
        }
    }
}

/// Store one page and advance the resume point. Returns true when the pass is complete.
pub fn record_page(
    tx: &Transaction<'_>,
    artist_id: &str,
    page: &ReleaseGroupPage,
    fetched_at: &str,
) -> AppResult<bool> {
    for rg in &page.release_groups {
        let secondary = serde_json::to_string(&rg.secondary_types)
            .map_err(|e| AppError::Internal(format!("cannot encode types: {e}")))?;
        let parts: Vec<StoredCredit> = rg
            .artist_credit
            .iter()
            .map(|c| StoredCredit {
                musicbrainz_artist_id: c.artist_id.clone(),
                name: c
                    .credited_name
                    .clone()
                    .unwrap_or_else(|| c.artist_name.clone()),
                join_phrase: c.join_phrase.clone(),
            })
            .collect();
        let parts = serde_json::to_string(&parts)
            .map_err(|e| AppError::Internal(format!("cannot encode credits: {e}")))?;
        tx.execute(
            "INSERT INTO catalogue_entry (id, artist_id, source, musicbrainz_release_group_id, title,
                 disambiguation, primary_type, secondary_types, original_date, original_year,
                 credit, credit_artists, fetched_at, credit_parts)
             VALUES (?1, ?2, 'musicbrainz', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT (artist_id, musicbrainz_release_group_id) DO UPDATE SET
                 title = excluded.title, disambiguation = excluded.disambiguation,
                 primary_type = excluded.primary_type, secondary_types = excluded.secondary_types,
                 original_date = excluded.original_date, original_year = excluded.original_year,
                 credit = excluded.credit, credit_artists = excluded.credit_artists,
                 fetched_at = excluded.fetched_at, credit_parts = excluded.credit_parts",
            params![
                new_id(),
                artist_id,
                rg.id,
                rg.title,
                rg.disambiguation,
                rg.primary_type,
                secondary,
                rg.original_date.value,
                rg.original_date.year,
                credit_text(&rg.artist_credit),
                rg.artist_credit.len() as i64,
                fetched_at,
                parts,
            ],
        )?;
    }
    let next = page.offset + page.received;
    let done = page.received == 0 || next >= page.total;
    tx.execute(
        "UPDATE artist_catalogue SET provider_total = ?2, page_fetched_at = ?3, last_error = NULL,
             next_offset = CASE WHEN ?4 THEN 0 ELSE ?5 END,
             completed_at = CASE WHEN ?4 THEN ?6 ELSE completed_at END
         WHERE artist_id = ?1",
        params![artist_id, page.total, fetched_at, done, next, utc_now()],
    )?;
    Ok(done)
}

/// Keep everything fetched so far; only note what went wrong.
pub fn record_pass_error(tx: &Transaction<'_>, artist_id: &str, message: &str) -> AppResult<()> {
    tx.execute(
        "UPDATE artist_catalogue SET last_error = ?2 WHERE artist_id = ?1",
        params![artist_id, message],
    )?;
    Ok(())
}

// ---------------------------------------------------------------- reference editions

pub const REPRESENTATIVE_RULE: &str = "The earliest-dated official release that has a \
     tracklist (releases with unknown dates come last; ties go to the lowest MusicBrainz ID). \
     Bonus-heavy reissues are usually later, so this tends to be the original tracklist.";

/// Pick one representative release. Official releases are preferred when any exist.
pub fn choose_representative(editions: &[EditionCandidate]) -> Option<&EditionCandidate> {
    let usable: Vec<&EditionCandidate> = editions.iter().filter(|e| e.track_count > 0).collect();
    let official: Vec<&EditionCandidate> = usable
        .iter()
        .copied()
        .filter(|e| e.status.as_deref() == Some("Official"))
        .collect();
    let pool = if official.is_empty() {
        usable
    } else {
        official
    };
    pool.into_iter().min_by(|a, b| {
        (a.date.value.is_none(), &a.date.value, &a.id).cmp(&(
            b.date.value.is_none(),
            &b.date.value,
            &b.id,
        ))
    })
}

pub fn record_representative(
    tx: &Transaction<'_>,
    release_group_id: &str,
    editions: &[EditionCandidate],
    fetched_at: &str,
) -> AppResult<Option<String>> {
    let chosen = choose_representative(editions);
    let formats = serde_json::to_string(&chosen.map(|e| e.formats.clone()).unwrap_or_default())
        .map_err(|e| AppError::Internal(format!("cannot encode formats: {e}")))?;
    let error = chosen
        .is_none()
        .then_some("MusicBrainz lists no release with a tracklist for this album");
    tx.execute(
        "INSERT INTO catalogue_reference (musicbrainz_release_group_id, musicbrainz_release_id, title,
             release_date, country, formats, editions_considered, editions_fetched_at, last_error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (musicbrainz_release_group_id) DO UPDATE SET
             musicbrainz_release_id = excluded.musicbrainz_release_id, title = excluded.title,
             release_date = excluded.release_date, country = excluded.country,
             formats = excluded.formats, editions_considered = excluded.editions_considered,
             editions_fetched_at = excluded.editions_fetched_at, last_error = excluded.last_error,
             -- A different release means the stored tracklist no longer applies.
             track_count = CASE WHEN catalogue_reference.musicbrainz_release_id IS excluded.musicbrainz_release_id
                                THEN catalogue_reference.track_count END,
             tracks_fetched_at = CASE WHEN catalogue_reference.musicbrainz_release_id IS excluded.musicbrainz_release_id
                                THEN catalogue_reference.tracks_fetched_at END",
        params![
            release_group_id,
            chosen.map(|e| e.id.clone()),
            chosen.map(|e| e.title.clone()),
            chosen.and_then(|e| e.date.value.clone()),
            chosen.and_then(|e| e.country.clone()),
            formats,
            editions.len() as i64,
            fetched_at,
            error,
        ],
    )?;
    Ok(chosen.map(|e| e.id.clone()))
}

pub fn record_tracklist(
    tx: &Transaction<'_>,
    release_group_id: &str,
    release: &ReleaseDetail,
    fetched_at: &str,
) -> AppResult<()> {
    tx.execute(
        "DELETE FROM catalogue_track WHERE musicbrainz_release_id = ?1",
        params![release.id],
    )?;
    for t in &release.tracks {
        tx.execute(
            "INSERT OR REPLACE INTO catalogue_track (musicbrainz_release_id, disc, position, title,
                 length_ms, musicbrainz_track_id, musicbrainz_recording_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                release.id,
                t.disc,
                t.position,
                t.title,
                t.length_ms,
                t.id,
                t.recording_id
            ],
        )?;
    }
    tx.execute(
        "UPDATE catalogue_reference SET track_count = ?3, tracks_fetched_at = ?4, last_error = NULL
         WHERE musicbrainz_release_group_id = ?1 AND musicbrainz_release_id = ?2",
        params![
            release_group_id,
            release.id,
            release.tracks.len() as i64,
            fetched_at
        ],
    )?;
    Ok(())
}

pub fn record_reference_error(
    tx: &Transaction<'_>,
    release_group_id: &str,
    message: &str,
) -> AppResult<()> {
    tx.execute(
        "INSERT INTO catalogue_reference (musicbrainz_release_group_id, last_error) VALUES (?1, ?2)
         ON CONFLICT (musicbrainz_release_group_id) DO UPDATE SET last_error = excluded.last_error",
        params![release_group_id, message],
    )?;
    Ok(())
}

/// Release groups in scope whose tracklist must come from a representative release and
/// is not loaded yet — what a tracklist run still has to do (so it can resume).
pub fn tracklist_targets(conn: &Connection, artist_id: &str) -> AppResult<Vec<String>> {
    let view = catalogue(conn, artist_id)?;
    Ok(view
        .entries
        .into_iter()
        .filter(|e| e.counted && e.reference.kind != ReferenceKind::YourEdition)
        .filter(|e| e.reference.track_count.is_none())
        .filter_map(|e| e.release_group_id)
        .collect())
}

/// The release group of a discovered entry (manual entries have none to import).
pub fn entry_release_group(conn: &Connection, entry_id: &str) -> AppResult<String> {
    let id = parse_uuid("catalogue entry", entry_id)?;
    conn.query_row(
        "SELECT musicbrainz_release_group_id FROM catalogue_entry WHERE id = ?1",
        params![id],
        |r| r.get::<_, Option<String>>(0),
    )
    .optional()?
    .ok_or_else(|| AppError::not_found("catalogue entry", &id))?
    .ok_or_else(|| {
        AppError::validation(
            "catalogue entry",
            "this album was added by hand; add it from Listen List → Add album instead",
        )
    })
}

/// An existing library edition of the release group, if the album is already here.
pub fn library_edition_for_group(conn: &Connection, group: &str) -> AppResult<Option<String>> {
    let album: Option<String> = conn
        .query_row(
            "SELECT id FROM album WHERE musicbrainz_release_group_id = ?1",
            params![group],
            |r| r.get(0),
        )
        .optional()?;
    Ok(match album {
        Some(a) => your_edition(conn, &a, true)?.map(|y| y.id),
        None => None,
    })
}

pub fn stored_representative(conn: &Connection, group: &str) -> AppResult<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT musicbrainz_release_id FROM catalogue_reference WHERE musicbrainz_release_group_id = ?1",
            params![group],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten())
}

// ---------------------------------------------------------------- corrections

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManualEntry {
    pub title: String,
    pub year: Option<i32>,
    pub primary_type: Option<String>,
    #[serde(default)]
    pub secondary_types: Vec<String>,
}

/// Add an album the online catalogue lacks (or the whole catalogue of a manual artist).
pub fn add_manual(tx: &Transaction<'_>, artist_id: &str, entry: &ManualEntry) -> AppResult<String> {
    let artist_id = parse_uuid("artist", artist_id)?;
    require(tx, Table::Artist, &artist_id)?;
    let title = required_text("title", &entry.title)?;
    if let Some(y) = entry.year
        && !(1000..=9999).contains(&y)
    {
        return Err(AppError::validation("year", "use four digits"));
    }
    let primary = entry
        .primary_type
        .as_deref()
        .map(|t| known_type("primary type", PRIMARY_TYPES, t))
        .transpose()?;
    let secondary = entry
        .secondary_types
        .iter()
        .map(|t| known_type("secondary type", SECONDARY_TYPES, t))
        .collect::<AppResult<Vec<_>>>()?;
    let id = new_id();
    tx.execute(
        "INSERT INTO catalogue_entry (id, artist_id, source, title, primary_type, secondary_types,
             original_date, original_year)
         VALUES (?1, ?2, 'manual', ?3, ?4, ?5, ?6, ?6)",
        params![
            id,
            artist_id,
            title,
            primary,
            serde_json::to_string(&secondary).unwrap_or_else(|_| "[]".into()),
            entry.year,
        ],
    )?;
    Ok(id)
}

pub fn remove_manual(tx: &Transaction<'_>, entry_id: &str) -> AppResult<()> {
    let id = parse_uuid("catalogue entry", entry_id)?;
    let n = tx.execute(
        "DELETE FROM catalogue_entry WHERE id = ?1 AND source = 'manual'",
        params![id],
    )?;
    if n == 0 {
        return Err(AppError::not_found("manual catalogue entry", &id));
    }
    Ok(())
}

/// Exclude (or re-include) an entry from progress, e.g. a mis-credited release group.
pub fn set_excluded(tx: &Transaction<'_>, entry_id: &str, excluded: bool) -> AppResult<()> {
    let id = parse_uuid("catalogue entry", entry_id)?;
    let n = tx.execute(
        "UPDATE catalogue_entry SET excluded = ?2 WHERE id = ?1",
        params![id, excluded],
    )?;
    if n == 0 {
        return Err(AppError::not_found("catalogue entry", &id));
    }
    Ok(())
}

pub fn set_scope(tx: &Transaction<'_>, artist_id: &str, scope: &Scope) -> AppResult<Scope> {
    let artist_id = parse_uuid("artist", artist_id)?;
    require(tx, Table::Artist, &artist_id)?;
    let scope = scope.validated()?;
    let json = (scope != Scope::default())
        .then(|| serde_json::to_string(&scope))
        .transpose()
        .map_err(|e| AppError::Internal(format!("cannot encode scope: {e}")))?;
    tx.execute(
        "INSERT INTO artist_catalogue (artist_id, scope) VALUES (?1, ?2)
         ON CONFLICT (artist_id) DO UPDATE SET scope = excluded.scope",
        params![artist_id, json],
    )?;
    Ok(scope)
}

// ---------------------------------------------------------------- read model

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogueKind {
    /// Linked to MusicBrainz: the online catalogue can be fetched.
    Online,
    /// Manual artist: only library albums and entries you add are known.
    LocalOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStatus {
    NotFetched,
    /// Some pages fetched; no full pass has finished.
    Partial,
    Complete,
    LocalOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Coverage {
    pub kind: CatalogueKind,
    pub status: CoverageStatus,
    pub musicbrainz_artist_id: Option<String>,
    pub provider_total: Option<u32>,
    pub fetched_entries: u32,
    /// Resume point of an unfinished pass (0 when none is in progress).
    pub next_offset: u32,
    pub started_at: Option<String>,
    pub page_fetched_at: Option<String>,
    pub completed_at: Option<String>,
    pub last_error: Option<String>,
    pub manual_entries: u32,
    /// Counted albums whose reference tracklist is known, of those counted.
    pub tracklists_known: u32,
    pub tracklists_needed: u32,
    pub note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntrySource {
    Musicbrainz,
    Manual,
    /// In your library but not in the fetched online catalogue.
    Library,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceKind {
    /// Your Listen List or Collection edition (or your only edition).
    YourEdition,
    /// Chosen by `REPRESENTATIVE_RULE` because you have no edition of the album.
    Representative,
    /// Not chosen yet (editions not loaded) or impossible (manual entry, no tracklist).
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reference {
    pub kind: ReferenceKind,
    pub edition_id: Option<String>,
    pub musicbrainz_release_id: Option<String>,
    pub name: Option<String>,
    pub date: Option<String>,
    pub country: Option<String>,
    pub formats: Vec<String>,
    /// `None` = tracklist unknown (not loaded, or none exists) — never reported as zero.
    pub track_count: Option<u32>,
    pub note: String,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackProgress {
    /// Distinct track identities in the reference tracklist.
    pub total: u32,
    pub listened: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditLink {
    pub name: String,
    pub join_phrase: String,
    pub artist_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalAverage {
    /// Mean of album scores, each canonical album once; `None` when nothing is rated.
    pub stars: Option<f64>,
    pub rated_albums: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// Catalogue entry ID; `None` for library albums outside the catalogue.
    pub id: Option<String>,
    /// Credited artists; `artist_id` is set when that artist has a page here.
    pub credits: Vec<CreditLink>,
    /// Average of the effective scores of this album's rated editions.
    pub score: Option<AlbumScore>,
    /// Position among rated albums on this page (1 = best).
    pub rank: Option<u32>,
    pub source: EntrySource,
    pub release_group_id: Option<String>,
    pub album_id: Option<String>,
    pub title: String,
    pub disambiguation: Option<String>,
    pub credit: String,
    pub collaboration: bool,
    pub original_year: Option<i64>,
    pub primary_type: Option<String>,
    pub secondary_types: Vec<String>,
    pub type_label: String,
    pub excluded: bool,
    /// Counts toward progress (in scope and not excluded).
    pub counted: bool,
    pub on_listen_list: bool,
    pub in_collection: bool,
    /// A full-album listen (any edition, dated or not) or confirmed earlier listening
    /// exists. Hearing tracks one by one never completes an album.
    pub listened: bool,
    pub reference: Reference,
    /// `None` when the reference tracklist is unknown.
    pub tracks: Option<TrackProgress>,
    pub fetched_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub albums_counted: u32,
    pub albums_listened: u32,
    /// Distinct track identities over counted albums whose reference tracklist is known
    /// (a recording shared by two albums counts once).
    pub tracks_total: u32,
    pub tracks_listened: u32,
    pub albums_without_tracklist: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeCount {
    pub label: String,
    pub count: u32,
    pub counted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistCatalogue {
    pub artist_id: String,
    pub artist_name: String,
    pub sort_name: Option<String>,
    pub disambiguation: Option<String>,
    pub musicbrainz_id: Option<String>,
    pub average: PersonalAverage,
    pub coverage: Coverage,
    pub scope: Scope,
    pub denominator: String,
    pub representative_rule: &'static str,
    pub entries: Vec<Entry>,
    pub types: Vec<TypeCount>,
    pub progress: Progress,
}

struct RawEntry {
    id: Option<String>,
    source: EntrySource,
    release_group_id: Option<String>,
    album_id: Option<String>,
    title: String,
    disambiguation: Option<String>,
    credit: String,
    credit_artists: i64,
    original_year: Option<i64>,
    primary_type: Option<String>,
    secondary_types: Vec<String>,
    excluded: bool,
    fetched_at: Option<String>,
    credit_parts: String,
}

/// Listening evidence: every track heard in a live (not deleted) listen, by identity.
#[derive(Default)]
struct Heard {
    tracks: HashSet<String>,
    provider_tracks: HashSet<String>,
    recordings: HashSet<String>,
}

fn heard(conn: &Connection) -> AppResult<Heard> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT t.id, t.musicbrainz_track_id, r.musicbrainz_recording_id
         FROM listen_event_track lt
         JOIN listen_event e ON e.id = lt.listen_event_id AND e.deleted_at IS NULL
         JOIN track t ON t.id = lt.track_id
         LEFT JOIN recording r ON r.id = t.recording_id",
    )?;
    let mut h = Heard::default();
    for row in stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    })? {
        let (id, mb_track, recording) = row?;
        h.tracks.insert(id);
        h.provider_tracks.extend(mb_track);
        h.recordings.extend(recording);
    }
    Ok(h)
}

/// (local track id, MusicBrainz track id, MusicBrainz recording id)
type TrackIdentity = (Option<String>, Option<String>, Option<String>);

/// The identity a reference track is counted under: its recording when known (shared
/// across editions and albums), otherwise its own track. Titles are never used.
fn identity_key(t: &TrackIdentity) -> String {
    match t {
        (_, _, Some(recording)) => format!("recording:{recording}"),
        (_, Some(mb_track), None) => format!("mb-track:{mb_track}"),
        (Some(local), None, None) => format!("track:{local}"),
        (None, None, None) => "unidentified".into(),
    }
}

fn has_evidence(t: &TrackIdentity, h: &Heard) -> bool {
    let (local, mb_track, recording) = t;
    local.as_ref().is_some_and(|x| h.tracks.contains(x))
        || mb_track
            .as_ref()
            .is_some_and(|x| h.provider_tracks.contains(x))
        || recording.as_ref().is_some_and(|x| h.recordings.contains(x))
}

/// The user's chosen edition of a local album: Listen List, else the first Collection
/// edition, else (albums only known locally) the earliest-created edition.
struct YourEdition {
    id: String,
    name: String,
    date: Option<String>,
    musicbrainz_release_id: Option<String>,
    how: String,
}

fn your_edition(
    conn: &Connection,
    album_id: &str,
    allow_any: bool,
) -> AppResult<Option<YourEdition>> {
    let row = conn
        .query_row(
            "SELECT e.id, e.name, e.release_date, e.musicbrainz_release_id,
                    CASE WHEN l.edition_id IS NOT NULL THEN 'listen_list'
                         WHEN c.edition_id IS NOT NULL THEN 'collection' ELSE 'library' END AS how
             FROM edition e
             LEFT JOIN listen_list_entry l ON l.edition_id = e.id
             LEFT JOIN collection_entry c ON c.edition_id = e.id
             WHERE e.album_id = ?1 AND (?2 OR l.edition_id IS NOT NULL OR c.edition_id IS NOT NULL)
             ORDER BY l.edition_id IS NULL, c.edition_id IS NULL, c.added_at, e.created_at, e.id
             LIMIT 1",
            params![album_id, allow_any],
            |r| {
                Ok(YourEdition {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    date: r.get(2)?,
                    musicbrainz_release_id: r.get(3)?,
                    how: r.get(4)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

fn local_tracks(conn: &Connection, edition_id: &str) -> AppResult<Vec<TrackIdentity>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.musicbrainz_track_id, r.musicbrainz_recording_id FROM track t
         LEFT JOIN recording r ON r.id = t.recording_id WHERE t.edition_id = ?1",
    )?;
    let rows = stmt.query_map(params![edition_id], |r| {
        Ok((Some(r.get(0)?), r.get(1)?, r.get(2)?))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn representative_tracks(conn: &Connection, release_id: &str) -> AppResult<Vec<TrackIdentity>> {
    let mut stmt = conn.prepare(
        "SELECT musicbrainz_track_id, musicbrainz_recording_id FROM catalogue_track
         WHERE musicbrainz_release_id = ?1",
    )?;
    let rows = stmt.query_map(params![release_id], |r| Ok((None, r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn raw_entries(conn: &Connection, artist_id: &str) -> AppResult<Vec<RawEntry>> {
    let mut out = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT c.id, c.source, c.musicbrainz_release_group_id,
                (SELECT a.id FROM album a WHERE a.musicbrainz_release_group_id = c.musicbrainz_release_group_id),
                c.title, c.disambiguation, c.credit, c.credit_artists, c.original_year,
                c.primary_type, c.secondary_types, c.excluded, c.fetched_at, c.credit_parts
         FROM catalogue_entry c WHERE c.artist_id = ?1",
    )?;
    let rows = stmt.query_map(params![artist_id], |r| {
        let secondary: String = r.get(10)?;
        Ok(RawEntry {
            id: Some(r.get(0)?),
            source: if r.get::<_, String>(1)? == "manual" {
                EntrySource::Manual
            } else {
                EntrySource::Musicbrainz
            },
            release_group_id: r.get(2)?,
            album_id: r.get(3)?,
            title: r.get(4)?,
            disambiguation: r.get(5)?,
            credit: r.get(6)?,
            credit_artists: r.get(7)?,
            original_year: r.get(8)?,
            primary_type: r.get(9)?,
            secondary_types: serde_json::from_str(&secondary).unwrap_or_default(),
            excluded: r.get(11)?,
            fetched_at: r.get(12)?,
            credit_parts: r.get(13)?,
        })
    })?;
    for row in rows {
        out.push(row?);
    }
    // Library albums credited to the artist that the catalogue doesn't contain.
    let known: HashSet<String> = out
        .iter()
        .filter_map(|e| e.release_group_id.clone())
        .collect();
    let mut stmt = conn.prepare(
        "SELECT a.id, a.musicbrainz_release_group_id, a.title, a.original_year,
                (SELECT group_concat(COALESCE(c2.credited_name, ar.name) || c2.join_phrase, '' ORDER BY c2.position)
                 FROM album_artist_credit c2 JOIN artist ar ON ar.id = c2.artist_id
                 WHERE c2.album_id = a.id),
                (SELECT COUNT(*) FROM album_artist_credit c3 WHERE c3.album_id = a.id)
         FROM album a JOIN album_artist_credit c ON c.album_id = a.id
         WHERE c.artist_id = ?1",
    )?;
    let rows = stmt.query_map(params![artist_id], |r| {
        Ok(RawEntry {
            id: None,
            source: EntrySource::Library,
            album_id: Some(r.get(0)?),
            release_group_id: r.get(1)?,
            title: r.get(2)?,
            original_year: r.get(3)?,
            credit: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
            credit_artists: r.get(5)?,
            disambiguation: None,
            primary_type: None,
            secondary_types: vec![],
            excluded: false,
            fetched_at: None,
            credit_parts: "[]".into(),
        })
    })?;
    for row in rows {
        let e = row?;
        if !e
            .release_group_id
            .as_ref()
            .is_some_and(|g| known.contains(g))
        {
            out.push(e);
        }
    }
    out.sort_by(|a, b| {
        (
            a.original_year.is_none(),
            a.original_year,
            a.title.to_lowercase(),
        )
            .cmp(&(
                b.original_year.is_none(),
                b.original_year,
                b.title.to_lowercase(),
            ))
    });
    Ok(out)
}

struct StoredReference {
    release: Option<String>,
    title: Option<String>,
    date: Option<String>,
    country: Option<String>,
    formats: String,
    considered: i64,
    count: Option<i64>,
    error: Option<String>,
}

fn reference_for(conn: &Connection, raw: &RawEntry) -> AppResult<(Reference, Vec<TrackIdentity>)> {
    if let Some(album) = &raw.album_id
        && let Some((edition_id, name, date, mb, how)) =
            your_edition(conn, album, raw.release_group_id.is_none())?
                .map(|y| (y.id, y.name, y.date, y.musicbrainz_release_id, y.how))
    {
        let tracks = local_tracks(conn, &edition_id)?;
        let note = match how.as_str() {
            "listen_list" => "Your Listen List edition.",
            "collection" => "Your Collection edition.",
            _ => "Your edition in the library (the album is on neither list).",
        };
        return Ok((
            Reference {
                kind: ReferenceKind::YourEdition,
                edition_id: Some(edition_id),
                musicbrainz_release_id: mb,
                name: Some(name),
                date,
                country: None,
                formats: vec![],
                track_count: (!tracks.is_empty()).then_some(tracks.len() as u32),
                note: note.into(),
                last_error: None,
            },
            tracks,
        ));
    }
    let none = |note: &str, last_error: Option<String>| Reference {
        kind: ReferenceKind::None,
        edition_id: None,
        musicbrainz_release_id: None,
        name: None,
        date: None,
        country: None,
        formats: vec![],
        track_count: None,
        note: note.into(),
        last_error,
    };
    let Some(group) = &raw.release_group_id else {
        return Ok((none("Added by hand; no tracklist is known.", None), vec![]));
    };
    let stored = conn
        .query_row(
            "SELECT musicbrainz_release_id, title, release_date, country, formats,
                    editions_considered, track_count, last_error
             FROM catalogue_reference WHERE musicbrainz_release_group_id = ?1",
            params![group],
            |r| {
                Ok(StoredReference {
                    release: r.get(0)?,
                    title: r.get(1)?,
                    date: r.get(2)?,
                    country: r.get(3)?,
                    formats: r.get(4)?,
                    considered: r.get(5)?,
                    count: r.get(6)?,
                    error: r.get(7)?,
                })
            },
        )
        .optional()?;
    match stored {
        Some(StoredReference {
            release: Some(release),
            title,
            date,
            country,
            formats,
            considered,
            count,
            error,
        }) => {
            let tracks = if count.is_some() {
                representative_tracks(conn, &release)?
            } else {
                vec![]
            };
            Ok((
                Reference {
                    kind: ReferenceKind::Representative,
                    edition_id: None,
                    musicbrainz_release_id: Some(release),
                    name: title,
                    date,
                    country,
                    formats: serde_json::from_str(&formats).unwrap_or_default(),
                    track_count: count.map(|c| c as u32),
                    note: format!(
                        "Representative of {considered} MusicBrainz editions (you have none of this album)."
                    ),
                    last_error: error,
                },
                tracks,
            ))
        }
        Some(StoredReference {
            release: None,
            error,
            ..
        }) => Ok((none("No representative edition yet.", error), vec![])),
        None => Ok((none("Tracklist not loaded yet.", None), vec![])),
    }
}

/// Library albums use their stored credits; discovered ones link credited artists that
/// exist here by MusicBrainz ID only.
fn credits_for(conn: &Connection, raw: &RawEntry) -> AppResult<Vec<CreditLink>> {
    if let Some(album) = &raw.album_id {
        let mut stmt = conn.prepare(
            "SELECT ar.id, COALESCE(c.credited_name, ar.name), c.join_phrase
             FROM album_artist_credit c JOIN artist ar ON ar.id = c.artist_id
             WHERE c.album_id = ?1 ORDER BY c.position",
        )?;
        let rows = stmt.query_map(params![album], |r| {
            Ok(CreditLink {
                artist_id: Some(r.get(0)?),
                name: r.get(1)?,
                join_phrase: r.get(2)?,
            })
        })?;
        return Ok(rows.collect::<Result<_, _>>()?);
    }
    let parts: Vec<StoredCredit> = serde_json::from_str(&raw.credit_parts).unwrap_or_default();
    parts
        .into_iter()
        .map(|p| {
            let artist_id = conn
                .query_row(
                    "SELECT id FROM artist WHERE musicbrainz_id = ?1",
                    params![p.musicbrainz_artist_id],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(CreditLink {
                name: p.name,
                join_phrase: p.join_phrase,
                artist_id,
            })
        })
        .collect()
}

pub fn catalogue(conn: &Connection, artist_id: &str) -> AppResult<ArtistCatalogue> {
    let artist_id = parse_uuid("artist", artist_id)?;
    let (artist_name, sort_name, disambiguation, mbid): (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = conn
        .query_row(
            "SELECT name, sort_name, disambiguation, musicbrainz_id FROM artist WHERE id = ?1",
            params![artist_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("artist", &artist_id))?;
    #[allow(clippy::type_complexity)]
    let state: Option<(Option<String>, Option<i64>, i64, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>)> =
        conn.query_row(
            "SELECT musicbrainz_artist_id, provider_total, next_offset, started_at, page_fetched_at,
                    completed_at, last_error, scope
             FROM artist_catalogue WHERE artist_id = ?1",
            params![artist_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)),
        )
        .optional()?;
    let scope: Scope = state
        .as_ref()
        .and_then(|s| s.7.as_deref())
        .and_then(|j| serde_json::from_str(j).ok())
        .unwrap_or_default();

    let evidence = heard(conn)?;
    let mut entries = Vec::new();
    let mut identities: Vec<Vec<TrackIdentity>> = Vec::new();
    for raw in raw_entries(conn, &artist_id)? {
        let (reference, tracks) = reference_for(conn, &raw)?;
        let progress = reference.track_count.map(|_| {
            let mut seen: HashMap<String, bool> = HashMap::new();
            for t in &tracks {
                *seen.entry(identity_key(t)).or_default() |= has_evidence(t, &evidence);
            }
            TrackProgress {
                total: seen.len() as u32,
                listened: seen.values().filter(|h| **h).count() as u32,
            }
        });
        let (on_list, in_collection, listened) = match &raw.album_id {
            Some(a) => conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM listen_list_entry WHERE album_id = ?1),
                        EXISTS (SELECT 1 FROM collection_entry WHERE album_id = ?1),
                        EXISTS (SELECT 1 FROM listen_event WHERE album_id = ?1
                                AND deleted_at IS NULL AND (is_full = 1 OR kind = 'prior_history'))",
                params![a],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?,
            None => (false, false, false),
        };
        // Library albums outside the catalogue have no known type; they count by default.
        let in_scope = raw.source == EntrySource::Library
            || scope.counts(raw.primary_type.as_deref(), &raw.secondary_types);
        let score = match &raw.album_id {
            Some(a) => ratings::album_score(conn, a)?,
            None => None,
        };
        identities.push(if progress.is_some() { tracks } else { vec![] });
        entries.push(Entry {
            credits: credits_for(conn, &raw)?,
            score,
            rank: None,
            id: raw.id,
            source: raw.source,
            release_group_id: raw.release_group_id,
            album_id: raw.album_id,
            title: raw.title,
            disambiguation: raw.disambiguation,
            credit: raw.credit,
            collaboration: raw.credit_artists > 1,
            original_year: raw.original_year,
            type_label: if raw.source == EntrySource::Library {
                "In your library (type unknown)".into()
            } else {
                type_label(raw.primary_type.as_deref(), &raw.secondary_types)
            },
            primary_type: raw.primary_type,
            secondary_types: raw.secondary_types,
            excluded: raw.excluded,
            counted: in_scope && !raw.excluded,
            on_listen_list: on_list,
            in_collection,
            listened,
            reference,
            tracks: progress,
            fetched_at: raw.fetched_at,
        });
    }

    // Rank rated albums: highest score first, then oldest, then title.
    let mut rated: Vec<usize> = (0..entries.len())
        .filter(|&i| entries[i].score.is_some())
        .collect();
    rated.sort_by(|&a, &b| {
        let (x, y) = (&entries[a], &entries[b]);
        let (sx, sy) = (
            x.score.as_ref().map_or(0.0, |s| s.stars),
            y.score.as_ref().map_or(0.0, |s| s.stars),
        );
        sy.total_cmp(&sx)
            .then(
                (x.original_year.is_none(), x.original_year)
                    .cmp(&(y.original_year.is_none(), y.original_year)),
            )
            .then_with(|| x.title.to_lowercase().cmp(&y.title.to_lowercase()))
    });
    for (rank, &i) in rated.iter().enumerate() {
        entries[i].rank = Some(rank as u32 + 1);
    }
    let average = PersonalAverage {
        stars: (!rated.is_empty()).then(|| {
            rated
                .iter()
                .filter_map(|&i| entries[i].score.as_ref().map(|s| s.stars))
                .sum::<f64>()
                / rated.len() as f64
        }),
        rated_albums: rated.len() as u32,
    };

    let counted: Vec<&Entry> = entries.iter().filter(|e| e.counted).collect();
    let known = counted.iter().filter(|e| e.tracks.is_some()).count();
    let mut distinct: HashMap<String, bool> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        if e.counted && e.tracks.is_some() {
            for t in &identities[i] {
                *distinct.entry(identity_key(t)).or_default() |= has_evidence(t, &evidence);
            }
        }
    }
    let progress = Progress {
        albums_counted: counted.len() as u32,
        albums_listened: counted.iter().filter(|e| e.listened).count() as u32,
        tracks_total: distinct.len() as u32,
        tracks_listened: distinct.values().filter(|h| **h).count() as u32,
        albums_without_tracklist: (counted.len() - known) as u32,
    };
    let mut types: Vec<TypeCount> = Vec::new();
    for e in &entries {
        let counted = e.counted;
        match types.iter_mut().find(|t| t.label == e.type_label) {
            Some(t) => {
                t.count += 1;
                t.counted |= counted;
            }
            None => types.push(TypeCount {
                label: e.type_label.clone(),
                count: 1,
                counted,
            }),
        }
    }
    types.sort_by(|a, b| b.counted.cmp(&a.counted).then(a.label.cmp(&b.label)));

    let fetched_entries = entries
        .iter()
        .filter(|e| e.source == EntrySource::Musicbrainz)
        .count() as u32;
    let manual_entries = entries
        .iter()
        .filter(|e| e.source == EntrySource::Manual)
        .count() as u32;
    let coverage = match (&mbid, &state) {
        (None, _) => Coverage {
            kind: CatalogueKind::LocalOnly,
            status: CoverageStatus::LocalOnly,
            musicbrainz_artist_id: None,
            provider_total: None,
            fetched_entries,
            next_offset: 0,
            started_at: None,
            page_fetched_at: None,
            completed_at: None,
            last_error: None,
            manual_entries,
            tracklists_known: known as u32,
            tracklists_needed: counted.len() as u32,
            note: "This artist isn't linked to MusicBrainz, so only albums in your library and \
                   ones you add are known. This is not a complete catalogue."
                .into(),
        },
        (Some(id), s) => {
            let same = s.as_ref().and_then(|s| s.0.as_deref()) == Some(id.as_str());
            let s = s.as_ref().filter(|_| same);
            let completed_at = s.and_then(|s| s.5.clone());
            let next_offset = s.map_or(0, |s| s.2) as u32;
            let status = if completed_at.is_some() {
                CoverageStatus::Complete
            } else if next_offset > 0 || fetched_entries > 0 {
                CoverageStatus::Partial
            } else {
                CoverageStatus::NotFetched
            };
            let total = s.and_then(|s| s.1).map(|t| t as u32);
            let note = match status {
                CoverageStatus::Complete => {
                    "The official MusicBrainz catalogue was fetched in full.".to_owned()
                }
                CoverageStatus::Partial => format!(
                    "Only part of the MusicBrainz catalogue has been fetched ({next_offset} of {}). \
                     Progress covers what is known so far.",
                    total.map_or("an unknown number".into(), |t| t.to_string())
                ),
                _ => "The MusicBrainz catalogue hasn't been fetched yet; only albums in your \
                      library are known."
                    .to_owned(),
            };
            Coverage {
                kind: CatalogueKind::Online,
                status,
                musicbrainz_artist_id: Some(id.clone()),
                provider_total: total,
                fetched_entries,
                next_offset,
                started_at: s.and_then(|s| s.3.clone()),
                page_fetched_at: s.and_then(|s| s.4.clone()),
                completed_at,
                last_error: s.and_then(|s| s.6.clone()),
                manual_entries,
                tracklists_known: known as u32,
                tracklists_needed: counted.len() as u32,
                note,
            }
        }
    };

    Ok(ArtistCatalogue {
        artist_id,
        artist_name,
        sort_name,
        disambiguation,
        musicbrainz_id: mbid,
        average,
        coverage,
        denominator: scope.describe(),
        scope,
        representative_rule: REPRESENTATIVE_RULE,
        entries,
        types,
        progress,
    })
}
