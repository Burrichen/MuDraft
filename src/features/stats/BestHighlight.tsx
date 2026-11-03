import { Link } from "react-router";
import type { Best } from "../../services/stats";
import { StarMeter } from "./charts";
import { formatStars, oneDecimal, plural } from "./format";

/**
 * The three highest-rated groups, best first, each with its sample size stated plainly.
 * Only groups with at least `best.minAlbums` rated albums are ranked.
 */
export function BestHighlight({
  title,
  best,
  href,
  unplaced,
}: {
  title: string;
  best: Best;
  href: (key: string) => string;
  /** e.g. "without a genre". */
  unplaced?: string;
}) {
  return (
    <section className="best" aria-label={title}>
      <h4 className="best-title">{title}</h4>
      {best.top.length === 0 ? (
        <p className="setting-hint">Nothing has {plural(best.minAlbums, "rated album")} yet.</p>
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
                {formatStars(g.meanStars)} from {plural(g.albums, "rated album")}
              </span>
            </li>
          ))}
        </ol>
      )}
      {best.unplacedAlbums > 0 && unplaced && (
        <p className="setting-hint">
          {plural(best.unplacedAlbums, "rated album")} {unplaced} not included.
        </p>
      )}
    </section>
  );
}
