import { useId, useState } from "react";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { localToday, logListen, type ListenKind, type Logged } from "../../services/listening";
import "./listening.css";

const KINDS: { value: ListenKind; label: string }[] = [
  { value: "first", label: "First listen" },
  { value: "relisten", label: "Re-listen" },
  { value: "unspecified", label: "Not sure" },
];

/**
 * Log a listen of the whole album (only whole albums are logged; tracks are ticked off on
 * the Listen List instead). Dates are the user's own calendar
 * dates (or unknown) — never invented. The request ID is fixed for the dialog's lifetime,
 * so double-clicking Save records one listen.
 */
export function ListenDialog({
  albumTitle,
  editionId,
  attemptId = null,
  onClose,
  onLogged,
}: {
  albumTitle: string;
  editionId: string;
  attemptId?: string | null;
  onClose: () => void;
  onLogged: (logged: Logged) => void;
}) {
  const ids = { date: useId(), unknown: useId(), kind: useId(), earlier: useId() };
  const today = localToday();
  const [requestId] = useState(() => crypto.randomUUID());
  const [date, setDate] = useState(today);
  const [dateUnknown, setDateUnknown] = useState(false);
  const [kind, setKind] = useState<ListenKind>("unspecified");
  const [earlier, setEarlier] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const reason = busy
    ? "Saving…"
    : !dateUnknown && !/^\d{4}-\d{2}-\d{2}$/.test(date)
      ? "Enter the date you listened, or tick “I don’t know the date”."
      : !dateUnknown && date > today
        ? "That date is in the future."
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
      title="Mark album listened"
      onClose={onClose}
      actions={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button variant="primary" disabledReason={reason} onClick={() => void save()}>
            Mark listened
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
        <p className="setting-hint">
          Records the whole album and the tracks it has now. Tracks added to this edition later
          aren’t counted as listened.
        </p>
        {error && (
          <p className="match-note match-note-error" role="alert">
            {error}
          </p>
        )}
      </div>
    </Dialog>
  );
}
