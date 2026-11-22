//! Lists: curated album lists (for example a magazine ranking) tracked against the library.
//!
//! A list entry is the list's own record: artist, title and year as the source wrote them.
//! Progress is derived live from the library — an entry is *listened* when a library album
//! with the same title and artist is in the Collection — so nothing here can drift out of
//! step with it. Albums only on the Listen List are reported but never counted as listened.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use super::{Table, optional_text, personal, require, required_text};
use crate::csv_import::mapping::{self, RowFields};
use crate::csv_import::parse::{ParsedFile, ParsedRecord};
use crate::csv_import::staging::{self, SessionSummary};
use crate::domain::ids::{new_id, parse_uuid};
use crate::error::{AppError, AppResult};

/// Most albums one list may hold, and most one call may add.
pub const MAX_ENTRIES: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewEntry {
    pub artist_name: String,
    pub title: String,
    pub year: Option<i64>,
    /// Rank in the source list (1 = first). Entries without one go to the end.
    pub position: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryStatus {
    /// In the Collection: counts toward progress.
    Listened,
    /// On the Listen List (and not in the Collection): shown, never counted.
    ListenList,
    /// In the library, but neither listened nor on the Listen List.
    InLibrary,
    Missing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub total: u32,
    pub listened: u32,
    pub on_listen_list: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSummary {
    pub id: String,
    pub name: String,
    pub source_url: Option<String>,
    pub progress: Progress,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryView {
    pub id: String,
    pub position: i64,
    pub artist_name: String,
    pub title: String,
    pub year: Option<i64>,
    pub status: EntryStatus,
    /// The library album this entry matches, when there is one.
    pub album_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDetail {
    pub id: String,
    pub name: String,
    pub source_url: Option<String>,
    pub progress: Progress,
    pub entries: Vec<EntryView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntriesAdded {
    pub added: u32,
    /// Entries skipped because the list already had that album.
    pub duplicates: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListenListAdded {
    /// Albums newly put on the Listen List.
    pub added: u32,
    /// Albums that aren't in the library at all: they need a reviewed MusicBrainz match
    /// before they can be added (see [`review_session`]).
    pub needs_match: u32,
}

// ------------------------------------------------------------------ matching

/// Comparison form for titles and artists: accents, case, punctuation, and spacing are
/// ignored ("Let’s Stay Friends" = "Lets stay friends", "Bjork" = "Björk").
fn key(s: &str) -> String {
    s.replace('&', " and ")
        .nfd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_alphanumeric())
        .collect()
}

fn artist_key(s: &str) -> String {
    let k = key(s);
    match k.strip_prefix("the") {
        // "The Replacements" = "Replacements", but not "Theory of a Deadman" → "ory…".
        Some(rest) if s.trim_start().to_lowercase().starts_with("the ") && !rest.is_empty() => {
            rest.to_owned()
        }
        _ => k,
    }
}

struct LibraryAlbum {
    id: String,
    collected: bool,
    on_listen_list: bool,
    artist_keys: HashSet<String>,
}

impl LibraryAlbum {
    fn status(&self) -> EntryStatus {
        if self.collected {
            EntryStatus::Listened
        } else if self.on_listen_list {
            EntryStatus::ListenList
        } else {
            EntryStatus::InLibrary
        }
    }

    fn rank(&self) -> u8 {
        match self.status() {
            EntryStatus::Listened => 3,
            EntryStatus::ListenList => 2,
            _ => 1,
        }
    }
}

/// Every library album, indexed for matching. Loaded once per operation.
struct Library {
    albums: HashMap<String, LibraryAlbum>,
    by_title: HashMap<String, Vec<String>>,
    /// Albums a reviewed import created or matched for a list's rows, by row fingerprint.
    imported: HashMap<String, String>,
}

impl Library {
    fn load(conn: &Connection) -> AppResult<Self> {
        let mut albums = HashMap::new();
        let mut by_title: HashMap<String, Vec<String>> = HashMap::new();
        let mut stmt = conn.prepare(
            "SELECT a.id, a.title,
                    EXISTS (SELECT 1 FROM collection_entry c WHERE c.album_id = a.id),
                    EXISTS (SELECT 1 FROM listen_list_entry l WHERE l.album_id = a.id)
             FROM album a",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, bool>(2)?,
                r.get::<_, bool>(3)?,
            ))
        })?;
        for row in rows {
            let (id, title, collected, on_listen_list) = row?;
            by_title.entry(key(&title)).or_default().push(id.clone());
            albums.insert(
                id.clone(),
                LibraryAlbum {
                    id,
                    collected,
                    on_listen_list,
                    artist_keys: HashSet::new(),
                },
            );
        }
        let mut stmt = conn.prepare(
            "SELECT c.album_id, ar.name, c.credited_name
             FROM album_artist_credit c JOIN artist ar ON ar.id = c.artist_id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?;
        for row in rows {
            let (album_id, name, credited) = row?;
            if let Some(album) = albums.get_mut(&album_id) {
                album.artist_keys.insert(artist_key(&name));
                if let Some(credited) = credited {
                    album.artist_keys.insert(artist_key(&credited));
                }
            }
        }
        let mut imported = HashMap::new();
        let mut stmt = conn.prepare("SELECT fingerprint, album_id FROM imported_row")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (fingerprint, album_id) = row?;
            if albums.contains_key(&album_id) {
                imported.insert(fingerprint, album_id);
            }
        }
        Ok(Self {
            albums,
            by_title,
            imported,
        })
    }

    /// The library album for an entry: the one it was linked to (by hand, or by a reviewed
    /// import of this row), or any album with the same title and an artist of the same
    /// name. With several, the most listened wins.
    fn resolve(&self, entry: &EntryRow) -> Option<&LibraryAlbum> {
        let linked = entry.album_id.as_ref().and_then(|id| self.albums.get(id));
        let imported = self
            .imported
            .get(&mapping::fingerprint(&row_fields(entry)))
            .and_then(|id| self.albums.get(id));
        let artist = artist_key(&entry.artist_name);
        let matched = self
            .by_title
            .get(&key(&entry.title))
            .into_iter()
            .flatten()
            .filter_map(|id| self.albums.get(id))
            .filter(|a| a.artist_keys.contains(&artist));
        linked
            .into_iter()
            .chain(imported)
            .chain(matched)
            .max_by_key(|a| (a.rank(), Reverse(a.id.as_str())))
    }
}

// ------------------------------------------------------------------- reading

struct EntryRow {
    id: String,
    list_id: String,
    position: i64,
    artist_name: String,
    title: String,
    year: Option<i64>,
    album_id: Option<String>,
}

/// The fields a CSV row for this entry carries, so a reviewed import's fingerprint of the
/// row can be found again.
fn row_fields(entry: &EntryRow) -> RowFields {
    RowFields {
        album: entry.title.clone(),
        artist: entry.artist_name.clone(),
        year: entry.year,
        edition: None,
        tags: Vec::new(),
        release_group_id: None,
        release_id: None,
    }
}

fn entry_rows(conn: &Connection, list_id: Option<&str>) -> AppResult<Vec<EntryRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, list_id, position, artist_name, title, year, album_id FROM list_entry
         WHERE ?1 IS NULL OR list_id = ?1 ORDER BY list_id, position, id",
    )?;
    let rows = stmt.query_map(params![list_id], |r| {
        Ok(EntryRow {
            id: r.get(0)?,
            list_id: r.get(1)?,
            position: r.get(2)?,
            artist_name: r.get(3)?,
            title: r.get(4)?,
            year: r.get(5)?,
            album_id: r.get(6)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn view(lib: &Library, row: EntryRow) -> EntryView {
    let album = lib.resolve(&row);
    EntryView {
        id: row.id,
        position: row.position,
        artist_name: row.artist_name,
        title: row.title,
        year: row.year,
        status: album.map_or(EntryStatus::Missing, LibraryAlbum::status),
        album_id: album.map(|a| a.id.clone()),
    }
}

fn progress(entries: &[EntryView]) -> Progress {
    let count = |s| entries.iter().filter(|e| e.status == s).count() as u32;
    Progress {
        total: entries.len() as u32,
        listened: count(EntryStatus::Listened),
        on_listen_list: count(EntryStatus::ListenList),
    }
}

type ListRow = (String, String, Option<String>);

fn list_row(conn: &Connection, list_id: &str) -> AppResult<ListRow> {
    conn.query_row(
        "SELECT id, name, source_url FROM list WHERE id = ?1",
        params![list_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .optional()?
    .ok_or_else(|| AppError::not_found("list", list_id))
}

/// Every list with its progress, oldest first.
pub fn overview(conn: &Connection) -> AppResult<Vec<ListSummary>> {
    let lib = Library::load(conn)?;
    let mut by_list: HashMap<String, Vec<EntryView>> = HashMap::new();
    for row in entry_rows(conn, None)? {
        by_list
            .entry(row.list_id.clone())
            .or_default()
            .push(view(&lib, row));
    }
    let mut stmt = conn.prepare("SELECT id, name, source_url FROM list ORDER BY created_at, id")?;
    let lists = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    lists
        .map(|row| {
            let (id, name, source_url): ListRow = row?;
            let entries = by_list.remove(&id).unwrap_or_default();
            Ok(ListSummary {
                progress: progress(&entries),
                id,
                name,
                source_url,
            })
        })
        .collect()
}

pub fn detail(conn: &Connection, list_id: &str) -> AppResult<ListDetail> {
    let list_id = parse_uuid("list", list_id)?;
    let (id, name, source_url) = list_row(conn, &list_id)?;
    let lib = Library::load(conn)?;
    let entries: Vec<EntryView> = entry_rows(conn, Some(&id))?
        .into_iter()
        .map(|row| view(&lib, row))
        .collect();
    Ok(ListDetail {
        progress: progress(&entries),
        id,
        name,
        source_url,
        entries,
    })
}

// ------------------------------------------------------------------- writing

fn list_name(raw: &str) -> AppResult<String> {
    required_text("list name", raw)
}

fn source_url(raw: Option<&str>) -> AppResult<Option<String>> {
    let url = optional_text("source address", raw)?;
    if url.as_deref().is_some_and(|u| !u.starts_with("https://")) {
        return Err(AppError::validation(
            "source address",
            "must start with https://",
        ));
    }
    Ok(url)
}

/// Create a list, optionally with its first entries, in the caller's transaction.
pub fn create(
    tx: &Transaction<'_>,
    name: &str,
    source: Option<&str>,
    entries: &[NewEntry],
) -> AppResult<String> {
    let name = list_name(name)?;
    let source = source_url(source)?;
    let id = new_id();
    tx.execute(
        "INSERT INTO list (id, name, source_url) VALUES (?1, ?2, ?3)",
        params![id, name, source],
    )?;
    add_entries(tx, &id, entries)?;
    Ok(id)
}

pub fn rename(tx: &Transaction<'_>, list_id: &str, name: &str) -> AppResult<()> {
    let list_id = parse_uuid("list", list_id)?;
    require(tx, Table::List, &list_id)?;
    tx.execute(
        "UPDATE list SET name = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
        params![list_id, list_name(name)?],
    )?;
    Ok(())
}

/// Delete a list and its entries. Library albums, the Collection and the Listen List are
/// untouched.
pub fn delete(tx: &Transaction<'_>, list_id: &str) -> AppResult<()> {
    let list_id = parse_uuid("list", list_id)?;
    require(tx, Table::List, &list_id)?;
    tx.execute("DELETE FROM list WHERE id = ?1", params![list_id])?;
    Ok(())
}

pub fn remove_entry(tx: &Transaction<'_>, entry_id: &str) -> AppResult<()> {
    let entry_id = parse_uuid("list entry", entry_id)?;
    require(tx, Table::ListEntry, &entry_id)?;
    tx.execute("DELETE FROM list_entry WHERE id = ?1", params![entry_id])?;
    Ok(())
}

/// Add albums to a list. An album the list already has (same artist and title) is skipped.
pub fn add_entries(
    tx: &Transaction<'_>,
    list_id: &str,
    entries: &[NewEntry],
) -> AppResult<EntriesAdded> {
    let list_id = parse_uuid("list", list_id)?;
    require(tx, Table::List, &list_id)?;
    if entries.len() > MAX_ENTRIES {
        return Err(AppError::validation(
            "albums",
            format!("add at most {MAX_ENTRIES} albums at a time"),
        ));
    }
    let mut seen: HashSet<(String, String)> = entry_rows(tx, Some(&list_id))?
        .iter()
        .map(|e| (artist_key(&e.artist_name), key(&e.title)))
        .collect();
    if seen.len() + entries.len() > MAX_ENTRIES {
        return Err(AppError::validation(
            "albums",
            format!("a list holds at most {MAX_ENTRIES} albums"),
        ));
    }
    let mut next: i64 = tx.query_row(
        "SELECT COALESCE(MAX(position), 0) + 1 FROM list_entry WHERE list_id = ?1",
        params![list_id],
        |r| r.get(0),
    )?;
    let (mut added, mut duplicates) = (0, 0);
    for entry in entries {
        let artist = required_text("artist", &entry.artist_name)?;
        let title = required_text("album title", &entry.title)?;
        if let Some(y) = entry.year
            && !(1000..=9999).contains(&y)
        {
            return Err(AppError::validation(
                "year",
                format!("{y} is not a four-digit year"),
            ));
        }
        if entry.position.is_some_and(|p| p < 1) {
            return Err(AppError::validation("position", "must be 1 or more"));
        }
        if !seen.insert((artist_key(&artist), key(&title))) {
            duplicates += 1;
            continue;
        }
        let position = entry.position.unwrap_or(next);
        next = next.max(position + 1);
        tx.execute(
            "INSERT INTO list_entry (id, list_id, position, artist_name, title, year)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![new_id(), list_id, position, artist, title, entry.year],
        )?;
        added += 1;
    }
    tx.execute(
        "UPDATE list SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
        params![list_id],
    )?;
    Ok(EntriesAdded { added, duplicates })
}

// ------------------------------------------------------------ Listen List

/// Put one entry's album on the Listen List. An album already listened to or already on
/// the list is left alone.
pub fn add_entry_to_listen_list(
    tx: &Transaction<'_>,
    entry_id: &str,
) -> AppResult<ListenListAdded> {
    let entry_id = parse_uuid("list entry", entry_id)?;
    let row = tx
        .query_row(
            "SELECT list_id FROM list_entry WHERE id = ?1",
            params![entry_id],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("list entry", &entry_id))?;
    let rows = entry_rows(tx, Some(&row))?
        .into_iter()
        .filter(|e| e.id == entry_id)
        .collect();
    add_to_listen_list(tx, rows)
}

/// Put every entry of a list that isn't listened to yet on the Listen List.
pub fn add_unlistened_to_listen_list(
    tx: &Transaction<'_>,
    list_id: &str,
) -> AppResult<ListenListAdded> {
    let list_id = parse_uuid("list", list_id)?;
    require(tx, Table::List, &list_id)?;
    add_to_listen_list(tx, entry_rows(tx, Some(&list_id))?)
}

fn add_to_listen_list(tx: &Transaction<'_>, rows: Vec<EntryRow>) -> AppResult<ListenListAdded> {
    let lib = Library::load(tx)?;
    let (mut added, mut needs_match) = (0, 0);
    for row in rows {
        let album = match lib.resolve(&row) {
            Some(a) if a.collected || a.on_listen_list => continue,
            Some(a) => a,
            None => {
                needs_match += 1;
                continue;
            }
        };
        let edition: String = tx.query_row(
            "SELECT id FROM edition WHERE album_id = ?1 ORDER BY created_at, id LIMIT 1",
            params![album.id],
            |r| r.get(0),
        )?;
        tx.execute(
            "UPDATE list_entry SET album_id = ?2 WHERE id = ?1",
            params![row.id, album.id],
        )?;
        personal::add_to_listen_list(tx, &edition)?;
        added += 1;
    }
    Ok(ListenListAdded { added, needs_match })
}

/// Link an entry to the library album the user chose for it (after a reviewed match), so it
/// is recognised even when the album's title differs from the list's wording.
pub fn link_entry(tx: &Transaction<'_>, entry_id: &str, album_id: &str) -> AppResult<()> {
    let entry_id = parse_uuid("list entry", entry_id)?;
    let album_id = parse_uuid("album", album_id)?;
    require(tx, Table::ListEntry, &entry_id)?;
    require(tx, Table::Album, &album_id)?;
    tx.execute(
        "UPDATE list_entry SET album_id = ?2 WHERE id = ?1",
        params![entry_id, album_id],
    )?;
    Ok(())
}

/// Stage the entries that aren't in the library as a CSV import, so each album is looked up
/// on MusicBrainz and matched only after the user reviews it, and nothing is duplicated.
/// With `entry_id`, only that entry. Committing the import links the albums back to the
/// entries (see [`Library::resolve`]).
pub fn review_session(
    tx: &Transaction<'_>,
    list_id: &str,
    entry_id: Option<&str>,
) -> AppResult<SessionSummary> {
    let list_id = parse_uuid("list", list_id)?;
    let (_, name, _) = list_row(tx, &list_id)?;
    let entry_id = entry_id.map(|e| parse_uuid("list entry", e)).transpose()?;
    let lib = Library::load(tx)?;
    let missing: Vec<EntryRow> = entry_rows(tx, Some(&list_id))?
        .into_iter()
        .filter(|e| entry_id.as_ref().is_none_or(|id| *id == e.id))
        .filter(|e| lib.resolve(e).is_none())
        .collect();
    if missing.is_empty() {
        return Err(AppError::validation(
            "albums",
            "every album here is already in your library",
        ));
    }
    let parsed = ParsedFile {
        headers: ["Album", "Artist", "Year"].map(str::to_owned).to_vec(),
        records: missing
            .iter()
            .enumerate()
            .map(|(i, e)| ParsedRecord {
                row_number: i as u32 + 2,
                cells: vec![
                    e.title.clone(),
                    e.artist_name.clone(),
                    e.year.map(|y| y.to_string()).unwrap_or_default(),
                ],
                problem: None,
            })
            .collect(),
    };
    let id = staging::create_session(tx, &format!("List: {name}"), &parsed)?;
    staging::apply_mapping(tx, &id, &mapping::suggest(&parsed.headers))?;
    staging::summary(tx, &id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_ignore_case_accents_punctuation_and_ampersands() {
        assert_eq!(key("Let’s Stay Friends"), key("lets stay friends"));
        assert_eq!(key("Björk"), key("Bjork"));
        assert_eq!(key("Songs: Ohia"), key("Songs Ohia"));
        assert_eq!(key("Mates & Dates"), key("Mates and Dates"));
        assert_ne!(key("Z"), key(""));
    }

    #[test]
    fn artist_keys_drop_only_a_leading_word_the() {
        assert_eq!(artist_key("The Replacements"), artist_key("Replacements"));
        assert_ne!(
            artist_key("Theory of a Deadman"),
            artist_key("ory of a Deadman")
        );
        assert_ne!(artist_key("The"), String::new());
    }
}
