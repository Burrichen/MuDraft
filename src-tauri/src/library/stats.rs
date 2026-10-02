//! Stats selectors. Read-only, computed on demand from stored evidence:
//! - Listen List counts are per canonical album (one Listen List entry per album), so
//!   extra editions never inflate them. Unknown years and runtimes are reported apart.
//! - Collection quality uses rated Collection albums only, scored by the canonical rule
//!   shared with Artist pages (`ratings::album_score`): one contribution per album however
//!   many editions or re-listens. Zero is a rating; unrated albums are left out. There are
//!   no minimum sample sizes, smoothing, or penalties; ties are all reported as winners.
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
    pub listen_asap: u32,
    pub recently_added: Vec<Waiting>,
    pub oldest_waiting: Vec<Waiting>,
    pub waiting: Option<WaitingSummary>,
    pub runtime: Runtime,
}

const LIST_LIMIT: usize = 5;

/// `now` is an RFC 3339 UTC timestamp (injected so tests are deterministic).
pub fn listen_list(conn: &Connection, now: &str) -> AppResult<ListenListStats> {
    let (entries, distinct_albums): (u32, u32) = conn.query_row(
        "SELECT COUNT(*), COUNT(DISTINCT album_id) FROM listen_list_entry",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;

    let mut artists: HashMap<String, (String, u32)> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT ar.id, ar.name, COUNT(DISTINCT c.album_id) FROM album_artist_credit c
         JOIN artist ar ON ar.id = c.artist_id
         WHERE c.album_id IN (SELECT album_id FROM listen_list_entry) GROUP BY ar.id",
    )?;
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
    let mut listen_asap = 0;
    let mut stmt = conn.prepare(
        "SELECT t.id, t.name, t.builtin_key = 'listen_asap', COUNT(DISTINCT x.album_id)
         FROM album_tag x JOIN tag t ON t.id = x.tag_id
         WHERE x.album_id IN (SELECT album_id FROM listen_list_entry) GROUP BY t.id",
    )?;
    for row in stmt.query_map([], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get::<_, Option<bool>>(2)?,
            r.get(3)?,
        ))
    })? {
        let (id, name, asap, n): (String, String, Option<bool>, u32) = row?;
        if asap == Some(true) {
            listen_asap = n;
        }
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
        listen_asap,
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
    /// Sample size: rated albums in the group. Shown, never used to penalize.
    pub albums: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Best {
    /// Every group sharing the highest mean (ties are all winners).
    pub winners: Vec<GroupScore>,
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
    pub favourite_tracks: Vec<FavouriteTrack>,
    pub runtime: ListeningRuntime,
}

fn best(groups: HashMap<String, (String, Vec<f64>)>, unplaced_albums: u32) -> Best {
    let mut groups: Vec<GroupScore> = groups
        .into_iter()
        .map(|(key, (label, scores))| GroupScore {
            key,
            label,
            mean_stars: scores.iter().sum::<f64>() / scores.len() as f64,
            albums: scores.len() as u32,
        })
        .collect();
    groups.sort_by(|a, b| {
        b.mean_stars
            .total_cmp(&a.mean_stars)
            .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
    });
    let top = groups.first().map(|g| g.mean_stars);
    let winners = groups
        .iter()
        .filter(|g| Some(g.mean_stars) == top)
        .cloned()
        .collect();
    Best {
        winners,
        groups,
        unplaced_albums,
    }
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
        let mut a = conn.prepare(
            "SELECT ar.id, ar.name FROM album_artist_credit c JOIN artist ar ON ar.id = c.artist_id
             WHERE c.album_id = ?1",
        )?;
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
