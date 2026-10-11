import { callNative, NativeError } from "../transport/native";
import { notifyLibraryChanged } from "./libraryEvents";

/**
 * Mirrors `RatingSummary` in `src-tauri/src/domain/rating.rs` — the one shared album
 * rating calculation. Ratings travel as half-stars (0–10); null is unrated, 0 is a rating.
 */
export interface RatingSummary {
  effective: number | null;
  source: "explicit" | "calculated" | "unrated";
  explicit: number | null;
  calculated: number | null;
  ratedTracks: number;
  totalTracks: number;
}

/** "Calculated from 3/12 rated tracks", or what the explicit rating overrides. */
export function describeRating(s: RatingSummary): string {
  const from = `${String(s.ratedTracks)}/${String(s.totalTracks)} rated tracks`;
  switch (s.source) {
    case "calculated":
      return `Calculated from ${from}`;
    case "explicit":
      return s.calculated !== null
        ? `Your rating (tracks alone would give ${String(s.calculated / 2)} from ${from})`
        : "Your rating";
    case "unrated":
      return s.totalTracks > 0
        ? `Not rated · 0/${String(s.totalTracks)} rated tracks`
        : "Not rated";
  }
}

function summary(raw: unknown): RatingSummary {
  if (typeof raw !== "object" || raw === null || !("source" in raw)) {
    throw new NativeError("invalid_response", "Rating had an unexpected shape");
  }
  return raw as RatingSummary;
}

export async function setTrackRating(
  trackId: string,
  halfStars: number | null,
): Promise<RatingSummary> {
  const s = summary(await callNative("rating_set_track", { trackId, rating: halfStars }));
  notifyLibraryChanged();
  return s;
}

/** Set (0–10 half-stars) or clear (null) the explicit album rating; tracks are untouched. */
export async function setAlbumRating(
  editionId: string,
  halfStars: number | null,
): Promise<RatingSummary> {
  const s = summary(await callNative("rating_set_album", { editionId, rating: halfStars }));
  notifyLibraryChanged();
  return s;
}

/** The heart on a track: liked or not. Stored natively as `is_favourite`. */
export async function setFavourite(trackId: string, favourite: boolean): Promise<void> {
  await callNative("track_set_favourite", { trackId, favourite });
  notifyLibraryChanged();
}

/** Mirrors `LikedTrack` in `src-tauri/src/library/ratings.rs`. */
export interface LikedTrack {
  trackId: string;
  title: string;
  lengthMs: number | null;
  albumId: string;
  albumTitle: string;
  credit: string;
  editionName: string;
}

export async function likedTracks(): Promise<LikedTrack[]> {
  const raw = await callNative("liked_tracks");
  if (!Array.isArray(raw))
    throw new NativeError("invalid_response", "Liked tracks had an unexpected shape");
  return raw as LikedTrack[];
}

export async function setReview(editionId: string, review: string | null): Promise<void> {
  await callNative("album_review_set", { editionId, review });
  notifyLibraryChanged();
}
