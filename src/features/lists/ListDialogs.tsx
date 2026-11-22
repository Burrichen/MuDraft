import { useId, useState } from "react";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import "../match/match.css";
import "./lists.css";

function message(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/** Name entry for a new list or a rename. Mount a fresh instance (new `key`) per opening. */
export function NameListDialog({
  open,
  onClose,
  title,
  confirmLabel,
  initialName = "",
  onSubmit,
}: {
  open: boolean;
  onClose: () => void;
  title: string;
  confirmLabel: string;
  initialName?: string;
  onSubmit: (name: string) => Promise<void>;
}) {
  const id = useId();
  const [name, setName] = useState(initialName);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit() {
    setBusy(true);
    setError(null);
    try {
      await onSubmit(name);
    } catch (err) {
      setError(message(err));
      setBusy(false);
    }
  }

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={title}
      actions={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            type="submit"
            form={`${id}-form`}
            disabledReason={busy ? "Saving…" : name.trim() ? undefined : "Enter a name."}
          >
            {confirmLabel}
          </Button>
        </>
      }
    >
      <form
        id={`${id}-form`}
        className="list-form"
        onSubmit={(e) => {
          e.preventDefault();
          if (name.trim() && !busy) void submit();
        }}
      >
        <div className="match-field">
          <label htmlFor={id}>List name</label>
          <input
            id={id}
            value={name}
            maxLength={300}
            onChange={(e) => {
              setName(e.target.value);
            }}
          />
        </div>
      </form>
      {error && (
        <p className="match-note match-note-error" role="alert">
          {error}
        </p>
      )}
    </Dialog>
  );
}

/** Add one album to a list by hand. Mount a fresh instance (new `key`) per opening. */
export function AddAlbumDialog({
  open,
  onClose,
  onSubmit,
}: {
  open: boolean;
  onClose: () => void;
  onSubmit: (album: { artistName: string; title: string; year: number | null }) => Promise<void>;
}) {
  const id = useId();
  const [artist, setArtist] = useState("");
  const [title, setTitle] = useState("");
  const [year, setYear] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const yearNumber = year.trim() === "" ? null : Number(year);
  const yearOk =
    yearNumber === null ||
    (Number.isInteger(yearNumber) && yearNumber >= 1000 && yearNumber <= 9999);
  const missing = !artist.trim()
    ? "Enter the artist."
    : !title.trim()
      ? "Enter the album title."
      : !yearOk
        ? "Enter a four-digit year, or leave it empty."
        : undefined;

  async function submit() {
    setBusy(true);
    setError(null);
    try {
      await onSubmit({ artistName: artist, title, year: yearNumber });
    } catch (err) {
      setError(message(err));
      setBusy(false);
    }
  }

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Add an album to this list"
      actions={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            type="submit"
            form={`${id}-form`}
            disabledReason={busy ? "Adding…" : missing}
          >
            Add album
          </Button>
        </>
      }
    >
      <form
        id={`${id}-form`}
        className="list-form"
        onSubmit={(e) => {
          e.preventDefault();
          if (!missing && !busy) void submit();
        }}
      >
        <div className="match-field">
          <label htmlFor={`${id}-artist`}>Artist</label>
          <input
            id={`${id}-artist`}
            value={artist}
            maxLength={300}
            onChange={(e) => {
              setArtist(e.target.value);
            }}
          />
        </div>
        <div className="match-field">
          <label htmlFor={`${id}-title`}>Album title</label>
          <input
            id={`${id}-title`}
            value={title}
            maxLength={300}
            onChange={(e) => {
              setTitle(e.target.value);
            }}
          />
        </div>
        <div className="match-field">
          <label htmlFor={`${id}-year`}>Year (optional)</label>
          <input
            id={`${id}-year`}
            inputMode="numeric"
            maxLength={4}
            value={year}
            aria-invalid={!yearOk || undefined}
            onChange={(e) => {
              setYear(e.target.value);
            }}
          />
        </div>
        <p className="match-note">
          If this album is already in your Collection, it shows as listened to automatically.
        </p>
      </form>
      {error && (
        <p className="match-note match-note-error" role="alert">
          {error}
        </p>
      )}
    </Dialog>
  );
}

/** Confirm deleting a list (never its albums). */
export function DeleteListDialog({
  open,
  onClose,
  name,
  onConfirm,
}: {
  open: boolean;
  onClose: () => void;
  name: string;
  onConfirm: () => Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function confirm() {
    setBusy(true);
    setError(null);
    try {
      await onConfirm();
    } catch (err) {
      setError(message(err));
      setBusy(false);
    }
  }

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Delete this list?"
      actions={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="danger"
            onClick={() => void confirm()}
            disabledReason={busy ? "Deleting…" : undefined}
          >
            Delete list
          </Button>
        </>
      }
    >
      <p>
        “{name}” will be deleted. The albums in your Collection and Listen List, and your ratings
        and reviews, are not affected.
      </p>
      {error && (
        <p className="match-note match-note-error" role="alert">
          {error}
        </p>
      )}
    </Dialog>
  );
}
