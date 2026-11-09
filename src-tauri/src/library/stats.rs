//! Stats selectors. Read-only, computed on demand from stored evidence:
//! - Listen List counts are per canonical album (one Listen List entry per album), so
//!   extra editions never inflate them. Unknown years and runtimes are reported apart.
//! - Collection quality uses rated Collection albums only, scored by the canonical rule
//!   shared with Artist pages (`ratings::album_score`): one contribution per album however
//!   many editions or re-listens. Zero is a rating; unrated albums are left out. No
//!   smoothing or penalties; the "Top rated" rankings consider only groups with at least
//!   [`TOP_MIN_ALBUMS`] rated albums and list the best [`TOP_LIMIT`].
//! - "Top rated" by tracks ranks the same groups by the mean of individual track ratings
//!   (each rated recording once per album, its editions' ratings averaged), placed by its
//!   album's year and genres and by its credited artists (the album's when it has none).
//!   The same minimum applies: tracks from at least [`TOP_MIN_ALBUMS`] different albums.
//! - "Various Artists" (MusicBrainz's placeholder for compilations) is not an artist and is
//!   left out of artist counts and rankings.
//! - Selection completion comes from stored attempt provenance, never from current tags.

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::{Connection, params};
use serde::Serialize;

use super::ratings;
use crate::error::AppResult;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Count {
    pub key: String,
    pub label: String,
    pub count: u32,
}

/// Highest count first, then label, so ties are listed side by side in a stable order.
fn counts(map: HashMap<String, (String, u32)>) -> Vec<Count> {
    let mut out: Vec<Count> = map
        .into_iter()
        .map(|(key, (label, count))| Count { key, label, count })
        .collect();
    out.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
            .then_with(|| a.key.cmp(&b.key))
    });
    out
}

fn chronological(map: BTreeMap<String, u32>) -> Vec<Count> {
    map.into_iter()
        .map(|(key, count)| Count {
            label: key.clone(),
            key,
            count,
        })
        .collect()
}

fn decade_of(year: i64) -> i64 {
    year - year.rem_euclid(10)
}

