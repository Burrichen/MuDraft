//! Next Up: rolls, pool resets, and manual picks over the current Listen List, recorded
//! through `selection`. Candidates are read fresh on every call, so list and tag edits
//! are reflected immediately; the current pick is revalidated rather than silently changed.
//! Nothing here rolls on read: a result changes only through roll, choose, clear, or a
//! completed listen.
//! Rolling never logs listening or changes ratings or list membership.

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use super::idempotency;
use super::listing::{self, ListItem, TagRef};
use super::required_text;
use super::selection::{self, AttemptStatus, SelectionMode, SelectionSource, enum_from};
use crate::domain::ids::parse_uuid;
use crate::domain::picker::{
    self, Candidate, EmptyReason, Filter, GuidedCriteria, Method, Pick, RandomSource, YearChoice,
};
use crate::error::{AppError, AppResult};

const MAX_CHOICES: usize = 200;

fn non_empty<T>(field: &'static str, items: &[T]) -> AppResult<()> {
    if items.is_empty() {
        return Err(AppError::validation(
            field,
            "choose at least one option, or Agnostic",
        ));
    }
    if items.len() > MAX_CHOICES {
        return Err(AppError::validation(field, "too many options"));
    }
    Ok(())
}

/// Validate and canonicalize (sorted, de-duplicated, normalized) so equal criteria
/// compare equal when deciding whether a roll continues the open session.
pub fn validate(conn: &Connection, method: Method) -> AppResult<Method> {
    let Method::Guided(g) = method else {
        return Ok(method);
    };
    let years = match g.years {
        Filter::Agnostic => Filter::Agnostic,
        Filter::AnyOf(mut choices) => {
            non_empty("decade", &choices)?;
            for c in &choices {
                if let YearChoice::Decade(d) = c
                    && (d % 10 != 0 || !(1000..=9990).contains(d))
                {
                    return Err(AppError::validation(
                        "decade",
                        "use the decade's first year, e.g. 1990",
                    ));
                }
            }
            choices.sort_unstable();
            choices.dedup();
            Filter::AnyOf(choices)
        }
    };
    let genres = match g.genres {
        Filter::Agnostic => Filter::Agnostic,
        Filter::AnyOf(names) => {
            non_empty("genre", &names)?;
            let mut keys = names
                .iter()
                .map(|n| required_text("genre", n).map(|n| picker::genre_key(&n)))
                .collect::<AppResult<Vec<_>>>()?;
            keys.sort_unstable();
            keys.dedup();
            Filter::AnyOf(keys)
        }
    };
    let tag_ids = match g.tag_ids {
        Filter::Agnostic => Filter::Agnostic,
        Filter::AnyOf(raw) => {
            non_empty("tag", &raw)?;
            let mut ids = raw
                .iter()
                .map(|t| parse_uuid("tag", t))
                .collect::<AppResult<Vec<_>>>()?;
            ids.sort_unstable();
            ids.dedup();
            for id in &ids {
                let exists = conn
                    .query_row("SELECT 1 FROM tag WHERE id = ?1", params![id], |_| Ok(()))
                    .optional()?
                    .is_some();
                if !exists {
                    return Err(AppError::not_found("tag", id));
                }
            }
            Filter::AnyOf(ids)
        }
    };
    Ok(Method::Guided(GuidedCriteria {
        years,
        genres,
        tag_ids,
    }))
}

/// Every current Listen List edition (re-listen entries included), ordered by album ID
/// so scripted randomness replays exactly.
pub fn candidates(conn: &Connection) -> AppResult<Vec<Candidate>> {
    let mut genres: HashMap<String, Vec<String>> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT ag.album_id, g.name FROM album_genre ag
         JOIN genre g ON g.id = ag.genre_id
         JOIN listen_list_entry l ON l.album_id = ag.album_id",
    )?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (album, name) = row?;
        genres
            .entry(album)
            .or_default()
            .push(picker::genre_key(&name));
    }
    let mut tags: HashMap<String, Vec<String>> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT t.album_id, t.tag_id FROM album_tag t
         JOIN listen_list_entry l ON l.album_id = t.album_id",
    )?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (album, tag) = row?;
        tags.entry(album).or_default().push(tag);
    }
    let mut stmt = conn.prepare(
        "SELECT l.album_id, l.edition_id, a.original_year,
                EXISTS (SELECT 1 FROM album_tag t JOIN tag g ON g.id = t.tag_id
                        WHERE t.album_id = l.album_id AND g.builtin_key = 'listen_asap')
         FROM listen_list_entry l JOIN album a ON a.id = l.album_id
         ORDER BY l.album_id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<i32>>(2)?,
            r.get::<_, bool>(3)?,
        ))
    })?;
    rows.map(|row| {
        let (album_id, edition_id, year, listen_asap) = row?;
        Ok(Candidate {
            genres: genres.remove(&album_id).unwrap_or_default(),
            tag_ids: tags.remove(&album_id).unwrap_or_default(),
            album_id,
            edition_id,
            year,
            listen_asap,
        })
    })
    .collect()
}

