import { callNative, NativeError } from "../transport/native";
import { notifyLibraryChanged } from "./libraryEvents";
import type { EditionCandidate, ReleaseGroupCandidate } from "./metadata";

/** Mirrors `src-tauri/src/csv_import/` types (camelCase over IPC). */
export const TAG_DELIMITER = ";";

export type Field =
  "album" | "artist" | "year" | "edition" | "tags" | "releaseGroupId" | "releaseId";
export type ColumnMapping = Record<Field, number | null>;

export const FIELDS: readonly { field: Field; label: string; required: boolean }[] = [
  { field: "album", label: "Album", required: true },
  { field: "artist", label: "Artist", required: true },
  { field: "year", label: "Year", required: false },
  { field: "edition", label: "Edition", required: false },
  { field: "tags", label: "Tags", required: false },
  { field: "releaseGroupId", label: "MusicBrainz release group ID", required: false },
  { field: "releaseId", label: "MusicBrainz release ID", required: false },
];

export interface Counts {
  rows: number;
  ready: number;
  errors: number;
  duplicates: number;
  existing: number;
  matched: number;
  manual: number;
  skipped: number;
  needsAttention: number;
  withCandidates: number;
  lookupPending: number;
}

export interface RowOutcome {
  rowNumber: number;
  outcome: "created" | "added_edition" | "reused" | "skipped" | "error";
  albumId: string | null;
  detail: string | null;
}

export interface CommitReport {
  albumsCreated: number;
  editionsCreated: number;
  reusedExisting: number;
  addedToListenList: number;
  alreadyOnListenList: number;
  skipped: number;
  errors: number;
  tagsCreated: string[];
  tagsSkipped: string[];
  tagLinksAdded: number;
  rows: RowOutcome[];
}

export interface SessionSummary {
  sessionId: string;
  fileName: string;
  headers: string[];
  mapping: ColumnMapping | null;
  suggestedMapping: ColumnMapping;
  sample: string[][];
  counts: Counts;
  unknownTags: { name: string; rows: number }[];
  status: "open" | "committed";
  report: CommitReport | null;
}

export interface RowFields {
  album: string;
  artist: string;
  year: number | null;
  edition: string | null;
  tags: string[];
  releaseGroupId: string | null;
  releaseId: string | null;
}

export type ExistingReason =
  "previous_import" | "release_id" | "release_group_edition" | "same_name";

export type Decision =
  | {
      kind: "match";
      releaseGroupId: string | null;
      releaseId: string | null;
      editionName: string | null;
      chosenBy: "csv_ids" | "user";
    }
  | { kind: "manual" }
  | { kind: "skip" }
  | { kind: "use_existing"; albumId: string; editionId: string; reason: ExistingReason };

export type Candidates =
  | { kind: "groups"; candidates: ReleaseGroupCandidate[] }
  | { kind: "editions"; editions: EditionCandidate[] }
  | { kind: "failed"; message: string };

export type Readiness = "ready" | "needs_lookup" | "needs_edition" | "error" | "skipped";

export interface StagedRow {
  rowNumber: number;
  raw: string[];
  fields: RowFields | null;
  issues: { field: string; message: string }[];
  duplicateOf: number | null;
  decision: Decision;
  enrichment: string;
  candidates: Candidates | null;
  hasDetails: boolean;
  readiness: Readiness;
  existing: { albumTitle: string; editionName: string } | null;
}

export type RowFilter =
  "all" | "attention" | "errors" | "duplicates" | "existing" | "candidates" | "skipped";

export interface EnrichProgress {
  processed: number;
  remaining: number;
  stopped: { code: string; message: string } | null;
}

export type DecisionInput =
  | { kind: "match"; releaseGroupId: string; releaseId: string; editionName: string | null }
  | { kind: "manual" }
  | { kind: "skip" }
  | { kind: "use_existing" };

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null;
}

function summary(raw: unknown): SessionSummary {
  if (
    !isRecord(raw) ||
    typeof raw.sessionId !== "string" ||
    !Array.isArray(raw.headers) ||
    !isRecord(raw.counts)
  ) {
    throw new NativeError("invalid_response", "Import summary had an unexpected shape");
  }
  return raw as unknown as SessionSummary;
}

export async function saveTemplate(): Promise<string | null> {
  const raw = await callNative("csv_import_save_template");
  return typeof raw === "string" ? raw : null;
}

/** Opens the native file picker. Null when the user cancels. */
export async function openCsv(): Promise<SessionSummary | null> {
  const raw = await callNative("csv_import_open");
  return raw === null ? null : summary(raw);
}

export async function latestImport(): Promise<SessionSummary | null> {
  const raw = await callNative("csv_import_latest");
  return raw === null ? null : summary(raw);
}

export async function applyMapping(
  sessionId: string,
  mapping: ColumnMapping,
): Promise<SessionSummary> {
  return summary(await callNative("csv_import_apply_mapping", { sessionId, mapping }));
}

export async function importSummary(sessionId: string): Promise<SessionSummary> {
  return summary(await callNative("csv_import_summary", { sessionId }));
}

export async function importRows(
  sessionId: string,
  filter: RowFilter,
  offset: number,
  limit: number,
): Promise<{ rows: StagedRow[]; total: number }> {
  const raw = await callNative("csv_import_rows", { sessionId, filter, offset, limit });
  if (!isRecord(raw) || !Array.isArray(raw.rows) || typeof raw.total !== "number") {
    throw new NativeError("invalid_response", "Import rows had an unexpected shape");
  }
  return raw as unknown as { rows: StagedRow[]; total: number };
}

/** One bounded batch of online lookups. Aborting cancels it in Rust; progress is kept. */
export async function enrichBatch(
  sessionId: string,
  maxRows: number,
  signal?: AbortSignal,
): Promise<EnrichProgress> {
  const requestId = crypto.randomUUID();
  const onAbort = () => {
    void callNative("metadata_cancel", { requestId }).catch(() => undefined);
  };
  signal?.addEventListener("abort", onAbort, { once: true });
  try {
    const raw = await callNative("csv_import_enrich", { sessionId, requestId, maxRows });
    if (!isRecord(raw) || typeof raw.remaining !== "number") {
      throw new NativeError("invalid_response", "Lookup progress had an unexpected shape");
    }
    return raw as unknown as EnrichProgress;
  } finally {
    signal?.removeEventListener("abort", onAbort);
  }
}

export async function decideRow(
  sessionId: string,
  rowNumber: number,
  decision: DecisionInput,
): Promise<StagedRow> {
  const raw = await callNative("csv_import_decide", {
    sessionId,
    requestId: crypto.randomUUID(),
    rowNumber,
    decision,
  });
  if (!isRecord(raw) || typeof raw.rowNumber !== "number") {
    throw new NativeError("invalid_response", "Import row had an unexpected shape");
  }
  return raw as unknown as StagedRow;
}

export async function commitImport(
  sessionId: string,
  tags: { create: string[]; skip: string[] },
): Promise<CommitReport> {
  const raw = await callNative("csv_import_commit", { sessionId, tags });
  notifyLibraryChanged();
  if (!isRecord(raw) || !Array.isArray(raw.rows)) {
    throw new NativeError("invalid_response", "Import report had an unexpected shape");
  }
  return raw as unknown as CommitReport;
}

export async function discardImport(sessionId: string): Promise<void> {
  await callNative("csv_import_discard", { sessionId });
}
