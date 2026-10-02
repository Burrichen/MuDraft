import { useEffect, useId, useState } from "react";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import {
  artworkPreference,
  clearArtworkCache,
  setArtworkPreference,
  type ArtworkPreference,
} from "../../services/album";
import { useLibraryVersion } from "../../services/libraryEvents";
import { ArtworkExplanation } from "./ArtworkConsent";
import { formatBytes } from "./formatBytes";

/** Settings → Artwork: the download preference and a separate cache-clear action. */
export function ArtworkSettings() {
  const version = useLibraryVersion();
  const switchId = useId();
  const [pref, setPref] = useState<ArtworkPreference | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);

  useEffect(() => {
    let cancelled = false;
    artworkPreference().then(
      (p) => {
        if (!cancelled) setPref(p);
      },
      (err: unknown) => {
        if (!cancelled) setError(err instanceof Error ? err.message : String(err));
      },
    );
    return () => {
      cancelled = true;
    };
  }, [version]);

  const clear = async () => {
    setConfirming(false);
    try {
      const r = await clearArtworkCache();
      setStatus(
        `Removed ${String(r.files)} downloaded image${r.files === 1 ? "" : "s"} (${formatBytes(r.bytes)}). Your own images are kept.`,
      );
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <section className="panel section" aria-labelledby="settings-artwork">
      <h2 id="settings-artwork" className="section-title">
        Artwork
      </h2>
      <div className="setting-row">
        <div className="setting-text">
          <label className="setting-label" htmlFor={switchId}>
            Download artwork from Cover Art Archive
          </label>
          <span id={`${switchId}-hint`} className="setting-hint">
            {pref?.allowed === null
              ? "You haven’t chosen yet; nothing is downloaded until you do."
              : "When off, MuDraft makes no artwork requests."}
          </span>
        </div>
        <input
          id={switchId}
          type="checkbox"
          role="switch"
          className="switch"
          aria-describedby={`${switchId}-hint`}
          checked={pref?.allowed === true}
          disabled={!pref}
          onChange={(e) => {
            const allowed = e.target.checked;
            setArtworkPreference(allowed).catch((err: unknown) => {
              setError(err instanceof Error ? err.message : String(err));
            });
          }}
        />
      </div>
      <details className="import-help">
        <summary>What this downloads</summary>
        <ArtworkExplanation />
      </details>
      <div className="setting-row">
        <div className="setting-text">
          <span className="setting-label">Downloaded artwork</span>
          <span className="setting-hint">
            {pref
              ? `${String(pref.cache.files)} image${pref.cache.files === 1 ? "" : "s"}, ${formatBytes(pref.cache.bytes)}`
              : "…"}
          </span>
        </div>
        <Button
          disabledReason={
            pref && pref.cache.files === 0 ? "No downloaded artwork to clear." : undefined
          }
          onClick={() => {
            setConfirming(true);
          }}
        >
          Clear downloaded artwork
        </Button>
      </div>
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
        title="Clear downloaded artwork?"
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
        Downloaded covers are deleted from this computer. Images you added yourself stay. Covers are
        downloaded again when needed if downloads are on.
      </Dialog>
    </section>
  );
}
