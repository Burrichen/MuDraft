//! Deterministic three-way merge of sync states: the common base (last state both sides
//! agreed on), this computer's state, and the remote state.
//!
//! - Rows are matched by stable primary keys. A row changed on one side only takes that
//!   side's version; a deletion propagates unless the other side changed the row.
//! - When both sides changed a row, columns merge field by field (grouped where they must
//!   move together). The same field changed differently on both sides is a [`Conflict`],
//!   never settled by clocks or "last write wins". Audit timestamps are the exception:
//!   they keep the larger value.
//! - Lists (credits, genres, tracks heard) merge as a whole per parent.
//! - Records created independently on both computers for the same real thing (same
//!   MusicBrainz ID, same tag or genre name, same edition name, same track position) are
//!   unified onto one ID first, so they merge instead of colliding.
//! - Deleting a record the other side still needs (RESTRICT references) is a conflict;
//!   CASCADE and SET NULL references follow the database's own rules.
//! - Merged rows that would break a unique index are reported as conflicts.
//!
//! Conflicts carry stable IDs; the caller resolves them by choosing a side and merging again.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::schema::{OnDelete, POLY_REFS, Schema, TableSchema};
use super::state::{Row, State, key_of};
use crate::profile::sha256_hex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Side {
    Local,
    Remote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConflictKind {
    /// Both sides changed the same field(s) differently.
    Field,
    /// One side deleted a record the other side changed.
    DeleteEdit,
    /// Both sides replaced the same list (credits, genres, tracks heard) differently.
    Collection,
    /// One side deleted a record the other side still needs (e.g. a listen on a deleted
    /// edition). Choosing the side that kept it restores it; the other side drops what
    /// depends on it.
    Orphan,
    /// The merged records would break a uniqueness rule (e.g. two editions, one name).
    Duplicate,
    /// First link of a populated computer: a record here was deleted on another computer.
    DeletedElsewhere,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    /// Stable for the same disagreement; it changes when either side's values change, so
    /// a stale resolution never applies to different data.
    pub id: String,
    pub kind: ConflictKind,
    /// The record in words, e.g. "Review of OK Computer (Standard)" (filled in by the engine).
    pub record: String,
    pub table: String,
    /// Primary key (list conflicts: the parent key; duplicates: the clashing values).
    pub key: Value,
    pub columns: Vec<String>,
    pub base: Value,
    pub local: Value,
    pub remote: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Resolution {
    pub id: String,
    pub choice: Side,
}

pub struct Inputs<'a> {
    pub base: &'a State,
    pub local: &'a State,
    pub remote: &'a State,
    /// False on first link (no common base); remote tombstones then flag local-only rows.
    pub base_known: bool,
    /// Local rows that couldn't be read; they count as unchanged since the base.
    pub unknown: &'a BTreeSet<(String, String)>,
    pub resolutions: &'a [Resolution],
}

pub struct Merged {
    pub state: State,
    /// Sorted by ID. Empty means `state` is ready to validate and apply.
    pub conflicts: Vec<Conflict>,
}

pub fn merge(schema: &Schema, input: &Inputs<'_>) -> Merged {
    let resolutions: HashMap<&str, Side> = input
        .resolutions
        .iter()
        .map(|r| (r.id.as_str(), r.choice))
        .collect();
    let mut b = input.base.clone();
    let mut l = input.local.clone();
    let mut r = input.remote.clone();
    for (table, key) in input.unknown {
        match b.row(table, key).cloned() {
            Some(row) => {
                l.rows_mut(table).insert(key.clone(), row);
            }
            None => {
                l.rows_mut(table).remove(key);
            }
        }
    }
    unify(schema, &mut b, &mut l, &mut r);

    let mut forced: HashMap<(String, String), Side> = HashMap::new();
    let mut last = None;
    for _ in 0..16 {
        let mut cx = Ctx {
            schema,
            b: &b,
            l: &l,
            r: &r,
            res: &resolutions,
            base_known: input.base_known,
            conflicts: BTreeMap::new(),
        };
        let mut m = cx.merge_rows(&forced);
        cx.fix_references(&mut m);
        let grew = cx.check_unique(&m, &mut forced);
        let conflicts = cx.conflicts;
        last = Some((m, conflicts));
        if !grew {
            break;
        }
    }
    let (mut m, conflicts) = last.expect("the loop runs at least once");
    normalize_next_up(schema, &mut m);
    m.tombstones = tombstones(schema, &b, &l, &r, &m);
    Merged {
        state: m,
        conflicts: conflicts.into_values().collect(),
    }
}

// ---------------------------------------------------------------- helpers

fn rank(v: &Value) -> u8 {
    match v {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Number(_) => 2,
        Value::String(_) => 3,
        Value::Array(_) => 4,
        Value::Object(_) => 5,
    }
}

/// Total order on JSON scalars, used only to pick audit timestamps deterministically.
fn cmp_value(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (Value::String(x), Value::String(y)) => x.cmp(y),
        (Value::Number(x), Value::Number(y)) => x
            .as_f64()
            .partial_cmp(&y.as_f64())
            .unwrap_or(Ordering::Equal),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        _ => rank(a).cmp(&rank(b)),
    }
}

