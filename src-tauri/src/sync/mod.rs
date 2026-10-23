//! Private GitHub sync: one person's MuDraft on several computers, synchronised on demand
//! through a separate, user-created **private** repository (suggested name `MuDraft-Sync`).
//!
//! Private GitHub storage is not end-to-end encryption: GitHub (and anyone the repository
//! is shared with) can read the data.
//!
//! # Repository layout (format version 1)
//! Everything MuDraft writes lives under `sync/`:
//! - `sync/format.json` — format marker, sync identity, schema version, and the size and
//!   SHA-256 of every other file ([`state::FormatFile`]).
//! - `sync/state/<table>.json` — synced rows in primary-key order; `tombstones.json`.
//! - `sync/assets/<sha256>.<ext>` — custom artwork, content-addressed and deduplicated.
//! - `sync/README.md`.
//!
//! Synced: the library (artists, albums, editions, tracks, credits, genres), metadata
//! provenance and manual overrides, tags and assignments, List and Collection membership,
//! listens (dated and undated), ratings (0 distinct from unrated), reviews, favourite
//! tracks, Next Up history and current pick, manual catalogue entries, CSV import
//! fingerprints, portable preferences, and custom/removed artwork. Not synced: downloaded
//! artwork and other caches, discovered discographies, CSV staging, idempotency keys,
//! window/route state, the artwork download consent, credentials, and file paths.
//!
//! # A sync
//! 1. Read the branch head, its file list, and (only for changed files) the state;
//!    validate it. An empty, uninitialised, unrelated, public, or newer-format repository
//!    is a setup error, never something to overwrite. A failed read is an error, never
//!    "the remote is empty".
//! 2. Three-way merge ([`merge`]) against the last common state stored on this computer.
//!    Conflicts stop the sync with nothing changed and are returned for resolution.
//! 3. Validate the result in a scratch database and check size limits.
//! 4. Back up the local database, upload changed files, and commit them as one tree; the
//!    branch moves only as a fast-forward. If another computer got there first, re-read
//!    and re-merge (bounded).
//! 5. Apply locally in one transaction, recording the new common state and commit with it.
//!    If GitHub succeeded but this step didn't, the next sync finds its own commit and
//!    finishes idempotently; if the upload failed, local changes simply stay pending.

pub mod apply;
pub mod github;
pub mod merge;
pub mod schema;
pub mod secrets;
pub mod state;
pub mod summary;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use self::github::{Client, GitHubTransport, RefUpdate, TreeEntry, git_blob_sha, validate_repo};
use self::merge::{Conflict, Inputs, Resolution};
use self::schema::Schema;
use self::secrets::SecretStore;
use self::state::{
    ASSET_DIR, Encoded, FORMAT_PATH, FormatFile, README, README_PATH, STATE_DIR, State, StoredBase,
    TOMBSTONES_PATH, asset_path, build_state, encode, mime_extension, parse_format, parse_table,
    parse_tombstones, snapshot, store_base, table_path, upgrade, validate,
};
use self::summary::{ChangeSummary, describe, summarize};
use crate::db::{SCHEMA_VERSION, backup};
use crate::domain::ids::new_id;
use crate::error::{AppError, AppResult};
use crate::profile::{TableData, sha256_hex};
use crate::state::AppState;

/// Bounds on what is read from or written to GitHub.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_file_bytes: u64,
    pub max_state_bytes: u64,
    pub max_asset_bytes: u64,
    pub max_assets_bytes: u64,
    pub max_files: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            // GitHub refuses files over 100 MB; stay well below.
            max_file_bytes: 50 * 1024 * 1024,
            max_state_bytes: 200 * 1024 * 1024,
            max_asset_bytes: crate::artwork::http::MAX_IMAGE_BYTES as u64,
            max_assets_bytes: 1024 * 1024 * 1024,
            max_files: 50_000,
        }
    }
}

const MAX_ATTEMPTS: usize = 4;
const MAX_FORMAT_BYTES: u64 = 4 * 1024 * 1024;
const KEEP_SYNC_BACKUPS: usize = 5;
const COMMIT_MESSAGE: &str = "MuDraft sync";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorInfo {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub configured: bool,
    pub owner: Option<String>,
    pub repo: Option<String>,
    pub branch: Option<String>,
    /// A token is stored in the OS secure store (the token itself never leaves Rust).
    pub has_token: bool,
    /// This computer has synced with the repository at least once.
    pub linked: bool,
    pub revision: Option<String>,
    pub last_synced_at: Option<String>,
    /// Local changes not yet synced (`None` until linked).
    pub pending_local_changes: Option<bool>,
    /// What those changes are.
    pub pending: Option<ChangeSummary>,
    /// Conflicts the last sync found that still need a choice.
    pub unresolved_conflicts: u32,
    pub last_error: Option<ErrorInfo>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub artists: u64,
    pub albums: u64,
    pub editions: u64,
    pub listens: u64,
    pub rated_tracks: u64,
    pub reviews: u64,
    pub tags: u64,
    pub listen_list: u64,
    pub collection: u64,
    pub artwork: u64,
}

impl Counts {
    fn of(s: &State) -> Self {
        let n = |t: &str| s.rows(t).len() as u64;
        Self {
            artists: n("artist"),
            albums: n("album"),
            editions: n("edition"),
            listens: n("listen_event"),
            rated_tracks: n("track_rating"),
            reviews: n("album_review"),
            tags: n("tag").saturating_sub(1),
            listen_list: n("listen_list_entry"),
            collection: n("collection_entry"),
            artwork: n("artwork"),
        }
    }