// ---------------------------------------------------------------- Listen List

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Waiting {
    pub album_id: String,
    pub edition_id: String,
    pub title: String,
    pub added_at: String,
    /// Whole days on the Listen List.
    pub waiting_days: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitingSummary {
    pub mean_days: f64,
    pub median_days: f64,
    pub longest_days: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Runtime {
    /// Sum over editions whose every track length is known.
    pub known_ms: i64,
    pub editions_known: u32,
    /// Editions with no tracklist or any track of unknown length (not in `known_ms`).
    pub editions_unknown: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListenListStats {
    pub entries: u32,
    pub distinct_albums: u32,
    pub distinct_artists: u32,
    /// Albums per credited artist; a collaboration counts once for each of its artists.
    pub artists: Vec<Count>,
    pub years: Vec<Count>,
    pub decades: Vec<Count>,
    pub unknown_years: u32,
    /// Albums per broad genre. Genres overlap: an album counts once in each of its genres.
    pub genres: Vec<Count>,
    pub albums_without_genre: u32,
    pub tags: Vec<Count>,
    pub recently_added: Vec<Waiting>,
    pub oldest_waiting: Vec<Waiting>,
    pub waiting: Option<WaitingSummary>,
    pub runtime: Runtime,
}

const LIST_LIMIT: usize = 5;

/// Rated albums a group needs before the "Top rated" rankings consider it.
pub const TOP_MIN_ALBUMS: u32 = 3;
/// Entries listed under each "Top rated" heading.
pub const TOP_LIMIT: usize = 3;

/// SQL condition (artist aliased `ar`) that keeps real artists only: not MusicBrainz's
/// special-purpose "Various Artists" (by ID, or by name for manual entries).
const REAL_ARTIST: &str = "ar.musicbrainz_id IS NOT '89ad4ac3-39f7-470e-963a-56509c546377'
     AND lower(trim(ar.name)) != 'various artists'";

/// `now` is an RFC 3339 UTC timestamp (injected so tests are deterministic).
pub fn listen_list(conn: &Connection, now: &str) -> AppResult<ListenListStats> {
    let (entries, distinct_albums): (u32, u32) = conn.query_row(
        "SELECT COUNT(*), COUNT(DISTINCT album_id) FROM listen_list_entry",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;

    let mut artists: HashMap<String, (String, u32)> = HashMap::new();
    let mut stmt = conn.prepare(&format!(
        "SELECT ar.id, ar.name, COUNT(DISTINCT c.album_id) FROM album_artist_credit c
             JOIN artist ar ON ar.id = c.artist_id
             WHERE c.album_id IN (SELECT album_id FROM listen_list_entry) AND {REAL_ARTIST}
             GROUP BY ar.id"
    ))?;
    for row in stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))? {
        let (id, name, n): (String, String, u32) = row?;
        artists.insert(id, (name, n));
    }

    let mut years: BTreeMap<String, u32> = BTreeMap::new();
    let mut decades: BTreeMap<String, u32> = BTreeMap::new();
    let mut unknown_years = 0;
    let mut stmt = conn.prepare(
        "SELECT a.original_year FROM listen_list_entry l JOIN album a ON a.id = l.album_id",
    )?;
    for year in stmt.query_map([], |r| r.get::<_, Option<i64>>(0))? {
        match year? {
            Some(y) => {
                *years.entry(format!("{y:04}")).or_default() += 1;
                *decades.entry(format!("{:04}", decade_of(y))).or_default() += 1;
            }
            None => unknown_years += 1,
        }
    }

    let mut genres: HashMap<String, (String, u32)> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT g.name, COUNT(DISTINCT ag.album_id) FROM album_genre ag JOIN genre g ON g.id = ag.genre_id
         WHERE ag.album_id IN (SELECT album_id FROM listen_list_entry) GROUP BY g.id",
    )?;
    for row in stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))? {
        let (name, n): (String, u32) = row?;
        genres.insert(name.clone(), (name, n));
    }
    let albums_without_genre: u32 = conn.query_row(
        "SELECT COUNT(*) FROM listen_list_entry l
         WHERE NOT EXISTS (SELECT 1 FROM album_genre ag WHERE ag.album_id = l.album_id)",
        [],
        |r| r.get(0),
    )?;

    let mut tags: HashMap<String, (String, u32)> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT t.id, t.name, COUNT(DISTINCT x.album_id)
         FROM album_tag x JOIN tag t ON t.id = x.tag_id
         WHERE x.album_id IN (SELECT album_id FROM listen_list_entry) GROUP BY t.id",
    )?;
    for row in stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))? {
        let (id, name, n): (String, String, u32) = row?;
        tags.insert(id, (name, n));
    }

    let mut stmt = conn.prepare(
        "SELECT l.album_id, l.edition_id, a.title, l.added_at,
                CAST(julianday(?1) - julianday(l.added_at) AS INTEGER)
         FROM listen_list_entry l JOIN album a ON a.id = l.album_id
         ORDER BY l.added_at, l.album_id",
    )?;
    let waiting: Vec<Waiting> = stmt
        .query_map(params![now], |r| {
            Ok(Waiting {
                album_id: r.get(0)?,
                edition_id: r.get(1)?,
                title: r.get(2)?,
                added_at: r.get(3)?,
                waiting_days: r.get::<_, i64>(4)?.max(0),
            })
        })?
        .collect::<Result<_, _>>()?;
    let summary = (!waiting.is_empty()).then(|| {
        let mut days: Vec<i64> = waiting.iter().map(|w| w.waiting_days).collect();
        days.sort_unstable();
        let n = days.len();
        let median = if n % 2 == 1 {
            days[n / 2] as f64
        } else {
            (days[n / 2 - 1] + days[n / 2]) as f64 / 2.0
        };
        WaitingSummary {
            mean_days: days.iter().sum::<i64>() as f64 / n as f64,
            median_days: median,
            longest_days: days[n - 1],
        }
    });
    let oldest_waiting = waiting.iter().take(LIST_LIMIT).cloned().collect();
    let recently_added = waiting.iter().rev().take(LIST_LIMIT).cloned().collect();

    let mut runtime = Runtime {
        known_ms: 0,
        editions_known: 0,
        editions_unknown: 0,
    };
    let mut stmt = conn.prepare(
        "SELECT COUNT(t.id), COUNT(COALESCE(t.length_ms, r.length_ms)),
                COALESCE(SUM(COALESCE(t.length_ms, r.length_ms)), 0)
         FROM listen_list_entry l
         LEFT JOIN track t ON t.edition_id = l.edition_id
         LEFT JOIN recording r ON r.id = t.recording_id
         GROUP BY l.edition_id",
    )?;
    for row in stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))? {
        let (tracks, known, ms): (i64, i64, i64) = row?;
        if tracks > 0 && known == tracks {
            runtime.known_ms += ms;
            runtime.editions_known += 1;
        } else {
            runtime.editions_unknown += 1;
        }
    }

    Ok(ListenListStats {
        entries,
        distinct_albums,
        distinct_artists: artists.len() as u32,
        artists: counts(artists),
        years: chronological(years),
        decades: chronological(decades),
        unknown_years,
        genres: counts(genres),
        albums_without_genre,
        tags: counts(tags),
        recently_added,
        oldest_waiting,
        waiting: summary,
        runtime,
    })
}