struct OpenSession {
    id: String,
    method: Method,
    pool_cycle: i64,
}

fn method_of(mode: &str, criteria: Option<String>) -> AppResult<Option<Method>> {
    let mode: SelectionMode = enum_from("selection mode", mode.to_owned())?;
    Ok(match mode {
        SelectionMode::CompletelyRandom => Some(Method::CompletelyRandom),
        SelectionMode::WeightedRandom => Some(Method::WeightedRandom),
        // Criteria written before v8 (or otherwise unreadable) simply never match.
        SelectionMode::Guided => criteria
            .and_then(|c| serde_json::from_str(&c).ok())
            .map(Method::Guided),
        SelectionMode::Manual => None,
    })
}

fn open_session(conn: &Connection) -> AppResult<Option<OpenSession>> {
    let row: Option<(String, String, Option<String>, i64)> = conn
        .query_row(
            "SELECT id, mode, criteria, pool_cycle FROM selection_session
             WHERE ended_at IS NULL AND mode != 'manual'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((id, mode, criteria, pool_cycle)) = row else {
        return Ok(None);
    };
    Ok(method_of(&mode, criteria)?.map(|method| OpenSession {
        id,
        method,
        pool_cycle,
    }))
}

fn mode_and_source(method: &Method) -> (SelectionMode, SelectionSource) {
    match method {
        Method::CompletelyRandom => (
            SelectionMode::CompletelyRandom,
            SelectionSource::CompletelyRandom,
        ),
        Method::WeightedRandom => (
            SelectionMode::WeightedRandom,
            SelectionSource::WeightedRandom,
        ),
        Method::Guided(_) => (SelectionMode::Guided, SelectionSource::Guided),
    }
}

/// The open session if it uses exactly this method; otherwise a new one.
fn session_for(tx: &Transaction<'_>, method: &Method) -> AppResult<OpenSession> {
    if let Some(open) = open_session(tx)?
        && open.method == *method
    {
        return Ok(open);
    }
    let (mode, _) = mode_and_source(method);
    let criteria = match method {
        Method::Guided(g) => Some(g),
        _ => None,
    };
    let id = selection::start_session(tx, mode, criteria)?;
    Ok(OpenSession {
        id,
        method: method.clone(),
        pool_cycle: 1,
    })
}

/// Albums shown or rejected in the session's current pool cycle.
fn seen(conn: &Connection, session_id: &str, cycle: i64) -> AppResult<HashSet<String>> {
    let mut stmt = conn.prepare(
        "SELECT album_id FROM selection_attempt WHERE session_id = ?1 AND pool_cycle = ?2",
    )?;
    let rows = stmt.query_map(params![session_id, cycle], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn current_shown_album(conn: &Connection) -> AppResult<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT a.album_id FROM current_selection c
             JOIN selection_attempt a ON a.id = c.attempt_id WHERE a.status = 'shown'",
            [],
            |r| r.get(0),
        )
        .optional()?)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shown {
    pub attempt_id: String,
    pub session_id: String,
    pub album_id: String,
    pub edition_id: String,
    pub source: SelectionSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum RollOutcome {
    Picked {
        shown: Shown,
        pool_size: usize,
        remaining: usize,
    },
    /// Nothing qualifies; the current pick is left as it was. No fallback is attempted.
    Empty { reason: EmptyReason },
    /// The pool is used up; only an explicit Reset pool continues. Nothing changes.
    Exhausted { pool_size: usize },
}

/// Roll (or reroll) with `method`. Same method and criteria continue the open session;
/// anything else begins a new one. `key` makes double clicks roll once.
pub fn roll(
    tx: &Transaction<'_>,
    key: Option<&str>,
    method: Method,
    current_year: i32,
    rng: &mut dyn RandomSource,
) -> AppResult<RollOutcome> {
    let method = validate(tx, method)?;
    idempotency::once(tx, key, "next_up_roll", &method, |tx| {
        let session = session_for(tx, &method)?;
        let all = candidates(tx)?;
        let seen = seen(tx, &session.id, session.pool_cycle)?;
        // After a reset, prefer a different album to the one already on screen.
        let mut avoid = seen.clone();
        if let Some(current) = current_shown_album(tx)? {
            avoid.insert(current);
        }
        let mut choice = picker::pick(&method, &all, &avoid, current_year, rng)?;
        if matches!(choice, Pick::Exhausted { .. }) && avoid.len() != seen.len() {
            choice = picker::pick(&method, &all, &seen, current_year, rng)?;
        }
        let (candidate, pool_size) = match choice {
            Pick::Chosen {
                candidate,
                pool_size,
                ..
            } => (candidate, pool_size),
            Pick::Empty(reason) => return Ok(RollOutcome::Empty { reason }),
            Pick::Exhausted { pool_size } => return Ok(RollOutcome::Exhausted { pool_size }),
        };
        let (_, source) = mode_and_source(&method);
        let attempt_id = selection::record_attempt(tx, &session.id, &candidate.edition_id, source)?;
        Ok(RollOutcome::Picked {
            shown: Shown {
                attempt_id,
                session_id: session.id,
                album_id: candidate.album_id.clone(),
                edition_id: candidate.edition_id.clone(),
                source,
            },
            pool_size,
            remaining: picker::eligible(&method, &all, current_year)
                .iter()
                .filter(|c| !seen.contains(&c.album_id) && c.album_id != candidate.album_id)
                .count(),
        })
    })
}

/// Start the open session's next pool cycle. Repeating it before anything new is shown
/// changes nothing.
pub fn reset_pool(tx: &Transaction<'_>) -> AppResult<i64> {
    let open =
        open_session(tx)?.ok_or_else(|| AppError::not_found("Next Up session", "open session"))?;
    selection::reset_pool(tx, &open.id)
}

/// Make a Listen List edition the current pick by hand (provenance: Manual). The open
/// reroll session and its pool are left as they are.
pub fn choose(tx: &Transaction<'_>, key: Option<&str>, edition_id: &str) -> AppResult<Shown> {
    let edition_id = parse_uuid("edition", edition_id)?;
    idempotency::once(tx, key, "next_up_choose", &edition_id.clone(), |tx| {
        let album_id = super::album_of_edition(tx, &edition_id)?;
        let listed: Option<String> = tx
            .query_row(
                "SELECT edition_id FROM listen_list_entry WHERE album_id = ?1",
                params![album_id],
                |r| r.get(0),
            )
            .optional()?;
        match listed {
            None => {
                return Err(AppError::validation(
                    "Next Up pick",
                    "add the album to your Listen List first",
                ));
            }
            Some(listed) if listed != edition_id => {
                return Err(AppError::validation(
                    "Next Up pick",
                    "a different edition of this album is on your Listen List",
                ));
            }
            Some(_) => {}
        }
        let session_id = selection::start_session(tx, SelectionMode::Manual, None)?;
        let attempt_id =
            selection::record_attempt(tx, &session_id, &edition_id, SelectionSource::Manual)?;
        Ok(Shown {
            attempt_id,
            session_id,
            album_id,
            edition_id,
            source: SelectionSource::Manual,
        })
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Eligibility {
    Eligible,
    NotOnListenList,
    /// The Listen List now holds a different edition of the album.
    EditionChanged,
    /// Tags, genres, or year no longer satisfy the method that picked it.
    NoLongerMatches,
}

/// Why the current pick qualified, for display. Only matching choices are listed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchedBy {
    /// The picking session's method; `None` for a manual pick.
    pub method: Option<Method>,
    pub years: Vec<YearChoice>,
    /// Display names of the album's genres that were asked for.
    pub genres: Vec<String>,
    /// The album's tags that were asked for (Listen ASAP for Weighted Random).
    pub tags: Vec<TagRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentPick {
    pub attempt_id: String,
    pub session_id: String,
    pub album_id: String,
    pub edition_id: String,
    pub source: SelectionSource,
    pub status: AttemptStatus,
    pub shown_at: String,
    pub eligibility: Eligibility,
    pub matched: MatchedBy,
    /// Everything the album card needs, even if it has left the Listen List.
    pub item: ListItem,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolView {
    pub session_id: String,
    pub method: Method,
    pub pool_cycle: i64,
    pub pool_size: usize,
    /// Eligible albums not yet shown this cycle.
    pub remaining: usize,
    pub exhausted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NextUpState {
    pub current: Option<CurrentPick>,
    pub session: Option<PoolView>,
}

fn matched_by(method: Option<Method>, item: &ListItem, current_year: i32) -> MatchedBy {
    let year = item.original_year.and_then(|y| i32::try_from(y).ok());
    let (years, genres, tags) = match &method {
        Some(Method::Guided(g)) => (
            match &g.years {
                Filter::AnyOf(choices) => choices
                    .iter()
                    .copied()
                    .filter(|c| match (c, year) {
                        (YearChoice::Decade(d), Some(y)) => (*d..*d + 10).contains(&y),
                        (YearChoice::CurrentYear, Some(y)) => y == current_year,
                        _ => false,
                    })
                    .collect(),
                Filter::Agnostic => vec![],
            },
            match &g.genres {
                Filter::AnyOf(keys) => item
                    .genres
                    .iter()
                    .filter(|n| keys.contains(&picker::genre_key(n)))
                    .cloned()
                    .collect(),
                Filter::Agnostic => vec![],
            },
            match &g.tag_ids {
                Filter::AnyOf(ids) => item
                    .tags
                    .iter()
                    .filter(|t| ids.contains(&t.id))
                    .cloned()
                    .collect(),
                Filter::Agnostic => vec![],
            },
        ),
        Some(Method::WeightedRandom) => (
            vec![],
            vec![],
            item.tags.iter().filter(|t| t.builtin).cloned().collect(),
        ),
        _ => (vec![], vec![], vec![]),
    };
    MatchedBy {
        method,
        years,
        genres,
        tags,
    }
}

/// The current pick (revalidated against today's list and tags) and the open pool.
/// Reading state never rolls.
pub fn state(conn: &Connection, current_year: i32) -> AppResult<NextUpState> {
    let all = candidates(conn)?;
    let current = match selection::current_selection(conn)? {
        None => None,
        Some(c) => {
            let (shown_at, mode, criteria): (String, String, Option<String>) = conn.query_row(
                "SELECT a.shown_at, s.mode, s.criteria
                 FROM selection_attempt a JOIN selection_session s ON s.id = a.session_id
                 WHERE a.id = ?1",
                params![c.attempt_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            let method = method_of(&mode, criteria)?;
            let listed = all.iter().find(|x| x.album_id == c.album_id);
            let eligibility = match (listed, &method) {
                (None, _) => Eligibility::NotOnListenList,
                (Some(l), _) if l.edition_id != c.edition_id => Eligibility::EditionChanged,
                (Some(l), Some(m)) if !picker::allows(m, l, current_year) => {
                    Eligibility::NoLongerMatches
                }
                _ => Eligibility::Eligible,
            };
            let item = listing::item(conn, &c.edition_id)?;
            Some(CurrentPick {
                matched: matched_by(method, &item, current_year),
                item,
                attempt_id: c.attempt_id,
                session_id: c.session_id,
                album_id: c.album_id,
                edition_id: c.edition_id,
                source: c.source,
                status: c.status,
                shown_at,
                eligibility,
            })
        }
    };
    let session = match open_session(conn)? {
        None => None,
        Some(open) => {
            let pool = picker::eligible(&open.method, &all, current_year);
            let seen = seen(conn, &open.id, open.pool_cycle)?;
            let remaining = pool.iter().filter(|c| !seen.contains(&c.album_id)).count();
            Some(PoolView {
                session_id: open.id,
                method: open.method,
                pool_cycle: open.pool_cycle,
                pool_size: pool.len(),
                remaining,
                exhausted: !pool.is_empty() && remaining == 0,
            })
        }
    };
    Ok(NextUpState { current, session })
}

/// Clear the current pick without listening: it counts as rejected in its pool and
/// nothing replaces it. Returns whether there was a pick.
pub fn clear(tx: &Transaction<'_>) -> AppResult<bool> {
    tx.execute(
        "UPDATE selection_attempt SET status = 'skipped',
             resolved_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE status = 'shown' AND id IN (SELECT attempt_id FROM current_selection)",
        [],
    )?;
    Ok(tx.execute("DELETE FROM current_selection", [])? > 0)
}

/// A choice with how many Listen List albums it would match given the *other*
/// categories' selections. Its own category is replaced by just this choice, so OR
/// options are never hidden by siblings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counted<T> {
    pub value: T,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenreOption {
    pub name: String,
    /// Normalized form used in criteria (stored criteria hold keys, not display names).
    pub key: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagOption {
    pub id: String,
    pub name: String,
    pub color: String,
    pub builtin: bool,
    pub count: usize,
    /// Listen List albums with this tag; zero means it cannot be chosen.
    pub on_listen_list: usize,
    /// Albums anywhere with this tag, to explain why an option is unavailable.
    pub anywhere: usize,
}

/// Every Next Up choice and live counts for `criteria`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuidedOptions {
    /// Represented decades, oldest first.
    pub decades: Vec<Counted<i32>>,
    /// Offered only when some album's original year is the current year.
    pub current_year: Option<Counted<i32>>,
    pub unknown_years: usize,
    pub genres: Vec<GenreOption>,
    /// Every defined tag, Listen ASAP first.
    pub tags: Vec<TagOption>,
    /// Listen List albums matching `criteria` exactly (never relaxed).
    pub match_count: usize,
    pub listen_asap: usize,
    pub total: usize,
}

pub fn options(
    conn: &Connection,
    criteria: GuidedCriteria,
    current_year: i32,
) -> AppResult<GuidedOptions> {
    let Method::Guided(criteria) = validate(conn, Method::Guided(criteria))? else {
        unreachable!("validate keeps the method")
    };
    let all = candidates(conn)?;
    let count_with = |edit: &dyn Fn(&mut GuidedCriteria)| {
        let mut c = criteria.clone();
        edit(&mut c);
        let m = Method::Guided(c);
        all.iter()
            .filter(|x| picker::allows(&m, x, current_year))
            .count()
    };
    let year_count = |choice: YearChoice| {
        count_with(&|c: &mut GuidedCriteria| c.years = Filter::AnyOf(vec![choice]))
    };

    let mut represented: Vec<i32> = all
        .iter()
        .filter_map(|c| c.year)
        .map(|y| y - y.rem_euclid(10))
        .collect();
    represented.sort_unstable();
    represented.dedup();
    let decades = represented
        .into_iter()
        .map(|d| Counted {
            value: d,
            count: year_count(YearChoice::Decade(d)),
        })
        .collect();
    let current_year_option = all
        .iter()
        .any(|c| c.year == Some(current_year))
        .then(|| Counted {
            value: current_year,
            count: year_count(YearChoice::CurrentYear),
        });

    let mut stmt = conn.prepare(
        "SELECT DISTINCT g.name FROM album_genre ag
         JOIN genre g ON g.id = ag.genre_id
         JOIN listen_list_entry l ON l.album_id = ag.album_id",
    )?;
    let mut names: BTreeMap<String, String> = BTreeMap::new();
    for name in stmt.query_map([], |r| r.get::<_, String>(0))? {
        let name = name?;
        names.entry(picker::genre_key(&name)).or_insert(name);
    }
    let genres = names
        .into_iter()
        .map(|(key, name)| GenreOption {
            name,
            count: count_with(&|c: &mut GuidedCriteria| {
                c.genres = Filter::AnyOf(vec![key.clone()]);
            }),
            key,
        })
        .collect();

    let mut stmt = conn.prepare(
        "SELECT t.id, t.name, t.color, t.builtin_key IS NOT NULL,
                (SELECT COUNT(*) FROM album_tag x JOIN listen_list_entry l ON l.album_id = x.album_id
                 WHERE x.tag_id = t.id),
                (SELECT COUNT(*) FROM album_tag x WHERE x.tag_id = t.id)
         FROM tag t ORDER BY t.builtin_key IS NULL, t.name COLLATE NOCASE",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, bool>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, i64>(5)?,
        ))
    })?;
    let mut tags = Vec::new();
    for row in rows {
        let (id, name, color, builtin, on_list, anywhere) = row?;
        let count = count_with(&|c: &mut GuidedCriteria| {
            c.tag_ids = Filter::AnyOf(vec![id.clone()]);
        });
        tags.push(TagOption {
            id,
            name,
            color,
            builtin,
            count,
            on_listen_list: on_list as usize,
            anywhere: anywhere as usize,
        });
    }

    Ok(GuidedOptions {
        decades,
        current_year: current_year_option,
        unknown_years: all.iter().filter(|c| c.year.is_none()).count(),
        genres,
        tags,
        match_count: count_with(&|_: &mut GuidedCriteria| {}),
        listen_asap: all.iter().filter(|c| c.listen_asap).count(),
        total: all.len(),
    })
}

/// Uniform indices from the operating system's generator (rejection sampling, no bias).
pub struct OsRandom;

impl RandomSource for OsRandom {
    fn below(&mut self, len: usize) -> AppResult<usize> {
        let n = len as u64;
        // Largest value keeping every residue equally likely.
        let accept_to = u64::MAX - (u64::MAX % n + 1) % n;
        loop {
            let x = getrandom::u64()
                .map_err(|e| AppError::Internal(format!("random generator failed: {e}")))?;
            if x <= accept_to {
                return Ok((x % n) as usize);
            }
        }
    }
}

/// The current year on the user's local calendar.
pub fn local_year() -> i32 {
    use chrono::Datelike;
    chrono::Local::now().year()
}
