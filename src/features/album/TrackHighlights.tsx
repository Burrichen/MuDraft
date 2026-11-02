import { RatingDisplay } from "../../components/Rating";
import type { AlbumDetail, TrackView } from "../../services/album";

/** Rated tracks sharing the extreme rating (ties all shown; zero is a rating). */
function extremes(tracks: readonly TrackView[], pick: (a: number, b: number) => number) {
  const rated = tracks.filter((t): t is TrackView & { rating: number } => t.rating !== null);
  if (rated.length === 0) return null;
  const value = rated.map((t) => t.rating).reduce((a, b) => pick(a, b));
  return { value, tracks: rated.filter((t) => t.rating === value) };
}

function Highlight({
  title,
  best,
}: {
  title: string;
  best: { value: number; tracks: TrackView[] };
}) {
  return (
    <div className="track-highlight">
      <h3 className="match-heading">{title}</h3>
      <RatingDisplay value={best.value} />
      <ul className="track-highlight-list">
        {best.tracks.map((t) => (
          <li key={t.id}>
            <span className="track-title">{t.title}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

/** Highest and lowest rated tracks of the shown edition, for albums in the Collection. */
export function TrackHighlights({ detail }: { detail: AlbumDetail }) {
  const tracks = detail.discs.flatMap((d) => d.tracks);
  const highest = extremes(tracks, Math.max);
  const lowest = extremes(tracks, Math.min);
  return (
    <section className="panel section album-section" aria-labelledby="album-track-highlights">
      <h2 id="album-track-highlights" className="section-title">
        Track ratings
      </h2>
      {highest && lowest ? (
        <div className="track-highlights">
          <Highlight title="Highest rated track" best={highest} />
          <Highlight title="Lowest rated track" best={lowest} />
        </div>
      ) : (
        <p className="setting-hint">Rate tracks above to see your highest and lowest rated.</p>
      )}
    </section>
  );
}
