//! Read models for the Listen List and Collection pages. Both show the same album-level
//! data (credits, genres, global tags), so a tag change appears everywhere at once.

use std::collections::HashMap;

use rusqlite::{Connection, ToSql, Transaction, params};
use serde::{Deserialize, Serialize};

use super::listening::{self, ListeningSummary};
use crate::artwork::{self, ArtworkRef};
use crate::domain::ids::parse_uuid;
use crate::domain::rating::{RatingSummary, summarize};
use crate::domain::text::norm;
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    ListenList,
    Collection,
}

impl Source {
    fn table(self) -> &'static str {
        match self {
            Self::ListenList => "listen_list_entry",
            Self::Collection => "collection_entry",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    #[default]
    AddedNewest,
    AddedOldest,
    Title,
    Artist,
    YearNewest,
    YearOldest,
    /// Latest known listening date first; albums listened only on unknown dates next;
    /// never-listened albums last.
    RecentlyListened,
    /// Effective rating (explicit or calculated); unrated albums last.
    RatingHighest,
    RatingLowest,
}

/// Search and filters. Within genres (or tags) any selected value matches; genres and
/// tags combine with AND.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct ListQuery {
    pub search: Option<String>,
    pub sort: SortOrder,
    pub genres: Vec<String>,
    pub tag_ids: Vec<String>,
    /// Drill-downs (e.g. from Stats), all on the canonical album's original year.
    pub year: Option<i64>,
    /// First year of a decade, e.g. 1990.
    pub decade: Option<i64>,
    /// Only albums whose original year is unknown.
    pub year_unknown: bool,
    /// Albums credited to this artist (collaborations included).
    pub artist_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagRef {
    pub id: String,
    pub name: String,
    pub color: String,
    pub builtin: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListItem {
    pub album_id: String,
    pub edition_id: String,
    pub title: String,
    pub artists: Vec<ArtistRef>,
    pub credit: String,
    pub original_year: Option<i64>,
    pub edition_name: String,
    pub edition_count: u32,
    pub genres: Vec<String>,
    pub tags: Vec<TagRef>,
    /// UTC RFC 3339 time it was added to this list.
    pub added_at: String,
    /// Cached artwork to show, if any (never triggers a download).
    pub artwork: Option<ArtworkRef>,
    /// Shared rating calculation for this edition.
    pub rating: RatingSummary,
    /// Album-level listening: last known date and listen counts.
    pub listening: ListeningSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Facet {
    pub name: String,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagFacet {
    #[serde(flatten)]
    pub tag: TagRef,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListResult {
    pub items: Vec<ListItem>,
    /// Albums in the list before search/filters (distinguishes "empty" from "no matches").
    pub total: u32,
    /// Counted over the whole list so filters stay available while narrowing.
    pub genres: Vec<Facet>,
    pub tags: Vec<TagFacet>,
}

fn load(conn: &Connection, source: Source) -> AppResult<Vec<ListItem>> {
    let table = source.table();
    load_rows(
        conn,
        &format!("SELECT album_id, edition_id, added_at FROM {table}"),
        &[],
    )
}

/// One edition shown as a card regardless of list membership (e.g. the Next Up pick).
/// `added_at` is when the edition was created, since it may be on no list.
pub fn item(conn: &Connection, edition_id: &str) -> AppResult<ListItem> {
    let edition_id = parse_uuid("edition", edition_id)?;
    load_rows(
        conn,
        "SELECT album_id, id AS edition_id, created_at AS added_at FROM edition WHERE id = ?1",
        &[&edition_id],
    )?
    .pop()
    .ok_or_else(|| AppError::not_found("edition", &edition_id))
}

/// `rows` yields (album_id, edition_id, added_at); `args` bind its placeholders.
fn load_rows(conn: &Connection, rows: &str, args: &[&dyn ToSql]) -> AppResult<Vec<ListItem>> {
    let scope = format!("SELECT album_id FROM ({rows})");

    let mut artists: HashMap<String, Vec<(ArtistRef, Option<String>, String)>> = HashMap::new();
    let mut stmt = conn.prepare(&format!(
        "SELECT c.album_id, ar.id, ar.name, c.credited_name, c.join_phrase FROM album_artist_credit c
         JOIN artist ar ON ar.id = c.artist_id WHERE c.album_id IN ({scope}) ORDER BY c.album_id, c.position"
    ))?;
    for row in stmt.query_map(args, |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
        ))
    })? {
        let (album, id, name, credited, join) = row?;
        artists
            .entry(album)
            .or_default()
            .push((ArtistRef { id, name }, credited, join));
    }

    let mut genres: HashMap<String, Vec<String>> = HashMap::new();
    let mut stmt = conn.prepare(&format!(
        "SELECT ag.album_id, g.name FROM album_genre ag JOIN genre g ON g.id = ag.genre_id
         WHERE ag.album_id IN ({scope}) ORDER BY ag.album_id, ag.position"
    ))?;
    for row in stmt.query_map(args, |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })? {
        let (album, name) = row?;
        genres.entry(album).or_default().push(name);
    }

    let mut tags: HashMap<String, Vec<TagRef>> = HashMap::new();
    let mut stmt = conn.prepare(&format!(
        "SELECT at.album_id, t.id, t.name, t.color, t.builtin_key IS NOT NULL FROM album_tag at
         JOIN tag t ON t.id = at.tag_id WHERE at.album_id IN ({scope})"
    ))?;
    for row in stmt.query_map(args, |r| {
        Ok((
            r.get::<_, String>(0)?,
            TagRef {
                id: r.get(1)?,
                name: r.get(2)?,
                color: r.get(3)?,
                builtin: r.get(4)?,
            },
        ))
    })? {
        let (album, tag) = row?;
        tags.entry(album).or_default().push(tag);
    }

    let mut stmt = conn.prepare(&format!(
        "SELECT l.album_id, l.edition_id, l.added_at, a.title, a.original_year, e.name,
                (SELECT COUNT(*) FROM edition e2 WHERE e2.album_id = a.id)
         FROM ({rows}) l JOIN album a ON a.id = l.album_id JOIN edition e ON e.id = l.edition_id"
    ))?;
    let rows = stmt.query_map(args, |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<i64>>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, u32>(6)?,
        ))
    })?;
    let mut items = Vec::new();
    for row in rows {
        let (album_id, edition_id, added_at, title, original_year, edition_name, edition_count) =
            row?;
        let credits = artists.remove(&album_id).unwrap_or_default();
        let credit = credits
            .iter()
            .map(|(a, credited, join)| format!("{}{join}", credited.as_deref().unwrap_or(&a.name)))
            .collect();
        let mut album_tags = tags.remove(&album_id).unwrap_or_default();
        album_tags.sort_by(|a, b| {
            b.builtin
                .cmp(&a.builtin)
                .then_with(|| norm(&a.name).cmp(&norm(&b.name)))
        });
        let artwork = artwork::lookup(conn, &edition_id)?.current;
        items.push(ListItem {
            artwork,
            rating: summarize(None, []),
            listening: listening::summary(conn, &album_id)?,
            genres: genres.remove(&album_id).unwrap_or_default(),
            tags: album_tags,
            artists: credits.into_iter().map(|(a, _, _)| a).collect(),
            credit,
            album_id,
            edition_id,
            title,
            original_year,
            edition_name,
            edition_count,
            added_at,
        });
    }
    let ids: Vec<String> = items.iter().map(|i| i.edition_id.clone()).collect();
    let mut ratings = super::ratings::summaries(conn, &ids)?;
    for item in &mut items {
        if let Some(r) = ratings.remove(&item.edition_id) {
            item.rating = r;
        }
    }
    Ok(items)
}

