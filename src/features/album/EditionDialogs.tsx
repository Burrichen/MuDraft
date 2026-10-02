import { useEffect, useId, useState } from "react";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { ErrorState, LoadingState } from "../../components/States";
import {
  addEdition,
  editionCandidates,
  previewEditionSwitch,
  switchEdition,
  type EditionChoice,
  type SwitchPreview,
} from "../../services/album";
import { formatRating } from "../../components/ratingFormat";

function message(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

function code(err: unknown): string {
  return typeof err === "object" && err && "code" in err ? String(err.code) : "unknown";
}

/**
 * Deliberately add another edition (Deluxe, Special…) from this album's MusicBrainz
 * releases. Releases already in the library can't be picked; ordinary pressings that
 * duplicate an existing edition are refused by Rust with an explanation.
 */
export function AddEditionDialog({
  albumId,
  albumTitle,
  onClose,
  onAdded,
}: {
  albumId: string;
  albumTitle: string;
  onClose: () => void;
  onAdded: (editionId: string, name: string) => void;
}) {
  const nameId = useId();
  const listId = useId();
  const [choices, setChoices] = useState<EditionChoice[] | null>(null);
  const [loadError, setLoadError] = useState<{ code: string; message: string } | null>(null);
  const [picked, setPicked] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const controller = new AbortController();
    editionCandidates(albumId, controller.signal).then(
      (r) => {
        setChoices(r.value);
      },
      (err: unknown) => {
        if (code(err) !== "cancelled") setLoadError({ code: code(err), message: message(err) });
      },
    );
    return () => {
      controller.abort();
    };
  }, [albumId]);

  const add = async () => {
    if (!picked) return;
    setBusy(true);
    setError(null);
    try {
      const out = await addEdition(albumId, picked, name.trim());
      onAdded(out.editionId, name.trim());
    } catch (err) {
      setError(message(err));
    } finally {
      setBusy(false);
    }
  };

  const reason = busy
    ? "Adding…"
    : !picked
      ? "Choose a release first."
      : !name.trim()
        ? "Name this edition, e.g. Deluxe."
        : undefined;

  return (
    <Dialog
      open
      size="wide"
      title={`Add an edition of “${albumTitle}”`}
      onClose={onClose}
      actions={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button variant="primary" disabledReason={reason} onClick={() => void add()}>
            Add edition
          </Button>
        </>
      }
    >
      <div className="match">
        <p className="match-note">
          Add an edition only when it’s genuinely different — for example a Deluxe edition with
          extra tracks. Another pressing with the same tracks isn’t added separately.
        </p>
        {!choices && !loadError && <LoadingState label="Loading editions from MusicBrainz…" />}
        {loadError && (
          <ErrorState
            title="Couldn’t load editions"
            message={loadError.message}
            code={loadError.code}
          />
        )}
        {choices && (
          <div className="match-options" role="radiogroup" aria-labelledby={listId}>
            <span id={listId} className="visually-hidden">
              Releases
            </span>
            {choices.map((c) => {
              const facts = [
                c.date.value ?? "Date unknown",
                c.country,
                c.formats.join(" + ") || null,
                `${String(c.trackCount)} tracks`,
              ]
                .filter(Boolean)
                .join(" · ");
              return (
                <label
                  key={c.id}
                  className="match-option"
                  data-checked={picked === c.id}
                  data-disabled={c.inLibraryAs !== null}
                >
                  <input
                    type="radio"
                    name={listId}
                    checked={picked === c.id}
                    disabled={c.inLibraryAs !== null}
                    aria-describedby={c.inLibraryAs ? `${c.id}-in` : undefined}
                    onChange={() => {
                      setPicked(c.id);
                      setError(null);
                      if (!name && c.disambiguation) setName(c.disambiguation);
                    }}
                  />
                  <span className="match-option-body">
                    <span className="match-option-title">
                      {c.title}
                      {c.disambiguation && (
                        <span className="match-muted"> ({c.disambiguation})</span>
                      )}
                    </span>
                    <span className="match-option-meta">{facts}</span>
                    {c.inLibraryAs && (
                      <span id={`${c.id}-in`} className="match-option-meta">
                        Already in your library as “{c.inLibraryAs}”.
                      </span>
                    )}
                  </span>
                </label>
              );
            })}
          </div>
        )}
        <div className="match-field">
          <label htmlFor={nameId}>Edition name</label>
          <input
            id={nameId}
            value={name}
            placeholder="Deluxe"
            onChange={(e) => {
              setName(e.target.value);
            }}
          />
        </div>
        {error && (
          <p className="match-note match-note-error" role="alert">
            {error}
          </p>
        )}
      </div>
    </Dialog>
  );
}

