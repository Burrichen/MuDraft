import { artworkSrc, callNative, NativeError } from "../transport/native";
import type { TagRef } from "./library";
import { notifyLibraryChanged } from "./libraryEvents";
import type { EditionCandidate, Fetched, ImportOutcome } from "./metadata";
import type { ListenView, ListeningSummary } from "./listening";
import type { RatingSummary } from "./ratings";

/** Mirrors `src-tauri/src/library/album.rs`, `editions.rs`, and `artwork/mod.rs`. */
export interface ArtworkRef {
  ownerType: "edition" | "album";
  ownerId: string;
  source: "cover_art_archive" | "local";
  canonicalFallback: boolean;
  version: string;
}

export interface ArtworkState {
  current: ArtworkRef | null;
  removed: boolean;
}

export function artworkUrl(ref: ArtworkRef | null | undefined): string | null {
  return ref ? artworkSrc(ref.ownerType, ref.ownerId, ref.version) : null;
}

export interface DateView {
  value: string | null;
  precision: "unknown" | "year" | "month" | "day";
  year: number | null;
}

export interface EditionView {
  id: string;
  name: string;
  releaseDate: DateView;
  musicbrainzReleaseId: string | null;
  trackCount: number;
  onListenList: boolean;
  inCollection: boolean;
}

export interface TrackView {
  id: string;
  position: number;
  title: string;
  lengthMs: number | null;
  credit: string | null;
  recordingId: string | null;
  rating: number | null;
  favourite: boolean;
  listened: boolean;
}

export interface AlbumDetail {
  albumId: string;
  title: string;
  credits: { artistId: string; name: string; creditedName: string | null; joinPhrase: string }[];
  originalDate: DateView;
  musicbrainzReleaseGroupId: string | null;
  genres: string[];
  tags: TagRef[];
  description: { text: string; source: "musicbrainz" | "manual" } | null;
  summary: string;
  lockedFields: string[];
  editions: EditionView[];
  editionId: string;
  discs: { number: number; tracks: TrackView[] }[];
  runtime: { knownMs: number; unknownTracks: number; trackCount: number };
  artwork: ArtworkState;
  rating: RatingSummary;
  review: string | null;
  listening: { summary: ListeningSummary; listens: ListenView[] };
}

export interface AlbumEdit {
  title?: string;
  originalDate?: string;
  description?: string;
}

export interface TrackRef {
  trackId: string;
  title: string;
  disc: number;
  position: number;
}

export interface SwitchPreview {
  from: { id: string; name: string };
  to: { id: string; name: string };
  carry: {
    from: TrackRef;
    to: TrackRef;
    rating: number | null;
    favourite: boolean;
    targetHasData: boolean;
  }[];
  stay: { track: TrackRef; rating: number | null; favourite: boolean; listened: boolean }[];
  listensKept: number;
}

export type EditionChoice = EditionCandidate & { inLibraryAs: string | null };

/** "4:44", "1:02:03". Unknown lengths are never shown as zero. */
export function formatDuration(ms: number): string {
  const secs = Math.floor(ms / 1000);
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = String(secs % 60).padStart(2, "0");
  return h > 0 ? `${String(h)}:${String(m).padStart(2, "0")}:${s}` : `${String(m)}:${s}`;
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null;
}

/** Light shape check of a native response before it is typed. */
function verified(
  raw: unknown,
  ok: (r: Record<string, unknown>) => boolean,
  what: string,
): unknown {
  if (!isRecord(raw) || !ok(raw))
    throw new NativeError("invalid_response", `${what} had an unexpected shape`);
  return raw;
}

async function withCancel<T>(
  run: (requestId: string) => Promise<T>,
  signal?: AbortSignal,
): Promise<T> {
  if (signal?.aborted) throw new NativeError("cancelled", "Request cancelled");
  const requestId = crypto.randomUUID();
  const onAbort = () => {
    void callNative("metadata_cancel", { requestId }).catch(() => undefined);
  };
  signal?.addEventListener("abort", onAbort, { once: true });
  try {
    return await run(requestId);
  } finally {
    signal?.removeEventListener("abort", onAbort);
  }
}

export async function albumDetail(
  albumId: string,
  editionId?: string | null,
): Promise<AlbumDetail> {
  const raw = await callNative("album_detail", { albumId, editionId: editionId ?? null });
  return verified(
    raw,
    (r) => typeof r.albumId === "string" && Array.isArray(r.discs),
    "Album",
  ) as AlbumDetail;
}