fn content_eq(ts: &TableSchema, a: &Row, b: &Row) -> bool {
    (0..ts.columns.len()).all(|i| ts.meta[i] || a[i] == b[i])
}

fn same(ts: &TableSchema, a: Option<&Row>, b: Option<&Row>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => content_eq(ts, x, y),
        _ => false,
    }
}

/// `l` with each audit timestamp replaced by the larger of the two.
fn combine(ts: &TableSchema, l: &Row, r: &Row) -> Row {
    let mut out = l.clone();
    for i in 0..out.len() {
        if ts.meta[i] && cmp_value(&r[i], &l[i]) == Ordering::Greater {
            out[i] = r[i].clone();
        }
    }
    out
}

fn pick<'a>(side: Side, l: Option<&'a Row>, r: Option<&'a Row>) -> Option<&'a Row> {
    match side {
        Side::Local => l,
        Side::Remote => r,
    }
}

fn obj(ts: &TableSchema, row: &Row, cols: &[usize]) -> Value {
    Value::Object(
        cols.iter()
            .map(|&i| (ts.columns[i].clone(), row[i].clone()))
            .collect(),
    )
}

/// A row's user-meaningful content (audit timestamps left out), or null.
fn full(ts: &TableSchema, row: Option<&Row>) -> Value {
    let cols: Vec<usize> = (0..ts.columns.len()).filter(|&i| !ts.meta[i]).collect();
    row.map_or(Value::Null, |r| obj(ts, r, &cols))
}

fn group_value(rows: &BTreeMap<String, Row>, ts: &TableSchema) -> Value {
    Value::Array(rows.values().map(|r| full(ts, Some(r))).collect())
}

fn values<'a>(row: &'a Row, cols: &[usize]) -> Vec<&'a Value> {
    cols.iter().map(|&i| &row[i]).collect()
}

fn json<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

fn parse(key: &str) -> Value {
    serde_json::from_str(key).unwrap_or(Value::Null)
}

fn id_key(id: &str) -> String {
    json(&[id])
}

fn conflict_id(
    kind: &str,
    table: &str,
    key: &str,
    part: &str,
    local: &Value,
    remote: &Value,
) -> String {
    let digest = sha256_hex(json(&(table, key, part, local, remote)).as_bytes());
    format!("{kind}:{}", &digest[..24])
}

struct Ctx<'a> {
    schema: &'a Schema,
    b: &'a State,
    l: &'a State,
    r: &'a State,
    res: &'a HashMap<&'a str, Side>,
    base_known: bool,
    conflicts: BTreeMap<String, Conflict>,
}

