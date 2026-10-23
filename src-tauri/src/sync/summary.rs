//! Human-facing descriptions of sync data: what changed, by kind, and which record a
//! conflict is about. Pure functions over states, so the numbers match what is applied.

use std::collections::BTreeSet;

use serde::Serialize;
use serde_json::Value;

use super::merge::{Conflict, ConflictKind};
use super::schema::{Schema, TableSchema};
use super::state::{Row, State};

/// Records that differ between two states, grouped the way people think about them.
/// Albums and listens are counted once each however many of their rows changed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeSummary {
    pub albums: u64,
    pub listens: u64,
    /// Track and album ratings, and liked tracks.
    pub ratings: u64,
    pub reviews: u64,
    /// Tags and tag assignments.
    pub tags: u64,
    pub artwork: u64,
    /// Preferences, Next Up history, artists, catalogue entries.
    pub other: u64,
}

impl ChangeSummary {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

fn key(id: &Value) -> String {
    serde_json::to_string(&[id]).unwrap_or_default()
}

fn content_eq(ts: &TableSchema, a: &Row, b: &Row) -> bool {
    (0..ts.columns.len()).all(|i| ts.meta[i] || a[i] == b[i])
}

struct Lookup<'a> {
    schema: &'a Schema,
    states: [&'a State; 2],
}

impl Lookup<'_> {
    fn row(&self, table: &str, id: &Value) -> Option<(&TableSchema, &Row)> {
        let ts = self.schema.table(table)?;
        let k = key(id);
        self.states
            .iter()
            .find_map(|s| s.row(table, &k))
            .map(|r| (ts, r))
    }

    fn field(&self, table: &str, id: &Value, col: &str) -> Option<Value> {
        let (ts, row) = self.row(table, id)?;
        Some(row[ts.col(col)?].clone())
    }

    fn album_of_edition(&self, edition: &Value) -> Option<String> {
        self.field("edition", edition, "album_id")?
            .as_str()
            .map(str::to_owned)
    }

    fn album_of_track(&self, track: &Value) -> Option<String> {
        self.album_of_edition(&self.field("track", track, "edition_id")?)
    }

    fn album_of(&self, entity_type: &str, id: &Value) -> Option<String> {
        match entity_type {
            "album" => id.as_str().map(str::to_owned),
            "edition" => self.album_of_edition(id),
            "track" => self.album_of_track(id),
            _ => None,
        }
    }
}

pub fn summarize(schema: &Schema, from: &State, to: &State) -> ChangeSummary {
    let look = Lookup {
        schema,
        states: [to, from],
    };
    let mut out = ChangeSummary::default();
    let mut albums: BTreeSet<String> = BTreeSet::new();
    let mut listens: BTreeSet<String> = BTreeSet::new();
    for ts in &schema.tables {
        let (a, b) = (from.rows(ts.name), to.rows(ts.name));
        let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
        for k in keys {
            let (old, new) = (a.get(k), b.get(k));
            let row = match (old, new) {
                (Some(x), Some(y)) if content_eq(ts, x, y) => continue,
                (_, Some(r)) | (Some(r), None) => r,
                (None, None) => continue,
            };
            let col = |name: &str| ts.col(name).map(|i| &row[i]);
            // A missing row reads as empty, so a new review with no rating isn't a rating.
            let differs = |name: &str| {
                ts.col(name).is_some_and(|i| {
                    old.map_or(&Value::Null, |r| &r[i]) != new.map_or(&Value::Null, |r| &r[i])
                })
            };
            let album = match ts.name {
                "album" => col("id").and_then(Value::as_str).map(str::to_owned),
                "edition"
                | "album_artist_credit"
                | "album_genre"
                | "album_source_tag"
                | "listen_list_entry"
                | "collection_entry" => col("album_id").and_then(Value::as_str).map(str::to_owned),
                "track" => col("edition_id").and_then(|e| look.album_of_edition(e)),
                "track_artist_credit" => col("track_id").and_then(|t| look.album_of_track(t)),
                "metadata_provenance" => {
                    match (col("entity_type").and_then(Value::as_str), col("entity_id")) {
                        (Some(t), Some(id)) => look.album_of(t, id),
                        _ => None,
                    }
                }
                _ => None,
            };
            if let Some(album) = album {
                albums.insert(album);
                continue;
            }
            match ts.name {
                "listen_event" => {
                    listens.insert(k.clone());
                }
                "listen_event_track" => {
                    if let Some(id) = col("listen_event_id") {
                        listens.insert(key(id));
                    }
                }
                "track_rating" => out.ratings += 1,
                "album_review" => {
                    // A new or removed review row counts for whatever it holds.
                    let rating = differs("rating");
                    let review = differs("review");
                    out.ratings += u64::from(rating);
                    out.reviews += u64::from(review || !rating);
                }
                "tag" | "album_tag" => out.tags += 1,
                "artwork" => out.artwork += 1,
                _ => out.other += 1,
            }
        }
    }
    out.albums = albums.len() as u64;
    out.listens = listens.len() as u64;
    out
}