export async function editAlbum(albumId: string, edit: AlbumEdit): Promise<void> {
  await callNative("album_edit", { albumId, edit });
  notifyLibraryChanged();
}

export async function unlockField(albumId: string, field: string): Promise<void> {
  await callNative("album_unlock", { albumId, field });
  notifyLibraryChanged();
}

export async function renameEdition(editionId: string, name: string): Promise<void> {
  await callNative("edition_rename", { editionId, name });
  notifyLibraryChanged();
}

export async function refreshAlbum(
  albumId: string,
  editionId: string,
  signal?: AbortSignal,
): Promise<ImportOutcome> {
  const raw = await withCancel(
    (requestId) => callNative("album_refresh", { requestId, albumId, editionId }),
    signal,
  );
  notifyLibraryChanged();
  return verified(raw, (r) => typeof r.albumId === "string", "Refresh result") as ImportOutcome;
}

export async function editionCandidates(
  albumId: string,
  signal?: AbortSignal,
): Promise<Fetched<EditionChoice[]>> {
  const raw = await withCancel(
    (requestId) => callNative("edition_candidates", { requestId, albumId }),
    signal,
  );
  return verified(raw, (r) => Array.isArray(r.value), "Edition list") as Fetched<EditionChoice[]>;
}

export async function addEdition(
  albumId: string,
  releaseId: string,
  editionName: string,
): Promise<ImportOutcome> {
  const raw = await withCancel((requestId) =>
    callNative("edition_add", { requestId, albumId, releaseId, editionName }),
  );
  notifyLibraryChanged();
  return verified(raw, (r) => typeof r.editionId === "string", "New edition") as ImportOutcome;
}

export async function previewEditionSwitch(
  albumId: string,
  editionId: string,
): Promise<SwitchPreview> {
  const raw = await callNative("edition_switch_preview", { albumId, editionId });
  return verified(raw, (r) => Array.isArray(r.carry), "Edition preview") as SwitchPreview;
}

export async function switchEdition(
  albumId: string,
  editionId: string,
  copyRatings: boolean,
): Promise<number> {
  const raw = await callNative("edition_switch", { albumId, editionId, copyRatings });
  notifyLibraryChanged();
  return (
    verified(raw, (r) => typeof r.copied === "number", "Edition switch") as { copied: number }
  ).copied;
}

export interface ArtworkPreference {
  allowed: boolean | null;
  cache: { files: number; bytes: number };
}

export async function artworkPreference(): Promise<ArtworkPreference> {
  const raw = await callNative("artwork_preference");
  return verified(
    raw,
    (r) => "allowed" in r && isRecord(r.cache),
    "Artwork preference",
  ) as ArtworkPreference;
}

export async function setArtworkPreference(allowed: boolean): Promise<void> {
  await callNative("artwork_set_preference", { allowed });
  notifyLibraryChanged();
}

function artworkState(raw: unknown): ArtworkState {
  return verified(raw, (r) => typeof r.removed === "boolean", "Artwork") as ArtworkState;
}

export async function fetchArtwork(editionId: string, signal?: AbortSignal): Promise<ArtworkState> {
  return artworkState(
    await withCancel((requestId) => callNative("artwork_fetch", { requestId, editionId }), signal),
  );
}

/** Download missing artwork for several editions in turn (bounded in Rust). */
export async function fetchArtworkMany(
  editionIds: readonly string[],
  signal?: AbortSignal,
): Promise<number> {
  const raw = await withCancel(
    (requestId) => callNative("artwork_fetch_many", { requestId, editionIds }),
    signal,
  );
  return typeof raw === "number" ? raw : 0;
}

/** Opens a native image picker; null when cancelled. */
export async function replaceArtwork(editionId: string): Promise<ArtworkState | null> {
  const raw = await callNative("artwork_replace", { editionId });
  if (raw === null) return null;
  notifyLibraryChanged();
  return artworkState(raw);
}

export async function removeArtwork(editionId: string): Promise<ArtworkState> {
  const state = artworkState(await callNative("artwork_remove", { editionId }));
  notifyLibraryChanged();
  return state;
}

export async function restoreArtwork(editionId: string): Promise<ArtworkState> {
  const state = artworkState(await callNative("artwork_restore", { editionId }));
  notifyLibraryChanged();
  return state;
}

export async function clearArtworkCache(): Promise<{ files: number; bytes: number }> {
  const raw = await callNative("artwork_clear_cache");
  notifyLibraryChanged();
  return verified(raw, (r) => typeof r.files === "number", "Cache clear") as {
    files: number;
    bytes: number;
  };
}
