//! Global tags. Tags belong to canonical albums (not to a list or an edition), so an
//! album's tags are the same on every page. Names are trimmed and unique ignoring case
//! and Unicode form. The built-in Listen ASAP tag has a stable ID; its colour and
//! assignments are editable, its name and existence are not.

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Serialize;

use super::{Table, require};
use crate::domain::ids::{new_id, parse_uuid};
use crate::domain::text::{norm, tidy};
use crate::error::{AppError, AppResult};

pub const MAX_NAME: usize = 60;
/// Tags in CSV files are separated by this, so a tag name can't contain it.
pub const RESERVED: char = ';';

/// Colours for new tags when none is given (cycled by tag count).
const PALETTE: &[&str] = &[
    "#60a5fa", "#34d399", "#f472b6", "#a78bfa", "#fb923c", "#22d3ee", "#facc15", "#f87171",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagInfo {
    pub id: String,
    pub name: String,
    pub color: String,
    pub builtin: bool,
    pub album_count: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkTagResult {
    pub added: u32,
    pub removed: u32,
}

pub fn validate_name(raw: &str) -> AppResult<String> {
    let name = tidy(raw);
    if name.is_empty() {
        return Err(AppError::validation("tag name", "enter a name"));
    }
    if name.chars().count() > MAX_NAME {
        return Err(AppError::validation(
            "tag name",
            format!("use at most {MAX_NAME} characters"),
        ));
    }
    if name.contains(RESERVED) {
        return Err(AppError::validation(
            "tag name",
            "can't contain “;” (it separates tags in CSV files)",
        ));
    }
    Ok(name)
}

/// `#RGB` or `#RRGGBB` (any case) → lowercase `#rrggbb`.
pub fn normalize_color(raw: &str) -> AppResult<String> {
    let hex = raw.trim().strip_prefix('#').unwrap_or("").to_lowercase();
    let full = match hex.len() {
        3 => hex.chars().flat_map(|c| [c, c]).collect(),
        6 => hex,
        _ => String::new(),
    };
    if full.len() == 6 && full.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(format!("#{full}"))
    } else {
        Err(AppError::validation(
            "tag colour",
            format!("“{raw}” isn’t a colour like #3b82f6"),
        ))
    }
}

/// Existing tag whose name equals `name` ignoring case, spacing, and Unicode form.
pub fn find_by_name(
    conn: &Connection,
    name: &str,
    except: Option<&str>,
) -> AppResult<Option<(String, String)>> {
    let wanted = norm(name);
    let mut stmt = conn.prepare("SELECT id, name FROM tag")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    for row in rows {
        let (id, existing) = row?;
        if Some(id.as_str()) != except && norm(&existing) == wanted {
            return Ok(Some((id, existing)));
        }
    }
    Ok(None)
}

pub fn list(conn: &Connection) -> AppResult<Vec<TagInfo>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.name, t.color, t.builtin_key IS NOT NULL,
                (SELECT COUNT(*) FROM album_tag at WHERE at.tag_id = t.id)
         FROM tag t",
    )?;
    let mut tags: Vec<TagInfo> = stmt
        .query_map([], |r| {
            Ok(TagInfo {
                id: r.get(0)?,
                name: r.get(1)?,
                color: r.get(2)?,
                builtin: r.get(3)?,
                album_count: r.get(4)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    tags.sort_by(|a, b| {
        b.builtin
            .cmp(&a.builtin)
            .then_with(|| norm(&a.name).cmp(&norm(&b.name)))
    });
    Ok(tags)
}

pub fn get(conn: &Connection, id: &str) -> AppResult<TagInfo> {
    list(conn)?
        .into_iter()
        .find(|t| t.id == id)
        .ok_or_else(|| AppError::not_found("tag", id))
}

pub fn create(tx: &Transaction<'_>, name: &str, color: Option<&str>) -> AppResult<TagInfo> {
    let name = validate_name(name)?;
    if let Some((_, existing)) = find_by_name(tx, &name, None)? {
        return Err(AppError::Conflict(format!(
            "a tag named “{existing}” already exists"
        )));
    }
    let color = match color {
        Some(c) => normalize_color(c)?,
        None => {
            let n: i64 = tx.query_row("SELECT COUNT(*) FROM tag", [], |r| r.get(0))?;
            PALETTE[n as usize % PALETTE.len()].to_owned()
        }
    };
    let id = new_id();
    tx.execute(
        "INSERT INTO tag (id, name, color) VALUES (?1, ?2, ?3)",
        params![id, name, color],
    )?;
    get(tx, &id)
}

/// Get or create by name (CSV import). Matching is case/Unicode-insensitive, so an
/// import can never duplicate an existing tag — including the built-in one.
pub fn ensure(tx: &Transaction<'_>, name: &str) -> AppResult<TagInfo> {
    let tidy_name = validate_name(name)?;
    match find_by_name(tx, &tidy_name, None)? {
        Some((id, _)) => get(tx, &id),
        None => create(tx, &tidy_name, None),
    }
}

pub fn update(
    tx: &Transaction<'_>,
    id: &str,
    name: Option<&str>,
    color: Option<&str>,
) -> AppResult<TagInfo> {
    let id = parse_uuid("tag", id)?;
    let current = get(tx, &id)?;
    if let Some(raw) = name {
        let name = validate_name(raw)?;
        if name != current.name {
            if current.builtin {
                return Err(AppError::validation(
                    "tag name",
                    "built-in tags can’t be renamed",
                ));
            }
            if let Some((_, existing)) = find_by_name(tx, &name, Some(&id))? {
                return Err(AppError::Conflict(format!(
                    "a tag named “{existing}” already exists"
                )));
            }
            tx.execute("UPDATE tag SET name = ?2 WHERE id = ?1", params![id, name])?;
        }
    }
    if let Some(raw) = color {
        tx.execute(
            "UPDATE tag SET color = ?2 WHERE id = ?1",
            params![id, normalize_color(raw)?],
        )?;
    }
    get(tx, &id)
}

/// Delete a tag and its assignments (albums are untouched). Returns how many albums
/// lost the tag.
pub fn delete(tx: &Transaction<'_>, id: &str) -> AppResult<u32> {
    let id = parse_uuid("tag", id)?;
    let tag = get(tx, &id)?;
    if tag.builtin {
        return Err(AppError::validation(
            "tag",
            "built-in tags can’t be deleted",
        ));
    }
    tx.execute("DELETE FROM tag WHERE id = ?1", params![id])?;
    Ok(tag.album_count)
}

/// Add and/or remove tags on many albums at once, atomically. Repeats are no-ops.
pub fn set_album_tags(
    tx: &Transaction<'_>,
    album_ids: &[String],
    add: &[String],
    remove: &[String],
) -> AppResult<BulkTagResult> {
    if album_ids.is_empty() {
        return Err(AppError::validation("albums", "select at least one album"));
    }
    let add: HashSet<String> = add
        .iter()
        .map(|t| parse_uuid("tag", t))
        .collect::<AppResult<_>>()?;
    let remove: HashSet<String> = remove
        .iter()
        .map(|t| parse_uuid("tag", t))
        .collect::<AppResult<_>>()?;
    if let Some(both) = add.intersection(&remove).next() {
        return Err(AppError::validation(
            "tags",
            format!("tag {both} is both added and removed"),
        ));
    }
    for t in add.iter().chain(&remove) {
        require(tx, Table::Tag, t)?;
    }
    let mut result = BulkTagResult::default();
    for album in album_ids {
        let album = parse_uuid("album", album)?;
        require(tx, Table::Album, &album)?;
        for t in &add {
            result.added += tx.execute(
                "INSERT INTO album_tag (album_id, tag_id) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
                params![album, t],
            )? as u32;
        }
        for t in &remove {
            result.removed += tx.execute(
                "DELETE FROM album_tag WHERE album_id = ?1 AND tag_id = ?2",
                params![album, t],
            )? as u32;
        }
    }
    Ok(result)
}

/// Tag IDs on an album (for tests and single-album views).
pub fn album_tag_ids(conn: &Connection, album_id: &str) -> AppResult<Vec<String>> {
    let mut stmt =
        conn.prepare("SELECT tag_id FROM album_tag WHERE album_id = ?1 ORDER BY tag_id")?;
    let ids = stmt
        .query_map(params![album_id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(ids)
}

pub fn builtin_count(conn: &Connection) -> AppResult<i64> {
    Ok(conn
        .query_row(
            "SELECT COUNT(*) FROM tag WHERE builtin_key IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_colours_are_validated() {
        assert_eq!(validate_name("  Road   trip ").unwrap(), "Road trip");
        assert!(validate_name("   ").is_err());
        assert!(validate_name("a;b").is_err());
        assert!(validate_name(&"x".repeat(61)).is_err());
        assert_eq!(normalize_color("#ABC").unwrap(), "#aabbcc");
        assert_eq!(normalize_color(" #3B82F6 ").unwrap(), "#3b82f6");
        for bad in ["3b82f6", "#12345", "#ggghhh", "red", ""] {
            assert!(normalize_color(bad).is_err(), "{bad}");
        }
    }
}
