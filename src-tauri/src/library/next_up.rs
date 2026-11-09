//! Next Up: rolls, pool resets, and manual picks over the current Listen List, recorded
//! through `selection`. There are up to [`MAX_SLOTS`] independent slots, each with its own
//! picking method, reroll pool, and current album; no album is the pick of two slots.
//! Candidates are read fresh on every call, so list and tag edits are reflected
//! immediately; a current pick is revalidated rather than silently changed.
//! Nothing here rolls on read: a result changes only through roll, choose, clear, or a
//! completed listen.
//! Rolling never logs listening or changes ratings or list membership.

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use super::idempotency;
use super::listing::{self, ListItem, TagRef};
use super::required_text;
use super::selection::{
    self, AttemptStatus, MAX_SLOTS, SelectionMode, SelectionSource, check_slot, enum_from,
};
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

/// `slot`'s open reroll session, if its method is readable.
fn open_session(conn: &Connection, slot: i64) -> AppResult<Option<OpenSession>> {
    let row: Option<(String, String, Option<String>, i64)> = conn
        .query_row(
            "SELECT id, mode, criteria, pool_cycle FROM selection_session
             WHERE ended_at IS NULL AND mode != 'manual' AND slot = ?1",
            params![slot],
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

/// Begin a new reroll session for `method` in `slot`, closing the slot's previous one.
fn new_session(tx: &Transaction<'_>, slot: i64, method: &Method) -> AppResult<OpenSession> {
    let (mode, _) = mode_and_source(method);
    let criteria = match method {
        Method::Guided(g) => Some(g),
        _ => None,
    };
    let id = selection::start_session(tx, slot, mode, criteria)?;
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

fn current_shown_album(conn: &Connection, slot: i64) -> AppResult<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT a.album_id FROM current_selection c
             JOIN selection_attempt a ON a.id = c.attempt_id
             WHERE a.status = 'shown' AND c.slot = ?1",
            params![slot],
            |r| r.get(0),
        )
        .optional()?)
}

