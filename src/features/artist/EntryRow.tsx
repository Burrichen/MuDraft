import { Link } from "react-router";
import { Button } from "../../components/Button";
import type { CatalogueEntry } from "../../services/discography";
import { formatHalfStars, formatStars } from "./artistFormat";

const SOURCE_LABEL: Record<string, string> = {
  explicit: "your rating",
  calculated: "from track ratings",
  unrated: "unrated",
};

/** One canonical album: title (linked when in the library), credits, evidence, score. */
export function EntryRow({
  entry: e,
  artistId,
  onAdd,
  adding = false,
}: {
  entry: CatalogueEntry;
  artistId: string;
  onAdd?: () => void;
  adding?: boolean;
}) {
  const badges = [
    e.onListenList && "Listen List",
    e.inCollection && "Collection",
    e.listened && "Listened in full",
    e.collaboration && "Collaboration",
    e.source === "manual" && "Added by you",
    e.excluded && "Excluded",
  ].filter(Boolean) as string[];
  return (
    <li className="artist-entry">
      <div className="artist-entry-main">
        <span className="artist-entry-title">
          {e.rank !== null && <span className="artist-rank">#{e.rank} </span>}
          {e.albumId ? <Link to={`/albums/${e.albumId}`}>{e.title}</Link> : e.title}
          {e.disambiguation && <span className="setting-hint"> ({e.disambiguation})</span>}
        </span>
        <span className="album-meta">
          {e.credits.map((c, i) => (
            <span key={`${c.name}-${String(i)}`}>
              {c.artistId && c.artistId !== artistId ? (
                <Link to={`/artists/${c.artistId}`}>{c.name}</Link>
              ) : (
                c.name
              )}
              {c.joinPhrase}
            </span>
          ))}
          {e.credits.length === 0 && e.credit}
          {" · "}
          {e.originalYear ?? "Year unknown"} · {e.typeLabel}
        </span>
        <span className="album-meta">
          {e.tracks
            ? `${String(e.tracks.listened)}/${String(e.tracks.total)} tracks heard`
            : "Tracklist not loaded"}
          {e.reference.name ? ` · counted against ${e.reference.name}` : ""}
          {e.reference.lastError ? ` · couldn’t load: ${e.reference.lastError}` : ""}
        </span>
        {badges.length > 0 && (
          <span className="artist-badges">
            {badges.map((b) => (
              <span key={b} className="edition-badge">
                {b}
              </span>
            ))}
          </span>
        )}
      </div>
      <div className="artist-entry-side">
        {e.score && (
          <div className="artist-score">
            <strong>{formatStars(e.score.stars)}</strong>
            <ul aria-label={`Edition scores for ${e.title}`}>
              {e.score.editions.map((s) => (
                <li key={s.editionId}>
                  {s.name}: {formatHalfStars(s.rating)} ({SOURCE_LABEL[s.source]})
                </li>
              ))}
            </ul>
          </div>
        )}
        {onAdd && !e.onListenList && (
          <Button
            variant="ghost"
            className="button-small"
            aria-label={`Add ${e.title} to Listen List`}
            disabledReason={adding ? "Adding…" : undefined}
            onClick={onAdd}
          >
            Add to Listen List
          </Button>
        )}
      </div>
    </li>
  );
}