impl Ctx<'_> {
    fn push(&mut self, c: Conflict) {
        self.conflicts.entry(c.id.clone()).or_insert(c);
    }

    fn merge_rows(&mut self, forced: &HashMap<(String, String), Side>) -> State {
        let mut m = State::empty(self.schema);
        for ts in &self.schema.tables {
            let (bt, lt, rt) = (
                self.b.rows(ts.name),
                self.l.rows(ts.name),
                self.r.rows(ts.name),
            );
            let mut out = BTreeMap::new();
            if ts.collection.is_empty() {
                let keys: BTreeSet<&String> = bt.keys().chain(lt.keys()).chain(rt.keys()).collect();
                for k in keys {
                    let (bo, lo, ro) = (bt.get(k), lt.get(k), rt.get(k));
                    let row = match forced.get(&(ts.name.to_owned(), k.clone())) {
                        Some(&side) => pick(side, lo, ro).cloned(),
                        None => self.merge_row(ts, k, bo, lo, ro),
                    };
                    if let Some(row) = row {
                        out.insert(k.clone(), row);
                    }
                }
            } else {
                let mut groups: BTreeMap<String, [BTreeMap<String, Row>; 3]> = BTreeMap::new();
                for (i, rows) in [bt, lt, rt].into_iter().enumerate() {
                    for (k, row) in rows {
                        let g = json(&values(row, &ts.collection));
                        groups.entry(g).or_default()[i].insert(k.clone(), row.clone());
                    }
                }
                for (g, [bg, lg, rg]) in &groups {
                    let side = lg
                        .keys()
                        .chain(rg.keys())
                        .find_map(|k| forced.get(&(ts.name.to_owned(), k.clone())));
                    let rows = match side {
                        Some(Side::Local) => lg.clone(),
                        Some(Side::Remote) => rg.clone(),
                        None => self.merge_collection(ts, g, bg, lg, rg),
                    };
                    out.extend(rows);
                }
            }
            *m.rows_mut(ts.name) = out;
        }
        m
    }

    fn merge_row(
        &mut self,
        ts: &TableSchema,
        key: &str,
        b: Option<&Row>,
        l: Option<&Row>,
        r: Option<&Row>,
    ) -> Option<Row> {
        if same(ts, l, r) {
            return match (l, r) {
                (Some(x), Some(y)) => Some(combine(ts, x, y)),
                _ => None,
            };
        }
        if same(ts, l, b) {
            return r.cloned();
        }
        if same(ts, r, b) {
            let tombstoned = !self.base_known
                && r.is_none()
                && self
                    .r
                    .tombstones
                    .get(ts.name)
                    .is_some_and(|t| t.contains(key));
            if !tombstoned {
                return l.cloned();
            }
            let (local, remote) = (full(ts, l), Value::Null);
            let id = conflict_id("deletedElsewhere", ts.name, key, "row", &local, &remote);
            return match self.res.get(id.as_str()) {
                Some(Side::Remote) => None,
                Some(Side::Local) => l.cloned(),
                None => {
                    self.push(Conflict {
                        id,
                        record: String::new(),
                        kind: ConflictKind::DeletedElsewhere,
                        table: ts.name.to_owned(),
                        key: parse(key),
                        columns: Vec::new(),
                        base: Value::Null,
                        local,
                        remote,
                    });
                    l.cloned()
                }
            };
        }
        let (Some(lr), Some(rr)) = (l, r) else {
            let (local, remote) = (full(ts, l), full(ts, r));
            let id = conflict_id("deleteEdit", ts.name, key, "row", &local, &remote);
            return match self.res.get(id.as_str()) {
                Some(&side) => pick(side, l, r).cloned(),
                None => {
                    self.push(Conflict {
                        id,
                        record: String::new(),
                        kind: ConflictKind::DeleteEdit,
                        table: ts.name.to_owned(),
                        key: parse(key),
                        columns: Vec::new(),
                        base: full(ts, b),
                        local,
                        remote,
                    });
                    l.cloned()
                }
            };
        };
        let mut out = combine(ts, lr, rr);
        for unit in &ts.units {
            let (lv, rv) = (values(lr, unit), values(rr, unit));
            if lv == rv {
                continue;
            }
            let bv = b.map(|br| values(br, unit));
            let take_remote = if bv.as_ref() == Some(&lv) {
                true
            } else if bv.as_ref() == Some(&rv) {
                false
            } else {
                let (local, remote) = (obj(ts, lr, unit), obj(ts, rr, unit));
                let part = unit
                    .iter()
                    .map(|&i| ts.columns[i].as_str())
                    .collect::<Vec<_>>()
                    .join(",");
                let id = conflict_id("field", ts.name, key, &part, &local, &remote);
                match self.res.get(id.as_str()) {
                    Some(Side::Remote) => true,
                    Some(Side::Local) => false,
                    None => {
                        self.push(Conflict {
                            id,
                            record: String::new(),
                            kind: ConflictKind::Field,
                            table: ts.name.to_owned(),
                            key: parse(key),
                            columns: unit.iter().map(|&i| ts.columns[i].clone()).collect(),
                            base: b.map_or(Value::Null, |br| obj(ts, br, unit)),
                            local,
                            remote,
                        });
                        false
                    }
                }
            };
            if take_remote {
                for &i in unit {
                    out[i] = rr[i].clone();
                }
            }
        }
        Some(out)
    }

    fn merge_collection(
        &mut self,
        ts: &TableSchema,
        group: &str,
        b: &BTreeMap<String, Row>,
        l: &BTreeMap<String, Row>,
        r: &BTreeMap<String, Row>,
    ) -> BTreeMap<String, Row> {
        let same_set = |x: &BTreeMap<String, Row>, y: &BTreeMap<String, Row>| {
            x.len() == y.len()
                && x.iter()
                    .zip(y)
                    .all(|((ka, ra), (kb, rb))| ka == kb && content_eq(ts, ra, rb))
        };
        if same_set(l, r) {
            return l
                .iter()
                .map(|(k, row)| (k.clone(), combine(ts, row, &r[k])))
                .collect();
        }
        if same_set(l, b) {
            return r.clone();
        }
        if same_set(r, b) {
            return l.clone();
        }
        let (local, remote) = (group_value(l, ts), group_value(r, ts));
        let id = conflict_id("collection", ts.name, group, "rows", &local, &remote);
        match self.res.get(id.as_str()) {
            Some(Side::Local) => l.clone(),
            Some(Side::Remote) => r.clone(),
            None => {
                self.push(Conflict {
                    id,
                    record: String::new(),
                    kind: ConflictKind::Collection,
                    table: ts.name.to_owned(),
                    key: parse(group),
                    columns: ts
                        .collection
                        .iter()
                        .map(|&i| ts.columns[i].clone())
                        .collect(),
                    base: group_value(b, ts),
                    local,
                    remote,
                });
                l.clone()
            }
        }
    }

    // ------------------------------------------------------------ references

    fn fix_references(&mut self, m: &mut State) {
        let mut restored: HashSet<(String, String)> = HashSet::new();
        // Each pass removes or restores at least one row, so this terminates; the bound
        // is only a safety net.
        for _ in 0..100_000 {
            if !self.fix_pass(m, &mut restored) {
                break;
            }
        }
    }

    fn fix_pass(&mut self, m: &mut State, restored: &mut HashSet<(String, String)>) -> bool {
        let schema = self.schema;
        let mut changed = false;
        for ts in &schema.tables {
            for fk in &ts.fks {
                let Some(pts) = schema.table(&fk.parent) else {
                    continue;
                };
                let Some(to_idx) = fk.to.iter().map(|c| pts.col(c)).collect::<Option<Vec<_>>>()
                else {
                    continue;
                };
                let present: HashSet<String> = m
                    .rows(pts.name)
                    .values()
                    .map(|row| json(&values(row, &to_idx)))
                    .collect();
                let orphans: Vec<(String, String)> = m
                    .rows(ts.name)
                    .iter()
                    .filter_map(|(k, row)| {
                        let vals = values(row, &fk.from);
                        if vals.iter().any(|v| v.is_null()) {
                            return None;
                        }
                        let j = json(&vals);
                        (!present.contains(&j)).then(|| (k.clone(), j))
                    })
                    .collect();
                for (ck, pv) in orphans {
                    changed = true;
                    let child_restored = restored.contains(&(ts.name.to_owned(), ck.clone()));
                    match fk.on_delete {
                        OnDelete::SetNull if !child_restored => {
                            if let Some(row) = m.rows_mut(ts.name).get_mut(&ck) {
                                for &i in &fk.from {
                                    row[i] = Value::Null;
                                }
                            }
                        }
                        OnDelete::Cascade if !child_restored => {
                            m.rows_mut(ts.name).remove(&ck);
                        }
                        _ => self.orphan(m, ts, &ck, pts, &to_idx, &pv, child_restored, restored),
                    }
                }
            }
        }
        for &(child, type_col, id_col) in POLY_REFS {
            let Some(cts) = schema.table(child) else {
                continue;
            };
            let (Some(tc), Some(ic)) = (cts.col(type_col), cts.col(id_col)) else {
                continue;
            };
            let orphans: Vec<(String, String, String)> = m
                .rows(child)
                .iter()
                .filter_map(|(k, row)| {
                    let parent = row[tc].as_str()?;
                    let pkey = json(&[&row[ic]]);
                    m.row(parent, &pkey)
                        .is_none()
                        .then(|| (k.clone(), parent.to_owned(), pkey))
                })
                .collect();
            for (ck, parent, pkey) in orphans {
                changed = true;
                if restored.contains(&(child.to_owned(), ck.clone()))
                    && let Some(pts) = schema.table(&parent)
                    && self.restore(m, pts, &pkey, None, restored)
                {
                    continue;
                }
                m.rows_mut(child).remove(&ck);
            }
        }
        changed
    }

    /// A child whose parent row (in a RESTRICT relationship) is missing from the merge.
    #[allow(clippy::too_many_arguments)]
    fn orphan(
        &mut self,
        m: &mut State,
        cts: &TableSchema,
        child_key: &str,
        pts: &TableSchema,
        to_idx: &[usize],
        parent_values: &str,
        child_restored: bool,
        restored: &mut HashSet<(String, String)>,
    ) {
        let find = |s: &State| {
            s.rows(pts.name)
                .iter()
                .find(|(_, row)| json(&values(row, to_idx)) == parent_values)
                .map(|(k, _)| k.clone())
        };
        let Some(pkey) = find(self.l)
            .or_else(|| find(self.r))
            .or_else(|| find(self.b))
        else {
            m.rows_mut(cts.name).remove(child_key);
            return;
        };
        if child_restored && self.restore(m, pts, &pkey, None, restored) {
            return;
        }
        let (lrow, rrow) = (self.l.row(pts.name, &pkey), self.r.row(pts.name, &pkey));
        let (local, remote) = (full(pts, lrow), full(pts, rrow));
        let id = conflict_id("orphan", pts.name, &pkey, "row", &local, &remote);
        match self.res.get(id.as_str()) {
            Some(&side) if pick(side, lrow, rrow).is_some() => {
                if !self.restore(m, pts, &pkey, Some(side), restored) {
                    m.rows_mut(cts.name).remove(child_key);
                }
            }
            Some(_) => {
                m.rows_mut(cts.name).remove(child_key);
            }
            None => {
                self.push(Conflict {
                    id,
                    record: String::new(),
                    kind: ConflictKind::Orphan,
                    table: pts.name.to_owned(),
                    key: parse(&pkey),
                    columns: Vec::new(),
                    base: full(pts, self.b.row(pts.name, &pkey)),
                    local,
                    remote,
                });
                m.rows_mut(cts.name).remove(child_key);
            }
        }
    }

    /// Put a deleted row back, with the rows that would have been cascaded away with it.
    /// Returns false when no side has the row.
    fn restore(
        &self,
        m: &mut State,
        ts: &TableSchema,
        key: &str,
        prefer: Option<Side>,
        restored: &mut HashSet<(String, String)>,
    ) -> bool {
        let order: [&State; 3] = match prefer {
            Some(Side::Remote) => [self.r, self.l, self.b],
            _ => [self.l, self.r, self.b],
        };
        let Some((src, row)) = order
            .into_iter()
            .find_map(|s| s.row(ts.name, key).map(|row| (s, row.clone())))
        else {
            return false;
        };
        if !restored.insert((ts.name.to_owned(), key.to_owned())) {
            return true;
        }
        m.rows_mut(ts.name)
            .entry(key.to_owned())
            .or_insert(row.clone());
        for cts in &self.schema.tables {
            for fk in cts
                .fks
                .iter()
                .filter(|f| f.parent == ts.name && f.on_delete == OnDelete::Cascade)
            {
                let Some(to_idx) = fk.to.iter().map(|c| ts.col(c)).collect::<Option<Vec<_>>>()
                else {
                    continue;
                };
                let pv = json(&values(&row, &to_idx));
                let children: Vec<String> = src
                    .rows(cts.name)
                    .iter()
                    .filter(|(ck, crow)| {
                        json(&values(crow, &fk.from)) == pv && m.row(cts.name, ck).is_none()
                    })
                    .map(|(ck, _)| ck.clone())
                    .collect();
                for ck in children {
                    let side = if std::ptr::eq(src, self.r) {
                        Side::Remote
                    } else {
                        Side::Local
                    };
                    self.restore(m, cts, &ck, Some(side), restored);
                }
            }
        }
        if let Some(id) = ts.col("id").map(|i| &row[i]) {
            for &(child, type_col, id_col) in POLY_REFS {
                let Some(cts) = self.schema.table(child) else {
                    continue;
                };
                let (Some(tc), Some(ic)) = (cts.col(type_col), cts.col(id_col)) else {
                    continue;
                };
                for (ck, crow) in src.rows(child) {
                    if crow[tc] == ts.name && &crow[ic] == id && m.row(child, ck).is_none() {
                        m.rows_mut(child).insert(ck.clone(), crow.clone());
                        restored.insert((child.to_owned(), ck.clone()));
                    }
                }
            }
        }
        true
    }

    // ------------------------------------------------------------ uniqueness

    /// Report unique-index clashes. Resolved clashes force every involved row to the chosen
    /// side; returns true when that added something, so the merge runs again.
    fn check_unique(&mut self, m: &State, forced: &mut HashMap<(String, String), Side>) -> bool {
        let mut grew = false;
        for ts in &self.schema.tables {
            for u in &ts.uniques {
                let mut seen: BTreeMap<String, Vec<String>> = BTreeMap::new();
                for (k, row) in m.rows(ts.name) {
                    let mut parts = Vec::with_capacity(u.columns.len());
                    for (j, &i) in u.columns.iter().enumerate() {
                        match (&row[i], u.nocase[j]) {
                            (Value::Null, _) => break,
                            (Value::String(s), true) => {
                                parts.push(Value::from(s.to_ascii_lowercase()))
                            }
                            (v, _) => parts.push(v.clone()),
                        }
                    }
                    if parts.len() == u.columns.len() {
                        seen.entry(json(&parts)).or_default().push(k.clone());
                    }
                }
                for (uv, keys) in seen.into_iter().filter(|(_, k)| k.len() > 1) {
                    let side_value = |s: &State| {
                        Value::Object(
                            keys.iter()
                                .map(|k| (k.clone(), full(ts, s.row(ts.name, k))))
                                .collect(),
                        )
                    };
                    let (local, remote) = (side_value(self.l), side_value(self.r));
                    let id = conflict_id("duplicate", ts.name, &uv, "unique", &local, &remote);
                    match self.res.get(id.as_str()) {
                        Some(&side) => {
                            for k in keys {
                                grew |= forced.insert((ts.name.to_owned(), k), side).is_none();
                            }
                        }
                        None => self.push(Conflict {
                            id,
                            record: String::new(),
                            kind: ConflictKind::Duplicate,
                            table: ts.name.to_owned(),
                            key: parse(&uv),
                            columns: u.columns.iter().map(|&i| ts.columns[i].clone()).collect(),
                            base: side_value(self.b),
                            local,
                            remote,
                        }),
                    }
                }
            }
        }
        grew
    }
}

