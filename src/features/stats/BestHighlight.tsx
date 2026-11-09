import { Link } from "react-router";
import type { Best } from "../../services/stats";
import { StarMeter } from "./charts";
import { formatStars, oneDecimal, plural } from "./format";

/**
 * The three highest-rated groups, best first, each with its sample size stated plainly.
 * Only groups with at least `best.minAlbums` rated albums (by tracks: albums with rated
 * tracks) are ranked.
 */
export function BestHighlight({
  title,
  best,
  href,
  byTracks = false,
}: {
  title: string;
  best: Best;
  href: (key: string) => string;
  /** Ranked by individual track ratings (wording only; `best` holds the data). */
  byTracks?: boolean;
}) {
  return (
    <section className="best" aria-label={title}>
      <h4 className="best-title">{title}</h4>
      {best.top.length === 0 ? (
        <p className="setting-hint">
          {byTracks
            ? `Nothing has rated tracks from ${plural(best.minAlbums, "album")} yet.`
            : `Nothing has ${plural(best.minAlbums, "rated album")} yet.`}
        </p>
      ) : (
        <ol className="best-ranking">
          {best.top.map((g) => (
            <li key={g.key} className="best-entry">
              <Link to={href(g.key)} className="best-name">
                {g.label}
              </Link>
              <span className="best-score">
                <span className="best-stars-number" aria-hidden="true">
                  {oneDecimal(g.meanStars)}
                </span>
                <StarMeter stars={g.meanStars} />
              </span>
              <span className="setting-hint">
                {formatStars(g.meanStars)} from{" "}
                {g.tracks === null
                  ? plural(g.albums, "rated album")
                  : `${plural(g.tracks, "rated track")} on ${plural(g.albums, "album")}`}
              </span>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}