/**
 * Preview, then point the Listen List at another edition. Ratings can be copied only to
 * the same recording on the new edition — never by track position — and never over an
 * existing rating. Listens stay with the edition that was heard.
 */
export function SwitchEditionDialog({
  albumId,
  toEditionId,
  onClose,
  onSwitched,
}: {
  albumId: string;
  toEditionId: string;
  onClose: () => void;
  onSwitched: (copied: number) => void;
}) {
  const [preview, setPreview] = useState<SwitchPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copy, setCopy] = useState(true);
  const [busy, setBusy] = useState(false);
  const copyId = useId();

  useEffect(() => {
    let cancelled = false;
    previewEditionSwitch(albumId, toEditionId).then(
      (p) => {
        if (!cancelled) setPreview(p);
      },
      (err: unknown) => {
        if (!cancelled) setError(message(err));
      },
    );
    return () => {
      cancelled = true;
    };
  }, [albumId, toEditionId]);

  const copyable = preview?.carry.filter((c) => !c.targetHasData) ?? [];
  const confirm = async () => {
    setBusy(true);
    setError(null);
    try {
      onSwitched(await switchEdition(albumId, toEditionId, copy && copyable.length > 0));
    } catch (err) {
      setError(message(err));
    } finally {
      setBusy(false);
    }
  };

  const describe = (rating: number | null, favourite: boolean, listened = false) =>
    [
      rating !== null ? formatRating(rating) : null,
      favourite ? "favourite" : null,
      listened ? "listened" : null,
    ]
      .filter(Boolean)
      .join(", ");

  return (
    <Dialog
      open
      size="wide"
      title={preview ? `Use “${preview.to.name}” on your Listen List?` : "Change edition"}
      onClose={onClose}
      actions={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            disabledReason={!preview ? "Loading preview…" : busy ? "Switching…" : undefined}
            onClick={() => void confirm()}
          >
            Switch edition
          </Button>
        </>
      }
    >
      <div className="match">
        {!preview && !error && <LoadingState label="Checking your ratings and listens…" />}
        {preview && (
          <>
            <p className="match-note">
              Nothing on “{preview.from.name}” is deleted.{" "}
              {preview.listensKept > 0 &&
                `Its ${String(preview.listensKept)} listen${preview.listensKept === 1 ? "" : "s"} stay with it, because that’s what you heard.`}
            </p>
            {preview.carry.length > 0 ? (
              <section className="match-section" aria-labelledby={`${copyId}-carry`}>
                <h3 id={`${copyId}-carry`} className="match-heading">
                  Same recordings on “{preview.to.name}”
                </h3>
                <ul className="switch-list">
                  {preview.carry.map((c) => (
                    <li key={c.from.trackId}>
                      {c.from.title} → {c.to.title} (disc {c.to.disc}, track {c.to.position}):{" "}
                      {describe(c.rating, c.favourite)}
                      {c.targetHasData && (
                        <span className="setting-hint"> · already rated there, left as is</span>
                      )}
                    </li>
                  ))}
                </ul>
                {copyable.length > 0 && (
                  <label className="import-tag">
                    <input
                      type="checkbox"
                      checked={copy}
                      onChange={(e) => {
                        setCopy(e.target.checked);
                      }}
                    />
                    Copy {copyable.length} rating{copyable.length === 1 ? "" : "s"} or favourite
                    {copyable.length === 1 ? "" : "s"} to the matching tracks
                  </label>
                )}
              </section>
            ) : (
              <p className="setting-hint">
                No rated tracks have the same recording on the new edition, so nothing is copied.
              </p>
            )}
            {preview.stay.length > 0 && (
              <section className="match-section" aria-labelledby={`${copyId}-stay`}>
                <h3 id={`${copyId}-stay`} className="match-heading">
                  Stays on “{preview.from.name}”
                </h3>
                <ul className="switch-list">
                  {preview.stay.map((s) => (
                    <li key={s.track.trackId}>
                      {s.track.title}: {describe(s.rating, s.favourite, s.listened)}
                    </li>
                  ))}
                </ul>
              </section>
            )}
          </>
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
