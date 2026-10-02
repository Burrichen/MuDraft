import { callNative, NativeError } from "../transport/native";

/** Mirrors `src-tauri/src/profile`. Files are chosen with native dialogs in Rust. */
export interface ProfileCounts {
  artists: number;
  albums: number;
  editions: number;
  tracks: number;
  listenList: number;
  collection: number;
  listens: number;
  ratedTracks: number;
  albumReviews: number;
  tags: number;
  selectionAttempts: number;
  catalogueEntries: number;
  artworkFiles: number;
}

export interface ExportOutcome {
  fileName: string;
  bytes: number;
  artwork: "included" | "omitted";
  counts: ProfileCounts;
  artworkMissing: number;
}

export interface ProfilePreview {
  formatVersion: number;
  schemaVersion: number;
  currentSchemaVersion: number;
  appVersion: string;
  createdAt: string;
  artwork: "included" | "omitted";
  counts: ProfileCounts;
  archiveBytes: number;
  sha256: string;
}

export interface RestoreOutcome {
  backupFileName: string;
  preview: ProfilePreview;
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null;
}

function invalid(what: string): NativeError {
  return new NativeError("invalid_response", `${what} had an unexpected shape`);
}

/** Resolves to null when the save dialog is cancelled. */
export async function exportProfile(includeArtwork: boolean): Promise<ExportOutcome | null> {
  const raw = await callNative("profile_export", { includeArtwork });
  if (raw === null) return null;
  if (!isRecord(raw) || typeof raw.fileName !== "string") throw invalid("Export result");
  return raw as unknown as ExportOutcome;
}

/** Choose and fully validate an archive. Nothing changes until `confirmRestore`. */
export async function chooseProfileToRestore(): Promise<ProfilePreview | null> {
  const raw = await callNative("profile_import_choose");
  if (raw === null) return null;
  if (!isRecord(raw) || typeof raw.sha256 !== "string" || !isRecord(raw.counts))
    throw invalid("Profile preview");
  return raw as unknown as ProfilePreview;
}

/** Back up the current profile, then replace it with the previewed one. */
export async function confirmRestore(sha256: string): Promise<RestoreOutcome> {
  const raw = await callNative("profile_import_confirm", { sha256 });
  if (!isRecord(raw) || typeof raw.backupFileName !== "string") throw invalid("Restore result");
  return raw as unknown as RestoreOutcome;
}

export async function cancelRestore(): Promise<void> {
  await callNative("profile_import_cancel");
}
