import { callNative, NativeError } from "../transport/native";

/** Mirrors `src-tauri/src/library/stats.rs`. Computed from stored evidence on each call. */
export interface Count {
  key: string;
  label: string;
  count: number;
}

export interface Waiting {
  albumId: string;
  editionId: string;
  title: string;
  addedAt: string;
  waitingDays: number;
}

export interface ListenListStats {
  entries: number;
  distinctAlbums: number;
  distinctArtists: number;
  artists: Count[];
  years: Count[];
  decades: Count[];
  unknownYears: number;
  /** Genres overlap: an album counts once in each of its genres. */
  genres: Count[];
  albumsWithoutGenre: number;
  tags: Count[];
  listenAsap: number;
  recentlyAdded: Waiting[];
  oldestWaiting: Waiting[];
  waiting: { meanDays: number; medianDays: number; longestDays: number } | null;
  runtime: { knownMs: number; editionsKnown: number; editionsUnknown: number };
}

export interface GroupScore {
  key: string;
  label: string;
  meanStars: number;
  /** Sample size, shown but never used to penalize. */
  albums: number;
}

export interface Best {
  /** Every group tied for the highest mean. */
  winners: GroupScore[];
  groups: GroupScore[];
  unplacedAlbums: number;
}

export interface CollectionStats {
  albums: number;
  distinctListenedAlbums: number;
  listens: {
    dated: number;
    undated: number;
    historical: number;
    first: number;
    relisten: number;
    unspecified: number;
    fullAlbum: number;
    tracksOnly: number;
  };
  byMonth: Count[];
  byYear: Count[];
  ratings: {
    ratedAlbums: number;
    unratedAlbums: number;
    averageStars: number | null;
    distribution: { stars: number; albums: number }[];
  };
  bestYear: Best;
  bestDecade: Best;
  bestGenre: Best;
  bestArtist: Best;
  favouriteTracks: {
    trackId: string;
    title: string;
    albumId: string;
    albumTitle: string;
    editionName: string;
  }[];
  runtime: {
    sessions: number;
    knownMs: number;
    tracksCounted: number;
    tracksWithoutLength: number;
    listensWithoutTracklist: number;
    /** False when any duration is unknown: label the estimate as incomplete. */
    complete: boolean;
  };
}

export interface MethodStats {
  source: "completely_random" | "weighted_random" | "guided" | "manual" | "all";
  shown: number;
  skipped: number;
  completed: number;
  pending: number;
}

export interface Stats {
  listenList: ListenListStats;
  collection: CollectionStats;
  selection: MethodStats[];
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null;
}

export async function statsOverview(): Promise<Stats> {
  const raw = await callNative("stats_overview");
  if (
    !isRecord(raw) ||
    !isRecord(raw.listenList) ||
    !isRecord(raw.collection) ||
    !Array.isArray(raw.selection)
  ) {
    throw new NativeError("invalid_response", "Stats had an unexpected shape");
  }
  return raw as unknown as Stats;
}

/** Completed share of resolved recommendations; null when nothing was resolved. */
export function completionRate(m: MethodStats): number | null {
  const resolved = m.completed + m.skipped;
  return resolved > 0 ? m.completed / resolved : null;
}
