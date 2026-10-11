import { useEffect, useState } from "react";
import { Link } from "react-router";
import { PageHeader } from "../../components/PageHeader";
import { EmptyState, ErrorState, LoadingState } from "../../components/States";
import { formatDuration } from "../../services/album";
import { useLibraryVersion } from "../../services/libraryEvents";
import { likedTracks, setFavourite, type LikedTrack } from "../../services/ratings";
import "../album/album.css";
import "../stats/stats.css";

type Load =
  | { kind: "loading" }
  | { kind: "error"; code: string; message: string }
  | { kind: "ready"; tracks: LikedTrack[] };

function errorInfo(err: unknown) {
  const code = typeof err === "object" && err && "code" in err ? String(err.code) : "unknown";
  return { code, message: err instanceof Error ? err.message : String(err) };
}

/** Tracks the user liked with the heart, across every album in the library. */
export function LikedPage() {
  const version = useLibraryVersion();
  const [load, setLoad] = useState<Load>({ kind: "loading" });
  const [retry, setRetry] = useState(0);
  const [actionError, setActionError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    likedTracks().then(
      (tracks) => {
        if (!cancelled) setLoad({ kind: "ready", tracks });
      },
      (err: unknown) => {
        if (!cancelled) setLoad({ kind: "error", ...errorInfo(err) });
      },
    );
    return () => {
      cancelled = true;
    };
  }, [version, retry]);

  const count = load.kind === "ready" ? load.tracks.length : null;

  return (
    <section>
      <PageHeader
        title="Liked"
        description={
          count === null
            ? "Tracks you’ve liked with the heart."
            : `${String(count)} liked track${count === 1 ? "" : "s"}.`
        }
      />
      {actionError && (
        <p className="match-note match-note-error library-note" role="alert">
          {actionError}
        </p>
      )}
      {load.kind === "loading" && <LoadingState label="Loading liked tracks…" />}
      {load.kind === "error" && (
        <ErrorState
          title="Couldn’t load liked tracks"
          message={load.message}
          code={load.code}
          onRetry={() => {
            setRetry((n) => n + 1);
          }}
        />
      )}
      {load.kind === "ready" &&
        (load.tracks.length === 0 ? (
          <EmptyState
            title="No liked tracks yet"
            description="Tap the heart next to a track on an album page to like it."
          />
        ) : (
          <div className="panel section">
            <div className="table-scroll" tabIndex={0} role="region" aria-label="Liked tracks">
              <table className="stat-table liked-table">
                <thead>
                  <tr>
                    <th scope="col">Track</th>
                    <th scope="col">Artist</th>
                    <th scope="col">Album</th>
                    <th scope="col">Length</th>
                    <th scope="col">
                      <span className="visually-hidden">Liked</span>
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {load.tracks.map((t) => (
                    <tr key={t.trackId}>
                      <td>{t.title}</td>
                      <td>{t.credit}</td>
                      <td>
                        <Link to={`/albums/${t.albumId}`}>{t.albumTitle}</Link>
                        {t.editionName.toLowerCase() !== "standard" && (
                          <span className="setting-hint"> ({t.editionName})</span>
                        )}
                      </td>
                      <td>{t.lengthMs === null ? "—" : formatDuration(t.lengthMs)}</td>
                      <td>
                        <button
                          type="button"
                          className="mini-button favourite-button"
                          aria-pressed="true"
                          aria-label={`Like: ${t.title}`}
                          title="Liked · click to unlike"
                          onClick={() => {
                            setActionError(null);
                            setFavourite(t.trackId, false).catch((err: unknown) => {
                              setActionError(errorInfo(err).message);
                            });
                          }}
                        >
                          <span aria-hidden="true">♥</span>
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        ))}
    </section>
  );
}
