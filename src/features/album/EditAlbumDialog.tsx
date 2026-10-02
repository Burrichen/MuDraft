import { useId, useState } from "react";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { editAlbum, unlockField, type AlbumDetail, type AlbumEdit } from "../../services/album";

const FIELD_LABEL: Record<string, string> = {
  title: "Title",
  original_date: "Original release date",
  description: "Description",
};

function message(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/** Edit album fields. Saved edits are kept when the album is refreshed from MusicBrainz. */
export function EditAlbumDialog({ detail, onClose }: { detail: AlbumDetail; onClose: () => void }) {
  const ids = { title: useId(), date: useId(), desc: useId() };
  const [title, setTitle] = useState(detail.title);
  const [date, setDate] = useState(detail.originalDate.value ?? "");
  const [description, setDescription] = useState(detail.description?.text ?? "");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const linked = detail.musicbrainzReleaseGroupId !== null;

  const save = async () => {
    const edit: AlbumEdit = {};
    if (title.trim() !== detail.title) edit.title = title;
    if (date.trim() !== (detail.originalDate.value ?? "")) edit.originalDate = date.trim();
    if (description.trim() !== (detail.description?.text ?? "")) edit.description = description;
    if (Object.keys(edit).length === 0) {
      onClose();
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await editAlbum(detail.albumId, edit);
      onClose();
    } catch (err) {
      setError(message(err));
    } finally {
      setBusy(false);
    }
  };

  const unlock = async (field: string) => {
    setError(null);
    try {
      await unlockField(detail.albumId, field);
    } catch (err) {
      setError(message(err));
    }
  };

  return (
    <Dialog
      open
      size="wide"
      title="Edit album details"
      onClose={onClose}
      actions={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            disabledReason={busy ? "Saving…" : title.trim() ? undefined : "Enter a title."}
            onClick={() => void save()}
          >
            Save
          </Button>
        </>
      }
    >
      <div className="match">
        <p className="match-note">
          {linked
            ? "Fields you change here are kept when the album is refreshed from MusicBrainz."
            : "This album isn’t linked to MusicBrainz; your details are the only source."}
        </p>
        <div className="match-field">
          <label htmlFor={ids.title}>Title</label>
          <input
            id={ids.title}
            value={title}
            onChange={(e) => {
              setTitle(e.target.value);
            }}
          />
        </div>
        <div className="match-field">
          <label htmlFor={ids.date}>Original release date</label>
          <input
            id={ids.date}
            value={date}
            placeholder="1997, 1997-05 or 1997-05-21"
            aria-describedby={`${ids.date}-hint`}
            onChange={(e) => {
              setDate(e.target.value);
            }}
          />
          <span id={`${ids.date}-hint`} className="match-muted">
            Use only what you know: a year, year and month, or a full date. Leave empty if unknown.
          </span>
        </div>
        <div className="match-field">
          <label htmlFor={ids.desc}>Description</label>
          <textarea
            id={ids.desc}
            className="album-textarea"
            rows={5}
            value={description}
            onChange={(e) => {
              setDescription(e.target.value);
            }}
          />
        </div>
        {linked && detail.lockedFields.some((f) => f in FIELD_LABEL) && (
          <div className="match-section">
            <h3 className="match-heading">Edited fields</h3>
            <ul className="locked-list">
              {detail.lockedFields
                .filter((f) => f in FIELD_LABEL)
                .map((f) => (
                  <li key={f}>
                    <span>{FIELD_LABEL[f]} is kept as you edited it.</span>{" "}
                    <Button variant="ghost" className="button-small" onClick={() => void unlock(f)}>
                      Let MusicBrainz update {FIELD_LABEL[f]?.toLowerCase()} again
                    </Button>
                  </li>
                ))}
            </ul>
          </div>
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
