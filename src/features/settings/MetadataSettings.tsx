import { useEffect, useState } from "react";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import {
  clearMetadataCache,
  metadataCacheStats,
  type MetadataCacheStats,
} from "../../services/metadata";
import { formatBytes } from "../artwork/formatBytes";

/** Settings → Metadata: where refreshes happen, and the saved MusicBrainz responses. */
export function MetadataSettings() {
  const [stats, setStats] = useState<MetadataCacheStats | null>(null);
  const [reload, setReload] = useState(0);
  const [confirming, setConfirming] = useState(false);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    metadataCacheStats().then(
      (s) => {
        if (!cancelled) setStats(s);
      },
      (err: unknown) => {
        if (!cancelled) setError(err instanceof Error ? err.message : String(err));
      },
    );
    return () => {
      cancelled = true;
    };
  }, [reload]);

  const clear = async () => {
    setConfirming(false);
    try {
      const n = await clearMetadataCache();
      setStatus(
        `Removed ${String(n)} saved response${n === 1 ? "" : "s"}. Your library is unchanged.`,
      );
      setReload((r) => r + 1);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <section className="panel section" aria-labelledby="settings-metadata">
      <h2 id="settings-metadata" className="section-title">
        Metadata
      </h2>
      <p className="setting-hint">
        Album details come from MusicBrainz. To update an album, open it and choose{" "}
        <strong>Refresh from MusicBrainz</strong>; fields you edited (including genres) stay as you
        set them. Artist catalogues are fetched and refreshed from each artist’s page.
      </p>
      <div className="setting-row">
        <div className="setting-text">
          <span className="setting-label">Saved MusicBrainz responses</span>
          <span className="setting-hint">
            {stats
              ? `${String(stats.entries)} saved (${formatBytes(stats.bytes)})${
                  stats.expired > 0 ? `, ${String(stats.expired)} older than a week` : ""
                }. Used to look things up again offline.`
              : "…"}
          </span>
        </div>
        <Button
          disabledReason={stats?.entries === 0 ? "Nothing saved to clear." : undefined}
          onClick={() => {
            setConfirming(true);
          }}
        >
          Clear saved responses
        </Button>
      </div>
      <p className="setting-hint">
        Your albums, listens, ratings, tags, and catalogues are stored in your library, not in this
        cache, and stay usable offline.
      </p>
      {status && (
        <p className="setting-hint" role="status">
          {status}
        </p>
      )}
      {error && (
        <p className="match-note match-note-error" role="alert">
          {error}
        </p>
      )}
      <Dialog
        open={confirming}
        title="Clear saved MusicBrainz responses?"
        onClose={() => {
          setConfirming(false);
        }}
        actions={
          <>
            <Button
              onClick={() => {
                setConfirming(false);
              }}
            >
              Keep
            </Button>
            <Button variant="danger" onClick={() => void clear()}>
              Clear
            </Button>
          </>
        }
      >
        Searches and lookups will need the internet again until they are repeated. Nothing in your
        library is removed.
      </Dialog>
    </section>
  );
}