// ---------------------------------------------------------------- identity unification

/// Natural identities, in dependency order. Rows with the same values (non-null) are the
/// same real thing; names compare case-insensitively like their unique indexes.
const IDENTITIES: &[(&str, &[&str], &[&str])] = &[
    ("artist", &["musicbrainz_id"], &[]),
    ("genre", &["name"], &["name"]),
    ("tag", &["name"], &["name"]),
    ("album", &["musicbrainz_release_group_id"], &[]),
    ("edition", &["musicbrainz_release_id"], &[]),
    ("edition", &["album_id", "name"], &["name"]),
    ("recording", &["musicbrainz_recording_id"], &[]),
    ("track", &["musicbrainz_track_id"], &[]),
    ("track", &["edition_id", "disc_number", "position"], &[]),
];

fn find(parent: &mut HashMap<String, String>, id: &str) -> String {
    let mut root = id.to_owned();
    while let Some(p) = parent.get(&root).filter(|p| **p != root) {
        root = p.clone();
    }
    let mut cur = id.to_owned();
    while cur != root {
        let next = parent
            .insert(cur.clone(), root.clone())
            .unwrap_or_else(|| root.clone());
        cur = next;
    }
    root
}

fn unify(schema: &Schema, b: &mut State, l: &mut State, r: &mut State) {
    for &(table, cols, nocase) in IDENTITIES {
        let Some(ts) = schema.table(table) else {
            continue;
        };
        let Some(idc) = ts.col("id") else {
            continue;
        };
        let Some(idx) = cols.iter().map(|c| ts.col(c)).collect::<Option<Vec<_>>>() else {
            continue;
        };
        let mut parent: HashMap<String, String> = HashMap::new();
        let mut first: HashMap<String, String> = HashMap::new();
        for s in [&*b, &*l, &*r] {
            for row in s.rows(table).values() {
                let Some(id) = row[idc].as_str() else {
                    continue;
                };
                let mut parts = Vec::with_capacity(idx.len());
                for (j, &i) in idx.iter().enumerate() {
                    match &row[i] {
                        Value::Null => break,
                        Value::String(v) if nocase.contains(&cols[j]) => {
                            parts.push(Value::from(v.to_ascii_lowercase()));
                        }
                        v => parts.push(v.clone()),
                    }
                }
                if parts.len() != idx.len() {
                    continue;
                }
                parent.entry(id.to_owned()).or_insert_with(|| id.to_owned());
                let natural = json(&parts);
                match first.get(&natural) {
                    Some(other) => {
                        let (ra, rb) = (find(&mut parent, other), find(&mut parent, id));
                        if ra != rb {
                            let (hi, lo) = if ra > rb { (ra, rb) } else { (rb, ra) };
                            parent.insert(hi, lo);
                        }
                    }
                    None => {
                        first.insert(natural, id.to_owned());
                    }
                }
            }
        }
        let ids: Vec<String> = parent.keys().cloned().collect();
        let mut comps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for id in ids {
            let root = find(&mut parent, &id);
            comps.entry(root).or_default().insert(id);
        }
        let mut map: HashMap<String, String> = HashMap::new();
        for comp in comps.values().filter(|c| c.len() > 1) {
            let count = |s: &State| {
                comp.iter()
                    .filter(|id| s.row(table, &id_key(id)).is_some())
                    .count()
            };
            // Two of them alive in one state are different things that happen to clash now;
            // leave them for the uniqueness check rather than collapse them.
            if count(b) > 1 || count(l) > 1 || count(r) > 1 {
                continue;
            }
            let canonical = comp
                .iter()
                .find(|id| b.row(table, &id_key(id)).is_some())
                .or_else(|| comp.iter().next())
                .cloned()
                .unwrap_or_default();
            for id in comp.iter().filter(|id| **id != canonical) {
                map.insert(id.clone(), canonical.clone());
            }
        }
        if !map.is_empty() {
            for s in [&mut *b, &mut *l, &mut *r] {
                remap(schema, s, table, &map);
            }
        }
    }
}

