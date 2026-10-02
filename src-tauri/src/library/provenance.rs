//! Where each catalogue field came from, and whether the user has overridden it.

use rusqlite::{Connection, Transaction, params};
use serde::{Deserialize, Serialize};

use super::{Table, optional_long_text, optional_text, require};
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntityKind {
    Artist,
    Album,
    Edition,
    Track,
}

impl EntityKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Artist => "artist",
            Self::Album => "album",
            Self::Edition => "edition",
            Self::Track => "track",
        }
    }

    fn table(self) -> Table {
        match self {
            Self::Artist => Table::Artist,
            Self::Album => Table::Album,
            Self::Edition => Table::Edition,
            Self::Track => Table::Track,
        }
    }

    /// Fields that carry provenance; anything else is rejected.
    fn fields(self) -> &'static [&'static str] {
        match self {
            Self::Artist => &["name", "sort_name"],
            Self::Album => &[
                "title",
                "original_date",
                "credits",
                "genres",
                "artwork",
                "description",
            ],
            Self::Edition => &["name", "release_date", "tracklist", "artwork"],
            Self::Track => &["title", "length_ms", "credits"],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataSource {
    Manual,
    Musicbrainz,
    Csv,
    CoverArtArchive,
}

impl MetadataSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Musicbrainz => "musicbrainz",
            Self::Csv => "csv",
            Self::CoverArtArchive => "cover_art_archive",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    pub entity: EntityKind,
    pub entity_id: String,
    pub field: String,
    pub source: MetadataSource,
    /// Provider-side reference, e.g. a MusicBrainz ID or CSV row.
    pub source_ref: Option<String>,
    /// The provider's value, kept so an override can be reverted.
    pub provider_value: Option<String>,
    pub is_override: bool,
}

/// Record (or replace) provenance for one field. Idempotent per (entity, id, field).
pub fn set(tx: &Transaction<'_>, p: &Provenance) -> AppResult<()> {
    require(tx, p.entity.table(), &p.entity_id)?;
    if !p.entity.fields().contains(&p.field.as_str()) {
        return Err(AppError::validation(
            "provenance field",
            format!("{} has no field '{}'", p.entity.as_str(), p.field),
        ));
    }
    tx.execute(
        "INSERT INTO metadata_provenance
             (entity_type, entity_id, field, source, source_ref, provider_value, is_override)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (entity_type, entity_id, field) DO UPDATE SET
             source = excluded.source, source_ref = excluded.source_ref,
             provider_value = excluded.provider_value, is_override = excluded.is_override,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        params![
            p.entity.as_str(),
            p.entity_id,
            p.field,
            p.source.as_str(),
            optional_text("source reference", p.source_ref.as_deref())?,
            optional_long_text("provider value", p.provider_value.as_deref())?,
            p.is_override,
        ],
    )?;
    Ok(())
}

pub fn for_entity(
    conn: &Connection,
    entity: EntityKind,
    entity_id: &str,
) -> AppResult<Vec<Provenance>> {
    let mut stmt = conn.prepare(
        "SELECT field, source, source_ref, provider_value, is_override FROM metadata_provenance
         WHERE entity_type = ?1 AND entity_id = ?2 ORDER BY field",
    )?;
    let rows = stmt.query_map(params![entity.as_str(), entity_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
        ))
    })?;
    rows.map(|row| {
        let (field, source, source_ref, provider_value, is_override) = row?;
        let source = serde_json::from_value(serde_json::Value::String(source.clone()))
            .map_err(|_| AppError::StorageCorrupt(format!("unknown metadata source '{source}'")))?;
        Ok(Provenance {
            entity,
            entity_id: entity_id.to_owned(),
            field,
            source,
            source_ref,
            provider_value,
            is_override,
        })
    })
    .collect()
}
