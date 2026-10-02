import { useEffect, useId, useState } from "react";
import { listeningSettings, setListeningSettings } from "../../services/listening";

/** Settings → Listening: whether a full-album listen leaves the Listen List. */
export function ListeningSettings() {
  const switchId = useId();
  const [removes, setRemoves] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    listeningSettings().then(
      (s) => {
        if (!cancelled) setRemoves(s.fullListenRemovesFromListenList);
      },
      (err: unknown) => {
        if (!cancelled) setError(err instanceof Error ? err.message : String(err));
      },
    );
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <section className="panel section" aria-labelledby="settings-listening">
      <h2 id="settings-listening" className="section-title">
        Listening
      </h2>
      <div className="setting-row">
        <div className="setting-text">
          <label className="setting-label" htmlFor={switchId}>
            Remove albums from your Listen List after a full listen
          </label>
          <span id={`${switchId}-hint`} className="setting-hint">
            Logging individual tracks never removes an album.
          </span>
        </div>
        <input
          id={switchId}
          type="checkbox"
          role="switch"
          className="switch"
          aria-describedby={`${switchId}-hint`}
          checked={removes === true}
          disabled={removes === null}
          onChange={(e) => {
            const next = e.target.checked;
            const previous = removes;
            setError(null);
            setRemoves(next);
            setListeningSettings(next).catch((err: unknown) => {
              setRemoves(previous);
              setError(err instanceof Error ? err.message : String(err));
            });
          }}
        />
      </div>
      {error && (
        <p className="match-note match-note-error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