/// Rewrite `table`'s IDs everywhere they're referenced, following composite references
/// (e.g. an album ID copied into `edition.album_id` and from there into `listen_event`).
fn remap(schema: &Schema, s: &mut State, table: &str, map: &HashMap<String, String>) {
    let mut targets: BTreeMap<&str, BTreeSet<usize>> = BTreeMap::new();
    let mut queue: Vec<(&str, String)> = vec![(table, "id".to_owned())];
    if let Some(i) = schema.table(table).and_then(|t| t.col("id")) {
        targets.entry(table).or_default().insert(i);
    }
    while let Some((pt, pc)) = queue.pop() {
        for cts in &schema.tables {
            for fk in cts.fks.iter().filter(|f| f.parent == pt) {
                for (j, to) in fk.to.iter().enumerate() {
                    if *to == pc && targets.entry(cts.name).or_default().insert(fk.from[j]) {
                        queue.push((cts.name, cts.columns[fk.from[j]].clone()));
                    }
                }
            }
        }
    }
    let swap = |v: &mut Value| -> bool {
        match v.as_str().and_then(|x| map.get(x)) {
            Some(new) => {
                *v = Value::from(new.as_str());
                true
            }
            None => false,
        }
    };
    for (t, cols) in &targets {
        let Some(ts) = schema.table(t) else {
            continue;
        };
        let rows = std::mem::take(s.rows_mut(t));
        let out = s.rows_mut(t);
        for (_, mut row) in rows {
            for &c in cols {
                swap(&mut row[c]);
            }
            out.insert(key_of(ts, &row), row);
        }
        if let Some(tomb) = s.tombstones.get_mut(*t) {
            let keyed: Vec<usize> = ts
                .pk
                .iter()
                .enumerate()
                .filter(|(_, c)| cols.contains(c))
                .map(|(j, _)| j)
                .collect();
            if !keyed.is_empty() {
                *tomb = std::mem::take(tomb)
                    .into_iter()
                    .map(|k| {
                        let mut v = parse(&k);
                        if let Some(arr) = v.as_array_mut() {
                            for &j in &keyed {
                                if let Some(x) = arr.get_mut(j) {
                                    swap(x);
                                }
                            }
                        }
                        json(&v)
                    })
                    .collect();
            }
        }
    }
    for &(child, type_col, id_col) in POLY_REFS {
        let Some(cts) = schema.table(child) else {
            continue;
        };
        let (Some(tc), Some(ic)) = (cts.col(type_col), cts.col(id_col)) else {
            continue;
        };
        let rows = std::mem::take(s.rows_mut(child));
        let out = s.rows_mut(child);
        for (_, mut row) in rows {
            if row[tc] == table {
                swap(&mut row[ic]);
            }
            out.insert(key_of(cts, &row), row);
        }
    }
}