/// Albums that are the current pick of a slot other than `slot`.
fn taken_elsewhere(conn: &Connection, slot: i64) -> AppResult<HashSet<String>> {
    let mut stmt = conn.prepare(
        "SELECT a.album_id FROM current_selection c
         JOIN selection_attempt a ON a.id = c.attempt_id WHERE c.slot != ?1",
    )?;
    let rows = stmt.query_map(params![slot], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Slots holding a current pick or an open reroll session, in order.
fn used_slots(conn: &Connection) -> AppResult<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT slot FROM current_selection
         UNION
         SELECT slot FROM selection_session WHERE ended_at IS NULL AND mode != 'manual'
         ORDER BY slot",
    )?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The lowest slot with neither a pick nor a picking method, if any.
pub fn next_free_slot(conn: &Connection) -> AppResult<Option<i64>> {
    let used = used_slots(conn)?;
    Ok((1..=MAX_SLOTS).find(|s| !used.contains(s)))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shown {
    pub slot: i64,
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

/// Albums `method` may pick in `slot`: every candidate, less the picks of other slots
/// unless nothing would match at all (so the empty reason stays about the Listen List).
/// `None` when every match is already another slot's pick.
fn pickable(
    method: &Method,
    all: Vec<Candidate>,
    taken: &HashSet<String>,
    current_year: i32,
) -> Option<Vec<Candidate>> {
    let matching = picker::eligible(method, &all, current_year);
    if matching.is_empty() {
        return Some(all);
    }
    if matching.iter().all(|c| taken.contains(&c.album_id)) {
        return None;
    }
    Some(
        all.into_iter()
            .filter(|c| !taken.contains(&c.album_id))
            .collect(),
    )
}

/// Roll (or reroll) `slot` with `method`. Same method and criteria continue the slot's open
/// session; anything else begins a new one once something is picked. Albums picked in
/// other slots are never offered. `key` makes double clicks roll once.
pub fn roll(
    tx: &Transaction<'_>,
    key: Option<&str>,
    slot: i64,
    method: Method,
    current_year: i32,
    rng: &mut dyn RandomSource,
) -> AppResult<RollOutcome> {
    let slot = check_slot(slot)?;
    let method = validate(tx, method)?;
    idempotency::once(tx, key, "next_up_roll", &(slot, &method), |tx| {
        let open = open_session(tx, slot)?.filter(|o| o.method == method);
        let Some(all) = pickable(
            &method,
            candidates(tx)?,
            &taken_elsewhere(tx, slot)?,
            current_year,
        ) else {
            return Ok(RollOutcome::Empty {
                reason: EmptyReason::AllInNextUp,
            });
        };
        let seen = match &open {
            Some(o) => seen(tx, &o.id, o.pool_cycle)?,
            None => HashSet::new(),
        };
        // After a reset, prefer a different album to the one already on screen.
        let mut avoid = seen.clone();
        if let Some(current) = current_shown_album(tx, slot)? {
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
        let session = match open {
            Some(o) => o,
            None => new_session(tx, slot, &method)?,
        };
        let (_, source) = mode_and_source(&method);
        let attempt_id = selection::record_attempt(tx, &session.id, &candidate.edition_id, source)?;
        Ok(RollOutcome::Picked {
            shown: Shown {
                slot,
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

/// Start the next pool cycle of `slot`'s open session. Repeating it before anything new
/// is shown changes nothing.
pub fn reset_pool(tx: &Transaction<'_>, slot: i64) -> AppResult<i64> {
    let slot = check_slot(slot)?;
    let open = open_session(tx, slot)?
        .ok_or_else(|| AppError::not_found("Next Up session", format!("slot {slot}")))?;
    selection::reset_pool(tx, &open.id)
}

/// Make a Listen List edition a pick by hand (provenance: Manual): in `slot`, replacing its
/// pick (which counts as skipped), or else in the lowest free slot. The slot's reroll
/// session and pool are left as they are.
pub fn choose(
    tx: &Transaction<'_>,
    key: Option<&str>,
    edition_id: &str,
    slot: Option<i64>,
) -> AppResult<Shown> {
    let edition_id = parse_uuid("edition", edition_id)?;
    let slot = slot.map(check_slot).transpose()?;
    idempotency::once(
        tx,
        key,
        "next_up_choose",
        &(edition_id.clone(), slot),
        |tx| {
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
            if selection::current_selections(tx)?
                .iter()
                .any(|c| c.album_id == album_id)
            {
                return Err(AppError::validation(
                    "Next Up pick",
                    "this album is already in your Next Up",
                ));
            }
            let slot = match slot {
                Some(s) => s,
                None => next_free_slot(tx)?.ok_or_else(|| {
                    AppError::validation(
                        "Next Up pick",
                        format!("all {MAX_SLOTS} Next Up slots are in use; choose one to replace"),
                    )
                })?,
            };
            let session_id = selection::start_session(tx, slot, SelectionMode::Manual, None)?;
            let attempt_id =
                selection::record_attempt(tx, &session_id, &edition_id, SelectionSource::Manual)?;
            Ok(Shown {
                slot,
                attempt_id,
                session_id,
                album_id,
                edition_id,
                source: SelectionSource::Manual,
            })
        },
    )
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
    pub slot: i64,
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
    /// Eligible albums not yet shown this cycle (other slots' picks excluded).
    pub remaining: usize,
    pub exhausted: bool,
}

/// One slot: its current pick, its picking method and pool, or both.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotState {
    pub slot: i64,
    pub current: Option<CurrentPick>,
    pub session: Option<PoolView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NextUpState {
    /// Slots in use, in slot order; free slots are omitted.
    pub slots: Vec<SlotState>,
    pub max_slots: i64,
    /// Where a new pick goes; `None` when every slot is in use.
    pub next_free_slot: Option<i64>,
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

/// Every slot in use: its pick (revalidated against today's list and tags) and its open
/// pool. Reading state never rolls.
pub fn state(conn: &Connection, current_year: i32) -> AppResult<NextUpState> {
    let all = candidates(conn)?;
    let mut picks = selection::current_selections(conn)?;
    let mut slots = Vec::new();
    for slot in used_slots(conn)? {
        let current = match picks.iter().position(|c| c.slot == slot) {
            Some(i) => Some(current_pick(
                conn,
                &all,
                picks.swap_remove(i),
                current_year,
            )?),
            None => None,
        };
        let session = match open_session(conn, slot)? {
            None => None,
            Some(open) => {
                let taken = taken_elsewhere(conn, slot)?;
                let pool: Vec<_> = picker::eligible(&open.method, &all, current_year)
                    .into_iter()
                    .filter(|c| !taken.contains(&c.album_id))
                    .collect();
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
        // A slot whose only session is unreadable shows as free.
        if current.is_some() || session.is_some() {
            slots.push(SlotState {
                slot,
                current,
                session,
            });
        }
    }
    let used: Vec<i64> = slots.iter().map(|s| s.slot).collect();
    Ok(NextUpState {
        slots,
        max_slots: MAX_SLOTS,
        next_free_slot: (1..=MAX_SLOTS).find(|s| !used.contains(s)),
    })
}

fn current_pick(
    conn: &Connection,
    all: &[Candidate],
    c: selection::CurrentSelection,
    current_year: i32,
) -> AppResult<CurrentPick> {
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
        (Some(l), Some(m)) if !picker::allows(m, l, current_year) => Eligibility::NoLongerMatches,
        _ => Eligibility::Eligible,
    };
    let item = listing::item(conn, &c.edition_id)?;
    Ok(CurrentPick {
        matched: matched_by(method, &item, current_year),
        item,
        slot: c.slot,
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

/// Remove `slot` without listening: its pick counts as rejected in its pool, its picking
/// method is closed, and nothing replaces it. Other slots are untouched. Returns whether
/// the slot was in use.
pub fn clear(tx: &Transaction<'_>, slot: i64) -> AppResult<bool> {
    let slot = check_slot(slot)?;
    let had_pick = selection::clear_current_selection(tx, slot)?;
    let had_session = selection::end_open_session(tx, slot)?;
    Ok(had_pick || had_session)
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
    /// Whether the built-in Listen ASAP tag still exists (the user may delete it). Without it
    /// Weighted Random has nothing to pick from.
    pub listen_asap_tag: bool,
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

    let listen_asap_tag = tags.iter().any(|t| t.builtin);
    Ok(GuidedOptions {
        decades,
        current_year: current_year_option,
        unknown_years: all.iter().filter(|c| c.year.is_none()).count(),
        genres,
        tags,
        match_count: count_with(&|_: &mut GuidedCriteria| {}),
        listen_asap: all.iter().filter(|c| c.listen_asap).count(),
        listen_asap_tag,
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
