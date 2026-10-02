//! Album artwork: optional Cover Art Archive downloads, a local cache for offline display,
//! and user-supplied replacements.
//!
//! - Nothing is requested unless the user allowed artwork downloads (consent prompt).
//! - The selected edition's (release's) front image is preferred; the release group's is a
//!   labelled canonical fallback.
//! - Images are size-capped and checked by their magic bytes; only JPEG, PNG, GIF and WebP
//!   are kept. Files live in `<data dir>/artwork` and are served by the `artwork:` protocol.
//! - Local replacements and user removals are never overwritten by downloads. Clearing the
//!   cache removes downloaded files only.

pub mod http;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use self::http::{ImageError, ImageTransport, MAX_IMAGE_BYTES};
use crate::domain::ids::{new_id, parse_uuid};
use crate::error::{AppError, AppResult};
use crate::library::settings;
use crate::metadata::queue::RateLimiter;
use crate::state::AppState;

const BASE: &str = "https://coverartarchive.org";
/// Re-check "no artwork" answers after this long.
const RECHECK_DAYS: i64 = 30;
const SERVICE: &str = "Cover Art Archive";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtworkRef {
    pub owner_type: String,
    pub owner_id: String,
    /// "cover_art_archive" or "local".
    pub source: String,
    /// True when showing the album's (release group's) art rather than this edition's.
    pub canonical_fallback: bool,
    /// Changes whenever the image changes (cache-busting for the UI).
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtworkState {
    pub current: Option<ArtworkRef>,
    /// The user removed artwork for this edition; downloads won't bring it back.
    pub removed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheCleared {
    pub files: u32,
    pub bytes: u64,
}

/// Recognise an image by its first bytes. Anything else (HTML error pages, SVG) is rejected.
pub fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0xFF, 0xD8, 0xFF, ..] => Some("image/jpeg"),
        [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, ..] => Some("image/png"),
        [b'G', b'I', b'F', b'8', b'7' | b'9', b'a', ..] => Some("image/gif"),
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'E',
            b'B',
            b'P',
            ..,
        ] => Some("image/webp"),
        _ => None,
    }
}

fn extension(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "jpg",
    }
}

pub fn validate_image(bytes: &[u8]) -> AppResult<&'static str> {
    if bytes.is_empty() {
        return Err(AppError::validation("image", "the file is empty"));
    }
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(AppError::validation(
            "image",
            format!("larger than {} MB", MAX_IMAGE_BYTES / (1024 * 1024)),
        ));
    }
    sniff_image(bytes)
        .ok_or_else(|| AppError::validation("image", "not a JPEG, PNG, GIF or WebP image"))
}

struct Row {
    source: String,
    checked_at: String,
}

fn row(conn: &Connection, owner_type: &str, owner_id: &str) -> AppResult<Option<Row>> {
    Ok(conn
        .query_row(
            "SELECT source, checked_at FROM artwork WHERE owner_type = ?1 AND owner_id = ?2",
            params![owner_type, owner_id],
            |r| {
                Ok(Row {
                    source: r.get(0)?,
                    checked_at: r.get(1)?,
                })
            },
        )
        .optional()?)
}

fn stale(conn: &Connection, checked_at: &str) -> AppResult<bool> {
    Ok(conn.query_row(
        "SELECT ?1 < strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?2)",
        params![checked_at, format!("-{RECHECK_DAYS} days")],
        |r| r.get(0),
    )?)
}

