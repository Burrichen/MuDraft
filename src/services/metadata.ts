import { callNative, NativeError, type NativeCommand } from "../transport/native";
import { notifyLibraryChanged } from "./libraryEvents";

/** Mirrors the Rust types in `src-tauri/src/metadata/mod.rs` (camelCase over IPC). */
export interface SearchQuery {
  title: string;
  artist?: string;
  year?: number;
}

export type FetchSource = "network" | "cache" | "stale_cache";

export interface Fetched<T> {
  value: T;
  fetchedAt: string;
  source: FetchSource;
}

export interface CreditPart {
  artistId: string;
  artistName: string;
  sortName: string | null;
  disambiguation: string | null;
  creditedName: string | null;
  joinPhrase: string;
}

export interface ProviderDate {
  value: string | null;
  precision: "unknown" | "year" | "month" | "day";
  year: number | null;
}

export interface ReleaseGroupCandidate {
  id: string;
  title: string;
  disambiguation: string | null;
  primaryType: string | null;
  secondaryTypes: string[];
  originalDate: ProviderDate;
  artistCredit: CreditPart[];
  score: number;
  exactTitle: boolean;
  yearMatches: boolean | null;
}

export interface SearchPage {
  candidates: ReleaseGroupCandidate[];
  total: number;
  offset: number;
}

export interface EditionCandidate {
  id: string;
  title: string;
  disambiguation: string | null;
  date: ProviderDate;
  country: string | null;
  status: string | null;
  formats: string[];
  trackCount: number;
}

export interface EditionList {
  editions: EditionCandidate[];
  total: number;
  truncated: boolean;
}

export interface ImportOutcome {
  albumId: string;
  editionId: string;
  albumCreated: boolean;
  editionCreated: boolean;
  lockedFieldsKept: string[];
  retainedTracks: number;
  genres: string[];
}

export interface ManualAlbum {
  title: string;
  artistName?: string;
  year?: number;
}

export interface ManualAlbumCreated {
  albumId: string;
  editionId: string;
  artistId: string;
}

export function creditText(parts: readonly CreditPart[]): string {
  return parts.map((p) => `${p.creditedName ?? p.artistName}${p.joinPhrase}`).join("");
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null;
}

function isFetched(v: unknown): v is Fetched<unknown> {
  return (
    isRecord(v) &&
    typeof v.fetchedAt === "string" &&
    (v.source === "network" || v.source === "cache" || v.source === "stale_cache") &&
    "value" in v
  );
}

function invalid(what: string): NativeError {
  return new NativeError("invalid_response", `${what} had an unexpected shape`);
}

/**
 * Run a cancellable native request. Each call gets its own request ID; aborting the
 * signal asks Rust to cancel it (dropping queued or in-flight provider requests).
 */
async function cancellable(
  command: NativeCommand,
  args: Record<string, unknown>,
  signal?: AbortSignal,
): Promise<unknown> {
  if (signal?.aborted) throw new NativeError("cancelled", "Request cancelled");
  const requestId = crypto.randomUUID();
  const onAbort = () => {
    void callNative("metadata_cancel", { requestId }).catch(() => {
      // The request may already have finished; nothing to cancel.
    });
  };
  signal?.addEventListener("abort", onAbort, { once: true });
  try {
    return await callNative(command, { ...args, requestId });
  } finally {
    signal?.removeEventListener("abort", onAbort);
  }
}

export async function searchAlbums(
  query: SearchQuery,
  options: { signal?: AbortSignal; offset?: number } = {},
): Promise<Fetched<SearchPage>> {
  const raw = await cancellable(
    "metadata_search",
    {
      query: { title: query.title, artist: query.artist ?? null, year: query.year ?? null },
      offset: options.offset ?? 0,
    },
    options.signal,
  );
  if (!isFetched(raw) || !isRecord(raw.value) || !Array.isArray(raw.value.candidates)) {
    throw invalid("Search results");
  }
  return raw as Fetched<SearchPage>;
}

export async function loadEditions(
  releaseGroupId: string,
  options: { signal?: AbortSignal } = {},
): Promise<Fetched<EditionList>> {
  const raw = await cancellable("metadata_editions", { releaseGroupId }, options.signal);
  if (!isFetched(raw) || !isRecord(raw.value) || !Array.isArray(raw.value.editions)) {
    throw invalid("Edition list");
  }
  return raw as Fetched<EditionList>;
}

export async function importRelease(
  choice: {
    releaseGroupId: string;
    releaseId: string;
    editionName?: string;
    addToListenList: boolean;
    addToCollection?: boolean;
  },
  options: { signal?: AbortSignal } = {},
): Promise<ImportOutcome> {
  const raw = await cancellable(
    "metadata_import",
    {
      releaseGroupId: choice.releaseGroupId,
      releaseId: choice.releaseId,
      editionName: choice.editionName ?? null,
      destination: {
        listenList: choice.addToListenList,
        collection: choice.addToCollection ?? false,
      },
    },
    options.signal,
  );
  if (!isRecord(raw) || typeof raw.albumId !== "string" || typeof raw.editionId !== "string") {
    throw invalid("Import result");
  }
  notifyLibraryChanged();
  return raw as unknown as ImportOutcome;
}

export async function addManualAlbum(
  album: ManualAlbum,
  addToListenList: boolean,
  addToCollection = false,
): Promise<ManualAlbumCreated> {
  const raw = await callNative("library_add_manual_album", {
    requestId: crypto.randomUUID(),
    album: {
      title: album.title,
      artistName: album.artistName ?? null,
      artistId: null,
      year: album.year ?? null,
      editionName: null,
    },
    destination: { listenList: addToListenList, collection: addToCollection },
  });
  if (!isRecord(raw) || typeof raw.albumId !== "string") throw invalid("Manual album result");
  notifyLibraryChanged();
  return raw as unknown as ManualAlbumCreated;
}

export async function setAlbumGenres(
  albumId: string,
  genres: readonly string[],
): Promise<string[]> {
  const raw = await callNative("library_set_album_genres", { albumId, genres });
  notifyLibraryChanged();
  if (!Array.isArray(raw) || !raw.every((g) => typeof g === "string")) throw invalid("Genres");
  return raw;
}

export async function genreVocabulary(): Promise<string[]> {
  const raw = await callNative("genre_vocabulary");
  if (!Array.isArray(raw) || !raw.every((g) => typeof g === "string"))
    throw invalid("Genre vocabulary");
  return raw;
}

export interface MetadataCacheStats {
  entries: number;
  bytes: number;
  expired: number;
}

/** Saved MusicBrainz responses (used for offline lookups). */
export async function metadataCacheStats(): Promise<MetadataCacheStats> {
  const raw = await callNative("metadata_cache_stats");
  if (
    typeof raw !== "object" ||
    raw === null ||
    typeof (raw as MetadataCacheStats).entries !== "number"
  )
    throw new NativeError("invalid_response", "Cache details had an unexpected shape");
  return raw as MetadataCacheStats;
}

/** Forget saved MusicBrainz responses; your library, ratings, and listens are kept. */
export async function clearMetadataCache(): Promise<number> {
  const raw = await callNative("metadata_cache_clear");
  return typeof raw === "number" ? raw : 0;
}
