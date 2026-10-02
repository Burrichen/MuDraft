import { Link } from "react-router";
import type { Best } from "../../services/stats";
import { formatStars, plural } from "./format";

/**
 * Highest-rated group with its sample size stated plainly. A single rated album can win;
 * ties name every winner. Nothing is hidden for having few albums.
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
  /** e.g. "with an unknown year". */
  unplaced?: string;
}) {
  return (
    <section className="best" aria-label={title}>
      <h4 className="best-title">{title}</h4>
      {best.winners.length === 0 ? (
        <p className="setting-hint">No rated albums yet.</p>
      ) : (
        <>
          <p className="best-winners">
            {best.winners.map((w, i) => (
              <span key={w.key}>
                {i > 0 && (i === best.winners.length - 1 ? " and " : ", ")}
                <Link to={href(w.key)}>{w.label}</Link>
              </span>
            ))}
          </p>
          <p className="setting-hint">
            {formatStars(best.winners[0]?.meanStars ?? 0)}
            {best.winners.length > 1
              ? ` each — tied, ${best.winners.map((w) => plural(w.albums, "rated album")).join(" and ")}`
              : ` from ${plural(best.winners[0]?.albums ?? 0, "rated album")}`}
            .
          </p>
        </>
      )}
      {best.unplacedAlbums > 0 && unplaced && (
        <p className="setting-hint">
          {plural(best.unplacedAlbums, "rated album")} {unplaced} not included.
        </p>
      )}
    </section>
  );
}