/// What to show for an edition: its own art, else its album's canonical art.
pub fn lookup(conn: &Connection, edition_id: &str) -> AppResult<ArtworkState> {
    let album_id: String = conn
        .query_row(
            "SELECT album_id FROM edition WHERE id = ?1",
            params![edition_id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("edition", edition_id))?;
    let make = |owner_type: &str, owner_id: &str, r: Row, fallback: bool| ArtworkRef {
        owner_type: owner_type.into(),
        owner_id: owner_id.into(),
        source: r.source,
        canonical_fallback: fallback,
        version: r.checked_at,
    };
    match row(conn, "edition", edition_id)? {
        Some(r) if r.source == "removed" => {
            return Ok(ArtworkState {
                current: None,
                removed: true,
            });
        }
        Some(r) if r.source != "none" => {
            return Ok(ArtworkState {
                current: Some(make("edition", edition_id, r, false)),
                removed: false,
            });
        }
        _ => {}
    }
    let current = match row(conn, "album", &album_id)? {
        Some(r) if r.source == "cover_art_archive" || r.source == "local" => {
            Some(make("album", &album_id, r, true))
        }
        _ => None,
    };
    Ok(ArtworkState {
        current,
        removed: false,
    })
}

pub struct ArtworkService {
    transport: Arc<dyn ImageTransport>,
    queue: RateLimiter,
    base: String,
}

impl ArtworkService {
    pub fn new(transport: Arc<dyn ImageTransport>) -> Self {
        // No published limit, but stay polite: one image request per second.
        Self {
            transport,
            queue: RateLimiter::new(Duration::from_secs(1)),
            base: BASE.into(),
        }
    }

    async fn download(
        &self,
        url: &str,
        cancel: &CancellationToken,
    ) -> AppResult<Option<(Vec<u8>, &'static str)>> {
        self.queue.acquire(cancel).await?;
        let response = tokio::select! {
            r = self.transport.get(url) => r,
            () = cancel.cancelled() => return Err(AppError::Cancelled),
        };
        match response {
            Ok(r) if r.status == 200 => {
                // Bad bytes from the server mean "no usable artwork", not a hard failure.
                Ok(validate_image(&r.bytes).ok().map(|mime| (r.bytes, mime)))
            }
            Ok(r) if r.status == 404 => Ok(None),
            Ok(r) if r.status == 503 || r.status == 429 => {
                Err(AppError::RateLimited(SERVICE.into()))
            }
            Ok(r) => Err(AppError::Provider(format!(
                "{SERVICE} returned {}",
                r.status
            ))),
            Err(ImageError::Timeout) => Err(AppError::Timeout(SERVICE.into())),
            Err(ImageError::Unreachable(_)) => Err(AppError::Offline(SERVICE.into())),
            Err(ImageError::TooLarge) => Ok(None),
            Err(ImageError::UnsafeRedirect(e)) => {
                Err(AppError::Provider(format!("{SERVICE}: {e}")))
            }
        }
    }

    /// Ensure artwork for an edition is cached, downloading only with consent and only
    /// what isn't already known. Returns what to display.
    pub async fn fetch(
        &self,
        state: &AppState,
        edition_id: &str,
        cancel: &CancellationToken,
    ) -> AppResult<ArtworkState> {
        let edition_id = parse_uuid("edition", edition_id)?;
        let plan = state.with_db(|db| {
            db.read(|c| {
                if settings::artwork_download(c)? != Some(true) {
                    return Err(AppError::validation(
                        "artwork",
                        "artwork downloads are turned off",
                    ));
                }
                let (album_id, release, group): (String, Option<String>, Option<String>) = c
                    .query_row(
                        "SELECT e.album_id, e.musicbrainz_release_id, a.musicbrainz_release_group_id
                         FROM edition e JOIN album a ON a.id = e.album_id WHERE e.id = ?1",
                        params![edition_id],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()?
                    .ok_or_else(|| AppError::not_found("edition", &edition_id))?;
                let edition_known = match row(c, "edition", &edition_id)? {
                    None => false,
                    Some(r) if r.source == "none" => !stale(c, &r.checked_at)?,
                    Some(_) => true, // cached, local, or removed: never re-download
                };
                let album_known = match row(c, "album", &album_id)? {
                    None => false,
                    Some(r) if r.source == "none" => !stale(c, &r.checked_at)?,
                    Some(_) => true,
                };
                Ok((album_id, release, group, edition_known, album_known))
            })
        })?;
        let (album_id, release, group, edition_known, album_known) = plan;
        // The user removed artwork for this edition: no requests at all, not even the fallback.
        let current = state.with_db(|db| db.read(|c| lookup(c, &edition_id)))?;
        if current.removed {
            return Ok(current);
        }

        if !edition_known && let Some(mbid) = &release {
            let url = format!("{}/release/{mbid}/front-500", self.base);
            let found = self.download(&url, cancel).await?;
            self.store(state, "edition", &edition_id, found, &url)?;
        }
        let has_edition_art = state.with_db(|db| {
            db.read(|c| {
                Ok(lookup(c, &edition_id)?
                    .current
                    .is_some_and(|a| !a.canonical_fallback))
            })
        })?;
        if !has_edition_art
            && !album_known
            && let Some(mbid) = &group
        {
            let url = format!("{}/release-group/{mbid}/front-500", self.base);
            let found = self.download(&url, cancel).await?;
            self.store(state, "album", &album_id, found, &url)?;
        }
        state.with_db(|db| db.read(|c| lookup(c, &edition_id)))
    }

    fn store(
        &self,
        state: &AppState,
        owner_type: &str,
        owner_id: &str,
        found: Option<(Vec<u8>, &'static str)>,
        url: &str,
    ) -> AppResult<()> {
        match found {
            Some((bytes, mime)) => write(state, owner_type, owner_id, "cover_art_archive", &bytes, mime, Some(url)),
            None => state.with_db(|db| {
                db.write(|tx| {
                    // Never replace the user's own choice with a "nothing found" marker.
                    tx.execute(
                        "INSERT INTO artwork (owner_type, owner_id, source, source_url) VALUES (?1, ?2, 'none', ?3)
                         ON CONFLICT (owner_type, owner_id) DO UPDATE SET source = 'none', source_url = excluded.source_url,
                             checked_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), file_name = NULL, mime = NULL, bytes = NULL
                         WHERE artwork.source IN ('none', 'cover_art_archive')",
                        params![owner_type, owner_id, url],
                    )?;
                    Ok(())
                })
            }),
        }
    }
}

pub fn artwork_dir(state: &AppState) -> PathBuf {
    state.data_dir.join("artwork")
}

fn file_of(conn: &Connection, owner_type: &str, owner_id: &str) -> AppResult<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT file_name FROM artwork WHERE owner_type = ?1 AND owner_id = ?2",
            params![owner_type, owner_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten())
}

/// Write an image file and its record; the previous file (if any) is deleted afterwards.
fn write(
    state: &AppState,
    owner_type: &str,
    owner_id: &str,
    source: &str,
    bytes: &[u8],
    mime: &str,
    url: Option<&str>,
) -> AppResult<()> {
    let dir = artwork_dir(state);
    std::fs::create_dir_all(&dir)
        .map_err(|e| AppError::StorageUnavailable(format!("artwork folder: {e}")))?;
    let name = format!("{owner_type}-{owner_id}-{}.{}", new_id(), extension(mime));
    let tmp = dir.join(format!("{name}.partial"));
    std::fs::write(&tmp, bytes)
        .map_err(|e| AppError::StorageUnavailable(format!("artwork file: {e}")))?;
    std::fs::rename(&tmp, dir.join(&name))
        .map_err(|e| AppError::StorageUnavailable(format!("artwork file: {e}")))?;
    let previous = state.with_db(|db| {
        db.write(|tx| {
            let previous = file_of(tx, owner_type, owner_id)?;
            // Downloads never replace a local image or a removal.
            let guard = if source == "cover_art_archive" { "WHERE artwork.source IN ('none', 'cover_art_archive')" } else { "" };
            let changed = tx.execute(
                &format!(
                    "INSERT INTO artwork (owner_type, owner_id, source, file_name, mime, bytes, source_url)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT (owner_type, owner_id) DO UPDATE SET source = excluded.source,
                         file_name = excluded.file_name, mime = excluded.mime, bytes = excluded.bytes,
                         source_url = excluded.source_url, checked_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                     {guard}"
                ),
                params![owner_type, owner_id, source, name, mime, bytes.len() as i64, url],
            )?;
            Ok(if changed > 0 { previous } else { Some(name.clone()) })
        })
    });
    match previous {
        Ok(Some(old)) => remove_file(&dir, &old),
        Ok(None) => {}
        Err(e) => {
            remove_file(&dir, &name);
            return Err(e);
        }
    }
    Ok(())
}

fn remove_file(dir: &Path, name: &str) {
    if let Err(e) = std::fs::remove_file(dir.join(name))
        && e.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!("MuDraft: couldn't delete artwork file {name}: {e}");
    }
}