fn text(v: Option<Value>) -> Option<String> {
    v.and_then(|v| v.as_str().map(str::to_owned))
}

/// A short name for the record a conflict is about, e.g. "Review of OK Computer
/// (Standard)". Looks names up in this computer's data first, then GitHub's.
pub fn describe(schema: &Schema, conflict: &Conflict, states: [&State; 2]) -> String {
    let look = Lookup { schema, states };
    let first = conflict
        .key
        .as_array()
        .and_then(|k| k.first())
        .cloned()
        .unwrap_or(Value::Null);
    let album =
        |id: &Value| text(look.field("album", id, "title")).unwrap_or_else(|| "an album".into());
    let edition = |id: &Value| {
        let name = text(look.field("edition", id, "name")).unwrap_or_default();
        let title = look
            .field("edition", id, "album_id")
            .map(|a| album(&a))
            .unwrap_or_else(|| "an album".into());
        if name.is_empty() {
            title
        } else {
            format!("{title} ({name})")
        }
    };
    let track = |id: &Value| {
        let title = text(look.field("track", id, "title")).unwrap_or_else(|| "a track".into());
        match look.field("track", id, "edition_id") {
            Some(e) => format!("“{title}” on {}", edition(&e)),
            None => format!("“{title}”"),
        }
    };
    let tag = |id: &Value| text(look.field("tag", id, "name")).unwrap_or_else(|| "a tag".into());
    let field_of = |name: &str| {
        let ts = schema.table(&conflict.table)?;
        let pos = ts.pk.iter().position(|&i| ts.columns[i] == name)?;
        conflict.key.as_array()?.get(pos).cloned()
    };
    if conflict.kind == ConflictKind::Duplicate {
        return match conflict.table.as_str() {
            "tag" => "Two tags with the same name".into(),
            "edition" => "Two editions with the same name".into(),
            "track" => "Two tracks in the same position".into(),
            _ => format!(
                "Two {} records that must be unique",
                conflict.table.replace('_', " ")
            ),
        };
    }
    match conflict.table.as_str() {
        "album" => album(&first),
        "edition" => edition(&first),
        "track" => track(&first),
        "artist" => {
            text(look.field("artist", &first, "name")).unwrap_or_else(|| "An artist".into())
        }
        "tag" => format!("Tag “{}”", tag(&first)),
        "album_tag" => match (field_of("album_id"), field_of("tag_id")) {
            (Some(a), Some(t)) => format!("Tag “{}” on {}", tag(&t), album(&a)),
            _ => "A tag assignment".into(),
        },
        "album_review" => format!("Review of {}", edition(&first)),
        "track_rating" => format!("Rating of {}", track(&first)),
        "listen_event" => {
            let on = look
                .field("listen_event", &first, "listened_on")
                .and_then(|d| d.as_str().map(str::to_owned));
            let ed = look
                .field("listen_event", &first, "edition_id")
                .map(|e| edition(&e));
            format!(
                "Listen to {} ({})",
                ed.unwrap_or_else(|| "an album".into()),
                on.unwrap_or_else(|| "date unknown".into())
            )
        }
        "listen_event_track" => "Tracks heard in a listen".into(),
        "listen_list_entry" => format!("{} on the Listen List", album(&first)),
        "collection_entry" => format!("{} in the Collection", edition(&first)),
        "album_artist_credit" => format!("Artists of {}", album(&first)),
        "album_genre" => format!("Genres of {}", album(&first)),
        "album_source_tag" => format!("MusicBrainz genres of {}", album(&first)),
        "track_artist_credit" => format!("Artists of {}", track(&first)),
        "metadata_provenance" => {
            let entity = field_of("entity_type").and_then(|v| v.as_str().map(str::to_owned));
            let id = field_of("entity_id").unwrap_or(Value::Null);
            let what = match entity.as_deref() {
                Some("album") => album(&id),
                Some("edition") => edition(&id),
                Some("track") => track(&id),
                _ => "a record".into(),
            };
            format!("Metadata source for {what}")
        }
        "artwork" => match field_of("owner_type")
            .and_then(|v| v.as_str().map(str::to_owned))
            .as_deref()
        {
            Some("edition") => format!(
                "Artwork for {}",
                edition(&field_of("owner_id").unwrap_or(Value::Null))
            ),
            _ => format!(
                "Artwork for {}",
                album(&field_of("owner_id").unwrap_or(Value::Null))
            ),
        },
        "setting" => format!("Preference “{}”", first.as_str().unwrap_or_default()),
        "current_selection" => "Next Up’s current album".into(),
        "selection_session" | "selection_attempt" => "Next Up history".into(),
        "catalogue_entry" => format!(
            "Catalogue entry “{}”",
            text(look.field("catalogue_entry", &first, "title")).unwrap_or_default()
        ),
        other => other.replace('_', " "),
    }
}
