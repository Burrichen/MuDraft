import { useId, useState } from "react";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { localToday, logListen, type ListenKind, type Logged } from "../../services/listening";
import "./listening.css";

export interface DialogTrack {
  id: string;
  label: string;
  listened: boolean;
}

const KINDS: { value: ListenKind; label: string }[] = [
  { value: "first", label: "First listen" },
  { value: "relisten", label: "Re-listen" },
  { value: "unspecified", label: "Not sure" },
];

/**
 * Log a listen: the whole album, or chosen tracks. Dates are the user's own calendar
 * dates (or unknown) — never invented. The request ID is fixed for the dialog's lifetime,
 * so double-clicking Save records one listen.
 */
export function ListenDialog({
  albumTitle,
  editionId,
  tracks,
  attemptId = null,
  onClose,
  onLogged,
}: {
  albumTitle: string;
  editionId: string;
  /** Present = log individual tracks instead of the whole album. */
  tracks?: DialogTrack[];
  attemptId?: string | null;
  onClose: () => void;
  onLogged: (logged: Logged) => void;
}) {
  const ids = { date: useId(), unknown: useId(), kind: useId(), earlier: useId(), tracks: useId() };
  const today = localToday();
  const [requestId] = useState(() => crypto.randomUUID());
  const [date, setDate] = useState(today);
  const [dateUnknown, setDateUnknown] = useState(false);
  const [kind, setKind] = useState<ListenKind>("unspecified");
  const [earlier, setEarlier] = useState(false);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const trackMode = tracks !== undefined;

  const reason = busy
    ? "Saving…"
    : !dateUnknown && !/^\d{4}-\d{2}-\d{2}$/.test(date)
      ? "Enter the date you listened, or tick “I don’t know the date”."
      : !dateUnknown && date > today
        ? "That date is in the future."
        : trackMode && picked.size === 0
          ? "Choose the tracks you listened to."
          : undefined;

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      onLogged(
        await logListen(requestId, {
          editionId,
          listenedOn: dateUnknown ? null : date,
          kind,
          earlierUndated: earlier,
          trackIds: trackMode ? [...picked] : null,
          attemptId,
        }),
      );
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setBusy(false); // the same request ID makes a retry safe
    }
  };

  return (
    <Dialog
      open
      title={trackMode ? "Log tracks" : "Mark album listened"}
      onClose={onClose}
      actions={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button variant="primary" disabledReason={reason} onClick={() => void save()}>
            {trackMode ? "Log tracks" : "Mark listened"}
          </Button>
        </>
      }
    >
      <div className="match">
        <p className="listen-album">{albumTitle}</p>
        <div className="match-field">
          <label htmlFor={ids.date}>Date listened</label>
          <input
            id={ids.date}
            type="date"
            value={date}
            max={today}
            disabled={dateUnknown}
            onChange={(e) => {
              setDate(e.target.value);
            }}
          />
          <label className="import-tag" htmlFor={ids.unknown}>
            <input
              id={ids.unknown}
              type="checkbox"
              checked={dateUnknown}
              onChange={(e) => {
                setDateUnknown(e.target.checked);
              }}
            />
            I don’t know the date
          </label>
        </div>
        <fieldset className="listen-kind">
          <legend>This was a</legend>
          {KINDS.map((k) => (
            <label key={k.value} className="import-tag">
              <input
                type="radio"
                name={ids.kind}
                checked={kind === k.value}
                onChange={() => {
                  setKind(k.value);
                }}
              />
              {k.label}
            </label>
          ))}
        </fieldset>
        <label className="import-tag" htmlFor={ids.earlier}>
          <input
            id={ids.earlier}
            type="checkbox"
            checked={earlier}
            onChange={(e) => {
              setEarlier(e.target.checked);
            }}
          />
          I also listened before, but don’t know when
        </label>
        {trackMode && (
          <fieldset className="listen-tracks">
            <legend id={ids.tracks}>Tracks you listened to</legend>
            {tracks.map((t) => (
              <label key={t.id} className="import-tag">
                <input
                  type="checkbox"
                  checked={picked.has(t.id)}
                  onChange={(e) => {
                    const next = new Set(picked);
                    if (e.target.checked) next.add(t.id);
                    else next.delete(t.id);
                    setPicked(next);
                  }}
                />
                {t.label}
                {t.listened && <span className="setting-hint"> (listened before)</span>}
              </label>
            ))}
          </fieldset>
        )}
        {!trackMode && (
          <p className="setting-hint">
            Records the whole album and the tracks it has now. Tracks added to this edition later
            aren’t counted as listened.
          </p>
        )}
        {error && (
          <p className="match-note match-note-error" role="alert">
            {error}
          </p>
        )}
      </div>
    </Dialog>
  );
}
