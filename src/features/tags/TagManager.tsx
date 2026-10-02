import { useEffect, useId, useState } from "react";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { ErrorState, LoadingState } from "../../components/States";
import { TagChip } from "../../components/TagChip";
import { createTag, deleteTag, listTags, updateTag, type TagInfo } from "../../services/library";
import { useLibraryVersion } from "../../services/libraryEvents";

const HEX = /^#[0-9a-f]{6}$/i;

function message(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

function usage(n: number): string {
  return n === 0 ? "Not used yet" : `On ${String(n)} album${n === 1 ? "" : "s"}`;
}

/** Colour input: native RGB picker plus an editable hex field for keyboard users. */
function ColorField({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (hex: string) => void;
}) {
  const id = useId();
  // Draft only while typing; otherwise show the stored value.
  const [draft, setDraft] = useState<string | null>(null);
  const text = draft ?? value;
  return (
    <span className="color-field">
      <input
        id={`${id}-picker`}
        type="color"
        className="color-swatch"
        aria-label={`${label} colour picker`}
        value={value}
        onChange={(e) => {
          onChange(e.target.value);
        }}
      />
      <label htmlFor={`${id}-hex`} className="visually-hidden">
        {label} colour (hex)
      </label>
      <input
        id={`${id}-hex`}
        className="color-hex"
        value={text}
        maxLength={7}
        spellCheck={false}
        aria-invalid={!HEX.test(text) || undefined}
        onChange={(e) => {
          setDraft(e.target.value);
          if (HEX.test(e.target.value)) onChange(e.target.value.toLowerCase());
        }}
        onBlur={() => {
          setDraft(null);
        }}
      />
    </span>
  );
}

function TagRow({
  tag,
  onError,
  onDelete,
}: {
  tag: TagInfo;
  onError: (m: string | null) => void;
  onDelete: () => void;
}) {
  const [draft, setDraft] = useState<string | null>(null);
  const name = draft ?? tag.name;
  const nameId = useId();

  const save = async (change: { name?: string; color?: string }) => {
    onError(null);
    try {
      await updateTag(tag.id, change);
    } catch (err) {
      onError(`${tag.name}: ${message(err)}`);
    } finally {
      setDraft(null);
    }
  };

  return (
    <li className="tag-row">
      <TagChip name={tag.name} color={tag.color} builtin={tag.builtin} />
      <div className="tag-row-name">
        <label htmlFor={nameId} className="visually-hidden">
          Name of {tag.name}
        </label>
        <input
          id={nameId}
          className="tag-picker-input"
          value={name}
          readOnly={tag.builtin}
          aria-describedby={tag.builtin ? `${nameId}-builtin` : undefined}
          onChange={(e) => {
            setDraft(e.target.value);
          }}
          onBlur={() => {
            if (!tag.builtin && draft !== null && draft.trim() !== tag.name)
              void save({ name: draft });
            else setDraft(null);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") e.currentTarget.blur();
            if (e.key === "Escape") setDraft(null);
          }}
        />
        {tag.builtin && (
          <span id={`${nameId}-builtin`} className="setting-hint">
            Built-in tag used by Weighted Random. Its name is fixed; colour and assignments can
            change.
          </span>
        )}
      </div>
      <ColorField
        label={tag.name}
        value={tag.color}
        onChange={(color) => {
          if (color !== tag.color) void save({ color });
        }}
      />
      <span className="setting-hint tag-row-usage">{usage(tag.albumCount)}</span>
      <Button
        variant="ghost"
        className="button-small"
        aria-label={`Delete tag ${tag.name}`}
        disabledReason={tag.builtin ? "Built-in tags can’t be deleted." : undefined}
        onClick={onDelete}
      >
        Delete
      </Button>
    </li>
  );
}

/** Settings → Tags: create, rename, recolour, delete. */
export function TagManager() {
  const version = useLibraryVersion();
  const [tags, setTags] = useState<TagInfo[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [newName, setNewName] = useState("");
  const [newColor, setNewColor] = useState("#60a5fa");
  const [deleting, setDeleting] = useState<TagInfo | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const newId = useId();

  useEffect(() => {
    let cancelled = false;
    listTags().then(
      (t) => {
        if (!cancelled) setTags(t);
      },
      (err: unknown) => {
        if (!cancelled) setLoadError(message(err));
      },
    );
    return () => {
      cancelled = true;
    };
  }, [version]);

  const create = async () => {
    setError(null);
    try {
      const t = await createTag(newName, newColor);
      setNewName("");
      setStatus(`Created “${t.name}”.`);
    } catch (err) {
      setError(message(err));
    }
  };

  const confirmDelete = async () => {
    if (!deleting) return;
    setError(null);
    try {
      const n = await deleteTag(deleting.id);
      setStatus(
        `Deleted “${deleting.name}”${n > 0 ? ` and removed it from ${String(n)} album${n === 1 ? "" : "s"}` : ""}.`,
      );
    } catch (err) {
      setError(message(err));
    } finally {
      setDeleting(null);
    }
  };

  return (
    <section className="panel section" aria-labelledby="settings-tags">
      <h2 id="settings-tags" className="section-title">
        Tags
      </h2>
      <p className="setting-hint">
        Tags belong to albums, so they appear on every page. Names are unique regardless of
        capitalisation.
      </p>
      <form
        className="tag-create"
        onSubmit={(e) => {
          e.preventDefault();
          void create();
        }}
      >
        <div className="match-field">
          <label htmlFor={newId}>New tag</label>
          <input
            id={newId}
            className="tag-picker-input"
            value={newName}
            maxLength={60}
            onChange={(e) => {
              setNewName(e.target.value);
            }}
          />
        </div>
        <ColorField label="New tag" value={newColor} onChange={setNewColor} />
        {newName.trim() && <TagChip name={newName.trim()} color={newColor} />}
        <Button
          type="submit"
          disabledReason={newName.trim() ? undefined : "Enter a name for the tag."}
        >
          Create tag
        </Button>
      </form>
      {error && (
        <p className="match-note match-note-error" role="alert">
          {error}
        </p>
      )}
      {status && (
        <p className="setting-hint" role="status">
          {status}
        </p>
      )}
      {loadError && <ErrorState title="Couldn’t load tags" message={loadError} />}
      {!tags && !loadError && <LoadingState label="Loading tags…" />}
      {tags && (
        <ul className="tag-rows" aria-label="Your tags">
          {tags.map((t) => (
            <TagRow
              key={t.id}
              tag={t}
              onError={setError}
              onDelete={() => {
                setDeleting(t);
              }}
            />
          ))}
        </ul>
      )}
      <Dialog
        open={deleting !== null}
        title={`Delete “${deleting?.name ?? ""}”?`}
        onClose={() => {
          setDeleting(null);
        }}
        actions={
          <>
            <Button
              onClick={() => {
                setDeleting(null);
              }}
            >
              Keep tag
            </Button>
            <Button variant="danger" onClick={() => void confirmDelete()}>
              Delete tag
            </Button>
          </>
        }
      >
        {deleting && deleting.albumCount > 0
          ? `It will be removed from ${String(deleting.albumCount)} album${deleting.albumCount === 1 ? "" : "s"}. The albums themselves stay.`
          : "It isn’t on any albums."}
      </Dialog>
    </section>
  );
}
