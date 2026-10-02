import { RatingInput } from "../../components/Rating";
import { formatDuration, type AlbumDetail } from "../../services/album";
import { runtimeText } from "./runtime";

export function Tracklist({
  detail,
  onRate,
  onFavourite,
}: {
  detail: AlbumDetail;
  /** Half-stars 0–10, or null to clear. */
  onRate: (trackId: string, halfStars: number | null) => void;
  onFavourite: (trackId: string, favourite: boolean) => void;
}) {
  if (detail.discs.length === 0) {
    return <p className="setting-hint">This edition has no tracklist yet.</p>;
  }
  const multi = detail.discs.length > 1;
  return (
    <div className="tracklist">
      {detail.discs.map((disc) => (
        <div key={disc.number} className="import-table-wrap">
          <table className="import-table tracklist-table">
            {multi && (
              <caption className="tracklist-disc">
                Disc {disc.number} of {detail.discs.length}
              </caption>
            )}
            <thead>
              <tr>
                <th scope="col" className="track-num">
                  #
                </th>
                <th scope="col">Title · your rating</th>
                <th scope="col" className="track-len">
                  Length
                </th>
              </tr>
            </thead>
            <tbody>
              {disc.tracks.map((t) => (
                <tr key={t.id}>
                  <td className="track-num">{t.position}</td>
                  <td>
                    <span className="track-title">{t.title}</span>
                    {t.credit && <span className="setting-hint"> · {t.credit}</span>}
                    {t.listened && <span className="track-flag"> · Listened</span>}
                    <div className="track-controls">
                      <button
                        type="button"
                        className="mini-button favourite-button"
                        aria-pressed={t.favourite}
                        aria-label={`Favourite: ${t.title}`}
                        title={t.favourite ? "Favourite track" : "Mark as favourite"}
                        onClick={() => {
                          onFavourite(t.id, !t.favourite);
                        }}
                      >
                        <span aria-hidden="true">{t.favourite ? "♥" : "♡"}</span>
                      </button>
                      <RatingInput
                        compact
                        label={`Rating for ${t.title}`}
                        value={t.rating}
                        onChange={(v) => {
                          onRate(t.id, v);
                        }}
                      />
                    </div>
                  </td>
                  <td className="track-len">
                    {t.lengthMs === null ? (
                      <span title="Length unknown">
                        <span aria-hidden="true">—</span>
                        <span className="visually-hidden">Unknown</span>
                      </span>
                    ) : (
                      formatDuration(t.lengthMs)
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ))}
      <p className="tracklist-total">
        <span className="visually-hidden">Total: </span>
        {runtimeText(detail.runtime)}
      </p>
    </div>
  );
}