fn edition_exists(state: &AppState, edition_id: &str) -> AppResult<()> {
    state.with_db(|db| {
        db.read(|c| {
            crate::library::require(c, crate::library::Table::Edition, edition_id)?;
            Ok(())
        })
    })
}

/// Use the user's own image for an edition (kept through refreshes and cache clears).
pub fn replace_local(state: &AppState, edition_id: &str, bytes: &[u8]) -> AppResult<ArtworkState> {
    let edition_id = parse_uuid("edition", edition_id)?;
    edition_exists(state, &edition_id)?;
    let mime = validate_image(bytes)?;
    write(state, "edition", &edition_id, "local", bytes, mime, None)?;
    state.with_db(|db| db.read(|c| lookup(c, &edition_id)))
}

/// Show the placeholder for this edition and stop downloading artwork for it.
pub fn remove(state: &AppState, edition_id: &str) -> AppResult<ArtworkState> {
    let edition_id = parse_uuid("edition", edition_id)?;
    edition_exists(state, &edition_id)?;
    let old = state.with_db(|db| {
        db.write(|tx| {
            let old = file_of(tx, "edition", &edition_id)?;
            tx.execute(
                "INSERT INTO artwork (owner_type, owner_id, source) VALUES ('edition', ?1, 'removed')
                 ON CONFLICT (owner_type, owner_id) DO UPDATE SET source = 'removed', file_name = NULL, mime = NULL,
                     bytes = NULL, source_url = NULL, checked_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                params![edition_id],
            )?;
            Ok(old)
        })
    })?;
    if let Some(name) = old {
        remove_file(&artwork_dir(state), &name);
    }
    state.with_db(|db| db.read(|c| lookup(c, &edition_id)))
}

