import type { ReactNode } from "react";
import { Link, useLocation } from "react-router";
import { Artwork } from "./Artwork";
import { RatingDisplay } from "./Rating";
import { TagChip } from "./TagChip";

/** Presentation shape for an album tile; services map domain data into it. */
export interface AlbumSummary {
  albumId: string;
  title: string;
  artists: readonly { id: string; name: string }[];
  year: number | null;
  editionName?: string | null;
  artworkUrl?: string | null;
  /** Half-stars 0–10; null or absent means unrated. */
  rating?: number | null;
  /** Where the rating comes from, e.g. "Calculated from 3/12 rated tracks". */
  ratingNote?: string | null;
  /** Distinguishes two editions of one album shown side by side. */
  editionId?: string;
  /** Shown as a badge (e.g. "Deluxe"); omit for a lone standard edition. */
  editionBadge?: string | null;
  genres?: readonly string[];
  tags?: readonly { id: string; name: string; color: string; builtin: boolean }[];
  /** ISO timestamp; shown as "Added …". */
  addedAt?: string | null;
  /** e.g. "Last listened 30 Sep 2026 · 3 listens". */
  listeningNote?: string | null;
}

/** Optional per-card controls supplied by the page. */
export interface CardExtras {
  /** Rendered right after the genres (the tag "+" button). */
  genreAction?: ReactNode;
  selection?: { checked: boolean; onChange: (checked: boolean) => void };
  actions?: ReactNode;
}

const addedFormat = new Intl.DateTimeFormat(undefined, { dateStyle: "medium" });

export type AlbumLayout = "grid" | "list";

function ArtistLine({ artists }: { artists: AlbumSummary["artists"] }) {
  if (artists.length === 0) return <span className="album-artists">Unknown artist</span>;
  const full = artists.map((a) => a.name).join(", ");
  return (
    <span className="album-artists clamp-1" title={full}>
      {artists.map((a, i) => (
        <span key={a.id}>
          {i > 0 && ", "}
          <Link className="album-artist-link" to={`/artists/${a.id}`}>
            {a.name}
          </Link>
        </span>
      ))}
    </span>
  );
}

/**
 * Grid tile or list row. The title link covers the whole card for pointer users while
 * artist links stay separately reachable. Long names clamp visually; the full text stays
 * in the DOM (and in `title`) so nothing is lost to assistive tech.
 */
export function AlbumCard({
  album,
  layout,
  extras = {},
}: {
  album: AlbumSummary;
  layout: AlbumLayout;
  extras?: CardExtras;
}) {
  const location = useLocation();
  const year = album.year !== null ? String(album.year) : "Year unknown";
  const meta = [year, album.editionBadge === undefined ? album.editionName : null]
    .filter(Boolean)
    .join(" · ");
  const showGenres = album.genres !== undefined;
  return (
    <article className="album-card" data-layout={layout} data-selected={extras.selection?.checked}>
      {extras.selection && (
        <label className="album-select">
          <input
            type="checkbox"
            checked={extras.selection.checked}
            onChange={(e) => {
              extras.selection?.onChange(e.target.checked);
            }}
          />
          <span className="visually-hidden">Select {album.title}</span>
        </label>
      )}
      <Artwork
        src={album.artworkUrl}
        title={album.title}
        size={layout === "grid" ? "card" : "row"}
      />
      <div className="album-text">
        <h3 className="album-title clamp-2" title={album.title}>
          <Link
            className="album-link"
            to={`/albums/${album.albumId}`}
            state={{ from: location.pathname }}
          >
            {album.title}
          </Link>
        </h3>
        <ArtistLine artists={album.artists} />
        <span className="album-meta">
          {meta}
          {album.editionBadge && <span className="edition-badge">{album.editionBadge}</span>}
        </span>
        {showGenres && (
          <span className="album-genres">
            <span className="album-genre-names">
              <span className="visually-hidden">Genres: </span>
              {album.genres && album.genres.length > 0 ? album.genres.join(" · ") : "No genres"}
            </span>
            {extras.genreAction}
          </span>
        )}
        {album.tags && album.tags.length > 0 && (
          <ul className="tag-list album-tags" aria-label={`Tags on ${album.title}`}>
            {album.tags.map((t) => (
              <li key={t.id}>
                <TagChip name={t.name} color={t.color} builtin={t.builtin} />
              </li>
            ))}
          </ul>
        )}
        {album.listeningNote && <span className="album-meta">{album.listeningNote}</span>}
        {album.addedAt && (
          <span className="album-meta">Added {addedFormat.format(new Date(album.addedAt))}</span>
        )}
      </div>
      {album.rating !== undefined && (
        <div className="album-rating">
          <RatingDisplay value={album.rating} />
          {album.ratingNote && <span className="album-meta">{album.ratingNote}</span>}
        </div>
      )}
      {extras.actions && <div className="album-actions">{extras.actions}</div>}
    </article>
  );
}

export function AlbumCollection({
  albums,
  layout,
  label,
  empty,
  cardExtras,
}: {
  albums: readonly AlbumSummary[];
  layout: AlbumLayout;
  label: string;
  empty: ReactNode;
  cardExtras?: (album: AlbumSummary) => CardExtras;
}) {
  if (albums.length === 0) return <>{empty}</>;
  return (
    <ul className="album-collection" data-layout={layout} aria-label={label}>
      {albums.map((album) => (
        <li key={`${album.albumId}:${album.editionId ?? ""}`}>
          <AlbumCard
            album={album}
            layout={layout}
            {...(cardExtras ? { extras: cardExtras(album) } : {})}
          />
        </li>
      ))}
    </ul>
  );
}
