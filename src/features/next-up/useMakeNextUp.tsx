import { useState } from "react";
import { Link } from "react-router";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { chooseNextUp, nextUpState } from "../../services/nextUp";

interface Pending {
  editionId: string;
  title: string;
  replacing: string;
  requestId: string;
}

/**
 * Manual "Make Next Up" for list and album views. Replacing an existing pick asks first;
 * the replaced pick counts as skipped. Render `element` where the status should appear.
 */
export function useMakeNextUp() {
  const [pending, setPending] = useState<Pending | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const choose = async (editionId: string, title: string, requestId: string) => {
    setBusy(true);
    try {
      await chooseNextUp(requestId, editionId);
      setPending(null);
      setStatus(`“${title}” is now your Next Up.`);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const start = async (editionId: string, title: string) => {
    setStatus(null);
    setError(null);
    try {
      const { current } = await nextUpState();
      if (current?.editionId === editionId) {
        setStatus(`“${title}” is already your Next Up.`);
      } else if (current) {
        setPending({
          editionId,
          title,
          replacing: current.item.title,
          requestId: crypto.randomUUID(),
        });
      } else {
        await choose(editionId, title, crypto.randomUUID());
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const element = (
    <>
      {status && (
        <p className="banner banner-ok" role="status">
          <span className="banner-text">{status}</span> <Link to="/next-up">Go to Next Up</Link>
        </p>
      )}
      {error && (
        <p className="match-note match-note-error library-note" role="alert">
          {error}
        </p>
      )}
      <Dialog
        open={pending !== null}
        title="Replace your Next Up?"
        onClose={() => {
          setPending(null);
        }}
        actions={
          <>
            <Button
              onClick={() => {
                setPending(null);
              }}
            >
              Keep current pick
            </Button>
            <Button
              variant="primary"
              disabledReason={busy ? "Saving…" : undefined}
              onClick={() => {
                if (pending) void choose(pending.editionId, pending.title, pending.requestId);
              }}
            >
              Make Next Up
            </Button>
          </>
        }
      >
        “{pending?.replacing}” is your current Next Up. Make “{pending?.title}” your Next Up
        instead? “{pending?.replacing}” stays on your Listen List.
      </Dialog>
    </>
  );

  return { start: (editionId: string, title: string) => void start(editionId, title), element };
}