pub fn query(conn: &Connection, source: Source, q: &ListQuery) -> AppResult<ListResult> {
    let all = load(conn, source)?;
    let total = all.len() as u32;

    let mut genre_counts: HashMap<String, u32> = HashMap::new();
    let mut tag_counts: HashMap<String, (TagRef, u32)> = HashMap::new();
    for item in &all {
        for g in &item.genres {
            *genre_counts.entry(g.clone()).or_default() += 1;
        }
        for t in &item.tags {
            tag_counts
                .entry(t.id.clone())
                .or_insert_with(|| (t.clone(), 0))
                .1 += 1;
        }
    }
    let mut genres: Vec<Facet> = genre_counts
        .into_iter()
        .map(|(name, count)| Facet { name, count })
        .collect();
    genres.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    let mut tags: Vec<TagFacet> = tag_counts
        .into_values()
        .map(|(tag, count)| TagFacet { tag, count })
        .collect();
    tags.sort_by(|a, b| {
        b.tag
            .builtin
            .cmp(&a.tag.builtin)
            .then_with(|| norm(&a.tag.name).cmp(&norm(&b.tag.name)))
    });

    let search = q.search.as_deref().map(norm).filter(|s| !s.is_empty());
    if search.as_ref().is_some_and(|s| s.chars().count() > 200) {
        return Err(AppError::validation("search", "use at most 200 characters"));
    }
    let wanted_genres: Vec<String> = q.genres.iter().map(|g| norm(g)).collect();
    let wanted_tags: Vec<String> = q
        .tag_ids
        .iter()
        .map(|t| parse_uuid("tag", t))
        .collect::<AppResult<_>>()?;

    if let Some(d) = q.decade
        && d.rem_euclid(10) != 0
    {
        return Err(AppError::validation(
            "decade",
            "use the decade's first year, e.g. 1990",
        ));
    }
    let artist = q
        .artist_id
        .as_deref()
        .map(|a| parse_uuid("artist", a))
        .transpose()?;
    let mut items: Vec<ListItem> = all
        .into_iter()
        .filter(|i| q.year.is_none_or(|y| i.original_year == Some(y)))
        .filter(|i| {
            q.decade
                .is_none_or(|d| i.original_year.is_some_and(|y| (d..d + 10).contains(&y)))
        })
        .filter(|i| !q.year_unknown || i.original_year.is_none())
        .filter(|i| {
            artist
                .as_ref()
                .is_none_or(|a| i.artists.iter().any(|x| &x.id == a))
        })
        .filter(|i| {
            search.as_ref().is_none_or(|s| {
                norm(&i.title).contains(s.as_str())
                    || norm(&i.credit).contains(s.as_str())
                    || i.artists.iter().any(|a| norm(&a.name).contains(s.as_str()))
            })
        })
        .filter(|i| {
            wanted_genres.is_empty() || i.genres.iter().any(|g| wanted_genres.contains(&norm(g)))
        })
        .filter(|i| wanted_tags.is_empty() || i.tags.iter().any(|t| wanted_tags.contains(&t.id)))
        .collect();

    let title_then_added = |a: &ListItem, b: &ListItem| {
        norm(&a.title)
            .cmp(&norm(&b.title))
            .then_with(|| b.added_at.cmp(&a.added_at))
    };
    match q.sort {
        SortOrder::AddedNewest => items.sort_by(|a, b| {
            b.added_at
                .cmp(&a.added_at)
                .then_with(|| title_then_added(a, b))
        }),
        SortOrder::AddedOldest => items.sort_by(|a, b| {
            a.added_at
                .cmp(&b.added_at)
                .then_with(|| title_then_added(a, b))
        }),
        SortOrder::Title => items.sort_by(title_then_added),
        SortOrder::Artist => items.sort_by(|a, b| {
            norm(&a.credit)
                .cmp(&norm(&b.credit))
                .then_with(|| title_then_added(a, b))
        }),
        // Unknown years always sort last.
        SortOrder::YearNewest => items.sort_by(|a, b| {
            b.original_year
                .is_some()
                .cmp(&a.original_year.is_some())
                .then(b.original_year.cmp(&a.original_year))
                .then_with(|| title_then_added(a, b))
        }),
        SortOrder::RecentlyListened => items.sort_by(|a, b| {
            let rank = |i: &ListItem| match (
                &i.listening.last_listened,
                i.listening.listen_count > 0 || i.listening.earlier_undated,
            ) {
                (Some(_), _) => 0,
                (None, true) => 1,
                (None, false) => 2,
            };
            rank(a)
                .cmp(&rank(b))
                .then_with(|| b.listening.last_listened.cmp(&a.listening.last_listened))
                .then_with(|| title_then_added(a, b))
        }),
        SortOrder::RatingHighest => items.sort_by(|a, b| {
            let (x, y) = (a.rating.effective, b.rating.effective);
            y.is_some()
                .cmp(&x.is_some())
                .then(y.cmp(&x))
                .then_with(|| title_then_added(a, b))
        }),
        SortOrder::RatingLowest => items.sort_by(|a, b| {
            let (x, y) = (a.rating.effective, b.rating.effective);
            y.is_some()
                .cmp(&x.is_some())
                .then(x.cmp(&y))
                .then_with(|| title_then_added(a, b))
        }),
        SortOrder::YearOldest => items.sort_by(|a, b| {
            b.original_year
                .is_some()
                .cmp(&a.original_year.is_some())
                .then(a.original_year.cmp(&b.original_year))
                .then_with(|| title_then_added(a, b))
        }),
    }
    Ok(ListResult {
        items,
        total,
        genres,
        tags,
    })
}

/// Remove albums from the Listen List only. Ratings, reviews, listens, collection
/// membership, and tags are untouched. Returns how many entries were removed.
pub fn remove_from_listen_list(tx: &Transaction<'_>, album_ids: &[String]) -> AppResult<u32> {
    if album_ids.is_empty() {
        return Err(AppError::validation("albums", "select at least one album"));
    }
    let mut removed = 0;
    for id in album_ids {
        let id = parse_uuid("album", id)?;
        removed += tx.execute(
            "DELETE FROM listen_list_entry WHERE album_id = ?1",
            params![id],
        )? as u32;
    }
    Ok(removed)
}
