import { useEffect, useState } from "react";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { artworkPreference, setArtworkPreference } from "../../services/album";

/** What downloading artwork involves, in plain words (shared with Settings). */
export function ArtworkExplanation() {
  return (
    <>
      <p>
        MuDraft can show album covers from the Cover Art Archive (coverartarchive.org, hosted by the
        Internet Archive). For albums you add, it requests the cover using the album’s MusicBrainz
        ID. No account or personal details are sent.
      </p>
      <p>
        Covers are saved on this computer (each up to 8 MB) so they keep working offline. You can
        clear them, replace any cover with your own image, or turn downloads off in Settings at any
        time.
      </p>
    </>
  );
}

/**
 * First-run consent for artwork downloads. Until the user chooses, nothing is requested.
 * Closing the dialog without choosing asks again next time.
 */
export function ArtworkConsent() {
  const [open, setOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    artworkPreference().then(
      (p) => {
        if (!cancelled && p.allowed === null) setOpen(true);
      },
      () => undefined, // storage problems are reported elsewhere
    );
    return () => {
      cancelled = true;
    };
  }, []);

  const choose = async (allowed: boolean) => {
    try {
      await setArtworkPreference(allowed);
      setOpen(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <Dialog
      open={open}
      title="Download album artwork?"
      onClose={() => {
        setOpen(false);
      }}
      actions={
        <>
          <Button onClick={() => void choose(false)}>Not now</Button>
          <Button variant="primary" onClick={() => void choose(true)}>
            Download artwork
          </Button>
        </>
      }
    >
      <ArtworkExplanation />
      {error && (
        <p className="match-note match-note-error" role="alert">
          {error}
        </p>
      )}
    </Dialog>
  );
}