// ---------------------------------------------------------------- finishing

/// At most one open reroll session may exist. If both computers opened one, keep the
/// session holding the current pick (else the larger ID) and close the others at their
/// last activity.
fn normalize_next_up(schema: &Schema, m: &mut State) {
    let (Some(sess), Some(att), Some(cur)) = (
        schema.table("selection_session"),
        schema.table("selection_attempt"),
        schema.table("current_selection"),
    ) else {
        return;
    };
    let (Some(s_id), Some(s_mode), Some(s_end), Some(s_start)) = (
        sess.col("id"),
        sess.col("mode"),
        sess.col("ended_at"),
        sess.col("started_at"),
    ) else {
        return;
    };
    let (Some(a_sess), Some(a_shown), Some(a_resolved)) = (
        att.col("session_id"),
        att.col("shown_at"),
        att.col("resolved_at"),
    ) else {
        return;
    };
    let open: Vec<String> = m
        .rows(sess.name)
        .iter()
        .filter(|(_, r)| r[s_end].is_null() && r[s_mode] != "manual")
        .map(|(k, _)| k.clone())
        .collect();
    if open.len() < 2 {
        return;
    }
    let current_session = cur.col("attempt_id").and_then(|ci| {
        let attempt = m.rows(cur.name).values().next()?[ci].clone();
        let row = m.row(att.name, &json(&[&attempt]))?;
        Some(row[a_sess].clone())
    });
    let keep = open
        .iter()
        .find(|k| {
            current_session
                .as_ref()
                .is_some_and(|c| m.row(sess.name, k).is_some_and(|r| &r[s_id] == c))
        })
        .or_else(|| open.iter().max())
        .cloned();
    for k in open.into_iter().filter(|k| Some(k) != keep.as_ref()) {
        let Some(id) = m.row(sess.name, &k).map(|r| r[s_id].clone()) else {
            continue;
        };
        let mut end = m
            .row(sess.name, &k)
            .map(|r| r[s_start].clone())
            .unwrap_or(Value::Null);
        for a in m.rows(att.name).values().filter(|a| a[a_sess] == id) {
            for v in [&a[a_shown], &a[a_resolved]] {
                if cmp_value(v, &end) == Ordering::Greater {
                    end = v.clone();
                }
            }
        }
        if let Some(row) = m.rows_mut(sess.name).get_mut(&k) {
            row[s_end] = end;
        }
    }
}

/// Every key ever seen and not live now, so a computer joining later can tell "deleted
/// elsewhere" from "never synced".
fn tombstones(
    schema: &Schema,
    b: &State,
    l: &State,
    r: &State,
    m: &State,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut out = BTreeMap::new();
    for ts in &schema.tables {
        let live = m.rows(ts.name);
        let mut set: BTreeSet<String> = BTreeSet::new();
        for s in [b, l, r] {
            set.extend(
                s.rows(ts.name)
                    .keys()
                    .filter(|k| !live.contains_key(*k))
                    .cloned(),
            );
        }
        for s in [b, r] {
            if let Some(t) = s.tombstones.get(ts.name) {
                set.extend(t.iter().filter(|k| !live.contains_key(*k)).cloned());
            }
        }
        if !set.is_empty() {
            out.insert(ts.name.to_owned(), set);
        }
    }
    out
}
