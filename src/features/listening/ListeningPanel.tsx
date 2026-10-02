import { useId, useState } from "react";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { UndoBanner } from "../../components/UndoBanner";
import type { AlbumDetail } from "../../services/album";
import {
  confirmCoverage,
  correctListen,
  deleteListen,
  describeListening,
  formatCalendarDate,
  localToday,
  restoreListen,
  undoLog,
  type ListenFields,
  type ListenKind,
  type ListenView,
  type Logged,
} from "../../services/listening";
import { ListenDialog } from "./ListenDialog";
import { loggedMessage } from "./loggedMessage";
import "./listening.css";

const KIND_LABEL: Record<ListenKind, string> = {
  first: "First listen",
  relisten: "Re-listen",
  unspecified: "Listen",
};

function scope(l: ListenView): string {
  if (l.coverage === "unknown") return "Whole album · tracks unknown when logged";
  if (l.isFull) return `Whole album · ${String(l.coveredTracks)} tracks`;
  return `${String(l.coveredTracks)} of ${String(l.editionTracks)} tracks`;
}

type Undo = { message: string; run: () => Promise<void> };

/** Album page listening history: log, correct, delete, each with immediate undo. */
export function ListeningPanel({ detail }: { detail: AlbumDetail }) {
  const ids = { date: useId(), unknown: useId(), kind: useId() };
  const [dialog, setDialog] = useState<"album" | "tracks" | null>(null);
  const [undo, setUndo] = useState<Undo | null>(null);
  const [editing, setEditing] = useState<ListenView | null>(null);
  const [draft, setDraft] = useState<ListenFields>({ listenedOn: null, kind: "unspecified" });
  const [confirming, setConfirming] = useState<ListenView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const edition = detail.editions.find((e) => e.id === detail.editionId);
  const tracks = detail.discs.flatMap((d) =>
    d.tracks.map((t) => ({
      id: t.id,
      label: `${detail.discs.length > 1 ? `${String(d.number)}.` : ""}${String(t.position)} ${t.title}`,
      listened: t.listened,
    })),
  );
  const today = localToday();

  const act = (task: () => Promise<Undo | null>) => {
    setError(null);
    task().then(
      (u) => {
        if (u) setUndo(u);
      },
      (err: unknown) => {
        setError(err instanceof Error ? err.message : String(err));
      },
    );
  };

  const onLogged = (logged: Logged) => {
    setDialog(null);
    setUndo({ message: loggedMessage(logged), run: () => undoLog(logged.undo) });
  };

  const draftInvalid =
    draft.listenedOn !== null &&
    (draft.listenedOn > today || !/^\d{4}-\d{2}-\d{2}$/.test(draft.listenedOn));

  return (
    <section className="panel section album-section" aria-labelledby="album-listening">
      <div className="toolbar">
        <h2 id="album-listening" className="section-title">
          Listening
        </h2>
        <div className="page-actions">
          <Button
            variant="primary"
            onClick={() => {
              setDialog("album");
            }}
          >
            Mark album listened
          </Button>
          <Button
            disabledReason={tracks.length === 0 ? "This edition has no tracklist yet." : undefined}
            onClick={() => {
              setDialog("tracks");
            }}
          >
            Log tracks…
          </Button>
        </div>
      </div>
      <p>{describeListening(detail.listening.summary)}</p>
      {undo && (
        <UndoBanner
          message={undo.message}
          onUndo={undo.run}
          onDismiss={() => {
            setUndo(null);
          }}
        />
      )}
      {error && (
        <p className="match-note match-note-error" role="alert">
          {error}
        </p>
      )}
      {detail.listening.listens.length > 0 && (
        <ul className="listen-list" aria-label="Listens">
          {detail.listening.listens.map((l) => (
            <li key={l.id} className="listen-row">
              <span>
                <strong>{l.listenedOn ? formatCalendarDate(l.listenedOn) : "Date unknown"}</strong>{" "}
                · {KIND_LABEL[l.kind]} · {scope(l)} · {l.editionName}
                {l.fromNextUp && <span className="edition-badge">From Next Up</span>}
              </span>
              <span className="page-actions">
                {l.coverage === "unknown" && l.editionTracks > 0 && (
                  <Button
                    variant="ghost"
                    className="button-small"
                    onClick={() => {
                      setConfirming(l);
                    }}
                  >
                    Count tracks as listened…
                  </Button>
                )}
                <Button
                  variant="ghost"
                  className="button-small"
                  aria-label={`Edit listen from ${l.listenedOn ?? "an unknown date"}`}
                  onClick={() => {
                    setEditing(l);
                    setDraft({ listenedOn: l.listenedOn, kind: l.kind });
                  }}
                >
                  Edit
                </Button>
                <Button
                  variant="ghost"
                  className="button-small"
                  aria-label={`Delete listen from ${l.listenedOn ?? "an unknown date"}`}
                  onClick={() => {
                    act(async () => {
                      await deleteListen(l.id);
                      return { message: "Listen deleted.", run: () => restoreListen(l.id) };
                    });
                  }}
                >
                  Delete
                </Button>
              </span>
            </li>
          ))}
        </ul>
      )}

      {dialog && edition && (
        <ListenDialog
          albumTitle={detail.title}
          editionId={edition.id}
          {...(dialog === "tracks" ? { tracks } : {})}
          onClose={() => {
            setDialog(null);
          }}
          onLogged={onLogged}
        />
      )}

      <Dialog
        open={editing !== null}
        title="Correct this listen"
        onClose={() => {
          setEditing(null);
        }}
        actions={
          <>
            <Button
              onClick={() => {
                setEditing(null);
              }}
            >
              Cancel
            </Button>
            <Button
              variant="primary"
              disabledReason={
                draftInvalid ? "Use a past date, or tick “I don’t know the date”." : undefined
              }
              onClick={() => {
                const target = editing;
                setEditing(null);
                if (!target) return;
                act(async () => {
                  const previous = await correctListen(target.id, draft);
                  return {
                    message: "Listen corrected.",
                    run: async () => {
                      await correctListen(target.id, previous);
                    },
                  };
                });
              }}
            >
              Save
            </Button>
          </>
        }
      >
        <div className="match">
          <div className="match-field">
            <label htmlFor={ids.date}>Date listened</label>
            <input
              id={ids.date}
              type="date"
              max={today}
              value={draft.listenedOn ?? ""}
              disabled={draft.listenedOn === null}
              onChange={(e) => {
                setDraft({ ...draft, listenedOn: e.target.value });
              }}
            />
            <label className="import-tag" htmlFor={ids.unknown}>
              <input
                id={ids.unknown}
                type="checkbox"
                checked={draft.listenedOn === null}
                onChange={(e) => {
                  setDraft({ ...draft, listenedOn: e.target.checked ? null : today });
                }}
              />
              I don’t know the date
            </label>
          </div>
          <fieldset className="listen-kind">
            <legend>This was a</legend>
            {(["first", "relisten", "unspecified"] as const).map((k) => (
              <label key={k} className="import-tag">
                <input
                  type="radio"
                  name={ids.kind}
                  checked={draft.kind === k}
                  onChange={() => {
                    setDraft({ ...draft, kind: k });
                  }}
                />
                {k === "unspecified" ? "Not sure" : KIND_LABEL[k]}
              </label>
            ))}
          </fieldset>
        </div>
      </Dialog>

      <Dialog
        open={confirming !== null}
        title="Count these tracks as listened?"
        onClose={() => {
          setConfirming(null);
        }}
        actions={
          <>
            <Button
              onClick={() => {
                setConfirming(null);
              }}
            >
              Not now
            </Button>
            <Button
              variant="primary"
              onClick={() => {
                const target = confirming;
                setConfirming(null);
                if (!target) return;
                act(async () => {
                  const n = await confirmCoverage(target.id);
                  setUndo(null);
                  setError(null);
                  return {
                    message: `Counted ${String(n)} tracks as listened for that listen.`,
                    run: () => Promise.resolve(),
                  };
                });
              }}
            >
              Count all {confirming?.editionTracks ?? 0} tracks
            </Button>
          </>
        }
      >
        This listen was logged before the tracklist was known. Only confirm if you listened to all{" "}
        {confirming?.editionTracks ?? 0} tracks that “{confirming?.editionName}” has now.
      </Dialog>
    </section>
  );
}