    fn populated(&self) -> bool {
        self.artists + self.albums + self.listens + self.tags + self.collection > 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfirmReason {
    /// Publishing this computer's library to an uninitialised repository.
    InitializeFromPopulated,
    /// Merging existing sync data into a computer that already has a library.
    MergeIntoPopulated,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum SyncOutcome {
    #[serde(rename_all = "camelCase")]
    UpToDate { revision: String },
    #[serde(rename_all = "camelCase")]
    Synced {
        revision: String,
        uploaded_files: u32,
        downloaded_files: u32,
        local_changes: u64,
        /// Applied on this computer.
        received: ChangeSummary,
        /// Published to GitHub.
        sent: ChangeSummary,
        /// Backup taken before local changes (in the data folder's `backups/`).
        backup_file_name: Option<String>,
    },
    /// Nothing changed; resolve these and sync again.
    #[serde(rename_all = "camelCase")]
    Conflicts {
        revision: String,
        conflicts: Vec<Conflict>,
    },
    /// Nothing changed; repeat with confirmation to proceed.
    #[serde(rename_all = "camelCase")]
    NeedsConfirmation {
        reason: ConfirmReason,
        local: Counts,
        remote: Option<Counts>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RemoteKind {
    /// No commits yet.
    Empty,
    /// Only a README/licence/ignore file: ready to initialise.
    Uninitialised,
    /// Holds other content; MuDraft won't write to it.
    Unrelated,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteCheck {
    pub branch: String,
    pub kind: RemoteKind,
    /// Ready data belongs to the sync this computer is linked to (`None` if not linked).
    pub same_sync: Option<bool>,
    pub remote: Option<Counts>,
    /// This computer's library, for previewing a first sync.
    pub local: Counts,
    /// What changed on GitHub since this computer last synced (linked only).
    pub remote_changes: Option<ChangeSummary>,
    pub revision: Option<String>,
}

/// How a computer that has never synced with this repository joins it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FirstSync {
    /// Keep both libraries; differences become conflicts.
    Merge,
    /// Replace this computer's synced library with GitHub's (backed up first).
    Download,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncRequest {
    /// Required on first link when this computer already has a library.
    #[serde(default)]
    pub first_sync: Option<FirstSync>,
    #[serde(default)]
    pub resolutions: Vec<Resolution>,
}

/// What a running sync is doing, for a progress display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "phase", rename_all = "camelCase")]
pub enum Progress {
    Reading,
    Downloading { done: u32, total: u32 },
    Merging,
    BackingUp,
    Uploading { done: u32, total: u32 },
    Publishing,
    Applying,
}

pub type Report<'a> = &'a (dyn Fn(Progress) + Send + Sync);

fn quiet(_: Progress) {}

struct Config {
    owner: String,
    repo: String,
    branch: Option<String>,
    sync_id: Option<String>,
    base_revision: Option<String>,
    base_state: Option<String>,
    last_synced_at: Option<String>,
    last_error: Option<String>,
    conflicts: u32,
}

fn load_config(conn: &Connection) -> AppResult<Option<Config>> {
    Ok(conn
        .query_row(
            "SELECT owner, repo, branch, sync_id, base_revision, base_state, last_synced_at, last_error,
                    conflicts
             FROM sync_state WHERE slot = 1",
            [],
            |r| {
                Ok(Config {
                    owner: r.get(0)?,
                    repo: r.get(1)?,
                    branch: r.get(2)?,
                    sync_id: r.get(3)?,
                    base_revision: r.get(4)?,
                    base_state: r.get(5)?,
                    last_synced_at: r.get(6)?,
                    last_error: r.get(7)?,
                    conflicts: r.get(8)?,
                })
            },
        )
        .optional()?)
}

struct Remote {
    head: Option<String>,
    tree: Option<String>,
    /// Blob entries by path.
    entries: HashMap<String, TreeEntry>,
    kind: RemoteKind,
    ready: Option<Ready>,
}

struct Ready {
    format: FormatFile,
    state: State,
    downloaded: u32,
}

struct Prepared {
    /// Image hash → file in the artwork folder, for every image in the merged state.
    files: HashMap<String, String>,
    /// Files written for this sync (removed again if it doesn't complete).
    written: Vec<String>,
    downloaded: u32,
}

fn remote_damaged(e: AppError) -> AppError {
    match e {
        AppError::Validation { reason, .. } => {
            AppError::SyncSetup(format!("the sync data on GitHub is damaged: {reason}"))
        }
        other => other,
    }
}

fn base_damaged(e: AppError) -> AppError {
    AppError::StorageCorrupt(format!(
        "this computer's sync record is damaged ({e}); disconnect sync and link again"
    ))
}

/// Root files a fresh GitHub repository may already have.
fn allowed_before_init(path: &str) -> bool {
    if path == README_PATH {
        return true;
    }
    let lower = path.to_ascii_lowercase();
    !path.contains('/')
        && (lower.starts_with("readme")
            || lower.starts_with("license")
            || lower.starts_with("licence")
            || lower == ".gitignore"
            || lower == ".gitattributes")
}

fn decode_base(schema: &Schema, text: &str, scratch: &Path) -> AppResult<State> {
    let stored: StoredBase =
        serde_json::from_str(text).map_err(|e| AppError::validation("sync data", e.to_string()))?;
    if stored.schema_version > SCHEMA_VERSION {
        return Err(AppError::validation(
            "sync data",
            "written by a newer MuDraft",
        ));
    }
    let tables = upgrade(stored.tables, stored.schema_version, scratch)?;
    build_state(schema, tables, stored.tombstones)
}

fn artwork_dir(state: &AppState) -> PathBuf {
    crate::artwork::artwork_dir(state)
}

fn scratch_dir(state: &AppState) -> PathBuf {
    state.data_dir.join(format!(".sync-scratch-{}", new_id()))
}

fn read_snapshot(state: &AppState, schema: &Schema) -> AppResult<state::Snapshot> {
    let dir = artwork_dir(state);
    state.with_db(|db| {
        db.read(|c| {
            let tx = c.unchecked_transaction()?;
            snapshot(&tx, schema, &dir)
        })
    })
}

fn remove_files(dir: &Path, names: &[String]) {
    for name in names {
        if let Err(e) = std::fs::remove_file(dir.join(name))
            && e.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("MuDraft: couldn't delete artwork file {name}: {e}");
        }
    }
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct Faults {
    /// Fail the local apply transaction once, after its writes.
    pub fail_apply: std::sync::atomic::AtomicBool,
    /// Pretend the data folder's drive has this many bytes free.
    pub free_space: std::sync::Mutex<Option<u64>>,
}

pub struct SyncService {
    transport: Arc<dyn GitHubTransport>,
    secrets: Arc<dyn SecretStore>,
    /// Serialises every operation that writes locally or to GitHub.
    lock: tokio::sync::Mutex<()>,
    limits: Limits,
    api_base: String,
    #[cfg(test)]
    pub(crate) faults: Faults,
}

impl SyncService {
    pub fn new(transport: Arc<dyn GitHubTransport>, secrets: Arc<dyn SecretStore>) -> Self {
        Self::with_limits(transport, secrets, Limits::default())
    }

    pub fn with_limits(
        transport: Arc<dyn GitHubTransport>,
        secrets: Arc<dyn SecretStore>,
        limits: Limits,
    ) -> Self {
        Self {
            transport,
            secrets,
            lock: tokio::sync::Mutex::new(()),
            limits,
            api_base: github::API_BASE.to_owned(),
            #[cfg(test)]
            faults: Faults::default(),
        }
    }

    // ------------------------------------------------------------ settings

    pub fn status(&self, state: &AppState) -> AppResult<SyncStatus> {
        let has_token = self.secrets.read()?.is_some();
        let cfg = state.with_db(|db| db.read(load_config))?;
        let Some(cfg) = cfg else {
            return Ok(SyncStatus {
                configured: false,
                owner: None,
                repo: None,
                branch: None,
                has_token,
                linked: false,
                revision: None,
                last_synced_at: None,
                pending_local_changes: None,
                pending: None,
                unresolved_conflicts: 0,
                last_error: None,
            });
        };
        let linked = cfg.base_state.is_some();
        let pending = match &cfg.base_state {
            Some(text) => {
                let schema = state.with_db(|db| db.read(Schema::load))?;
                let base = decode_base(&schema, text, &scratch_dir(state)).map_err(base_damaged)?;
                let snap = read_snapshot(state, &schema)?;
                let mut local = snap.state;
                for (t, k) in &snap.unknown {
                    if let Some(row) = base.row(t, k) {
                        local.rows_mut(t).insert(k.clone(), row.clone());
                    }
                }
                Some(summarize(&schema, &base, &local))
            }
            None => None,
        };
        Ok(SyncStatus {
            configured: true,
            owner: Some(cfg.owner),
            repo: Some(cfg.repo),
            branch: cfg.branch,
            has_token,
            linked,
            revision: cfg.base_revision,
            last_synced_at: cfg.last_synced_at,
            pending_local_changes: pending.as_ref().map(|p| !p.is_empty()),
            pending,
            unresolved_conflicts: cfg.conflicts,
            last_error: cfg.last_error.and_then(|e| serde_json::from_str(&e).ok()),
        })
    }

    /// Choose the repository. Switching to a different one forgets the old link (the
    /// library itself is untouched).
    pub async fn configure(
        &self,
        state: &AppState,
        owner: &str,
        repo: &str,
    ) -> AppResult<SyncStatus> {
        let (owner, repo) = validate_repo(owner, repo)?;
        let _guard = self.lock.lock().await;
        state.with_db(|db| {
            db.write(|tx| {
                let same = load_config(tx)?.is_some_and(|c| {
                    c.owner.eq_ignore_ascii_case(&owner) && c.repo.eq_ignore_ascii_case(&repo)
                });
                if !same {
                    tx.execute("DELETE FROM sync_state", [])?;
                    tx.execute(
                        "INSERT INTO sync_state (slot, owner, repo) VALUES (1, ?1, ?2)",
                        params![owner, repo],
                    )?;
                }
                Ok(())
            })
        })?;
        self.status(state)
    }

    /// Forget the repository and the last common state. Library data and the token stay.
    pub async fn disconnect(&self, state: &AppState) -> AppResult<SyncStatus> {
        let _guard = self.lock.lock().await;
        state.with_db(|db| db.write(|tx| Ok(tx.execute("DELETE FROM sync_state", [])?)))?;
        self.status(state)
    }

    pub fn set_token(&self, raw: &str) -> AppResult<()> {
        let token = secrets::validate_token(raw)?;
        self.secrets.write(&token)
    }

    pub fn remove_token(&self) -> AppResult<()> {
        self.secrets.delete()
    }

    fn token(&self) -> AppResult<String> {
        self.secrets.read()?.ok_or_else(|| {
            AppError::AuthRequired("add a GitHub access token for this computer first".into())
        })
    }

    fn config(state: &AppState) -> AppResult<Config> {
        state
            .with_db(|db| db.read(load_config))?
            .ok_or_else(|| AppError::SyncSetup("choose a sync repository first".into()))
    }

    fn record(&self, state: &AppState, result: &AppResult<SyncOutcome>) {
        let error = result.as_ref().err().map(|e| {
            serde_json::to_string(&ErrorInfo {
                code: e.code().to_owned(),
                message: e.to_string(),
            })
            .unwrap_or_default()
        });
        // Errors leave earlier conflicts standing; only a finished sync clears them.
        let conflicts: Option<i64> = match result {
            Ok(SyncOutcome::Conflicts { conflicts, .. }) => Some(conflicts.len() as i64),
            Ok(SyncOutcome::Synced { .. } | SyncOutcome::UpToDate { .. }) => Some(0),
            _ => None,
        };
        let saved = state.with_db(|db| {
            db.write(|tx| {
                tx.execute(
                    "UPDATE sync_state SET last_error = ?1, conflicts = COALESCE(?2, conflicts),
                         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE slot = 1",
                    params![error, conflicts],
                )?;
                Ok(())
            })
        });
        if let Err(e) = saved {
            eprintln!("MuDraft: couldn't record the sync result [{}]", e.code());
        }
    }

    // ------------------------------------------------------------ remote

    /// Read the repository; `branch` is the stored branch or the repository default.
    async fn open_remote(&self, gh: &Client<'_>, cfg: &Config) -> AppResult<String> {
        let info = gh.repo_info().await?;
        if !info.private {
            return Err(AppError::SyncSetup(
                "the repository is public. MuDraft only syncs to a private repository".into(),
            ));
        }
        Ok(cfg.branch.clone().unwrap_or(info.default_branch))
    }

    async fn read_remote(
        &self,
        gh: &Client<'_>,
        schema: &Schema,
        branch: &str,
        base: Option<(&State, &str)>,
        scratch: &Path,
        report: Report<'_>,
    ) -> AppResult<Remote> {
        report(Progress::Reading);
        let Some(head) = gh.head(branch).await? else {
            return Ok(Remote {
                head: None,
                tree: None,
                entries: HashMap::new(),
                kind: RemoteKind::Empty,
                ready: None,
            });
        };
        let tree = gh.commit_tree(&head).await?;
        let listing = gh.tree(&tree).await?;
        if listing.len() > self.limits.max_files {
            return Err(AppError::SyncSetup(
                "the repository holds too many files".into(),
            ));
        }
        let entries: HashMap<String, TreeEntry> = listing
            .into_iter()
            .filter(|e| e.kind == "blob")
            .map(|e| (e.path.clone(), e))
            .collect();
        let mut remote = Remote {
            head: Some(head.clone()),
            tree: Some(tree),
            kind: RemoteKind::Unrelated,
            ready: None,
            entries,
        };
        if let Some(format_entry) = remote.entries.get(FORMAT_PATH) {
            if format_entry.size.is_some_and(|s| s > MAX_FORMAT_BYTES) {
                return Err(AppError::SyncSetup("sync/format.json is too large".into()));
            }
            let bytes = gh.blob(&format_entry.sha, MAX_FORMAT_BYTES).await?;
            let format = parse_format(&bytes)?;
            let ready = self
                .read_state(gh, schema, &remote, format, &head, base, scratch, report)
                .await
                .map_err(remote_damaged)?;
            remote.kind = RemoteKind::Ready;
            remote.ready = Some(ready);
        } else if remote.entries.keys().all(|p| allowed_before_init(p)) {
            remote.kind = RemoteKind::Uninitialised;
        }
        Ok(remote)
    }

    #[allow(clippy::too_many_arguments)]
    async fn read_state(
        &self,
        gh: &Client<'_>,
        schema: &Schema,
        remote: &Remote,
        format: FormatFile,
        head: &str,
        base: Option<(&State, &str)>,
        scratch: &Path,
        report: Report<'_>,
    ) -> AppResult<Ready> {
        let bad = |m: String| AppError::validation("sync data", m);
        let mut listed = std::collections::BTreeSet::new();
        let mut total = 0_u64;
        for f in &format.files {
            let ok_path = f.path == TOMBSTONES_PATH
                || f.path
                    .strip_prefix(STATE_DIR)
                    .and_then(|n| n.strip_suffix(".json"))
                    .is_some_and(|n| {
                        !n.is_empty() && n.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
                    });
            if !ok_path || !listed.insert(f.path.clone()) {
                return Err(bad(format!("unexpected file {}", f.path)));
            }
            let entry = remote
                .entries
                .get(&f.path)
                .ok_or_else(|| bad(format!("{} is missing", f.path)))?;
            if entry.size.is_some_and(|s| s != f.bytes) {
                return Err(bad(format!("{} has the wrong size", f.path)));
            }
            if f.bytes > self.limits.max_file_bytes {
                return Err(bad(format!("{} is larger than MuDraft allows", f.path)));
            }
            total += f.bytes;
        }
        if total > self.limits.max_state_bytes {
            return Err(bad("the sync data is larger than MuDraft allows".into()));
        }
        if let Some(extra) = remote
            .entries
            .keys()
            .find(|p| p.starts_with(STATE_DIR) && !listed.contains(*p))
        {
            return Err(bad(format!("unexpected file {extra}")));
        }
        let mut asset_total = 0_u64;
        for a in &format.assets {
            let entry = remote.entries.get(&a.path);
            let ok = a.path == asset_path(&a.sha256, &a.mime)
                && mime_extension(&a.mime).is_some()
                && a.bytes <= self.limits.max_asset_bytes
                && entry.is_some_and(|e| e.size.is_none_or(|s| s == a.bytes));
            if !ok {
                return Err(bad(format!("artwork {} is missing or invalid", a.path)));
            }
            asset_total += a.bytes;
        }
        if asset_total > self.limits.max_assets_bytes {
            return Err(bad(
                "the synced artwork is larger than MuDraft allows".into()
            ));
        }

        // Files identical to the stored common state are taken from it, not downloaded.
        let reuse = base.filter(|_| format.schema_version == SCHEMA_VERSION);
        if let Some((state, revision)) = reuse
            && revision == head
        {
            return Ok(Ready {
                format,
                state: state.clone(),
                downloaded: 0,
            });
        }
        let base_files: HashMap<String, String> = match reuse {
            Some((state, _)) => encode(schema, state, &format.sync_id)?
                .files
                .into_iter()
                .map(|(p, b)| (p, git_blob_sha(&b)))
                .collect(),
            None => HashMap::new(),
        };
        let mut tables = Vec::new();
        let mut tombstones = BTreeMap::new();
        let mut downloaded = 0;
        let total = format
            .files
            .iter()
            .filter(|f| base_files.get(&f.path) != Some(&remote.entries[&f.path].sha))
            .count() as u32;
        for f in &format.files {
            let sha = &remote.entries[&f.path].sha;
            if let Some((state, _)) = reuse
                && base_files.get(&f.path) == Some(sha)
            {
                if f.path == TOMBSTONES_PATH {
                    tombstones = state
                        .tombstones
                        .iter()
                        .map(|(t, keys)| {
                            (
                                t.clone(),
                                keys.iter()
                                    .map(|k| serde_json::from_str(k).unwrap_or(Value::Null))
                                    .collect(),
                            )
                        })
                        .collect();
                } else {
                    let name = f
                        .path
                        .trim_start_matches(STATE_DIR)
                        .trim_end_matches(".json");
                    let ts = schema
                        .table(name)
                        .ok_or_else(|| bad(format!("unknown table {name}")))?;
                    tables.push(TableData {
                        table: ts.name.to_owned(),
                        columns: ts.columns.clone(),
                        rows: state.rows(ts.name).values().cloned().collect(),
                    });
                }
                continue;
            }
            report(Progress::Downloading {
                done: downloaded,
                total,
            });
            let bytes = gh.blob(sha, f.bytes).await?;
            downloaded += 1;
            if bytes.len() as u64 != f.bytes || sha256_hex(&bytes) != f.sha256 {
                return Err(bad(format!("{} failed its checksum", f.path)));
            }
            if f.path == TOMBSTONES_PATH {
                tombstones = parse_tombstones(&bytes)?;
            } else {
                let data = parse_table(&bytes)?;
                if table_path(&data.table) != f.path {
                    return Err(bad(format!("{} holds the wrong table", f.path)));
                }
                tables.push(data);
            }
        }
        let tables = upgrade(tables, format.schema_version, scratch)?;
        let state = build_state(schema, tables, tombstones)?;
        for row in state.rows("artwork").values() {
            if let (Some(sha), Some(mime)) = (row[3].as_str(), row[4].as_str())
                && !format
                    .assets
                    .iter()
                    .any(|a| a.sha256 == sha && a.mime == mime)
            {
                return Err(bad("artwork is referenced but not listed".into()));
            }
        }
        Ok(Ready {
            format,
            state,
            downloaded,
        })
    }

    fn check_size(&self, enc: &Encoded) -> AppResult<()> {
        let mb = |b: u64| b.div_ceil(1024 * 1024);
        for (path, bytes) in &enc.files {
            if bytes.len() as u64 > self.limits.max_file_bytes {
                return Err(AppError::validation(
                    "sync data",
                    format!(
                        "{path} would be {} MB; the limit is {} MB",
                        mb(bytes.len() as u64),
                        mb(self.limits.max_file_bytes)
                    ),
                ));
            }
        }
        if enc.total_bytes() > self.limits.max_state_bytes {
            return Err(AppError::validation(
                "sync data",
                format!(
                    "the library would be {} MB; the limit is {} MB",
                    mb(enc.total_bytes()),
                    mb(self.limits.max_state_bytes)
                ),
            ));
        }
        let assets: u64 = enc.assets.values().map(|a| a.bytes).sum();
        if enc
            .assets
            .values()
            .any(|a| a.bytes > self.limits.max_asset_bytes)
            || assets > self.limits.max_assets_bytes
        {
            return Err(AppError::validation(
                "sync data",
                "custom artwork is larger than MuDraft syncs",
            ));
        }
        if enc.files.len() + enc.assets.len() > self.limits.max_files {
            return Err(AppError::validation("sync data", "too many files to sync"));
        }
        Ok(())
    }

    /// Upload changed files and commit them on top of the remote head. Returns the new
    /// commit, or `None` when the remote already holds exactly this data.
    async fn upload(
        &self,
        gh: &Client<'_>,
        enc: &Encoded,
        remote: &Remote,
        files: &HashMap<String, String>,
        art_dir: &Path,
        report: Report<'_>,
    ) -> AppResult<Option<(String, u32)>> {
        let (Some(head), Some(tree)) = (&remote.head, &remote.tree) else {
            return Err(AppError::Internal("upload needs a remote commit".into()));
        };
        let mut changes: Vec<(String, Option<String>)> = Vec::new();
        let mut uploaded = 0;
        let total = (enc
            .files
            .iter()
            .filter(|(p, b)| remote.entries.get(*p).map(|e| &e.sha) != Some(&git_blob_sha(b)))
            .count()
            + enc
                .assets
                .keys()
                .filter(|p| !remote.entries.contains_key(*p))
                .count()) as u32;
        for (path, bytes) in &enc.files {
            let sha = git_blob_sha(bytes);
            if remote.entries.get(path).map(|e| &e.sha) != Some(&sha) {
                report(Progress::Uploading {
                    done: uploaded,
                    total,
                });
                gh.create_blob(bytes).await?;
                changes.push((path.clone(), Some(sha)));
                uploaded += 1;
            }
        }
        for (path, asset) in &enc.assets {
            if remote.entries.contains_key(path) {
                continue;
            }
            let name = files.get(&asset.sha256).ok_or_else(|| {
                AppError::Internal(format!("no local file for artwork {}", asset.sha256))
            })?;
            let bytes = std::fs::read(art_dir.join(name))
                .map_err(|e| AppError::StorageUnavailable(format!("artwork file: {e}")))?;
            if sha256_hex(&bytes) != asset.sha256 {
                return Err(AppError::Conflict(
                    "artwork changed during sync; try again".into(),
                ));
            }
            report(Progress::Uploading {
                done: uploaded,
                total,
            });
            let sha = gh.create_blob(&bytes).await?;
            changes.push((path.clone(), Some(sha)));
            uploaded += 1;
        }
        for path in remote.entries.keys() {
            let ours = path.starts_with(STATE_DIR) || path.starts_with(ASSET_DIR);
            if ours && !enc.files.contains_key(path) && !enc.assets.contains_key(path) {
                changes.push((path.clone(), None));
            }
        }
        if changes.is_empty() {
            return Ok(None);
        }
        changes.sort();
        report(Progress::Publishing);
        let new_tree = gh.create_tree(tree, &changes).await?;
        let commit = gh.create_commit(COMMIT_MESSAGE, &new_tree, head).await?;
        Ok(Some((commit, uploaded)))
    }

    /// Make every image in `merged` available as a local file, downloading missing ones.
    async fn prepare_artwork(
        &self,
        gh: &Client<'_>,
        merged: &State,
        local: &HashMap<String, String>,
        remote: &Remote,
        art_dir: &Path,
        report: Report<'_>,
    ) -> AppResult<Prepared> {
        let mut prepared = Prepared {
            files: HashMap::new(),
            written: Vec::new(),
            downloaded: 0,
        };
        let result = async {
            for row in merged.rows("artwork").values() {
                let (Some(sha), Some(mime)) = (row[3].as_str(), row[4].as_str()) else {
                    continue;
                };
                if prepared.files.contains_key(sha) {
                    continue;
                }
                if let Some(name) = local.get(sha) {
                    prepared.files.insert(sha.to_owned(), name.clone());
                    continue;
                }
                let path = asset_path(sha, mime);
                let entry = remote.entries.get(&path).ok_or_else(|| {
                    AppError::SyncSetup(format!("{path} is missing from the sync data"))
                })?;
                report(Progress::Downloading {
                    done: prepared.downloaded,
                    total: prepared.downloaded + 1,
                });
                let bytes = gh.blob(&entry.sha, self.limits.max_asset_bytes).await?;
                prepared.downloaded += 1;
                if sha256_hex(&bytes) != sha
                    || crate::artwork::validate_image(&bytes).ok() != Some(mime)
                {
                    return Err(AppError::SyncSetup(format!("{path} is damaged")));
                }
                std::fs::create_dir_all(art_dir)
                    .map_err(|e| AppError::StorageUnavailable(format!("artwork folder: {e}")))?;
                let name = format!(
                    "synced-{}.{}",
                    new_id(),
                    mime_extension(mime).unwrap_or("bin")
                );
                let tmp = art_dir.join(format!("{name}.partial"));
                std::fs::write(&tmp, &bytes)
                    .and_then(|()| std::fs::rename(&tmp, art_dir.join(&name)))
                    .map_err(|e| AppError::StorageUnavailable(format!("artwork file: {e}")))?;
                prepared.written.push(name.clone());
                prepared.files.insert(sha.to_owned(), name);
            }
            Ok(())
        }
        .await;
        match result {
            Ok(()) => Ok(prepared),
            Err(e) => {
                remove_files(art_dir, &prepared.written);
                Err(e)
            }
        }
    }

    /// Apply `merged` and record it as the new common state, in one transaction. Returns
    /// `None` (nothing written) if local data changed since `expected` was read.
    #[allow(clippy::too_many_arguments)]
    fn apply_local(
        &self,
        state: &AppState,
        schema: &Schema,
        expected: &State,
        merged: &State,
        files: &HashMap<String, String>,
        revision: &str,
        sync_id: &str,
    ) -> AppResult<Option<u64>> {
        let art_dir = artwork_dir(state);
        let base = store_base(schema, merged)?;
        let applied = state.with_db(|db| {
            db.write_unordered(|tx| {
                let current = snapshot(tx, schema, &art_dir)?;
                if current.state != *expected {
                    return Ok(None);
                }
                let applied = apply::apply(tx, schema, &current.state, merged, files)?;
                #[cfg(test)]
                if self
                    .faults
                    .fail_apply
                    .swap(false, std::sync::atomic::Ordering::SeqCst)
                {
                    return Err(AppError::StorageUnavailable("injected failure".into()));
                }
                tx.execute(
                    "UPDATE sync_state SET sync_id = ?1, base_revision = ?2, base_state = ?3,
                         last_synced_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), last_error = NULL,
                         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                     WHERE slot = 1",
                    params![sync_id, revision, base],
                )?;
                Ok(Some(applied))
            })
        })?;
        Ok(applied.map(|a| {
            remove_files(&art_dir, &a.stale_files);
            a.changes
        }))
    }

    fn backup(state: &AppState) -> AppResult<String> {
        state.with_db(|db| {
            let name = db.create_backup("pre-sync")?;
            let dir = db.backup_dir().to_path_buf();
            let old: Vec<_> = backup::list(&dir)?
                .into_iter()
                .filter(|b| b.file_name.ends_with("-pre-sync.sqlite3"))
                .collect();
            for b in old.iter().take(old.len().saturating_sub(KEEP_SYNC_BACKUPS)) {
                let _ = std::fs::remove_file(dir.join(&b.file_name));
            }
            Ok(name)
        })
    }

    // ------------------------------------------------------------ operations

    /// Make sure the data folder's drive can hold a backup plus the changes.
    fn ensure_space(&self, state: &AppState, downloads: u64) -> AppResult<()> {
        let size = |name: &str| {
            std::fs::metadata(state.data_dir.join(name))
                .map(|m| m.len())
                .unwrap_or(0)
        };
        let db =
            size(crate::paths::DB_FILE_NAME) + size(&format!("{}-wal", crate::paths::DB_FILE_NAME));
        // A backup copy, room for the transaction, new images, and a margin.
        let needed = db * 2 + downloads + 64 * 1024 * 1024;
        #[cfg(test)]
        let forced = *self.faults.free_space.lock().unwrap();
        #[cfg(not(test))]
        let forced: Option<u64> = None;
        let free = match forced {
            Some(f) => f,
            None => match fs4::available_space(&state.data_dir) {
                Ok(f) => f,
                // Unknown: carry on; a full disk still fails safely inside the transaction.
                Err(_) => return Ok(()),
            },
        };
        if free < needed {
            let mb = |b: u64| b.div_ceil(1024 * 1024);
            return Err(AppError::DiskFull(format!(
                "syncing needs about {} MB free on the drive holding MuDraft's data; {} MB is free. Nothing was changed",
                mb(needed),
                mb(free)
            )));
        }
        Ok(())
    }

    /// Inspect the repository without changing anything.
    pub async fn check(&self, state: &AppState) -> AppResult<RemoteCheck> {
        let cfg = Self::config(state)?;
        let token = self.token()?;
        let gh = Client::new(
            &*self.transport,
            &token,
            &self.api_base,
            &cfg.owner,
            &cfg.repo,
        );
        let branch = self.open_remote(&gh, &cfg).await?;
        let schema = state.with_db(|db| db.read(Schema::load))?;
        let scratch = scratch_dir(state);
        let base = match (&cfg.base_state, &cfg.base_revision) {
            (Some(text), Some(rev)) => Some((
                decode_base(&schema, text, &scratch).map_err(base_damaged)?,
                rev.clone(),
            )),
            _ => None,
        };
        let remote = self
            .read_remote(
                &gh,
                &schema,
                &branch,
                base.as_ref().map(|(s, r)| (s, r.as_str())),
                &scratch,
                &quiet,
            )
            .await?;
        let local = Counts::of(&read_snapshot(state, &schema)?.state);
        let same_sync = match (&cfg.sync_id, &remote.ready) {
            (Some(id), Some(r)) => Some(*id == r.format.sync_id),
            _ => None,
        };
        let remote_changes = match (&base, &remote.ready, same_sync) {
            (Some((b, _)), Some(r), Some(true)) => Some(summarize(&schema, b, &r.state)),
            _ => None,
        };
        Ok(RemoteCheck {
            branch,
            kind: remote.kind,
            same_sync,
            remote: remote.ready.map(|r| Counts::of(&r.state)),
            local,
            remote_changes,
            revision: remote.head,
        })
    }

    /// Publish this computer's library to an empty or uninitialised repository.
    pub async fn initialize(&self, state: &AppState, confirm: bool) -> AppResult<SyncOutcome> {
        self.initialize_reporting(state, confirm, &quiet).await
    }

    pub async fn initialize_reporting(
        &self,
        state: &AppState,
        confirm: bool,
        report: Report<'_>,
    ) -> AppResult<SyncOutcome> {
        let _guard = self.lock.lock().await;
        let result = self.initialize_inner(state, confirm, report).await;
        self.record(state, &result);
        result
    }

    async fn initialize_inner(
        &self,
        state: &AppState,
        confirm: bool,
        report: Report<'_>,
    ) -> AppResult<SyncOutcome> {
        let cfg = Self::config(state)?;
        let token = self.token()?;
        let gh = Client::new(
            &*self.transport,
            &token,
            &self.api_base,
            &cfg.owner,
            &cfg.repo,
        );
        let branch = self.open_remote(&gh, &cfg).await?;
        let schema = state.with_db(|db| db.read(Schema::load))?;
        let scratch = scratch_dir(state);
        let mut remote = self
            .read_remote(&gh, &schema, &branch, None, &scratch, report)
            .await?;
        match remote.kind {
            RemoteKind::Empty | RemoteKind::Uninitialised => {}
            RemoteKind::Ready => {
                return Err(AppError::SyncSetup(
                    "the repository already holds MuDraft sync data; use Sync instead".into(),
                ));
            }
            RemoteKind::Unrelated => {
                return Err(AppError::SyncSetup(
                    "the repository holds other files. Use a new, empty private repository for MuDraft".into(),
                ));
            }
        }
        let snap = read_snapshot(state, &schema)?;
        let local = Counts::of(&snap.state);
        if local.populated() && !confirm {
            return Ok(SyncOutcome::NeedsConfirmation {
                reason: ConfirmReason::InitializeFromPopulated,
                local,
                remote: None,
            });
        }
        report(Progress::Merging);
        validate(&schema, &snap.state)?;
        let sync_id = new_id();
        let enc = encode(&schema, &snap.state, &sync_id)?;
        self.check_size(&enc)?;
        if remote.kind == RemoteKind::Empty {
            gh.put_first_file(README_PATH, README.as_bytes(), "Prepare MuDraft sync")
                .await?;
            remote = self
                .read_remote(&gh, &schema, &branch, None, &scratch, report)
                .await?;
            if remote.kind != RemoteKind::Uninitialised {
                return Err(AppError::Conflict(
                    "the repository changed during setup; try again".into(),
                ));
            }
        }
        let (commit, uploaded) = self
            .upload(
                &gh,
                &enc,
                &remote,
                &snap.assets,
                &artwork_dir(state),
                report,
            )
            .await?
            .ok_or_else(|| AppError::Internal("nothing to publish".into()))?;
        if gh.update_ref(&branch, &commit).await? != RefUpdate::Updated {
            return Err(AppError::Conflict(
                "the repository changed during setup; try again".into(),
            ));
        }
        report(Progress::Applying);
        let base = store_base(&schema, &snap.state)?;
        state.with_db(|db| {
            db.write(|tx| {
                tx.execute(
                    "UPDATE sync_state SET branch = ?1, sync_id = ?2, base_revision = ?3, base_state = ?4,
                         last_synced_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), last_error = NULL,
                         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                     WHERE slot = 1",
                    params![branch, sync_id, commit, base],
                )?;
                Ok(())
            })
        })?;
        let empty = State::empty(&schema);
        Ok(SyncOutcome::Synced {
            revision: commit,
            uploaded_files: uploaded,
            downloaded_files: 0,
            local_changes: 0,
            received: ChangeSummary::default(),
            sent: summarize(&schema, &empty, &snap.state),
            backup_file_name: None,
        })
    }

    /// Two-way sync with the repository.
    pub async fn sync(&self, state: &AppState, request: &SyncRequest) -> AppResult<SyncOutcome> {
        self.sync_reporting(state, request, &quiet).await
    }

    pub async fn sync_reporting(
        &self,
        state: &AppState,
        request: &SyncRequest,
        report: Report<'_>,
    ) -> AppResult<SyncOutcome> {
        let _guard = self.lock.lock().await;
        let result = self.sync_inner(state, request, report).await;
        self.record(state, &result);
        result
    }

    async fn sync_inner(
        &self,
        state: &AppState,
        request: &SyncRequest,
        report: Report<'_>,
    ) -> AppResult<SyncOutcome> {
        let cfg = Self::config(state)?;
        let token = self.token()?;
        let gh = Client::new(
            &*self.transport,
            &token,
            &self.api_base,
            &cfg.owner,
            &cfg.repo,
        );
        let schema = state.with_db(|db| db.read(Schema::load))?;
        let scratch = scratch_dir(state);
        let art_dir = artwork_dir(state);
        let base = match (&cfg.base_state, &cfg.base_revision, &cfg.sync_id) {
            (Some(text), Some(rev), Some(id)) => Some((
                decode_base(&schema, text, &scratch).map_err(base_damaged)?,
                rev.clone(),
                id.clone(),
            )),
            _ => None,
        };
        let branch = self.open_remote(&gh, &cfg).await?;
        if cfg.branch.as_deref() != Some(&branch) {
            state.with_db(|db| {
                db.write(|tx| {
                    Ok(tx.execute(
                        "UPDATE sync_state SET branch = ?1 WHERE slot = 1",
                        [&branch],
                    )?)
                })
            })?;
        }

        for _ in 0..MAX_ATTEMPTS {
            let remote = self
                .read_remote(
                    &gh,
                    &schema,
                    &branch,
                    base.as_ref().map(|(s, r, _)| (s, r.as_str())),
                    &scratch,
                    report,
                )
                .await?;
            let ready = match (remote.kind, &remote.ready) {
                (RemoteKind::Ready, Some(r)) => r,
                (RemoteKind::Empty | RemoteKind::Uninitialised, _) => {
                    return Err(AppError::SyncSetup(
                        "the repository isn't set up for MuDraft yet; initialise it from one computer first".into(),
                    ));
                }
                _ => {
                    return Err(AppError::SyncSetup(
                        "the repository holds other files, not MuDraft sync data".into(),
                    ));
                }
            };
            let head = remote.head.clone().unwrap_or_default();
            let linked = base
                .as_ref()
                .filter(|(_, _, id)| *id == ready.format.sync_id);
            if base.is_some() && linked.is_none() {
                return Err(AppError::SyncSetup(
                    "the repository now holds different sync data (it was reset or replaced). Disconnect and link again to merge with it".into(),
                ));
            }
            report(Progress::Merging);
            let snap = read_snapshot(state, &schema)?;
            let empty = State::empty(&schema);
            let merged = match linked {
                Some((b, _, _)) => merge::merge(
                    &schema,
                    &Inputs {
                        base: b,
                        local: &snap.state,
                        remote: &ready.state,
                        base_known: true,
                        unknown: &snap.unknown,
                        resolutions: &request.resolutions,
                    },
                ),
                None => {
                    let local = Counts::of(&snap.state);
                    match request.first_sync {
                        // A fresh computer, or an explicit choice, takes GitHub's library.
                        Some(FirstSync::Download) => merge::Merged {
                            state: ready.state.clone(),
                            conflicts: Vec::new(),
                        },
                        None if !local.populated() => merge::Merged {
                            state: ready.state.clone(),
                            conflicts: Vec::new(),
                        },
                        None => {
                            return Ok(SyncOutcome::NeedsConfirmation {
                                reason: ConfirmReason::MergeIntoPopulated,
                                local,
                                remote: Some(Counts::of(&ready.state)),
                            });
                        }
                        Some(FirstSync::Merge) => merge::merge(
                            &schema,
                            &Inputs {
                                base: &empty,
                                local: &snap.state,
                                remote: &ready.state,
                                base_known: false,
                                unknown: &snap.unknown,
                                resolutions: &request.resolutions,
                            },
                        ),
                    }
                }
            };
            if !merged.conflicts.is_empty() {
                let mut conflicts = merged.conflicts;
                for c in &mut conflicts {
                    c.record = describe(&schema, c, [&snap.state, &ready.state]);
                }
                return Ok(SyncOutcome::Conflicts {
                    revision: head,
                    conflicts,
                });
            }
            let merged = merged.state;
            validate(&schema, &merged)?;
            let sync_id = ready.format.sync_id.clone();
            let need_upload = merged != ready.state;
            let need_local = merged.tables != snap.state.tables;
            let base_current = linked.is_some_and(|(b, rev, _)| *rev == head && *b == merged);
            if !need_upload && !need_local && base_current {
                state.with_db(|db| {
                    db.write(|tx| {
                        Ok(tx.execute(
                            "UPDATE sync_state SET last_synced_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE slot = 1",
                            [],
                        )?)
                    })
                })?;
                return Ok(SyncOutcome::UpToDate { revision: head });
            }
            let enc = encode(&schema, &merged, &sync_id)?;
            if need_upload {
                self.check_size(&enc)?;
            }
            let backup_file_name = if need_local {
                let downloads: u64 = enc
                    .assets
                    .values()
                    .filter(|a| !snap.assets.contains_key(&a.sha256))
                    .map(|a| a.bytes)
                    .sum();
                self.ensure_space(state, downloads)?;
                report(Progress::BackingUp);
                Some(Self::backup(state)?)
            } else {
                None
            };
            let prepared = self
                .prepare_artwork(&gh, &merged, &snap.assets, &remote, &art_dir, report)
                .await?;
            let cleanup = |p: &Prepared| remove_files(&art_dir, &p.written);
            let mut uploaded = 0;
            let revision = if need_upload {
                let published = match self
                    .upload(&gh, &enc, &remote, &prepared.files, &art_dir, report)
                    .await
                {
                    Ok(p) => p,
                    Err(e) => {
                        cleanup(&prepared);
                        return Err(e);
                    }
                };
                match published {
                    None => head.clone(),
                    Some((commit, n)) => match gh.update_ref(&branch, &commit).await {
                        Ok(RefUpdate::Updated) => {
                            uploaded = n;
                            commit
                        }
                        // Another computer synced first (or the reply was lost): start over
                        // from the new head.
                        Ok(RefUpdate::NotFastForward)
                        | Err(AppError::SyncOffline(_) | AppError::Timeout(_)) => {
                            cleanup(&prepared);
                            continue;
                        }
                        Err(e) => {
                            cleanup(&prepared);
                            return Err(e);
                        }
                    },
                }
            } else {
                head.clone()
            };
            report(Progress::Applying);
            match self.apply_local(
                state,
                &schema,
                &snap.state,
                &merged,
                &prepared.files,
                &revision,
                &sync_id,
            ) {
                Ok(Some(local_changes)) => {
                    return Ok(SyncOutcome::Synced {
                        revision,
                        uploaded_files: uploaded,
                        downloaded_files: ready.downloaded + prepared.downloaded,
                        local_changes,
                        received: summarize(&schema, &snap.state, &merged),
                        sent: summarize(&schema, &ready.state, &merged),
                        backup_file_name,
                    });
                }
                // Edited while syncing; the next round merges those edits too.
                Ok(None) => cleanup(&prepared),
                Err(e) => {
                    cleanup(&prepared);
                    let backup = backup_file_name
                        .map(|b| format!(" A backup from before this sync is in the data folder's backups as “{b}”."))
                        .unwrap_or_default();
                    let github = if need_upload {
                        " GitHub already has the merged library; sync again to finish here."
                    } else {
                        " Sync again to try once more."
                    };
                    return Err(match e {
                        AppError::StorageUnavailable(m) => AppError::StorageUnavailable(format!(
                            "{m}. Your library on this computer wasn't changed.{github}{backup}"
                        )),
                        AppError::DiskFull(m) => AppError::DiskFull(format!(
                            "{m}. Your library on this computer wasn't changed.{github}{backup}"
                        )),
                        other => other,
                    });
                }
            }
        }
        Err(AppError::Conflict(
            "the library kept changing during sync (here or on GitHub); try again".into(),
        ))
    }
}