/// Undo a removal or local replacement: artwork may be downloaded again.
pub fn restore(state: &AppState, edition_id: &str) -> AppResult<ArtworkState> {
    let edition_id = parse_uuid("edition", edition_id)?;
    let old = state.with_db(|db| {
        db.write(|tx| {
            let old = file_of(tx, "edition", &edition_id)?;
            tx.execute(
                "DELETE FROM artwork WHERE owner_type = 'edition' AND owner_id = ?1",
                params![edition_id],
            )?;
            Ok(old)
        })
    })?;
    if let Some(name) = old {
        remove_file(&artwork_dir(state), &name);
    }
    state.with_db(|db| db.read(|c| lookup(c, &edition_id)))
}

/// Delete downloaded artwork and "not found" markers. Local images and removals stay.
pub fn clear_cache(state: &AppState) -> AppResult<CacheCleared> {
    let dir = artwork_dir(state);
    let (files, keep): (Vec<(String, i64)>, Vec<String>) = state.with_db(|db| {
        db.write(|tx| {
            let mut stmt = tx.prepare(
                "SELECT file_name, bytes FROM artwork WHERE source = 'cover_art_archive'",
            )?;
            let files = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            tx.execute(
                "DELETE FROM artwork WHERE source IN ('cover_art_archive', 'none')",
                [],
            )?;
            let mut stmt =
                tx.prepare("SELECT file_name FROM artwork WHERE file_name IS NOT NULL")?;
            let keep = stmt
                .query_map([], |r| r.get(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok((files, keep))
        })
    })?;
    let mut cleared = CacheCleared { files: 0, bytes: 0 };
    for (name, bytes) in files {
        remove_file(&dir, &name);
        cleared.files += 1;
        cleared.bytes += bytes as u64;
    }
    // Leftovers from interrupted writes.
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !keep.contains(&name) {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                remove_file(&dir, &name);
                cleared.files += 1;
                cleared.bytes += size;
            }
        }
    }
    Ok(cleared)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheSize {
    pub files: u32,
    pub bytes: u64,
}

pub fn cache_size(conn: &Connection) -> AppResult<CacheSize> {
    Ok(conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(bytes), 0) FROM artwork WHERE source = 'cover_art_archive'",
        [],
        |r| {
            Ok(CacheSize {
                files: r.get(0)?,
                bytes: r.get::<_, i64>(1)? as u64,
            })
        },
    )?)
}

/// Bytes for the `artwork:` protocol. Only files recorded in the database are served.
pub fn serve(state: &AppState, owner_type: &str, owner_id: &str) -> AppResult<(Vec<u8>, String)> {
    if owner_type != "edition" && owner_type != "album" {
        return Err(AppError::not_found("artwork", owner_type));
    }
    let owner_id = parse_uuid("artwork owner", owner_id)?;
    let (name, mime): (String, String) = state
        .with_db(|db| {
            db.read(|c| {
                Ok(c.query_row(
                    "SELECT file_name, mime FROM artwork WHERE owner_type = ?1 AND owner_id = ?2 AND file_name IS NOT NULL",
                    params![owner_type, owner_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?)
            })
        })?
        .ok_or_else(|| AppError::not_found("artwork", &owner_id))?;
    if !crate::profile::safe_file_name(&name) {
        return Err(AppError::not_found("artwork", &owner_id));
    }
    let bytes = std::fs::read(artwork_dir(state).join(&name))
        .map_err(|e| AppError::StorageUnavailable(format!("artwork file: {e}")))?;
    // Only ever serve bytes that really are the recorded image type.
    if sniff_image(&bytes) != Some(mime.as_str()) {
        return Err(AppError::not_found("artwork", &owner_id));
    }
    Ok((bytes, mime))
}

#[cfg(test)]
mod tests;