// ---------------------------------------------------------------- Collection

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListenCounts {
    /// Listens with a known date (first listens and re-listens alike).
    pub dated: u32,
    /// Listens logged without a date.
    pub undated: u32,
    /// "I listened before, dates unknown" records (one per edition).
    pub historical: u32,
    pub first: u32,
    pub relisten: u32,
    pub unspecified: u32,
    pub full_album: u32,
    pub tracks_only: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreBucket {
    pub stars: f64,
    pub albums: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RatingStats {
    pub rated_albums: u32,
    pub unrated_albums: u32,
    pub average_stars: Option<f64>,
    /// Canonical album scores, exact (no rounding), ascending.
    pub distribution: Vec<ScoreBucket>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupScore {
    pub key: String,
    pub label: String,
    pub mean_stars: f64,
    /// Sample size: rated albums in the group (albums with rated tracks, when ranking by
    /// tracks). Shown, never used to penalize.
    pub albums: u32,
    /// Rated tracks behind `mean_stars` when ranking by tracks; `None` for album rankings.
    pub tracks: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Best {
    /// The best groups with at least [`TOP_MIN_ALBUMS`] rated albums, at most [`TOP_LIMIT`].
    pub top: Vec<GroupScore>,
    /// Rated albums a group needs to be ranked in `top`.
    pub min_albums: u32,
    /// All groups, best first.
    pub groups: Vec<GroupScore>,
    /// Rated albums that couldn't be placed (unknown year, no genre).
    pub unplaced_albums: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FavouriteTrack {
    pub track_id: String,
    pub title: String,
    pub album_id: String,
    pub album_title: String,
    pub edition_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListeningRuntime {
    /// Listening sessions: one album on one date (undated listens are separate sessions).
    pub sessions: u32,
    /// Sum of distinct heard tracks per session with a known length.
    pub known_ms: i64,
    pub tracks_counted: u32,
    pub tracks_without_length: u32,
    /// Full-album listens logged before the tracklist was known (no duration).
    pub listens_without_tracklist: u32,
    /// True only when nothing above is unknown.
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionStats {
    pub albums: u32,
    pub distinct_listened_albums: u32,
    pub listens: ListenCounts,
    pub by_month: Vec<Count>,
    pub by_year: Vec<Count>,
    pub ratings: RatingStats,
    /// Original release year of the canonical album (never a reissue or listening year).
    pub best_year: Best,
    pub best_decade: Best,
    /// Genres overlap: each album counts once in every genre it has.
    pub best_genre: Best,
    /// Collaborations count once for each credited artist.
    pub best_artist: Best,
    /// The same rankings, scored by individual track ratings instead of album scores.
    pub track_rankings: TrackRankings,
    pub favourite_tracks: Vec<FavouriteTrack>,
    pub runtime: ListeningRuntime,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackRankings {
    /// Rated tracks counted (after merging a recording's editions).
    pub rated_tracks: u32,
    pub best_year: Best,
    pub best_decade: Best,
    pub best_genre: Best,
    pub best_artist: Best,
}

fn best(groups: HashMap<String, (String, Vec<f64>)>, unplaced_albums: u32) -> Best {
    rank(
        groups
            .into_iter()
            .map(|(key, (label, scores))| GroupScore {
                key,
                label,
                mean_stars: scores.iter().sum::<f64>() / scores.len() as f64,
                albums: scores.len() as u32,
                tracks: None,
            })
            .collect(),
        unplaced_albums,
    )
}

/// A group of rated tracks: its label, the tracks' stars, and the albums they come from.
type TrackGroup = (String, Vec<f64>, HashSet<String>);

fn best_by_tracks(groups: HashMap<String, TrackGroup>, unplaced: u32) -> Best {
    rank(
        groups
            .into_iter()
            .map(|(key, (label, scores, albums))| GroupScore {
                key,
                label,
                mean_stars: scores.iter().sum::<f64>() / scores.len() as f64,
                albums: albums.len() as u32,
                tracks: Some(scores.len() as u32),
            })
            .collect(),
        unplaced,
    )
}

fn rank(mut groups: Vec<GroupScore>, unplaced_albums: u32) -> Best {
    groups.sort_by(|a, b| {
        b.mean_stars
            .total_cmp(&a.mean_stars)
            .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
    });
    let top = groups
        .iter()
        .filter(|g| g.albums >= TOP_MIN_ALBUMS)
        .take(TOP_LIMIT)
        .cloned()
        .collect();
    Best {
        top,
        min_albums: TOP_MIN_ALBUMS,
        groups,
        unplaced_albums,
    }
}

/// Rankings from individual track ratings of Collection albums.
fn track_rankings(conn: &Connection) -> AppResult<TrackRankings> {
    struct Rated {
        album_id: String,
        year: Option<i64>,
        stars: Vec<f64>,
        track_ids: Vec<String>,
    }
    let mut stmt = conn.prepare(
        "SELECT e.album_id, COALESCE(t.recording_id, t.id), t.id, tr.rating, a.original_year
         FROM track_rating tr
         JOIN track t ON t.id = tr.track_id
         JOIN edition e ON e.id = t.edition_id
         JOIN album a ON a.id = e.album_id
         WHERE tr.rating IS NOT NULL
           AND e.album_id IN (SELECT album_id FROM collection_entry)
         ORDER BY e.album_id, t.id",
    )?;
    // One entry per recording per album; editions of the same recording are averaged.
    let mut recordings: BTreeMap<(String, String), Rated> = BTreeMap::new();
    for row in stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, Option<i64>>(4)?,
        ))
    })? {
        let (album_id, recording, track_id, rating, year) = row?;
        let entry = recordings
            .entry((album_id.clone(), recording))
            .or_insert_with(|| Rated {
                album_id,
                year,
                stars: vec![],
                track_ids: vec![],
            });
        entry.stars.push(rating as f64 / 2.0);
        entry.track_ids.push(track_id);
    }

    let mut genres_of: HashMap<String, Vec<String>> = HashMap::new();
    let mut album_artists: HashMap<String, Vec<(String, String)>> = HashMap::new();
    let mut g = conn.prepare(
        "SELECT g.name FROM album_genre ag JOIN genre g ON g.id = ag.genre_id WHERE ag.album_id = ?1",
    )?;
    let mut aa = conn.prepare(&format!(
        "SELECT ar.id, ar.name FROM album_artist_credit c JOIN artist ar ON ar.id = c.artist_id
         WHERE c.album_id = ?1 AND {REAL_ARTIST}"
    ))?;
    let mut ta = conn.prepare(&format!(
        "SELECT ar.id, ar.name, ({REAL_ARTIST})
         FROM track_artist_credit c JOIN artist ar ON ar.id = c.artist_id
         WHERE c.track_id = ?1 ORDER BY c.position"
    ))?;

    let mut by_year: HashMap<String, TrackGroup> = HashMap::new();
    let mut by_decade: HashMap<String, TrackGroup> = HashMap::new();
    let mut by_genre: HashMap<String, TrackGroup> = HashMap::new();
    let mut by_artist: HashMap<String, TrackGroup> = HashMap::new();
    let (mut no_year, mut no_genre) = (0, 0);
    let add =
        |map: &mut HashMap<String, TrackGroup>, key: &str, label: &str, stars, album: &str| {
            let e = map
                .entry(key.to_owned())
                .or_insert_with(|| (label.to_owned(), vec![], HashSet::new()));
            e.1.push(stars);
            e.2.insert(album.to_owned());
        };
    for r in recordings.values() {
        let stars = r.stars.iter().sum::<f64>() / r.stars.len() as f64;
        match r.year {
            Some(y) => {
                let y_key = format!("{y:04}");
                add(&mut by_year, &y_key, &y_key, stars, &r.album_id);
                let d = format!("{:04}", decade_of(y));
                add(&mut by_decade, &d, &format!("{d}s"), stars, &r.album_id);
            }
            None => no_year += 1,
        }
        if !genres_of.contains_key(&r.album_id) {
            let names = g
                .query_map(params![r.album_id], |row| row.get(0))?
                .collect::<Result<Vec<String>, _>>()?;
            genres_of.insert(r.album_id.clone(), names);
        }
        let genres = &genres_of[&r.album_id];
        if genres.is_empty() {
            no_genre += 1;
        }
        for name in genres {
            add(&mut by_genre, name, name, stars, &r.album_id);
        }
        // The first edition's track credits; the album's credits when the track has none.
        let credits: Vec<(String, String, bool)> = ta
            .query_map(params![r.track_ids[0]], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<Result<_, _>>()?;
        let artists: Vec<(String, String)> = if credits.is_empty() {
            if !album_artists.contains_key(&r.album_id) {
                let list = aa
                    .query_map(params![r.album_id], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect::<Result<Vec<_>, _>>()?;
                album_artists.insert(r.album_id.clone(), list);
            }
            album_artists[&r.album_id].clone()
        } else {
            credits
                .into_iter()
                .filter(|(_, _, real)| *real)
                .map(|(id, name, _)| (id, name))
                .collect()
        };
        for (id, name) in &artists {
            add(&mut by_artist, id, name, stars, &r.album_id);
        }
    }
    Ok(TrackRankings {
        rated_tracks: recordings.len() as u32,
        best_year: best_by_tracks(by_year, no_year),
        best_decade: best_by_tracks(by_decade, no_year),
        best_genre: best_by_tracks(by_genre, no_genre),
        best_artist: best_by_tracks(by_artist, 0),
    })
}

struct RatedAlbum {
    stars: f64,
    year: Option<i64>,
    genres: Vec<String>,
    artists: Vec<(String, String)>,
}

pub fn collection(conn: &Connection) -> AppResult<CollectionStats> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT c.album_id, a.original_year FROM collection_entry c
         JOIN album a ON a.id = c.album_id ORDER BY c.album_id",
    )?;
    let albums: Vec<(String, Option<i64>)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;

    let mut rated = Vec::new();
    for (album_id, year) in &albums {
        let Some(score) = ratings::album_score(conn, album_id)? else {
            continue;
        };
        let mut g = conn.prepare(
            "SELECT g.name FROM album_genre ag JOIN genre g ON g.id = ag.genre_id WHERE ag.album_id = ?1",
        )?;
        let genres = g
            .query_map(params![album_id], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let mut a = conn.prepare(&format!(
            "SELECT ar.id, ar.name FROM album_artist_credit c JOIN artist ar ON ar.id = c.artist_id
             WHERE c.album_id = ?1 AND {REAL_ARTIST}"
        ))?;
        let artists = a
            .query_map(params![album_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        rated.push(RatedAlbum {
            stars: score.stars,
            year: *year,
            genres,
            artists,
        });
    }

    let mut by_year: HashMap<String, (String, Vec<f64>)> = HashMap::new();
    let mut by_decade: HashMap<String, (String, Vec<f64>)> = HashMap::new();
    let mut by_genre: HashMap<String, (String, Vec<f64>)> = HashMap::new();
    let mut by_artist: HashMap<String, (String, Vec<f64>)> = HashMap::new();
    let (mut no_year, mut no_genre) = (0, 0);
    let mut distribution: BTreeMap<u64, u32> = BTreeMap::new();
    for r in &rated {
        *distribution.entry(r.stars.to_bits()).or_default() += 1;
        match r.year {
            Some(y) => {
                let y_key = format!("{y:04}");
                by_year
                    .entry(y_key.clone())
                    .or_insert_with(|| (y_key, vec![]))
                    .1
                    .push(r.stars);
                let d = format!("{:04}", decade_of(y));
                by_decade
                    .entry(d.clone())
                    .or_insert_with(|| (format!("{d}s"), vec![]))
                    .1
                    .push(r.stars);
            }
            None => no_year += 1,
        }
        if r.genres.is_empty() {
            no_genre += 1;
        }
        for g in &r.genres {
            by_genre
                .entry(g.clone())
                .or_insert_with(|| (g.clone(), vec![]))
                .1
                .push(r.stars);
        }
        for (id, name) in &r.artists {
            by_artist
                .entry(id.clone())
                .or_insert_with(|| (name.clone(), vec![]))
                .1
                .push(r.stars);
        }
    }
    let mut distribution: Vec<ScoreBucket> = distribution
        .into_iter()
        .map(|(bits, albums)| ScoreBucket {
            stars: f64::from_bits(bits),
            albums,
        })
        .collect();
    distribution.sort_by(|a, b| a.stars.total_cmp(&b.stars));
    let ratings = RatingStats {
        rated_albums: rated.len() as u32,
        unrated_albums: (albums.len() - rated.len()) as u32,
        average_stars: (!rated.is_empty())
            .then(|| rated.iter().map(|r| r.stars).sum::<f64>() / rated.len() as f64),
        distribution,
    };

    // Listening evidence (deleted listens excluded everywhere).
    let distinct_listened_albums: u32 = conn.query_row(
        "SELECT COUNT(DISTINCT album_id) FROM listen_event WHERE deleted_at IS NULL",
        [],
        |r| r.get(0),
    )?;
    let listens = conn.query_row(
        "SELECT
            COUNT(*) FILTER (WHERE kind != 'prior_history' AND date_status = 'known'),
            COUNT(*) FILTER (WHERE kind != 'prior_history' AND date_status = 'undated'),
            COUNT(*) FILTER (WHERE kind = 'prior_history'),
            COUNT(*) FILTER (WHERE kind = 'first'),
            COUNT(*) FILTER (WHERE kind = 'relisten'),
            COUNT(*) FILTER (WHERE kind = 'unspecified'),
            COUNT(*) FILTER (WHERE kind != 'prior_history' AND is_full = 1),
            COUNT(*) FILTER (WHERE kind != 'prior_history' AND is_full = 0)
         FROM listen_event WHERE deleted_at IS NULL",
        [],
        |r| {
            Ok(ListenCounts {
                dated: r.get(0)?,
                undated: r.get(1)?,
                historical: r.get(2)?,
                first: r.get(3)?,
                relisten: r.get(4)?,
                unspecified: r.get(5)?,
                full_album: r.get(6)?,
                tracks_only: r.get(7)?,
            })
        },
    )?;
    let mut months: BTreeMap<String, u32> = BTreeMap::new();
    let mut years: BTreeMap<String, u32> = BTreeMap::new();
    let mut stmt = conn.prepare(
        "SELECT listened_on FROM listen_event
         WHERE deleted_at IS NULL AND kind != 'prior_history' AND listened_on IS NOT NULL",
    )?;
    for d in stmt.query_map([], |r| r.get::<_, String>(0))? {
        let d = d?;
        *months.entry(d[..7].to_owned()).or_default() += 1;
        *years.entry(d[..4].to_owned()).or_default() += 1;
    }

    let mut stmt = conn.prepare(
        "SELECT t.id, t.title, e.album_id, a.title, e.name FROM track_rating tr
         JOIN track t ON t.id = tr.track_id JOIN edition e ON e.id = t.edition_id
         JOIN album a ON a.id = e.album_id
         WHERE tr.is_favourite = 1 AND e.id IN (SELECT edition_id FROM collection_entry)
         ORDER BY a.title COLLATE NOCASE, e.name, t.disc_number, t.position",
    )?;
    let favourite_tracks = stmt
        .query_map([], |r| {
            Ok(FavouriteTrack {
                track_id: r.get(0)?,
                title: r.get(1)?,
                album_id: r.get(2)?,
                album_title: r.get(3)?,
                edition_name: r.get(4)?,
            })
        })?
        .collect::<Result<_, _>>()?;

    Ok(CollectionStats {
        albums: albums.len() as u32,
        distinct_listened_albums,
        listens,
        by_month: chronological(months),
        by_year: chronological(years),
        ratings,
        best_year: best(by_year, no_year),
        best_decade: best(by_decade, no_year),
        best_genre: best(by_genre, no_genre),
        best_artist: best(by_artist, 0),
        track_rankings: track_rankings(conn)?,
        favourite_tracks,
        runtime: listening_runtime(conn)?,
    })
}

/// Estimated time spent listening. Within one session (an album on one date) a full listen
/// and track listens of the same recordings count each recording once.
fn listening_runtime(conn: &Connection) -> AppResult<ListeningRuntime> {
    let mut stmt = conn.prepare(
        "SELECT e.id, e.album_id, e.listened_on, e.coverage,
                t.id, t.recording_id, COALESCE(t.length_ms, r.length_ms)
         FROM listen_event e
         LEFT JOIN listen_event_track lt ON lt.listen_event_id = e.id
         LEFT JOIN track t ON t.id = lt.track_id
         LEFT JOIN recording r ON r.id = t.recording_id
         WHERE e.deleted_at IS NULL AND e.kind != 'prior_history'",
    )?;
    let mut sessions: HashMap<String, HashMap<String, Option<i64>>> = HashMap::new();
    let mut without_tracklist: HashSet<String> = HashSet::new();
    for row in stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, Option<i64>>(6)?,
        ))
    })? {
        let (event, album, date, coverage, track, recording, length) = row?;
        let session = match date {
            Some(d) => format!("{album}|{d}"),
            None => format!("event|{event}"),
        };
        let heard = sessions.entry(session).or_default();
        match track {
            Some(t) => {
                heard.insert(recording.unwrap_or(t), length);
            }
            None if coverage == "unknown" => {
                without_tracklist.insert(event);
            }
            None => {}
        }
    }
    let mut out = ListeningRuntime {
        sessions: sessions.len() as u32,
        known_ms: 0,
        tracks_counted: 0,
        tracks_without_length: 0,
        listens_without_tracklist: without_tracklist.len() as u32,
        complete: false,
    };
    for length in sessions.values().flat_map(|s| s.values()) {
        match length {
            Some(ms) => {
                out.known_ms += ms;
                out.tracks_counted += 1;
            }
            None => out.tracks_without_length += 1,
        }
    }
    out.complete = out.tracks_without_length == 0 && out.listens_without_tracklist == 0;
    Ok(out)
}

// ---------------------------------------------------------------- Next Up

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MethodStats {
    /// `completely_random`, `weighted_random`, `guided`, `manual`, or `all`.
    pub source: String,
    /// Every recommendation shown (each attempt).
    pub shown: u32,
    pub skipped: u32,
    pub completed: u32,
    /// Shown and not yet resolved (the current pick).
    pub pending: u32,
}

pub fn selection(conn: &Connection) -> AppResult<Vec<MethodStats>> {
    let mut by: HashMap<String, MethodStats> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT source, COUNT(*), COUNT(*) FILTER (WHERE status = 'skipped'),
                COUNT(*) FILTER (WHERE status = 'completed'), COUNT(*) FILTER (WHERE status = 'shown')
         FROM selection_attempt GROUP BY source",
    )?;
    for row in stmt.query_map([], |r| {
        Ok(MethodStats {
            source: r.get(0)?,
            shown: r.get(1)?,
            skipped: r.get(2)?,
            completed: r.get(3)?,
            pending: r.get(4)?,
        })
    })? {
        let m = row?;
        by.insert(m.source.clone(), m);
    }
    let mut out: Vec<MethodStats> = ["completely_random", "weighted_random", "guided", "manual"]
        .into_iter()
        .map(|s| {
            by.remove(s).unwrap_or(MethodStats {
                source: s.into(),
                shown: 0,
                skipped: 0,
                completed: 0,
                pending: 0,
            })
        })
        .collect();
    let total = out.iter().fold(
        MethodStats {
            source: "all".into(),
            shown: 0,
            skipped: 0,
            completed: 0,
            pending: 0,
        },
        |mut t, m| {
            t.shown += m.shown;
            t.skipped += m.skipped;
            t.completed += m.completed;
            t.pending += m.pending;
            t
        },
    );
    out.push(total);
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub listen_list: ListenListStats,
    pub collection: CollectionStats,
    pub selection: Vec<MethodStats>,
}

pub fn overview(conn: &Connection, now: &str) -> AppResult<Stats> {
    Ok(Stats {
        listen_list: listen_list(conn, now)?,
        collection: collection(conn)?,
        selection: selection(conn)?,
    })
}
