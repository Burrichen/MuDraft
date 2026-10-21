import { callNative, listenNative, NativeError } from "../transport/native";
import { notifyLibraryChanged } from "./libraryEvents";

/**
 * Mirrors `src-tauri/src/sync`. The GitHub token goes to Rust once (`saveSyncToken`) and is
 * never returned: status only says whether one is stored.
 */

/** Records that differ, grouped for people. */
export interface ChangeSummary {
  albums: number;
  listens: number;
  ratings: number;
  reviews: number;
  tags: number;
  artwork: number;
  other: number;
}

export interface SyncCounts {
  artists: number;
  albums: number;
  editions: number;
  listens: number;
  ratedTracks: number;
  reviews: number;
  tags: number;
  listenList: number;
  collection: number;
  artwork: number;
}

export interface SyncErrorInfo {
  code: string;
  message: string;
}

export interface SyncStatus {
  configured: boolean;
  owner: string | null;
  repo: string | null;
  branch: string | null;
  hasToken: boolean;
  linked: boolean;
  revision: string | null;
  lastSyncedAt: string | null;
  pendingLocalChanges: boolean | null;
  pending: ChangeSummary | null;
  unresolvedConflicts: number;
  lastError: SyncErrorInfo | null;
}

export type RemoteKind = "empty" | "uninitialised" | "unrelated" | "ready";

export interface RemoteCheck {
  branch: string;
  kind: RemoteKind;
  /** Ready data belongs to the sync this computer is linked to (null if not linked). */
  sameSync: boolean | null;
  remote: SyncCounts | null;
  local: SyncCounts;
  remoteChanges: ChangeSummary | null;
  revision: string | null;
}

export type Side = "local" | "remote";

export type ConflictKind =
  "field" | "deleteEdit" | "collection" | "orphan" | "duplicate" | "deletedElsewhere";

export interface SyncConflict {
  id: string;
  kind: ConflictKind;
  /** The record in words, e.g. "Review of OK Computer (Standard)". */
  record: string;
  table: string;
  key: unknown;
  columns: string[];
  base: unknown;
  local: unknown;
  remote: unknown;
}

export interface Resolution {
  id: string;
  choice: Side;
}

export type FirstSync = "merge" | "download";

export type SyncOutcome =
  | { status: "upToDate"; revision: string }
  | {
      status: "synced";
      revision: string;
      uploadedFiles: number;
      downloadedFiles: number;
      localChanges: number;
      received: ChangeSummary;
      sent: ChangeSummary;
      backupFileName: string | null;
    }
  | { status: "conflicts"; revision: string; conflicts: SyncConflict[] }
  | {
      status: "needsConfirmation";
      reason: "initializeFromPopulated" | "mergeIntoPopulated";
      local: SyncCounts;
      remote: SyncCounts | null;
    };

export type SyncProgress =
  | { phase: "reading" }
  | { phase: "downloading"; done: number; total: number }
  | { phase: "merging" }
  | { phase: "backingUp" }
  | { phase: "uploading"; done: number; total: number }
  | { phase: "publishing" }
  | { phase: "applying" };

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null;
}

function invalid(what: string): NativeError {
  return new NativeError("invalid_response", `${what} had an unexpected shape`);
}

function status(raw: unknown): SyncStatus {
  if (
    !isRecord(raw) ||
    typeof raw.configured !== "boolean" ||
    typeof raw.hasToken !== "boolean" ||
    typeof raw.unresolvedConflicts !== "number"
  )
    throw invalid("Sync status");
  return raw as unknown as SyncStatus;
}

const OUTCOMES = new Set(["upToDate", "synced", "conflicts", "needsConfirmation"]);

function outcome(raw: unknown): SyncOutcome {
  if (!isRecord(raw) || typeof raw.status !== "string" || !OUTCOMES.has(raw.status))
    throw invalid("Sync result");
  if (raw.status === "conflicts" && !Array.isArray(raw.conflicts)) throw invalid("Sync result");
  const out = raw as unknown as SyncOutcome;
  if (out.status === "synced" && out.localChanges > 0) notifyLibraryChanged();
  return out;
}

export async function getSyncStatus(): Promise<SyncStatus> {
  return status(await callNative("sync_status"));
}

/** Choose the repository. Switching to another one forgets this computer's link. */
export async function configureSync(owner: string, repo: string): Promise<SyncStatus> {
  return status(await callNative("sync_configure", { owner, repo }));
}

/** Forget the repository link on this computer. The library and the token stay. */
export async function disconnectSync(): Promise<SyncStatus> {
  return status(await callNative("sync_disconnect"));
}

/** Store the token in the OS credential store. It can't be read back. */
export async function saveSyncToken(token: string): Promise<SyncStatus> {
  return status(await callNative("sync_set_token", { token }));
}

export async function removeSyncToken(): Promise<SyncStatus> {
  return status(await callNative("sync_remove_token"));
}

/** Read-only look at the repository: what's there and what changed. */
export async function checkSync(): Promise<RemoteCheck> {
  const raw = await callNative("sync_check");
  if (!isRecord(raw) || typeof raw.kind !== "string" || !isRecord(raw.local))
    throw invalid("Connection check");
  return raw as unknown as RemoteCheck;
}

/** Publish this computer's library to an empty repository. */
export async function initializeSync(confirm: boolean): Promise<SyncOutcome> {
  return outcome(await callNative("sync_initialize", { confirm }));
}

export async function syncNow(
  options: { firstSync?: FirstSync; resolutions?: Resolution[] } = {},
): Promise<SyncOutcome> {
  return outcome(
    await callNative("sync_now", {
      request: {
        ...(options.firstSync ? { firstSync: options.firstSync } : {}),
        resolutions: options.resolutions ?? [],
      },
    }),
  );
}

const PHASES = new Set([
  "reading",
  "downloading",
  "merging",
  "backingUp",
  "uploading",
  "publishing",
  "applying",
]);

/** Progress of a running sync; resolves to an unsubscribe function. */
export function onSyncProgress(handler: (p: SyncProgress) => void): Promise<() => void> {
  return listenNative("sync-progress", (payload) => {
    if (isRecord(payload) && typeof payload.phase === "string" && PHASES.has(payload.phase))
      handler(payload as unknown as SyncProgress);
  });
}
