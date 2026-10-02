import type { AlbumSummary } from "../components/AlbumCard";
import { artworkUrl } from "../services/album";
import type { LibrarySource, ListItem } from "../services/library";
import { describeListening } from "../services/listening";
import { describeRating } from "../services/ratings";

/** Map a library item to an album card; shared by the lists and Next Up. */
export function toSummary(item: ListItem, source: LibrarySource | "next_up"): AlbumSummary {
  const badge =
    item.editionCount > 1 || item.editionName.toLowerCase() !== "standard"
      ? item.editionName
      : null;
  return {
    albumId: item.albumId,
    editionId: item.editionId,
    title: item.title,
    artists: item.artists,
    year: item.originalYear,
    editionBadge: badge,
    genres: item.genres,
    tags: item.tags,
    addedAt: item.addedAt,
    artworkUrl: artworkUrl(item.artwork),
    rating: item.rating.effective,
    ratingNote: item.rating.source === "calculated" ? describeRating(item.rating) : null,
    listeningNote:
      source === "collection" || item.listening.listenCount > 0 || item.listening.earlierUndated
        ? describeListening(item.listening)
        : null,
  };
}
