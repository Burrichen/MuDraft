import { useId, useState } from "react";
import { Link } from "react-router";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { chooseNextUp, nextUpState, type NextUpSlot } from "../../services/nextUp";
import { METHOD_LABEL } from "./criteria";
import "./nextUp.css";

interface Option {
  slot: number;
  title: string;
  detail: string;
}

interface Pending {
  editionId: string;
  title: string;
  requestId: string;
  max: number;
  options: Option[];
}

function option(s: NextUpSlot): Option {
  if (s.current)
    return { slot: s.slot, title: s.current.item.title, detail: s.current.item.credit };
  return {
    slot: s.slot,
    title: "Nothing picked yet",
    detail: s.session ? `Picks with ${METHOD_LABEL[s.session.method.mode]}` : "",
  };
}

/**
 * Manual "Make Next Up" for list and album views. The album is added to a free Next Up
 * slot; when every slot is in use, it asks which one to replace (that pick counts as
 * skipped and stays on the Listen List). Render `element` where the status should appear.
 */
export function useMakeNextUp() {
  const [pending, setPending] = useState<Pending | null>(null);
  const [replace, setReplace] = useState<number | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const name = useId();

  const choose = async (
    editionId: string,
    title: string,
    requestId: string,
    slot?: { number: number; replacing: string },
  ) => {
    setBusy(true);
    try {
      await chooseNextUp(requestId, editionId, slot?.number);
      setPending(null);
      setStatus(
        slot
          ? `“${title}” replaced “${slot.replacing}” in your Next Up.`
          : `“${title}” is now in your Next Up.`,
      );
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
      const state = await nextUpState();
      if (state.slots.some((s) => s.current?.editionId === editionId)) {
        setStatus(`“${title}” is already in your Next Up.`);
      } else if (state.nextFreeSlot === null) {
        setReplace(null);
        setPending({
          editionId,
          title,
          requestId: crypto.randomUUID(),
          max: state.maxSlots,
          options: state.slots.map(option),
        });
      } else {
        await choose(editionId, title, crypto.randomUUID());
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const chosen = pending?.options.find((o) => o.slot === replace);
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
        title="Your Next Up is full"
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
              Cancel
            </Button>
            <Button
              variant="primary"
              disabledReason={
                busy ? "Saving…" : chosen ? undefined : "Choose which pick to replace first."
              }
              onClick={() => {
                if (pending && chosen)
                  void choose(pending.editionId, pending.title, pending.requestId, {
                    number: chosen.slot,
                    replacing: chosen.title,
                  });
              }}
            >
              Replace
            </Button>
          </>
        }
      >
        <p>
          All {pending?.max} Next Up slots are in use. Choose one to replace with “{pending?.title}
          ”. The album you replace stays on your Listen List.
        </p>
        <fieldset className="next-up-replace">
          <legend className="visually-hidden">Replace</legend>
          {pending?.options.map((o) => (
            <label
              key={o.slot}
              className="next-up-replace-option"
              data-checked={replace === o.slot}
            >
              <input
                type="radio"
                name={name}
                checked={replace === o.slot}
                onChange={() => {
                  setReplace(o.slot);
                }}
              />
              <span className="next-up-replace-body">
                <span className="next-up-replace-title">{o.title}</span>
                {o.detail && <span className="setting-hint">{o.detail}</span>}
              </span>
            </label>
          ))}
        </fieldset>
      </Dialog>
    </>
  );

  return { start: (editionId: string, title: string) => void start(editionId, title), element };
}
