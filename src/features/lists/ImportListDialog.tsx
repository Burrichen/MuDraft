import { useId, useState } from "react";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import {
  addListEntries,
  createList,
  previewListImport,
  type ListImport,
} from "../../services/lists";
import "../match/match.css";
import "./lists.css";

const PREVIEW_ROWS = 6;

function message(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

function plural(n: number, word: string): string {
  return `${String(n)} ${word}${n === 1 ? "" : "s"}`;
}

/**
 * Read a list from a web page, show what was found, and only then save it. With `target`
 * the albums are added to that list (skipping ones it already has); without it a new
 * list is created. Mount a fresh instance (new `key`) each time it opens.
 */
export function ImportListDialog({
  open,
  onClose,
  target,
  onImported,
}: {
  open: boolean;
  onClose: () => void;
  target?: { id: string; name: string };
  onImported: (listId: string, summary: string) => void;
}) {
  const urlId = useId();
  const nameId = useId();
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [preview, setPreview] = useState<ListImport | null>(null);
  const [name, setName] = useState("");

  async function readPage() {
    setBusy(true);
    setError(null);
    try {
      const found = await previewListImport(url);
      setPreview(found);
      setName(found.name);
    } catch (err) {
      setPreview(null);
      setError(message(err));
    } finally {
      setBusy(false);
    }
  }

  async function save() {
    if (!preview) return;
    setBusy(true);
    setError(null);
    try {
      if (target) {
        const albums = preview.entries.map((e) => ({
          artistName: e.artistName,
          title: e.title,
          year: e.year,
        }));
        const { added, duplicates } = await addListEntries(target.id, albums);
        onImported(
          target.id,
          duplicates > 0
            ? `Added ${plural(added, "album")}; ${String(duplicates)} already in this list.`
            : `Added ${plural(added, "album")}.`,
        );
      } else {
        const id = await createList(name, preview.sourceUrl, preview.entries);
        onImported(id, `Imported ${plural(preview.entries.length, "album")}.`);
      }
    } catch (err) {
      setError(message(err));
      setBusy(false);
    }
  }

  const count = preview?.entries.length ?? 0;
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={target ? `Import albums into ${target.name}` : "Import a list from a web page"}
      actions={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          {preview ? (
            <Button
              variant="primary"
              onClick={() => void save()}
              disabledReason={
                busy
                  ? "Saving…"
                  : !target && !name.trim()
                    ? "Enter a name for the list."
                    : undefined
              }
            >
              {target
                ? `Add ${plural(count, "album")}`
                : `Create list with ${plural(count, "album")}`}
            </Button>
          ) : (
            <Button
              variant="primary"
              type="submit"
              form={`${urlId}-form`}
              disabledReason={
                busy ? "Reading the page…" : url.trim() ? undefined : "Paste a web address first."
              }
            >
              Read page
            </Button>
          )}
        </>
      }
    >
      <form
        id={`${urlId}-form`}
        className="list-form"
        onSubmit={(e) => {
          e.preventDefault();
          if (url.trim() && !busy) void readPage();
        }}
      >
        <div className="match-field">
          <label htmlFor={urlId}>Web address</label>
          <input
            id={urlId}
            type="url"
            inputMode="url"
            placeholder="https://pitchfork.com/story/…"
            value={url}
            disabled={preview !== null}
            onChange={(e) => {
              setUrl(e.target.value);
            }}
          />
        </div>
        <p className="match-note">
          Works with Pitchfork ranked-list pages. MuDraft reads the page once, shows you what it
          found, and saves nothing until you confirm.
        </p>
      </form>
      {error && (
        <p className="match-note match-note-error" role="alert">
          {error}
        </p>
      )}
      {preview && (
        <div className="list-form">
          {!target && (
            <div className="match-field">
              <label htmlFor={nameId}>List name</label>
              <input
                id={nameId}
                value={name}
                maxLength={300}
                onChange={(e) => {
                  setName(e.target.value);
                }}
              />
            </div>
          )}
          <p role="status">
            Found {plural(count, "album")} on “{preview.name}”.
          </p>
          <ol className="import-preview" aria-label="First albums found">
            {preview.entries.slice(0, PREVIEW_ROWS).map((e) => (
              <li key={e.position}>
                <span className="import-rank">{e.position}.</span> {e.artistName} — {e.title}
                {e.year !== null && <span className="list-muted"> ({e.year})</span>}
              </li>
            ))}
          </ol>
          {count > PREVIEW_ROWS && (
            <p className="list-muted">…and {String(count - PREVIEW_ROWS)} more.</p>
          )}
        </div>
      )}
    </Dialog>
  );
}
