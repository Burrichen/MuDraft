import { callNative, NativeError } from "../transport/native";
import { notifyLibraryChanged } from "./libraryEvents";

/** Mirrors `src-tauri/src/library/discography.rs` and `commands/discography.rs`. */
export interface Scope {
  primaryTypes: string[];
  secondaryTypes: string[];
}

export interface Coverage {
  kind: "online" | "local_only";
  status: "not_fetched" | "partial" | "complete" | "local_only";
  musicbrainzArtistId: string | null;
  providerTotal: number | null;
  fetchedEntries: number;
  nextOffset: number;
  startedAt: string | null;
  pageFetchedAt: string | null;
  completedAt: string | null;
  lastError: string | null;
  manualEntries: number;
  tracklistsKnown: number;
  tracklistsNeeded: number;
  note: string;
}

export interface Reference {
  kind: "your_edition" | "representative" | "none";
  editionId: string | null;
  musicbrainzReleaseId: string | null;
  name: string | null;
  date: string | null;
  country: string | null;
  formats: string[];
  /** null = tracklist unknown, never zero. */
  trackCount: number | null;
  note: string;
  lastError: string | null;
}

export interface EditionScore {
  editionId: string;
  name: string;
  /** Half-stars 0–10; zero is a rating. */
  rating: number;
  source: "explicit" | "calculated" | "unrated";
}

export interface CatalogueEntry {
  id: string | null;
  credits: { name: string; joinPhrase: string; artistId: string | null }[];
  /** Mean of the rated editions' effective ratings, in stars. */
  score: { stars: number; editions: EditionScore[] } | null;
  rank: number | null;
  source: "musicbrainz" | "manual" | "library";
  releaseGroupId: string | null;
  albumId: string | null;
  title: string;
  disambiguation: string | null;
  credit: string;
  collaboration: boolean;
  originalYear: number | null;
  primaryType: string | null;
  secondaryTypes: string[];
  typeLabel: string;
  excluded: boolean;
  counted: boolean;
  onListenList: boolean;
  inCollection: boolean;
  listened: boolean;
  reference: Reference;
  tracks: { total: number; listened: number } | null;
  fetchedAt: string | null;
}

export interface ArtistCatalogue {
  artistId: string;
  artistName: string;
  sortName: string | null;
  disambiguation: string | null;
  musicbrainzId: string | null;
  average: { stars: number | null; ratedAlbums: number };
  coverage: Coverage;
  scope: Scope;
  denominator: string;
  representativeRule: string;
  entries: CatalogueEntry[];
  types: { label: string; count: number; counted: boolean }[];
  progress: {
    albumsCounted: number;
    albumsListened: number;
    tracksTotal: number;
    tracksListened: number;
    albumsWithoutTracklist: number;
  };
}

export interface FetchRun {
  pagesFetched: number;
  complete: boolean;
  usedOfflineCache: boolean;
}

export interface TracklistRun {
  loaded: number;
  failed: { releaseGroupId: string; code: string; message: string }[];
  remaining: number;
  stopped: string | null;
  usedOfflineCache: boolean;
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null;
}

function invalid(what: string): NativeError {
  return new NativeError("invalid_response", `${what} had an unexpected shape`);
}

export async function artistCatalogue(artistId: string): Promise<ArtistCatalogue> {
  const raw = await callNative("artist_catalogue", { artistId });
  if (!isRecord(raw) || !Array.isArray(raw.entries) || !isRecord(raw.coverage))
    throw invalid("Artist catalogue");
  return raw as unknown as ArtistCatalogue;
}

/** Fetch or resume the online catalogue. Cancel with `cancelCatalogueRequest(requestId)`. */
export async function fetchArtistCatalogue(requestId: string, artistId: string): Promise<FetchRun> {
  const raw = await callNative("artist_catalogue_fetch", { requestId, artistId });
  if (!isRecord(raw) || typeof raw.complete !== "boolean") throw invalid("Catalogue fetch");
  return raw as unknown as FetchRun;
}

/** Load missing reference tracklists (on request only); resumable and cancellable. */
export async function loadArtistTracklists(
  requestId: string,
  artistId: string,
): Promise<TracklistRun> {
  const raw = await callNative("artist_tracklists_load", { requestId, artistId });
  if (!isRecord(raw) || !Array.isArray(raw.failed)) throw invalid("Tracklist run");
  return raw as unknown as TracklistRun;
}

export async function setCatalogueScope(artistId: string, scope: Scope): Promise<Scope> {
  const raw = await callNative("artist_catalogue_scope_set", { artistId, scope });
  if (!isRecord(raw) || !Array.isArray(raw.primaryTypes)) throw invalid("Catalogue scope");
  return raw as unknown as Scope;
}

export async function addCatalogueEntry(
  artistId: string,
  entry: {
    title: string;
    year: number | null;
    primaryType: string | null;
    secondaryTypes?: string[];
  },
): Promise<string> {
  const raw = await callNative("artist_catalogue_add_manual", {
    artistId,
    entry: { ...entry, secondaryTypes: entry.secondaryTypes ?? [] },
  });
  if (typeof raw !== "string") throw invalid("Catalogue entry");
  return raw;
}

export async function removeCatalogueEntry(entryId: string): Promise<void> {
  await callNative("artist_catalogue_remove_manual", { entryId });
}

export async function setCatalogueEntryExcluded(entryId: string, excluded: boolean): Promise<void> {
  await callNative("artist_catalogue_exclude", { entryId, excluded });
}

/** Stops a running fetch or tracklist run; what was already stored is kept. */
export async function cancelCatalogueRequest(requestId: string): Promise<boolean> {
  return (await callNative("metadata_cancel", { requestId })) === true;
}

/** Explicitly add a discovered album (its existing or reference edition) to the Listen List. */
export async function addDiscoveredToListenList(
  requestId: string,
  entryId: string,
): Promise<{ albumId: string; editionId: string; imported: boolean }> {
  const raw = await callNative("artist_catalogue_add_to_listen_list", { requestId, entryId });
  if (!isRecord(raw) || typeof raw.albumId !== "string") throw invalid("Listen List add");
  notifyLibraryChanged();
  return raw as unknown as { albumId: string; editionId: string; imported: boolean };
}
