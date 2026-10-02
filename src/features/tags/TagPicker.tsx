import { useEffect, useId, useState } from "react";
import { Button } from "../../components/Button";
import { TagChip } from "../../components/TagChip";
import { createTag, listTags, updateAlbumTags, type TagInfo } from "../../services/library";
import { useLibraryVersion } from "../../services/libraryEvents";

function norm(s: string): string {
  return s.normalize("NFC").toLowerCase().trim().replace(/\s+/g, " ");
}

function message(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

type Change = "add" | "remove";

/**
 * Assign tags to one or more albums. Tags are global and belong to the album, so the
 * change shows on every page. One album: each tick applies at once. Several albums:
 * changes are staged, and removing tags from several albums asks for confirmation.
 */
export function TagPicker({
  albumIds,
  assigned,
  onDone,
}: {
  albumIds: readonly string[];
  /** For each tag ID, how many of `albumIds` currently have it. */
  assigned: ReadonlyMap<string, number>;
  onDone: () => void;
}) {
  const version = useLibraryVersion();
  const [tags, setTags] = useState<TagInfo[] | null>(null);
  const [filter, setFilter] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [changes, setChanges] = useState<Map<string, Change>>(new Map());
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const inputId = useId();
  const single = albumIds.length === 1;
  const total = albumIds.length;

  useEffect(() => {
    let cancelled = false;
    listTags().then(
      (t) => {
        if (!cancelled) setTags(t);
      },
      (err: unknown) => {
        if (!cancelled) setError(message(err));
      },
    );
    return () => {
      cancelled = true;
    };
  }, [version]);

  const state = (tag: TagInfo): "all" | "some" | "none" => {
    const change = changes.get(tag.id);
    if (change === "add") return "all";
    if (change === "remove") return "none";
    const n = assigned.get(tag.id) ?? 0;
    return n === 0 ? "none" : n >= total ? "all" : "some";
  };

  const apply = async (add: string[], remove: string[]) => {
    setBusy(true);
    setError(null);
    try {
      await updateAlbumTags(albumIds, { add, remove });
      return true;
    } catch (err) {
      setError(message(err));
      return false;
    } finally {
      setBusy(false);
    }
  };

  const toggle = (tag: TagInfo) => {
    const on = state(tag) !== "all";
    if (single) {
      void apply(on ? [tag.id] : [], on ? [] : [tag.id]);
      return;
    }
    const next = new Map(changes);
    next.set(tag.id, on ? "add" : "remove");
    setChanges(next);
    setConfirming(false);
  };

  const wanted = norm(filter);
  const visible = (tags ?? []).filter((t) => wanted === "" || norm(t.name).includes(wanted));
  const exact = (tags ?? []).some((t) => norm(t.name) === wanted);

  const create = async () => {
    setBusy(true);
    setError(null);
    try {
      const tag = await createTag(filter);
      setFilter("");
      if (single) {
        await updateAlbumTags(albumIds, { add: [tag.id] });
      } else {
        setChanges(new Map(changes).set(tag.id, "add"));
      }
    } catch (err) {
      setError(message(err));
    } finally {
      setBusy(false);
    }
  };

  const adds = [...changes].filter(([, c]) => c === "add").map(([id]) => id);
  const removes = [...changes].filter(([, c]) => c === "remove").map(([id]) => id);

  const commit = async () => {
    if (removes.length > 0 && !confirming) {
      setConfirming(true);
      return;
    }
    if (await apply(adds, removes)) onDone();
  };

  return (
    <div className="tag-picker">
      <label htmlFor={inputId} className="tag-picker-label">
        {single ? "Tags" : `Tags for ${String(total)} albums`}
      </label>
      <input
        id={inputId}
        className="tag-picker-input"
        value={filter}
        placeholder="Find or create a tag"
        autoComplete="off"
        onChange={(e) => {
          setFilter(e.target.value);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter" && filter.trim() && !exact) {
            e.preventDefault();
            void create();
          }
        }}
      />
      {!tags && !error && <p className="setting-hint">Loading tags…</p>}
      {tags && (
        <ul className="tag-picker-list">
          {visible.map((t) => {
            const s = state(t);
            return (
              <li key={t.id}>
                <label className="tag-picker-option">
                  <input
                    type="checkbox"
                    checked={s === "all"}
                    ref={(el) => {
                      if (el) el.indeterminate = s === "some";
                    }}
                    aria-checked={s === "some" ? "mixed" : s === "all"}
                    disabled={busy}
                    onChange={() => {
                      toggle(t);
                    }}
                  />
                  <TagChip name={t.name} color={t.color} builtin={t.builtin} />
                  {s === "some" && <span className="setting-hint">on some</span>}
                </label>
              </li>
            );
          })}
        </ul>
      )}
      {tags && visible.length === 0 && wanted === "" && (
        <p className="setting-hint">No tags yet.</p>
      )}
      {filter.trim() !== "" && !exact && (
        <Button variant="ghost" className="button-small" onClick={() => void create()}>
          Create tag “{filter.trim()}”
        </Button>
      )}
      {error && (
        <p className="match-note match-note-error" role="alert">
          {error}
        </p>
      )}
      {!single && (
        <div className="tag-picker-footer">
          {confirming && (
            <p className="match-note match-note-warning" role="alert">
              Remove {removes.length} tag{removes.length === 1 ? "" : "s"} from {total} albums? The
              albums and other tags stay.
            </p>
          )}
          <Button onClick={onDone}>Cancel</Button>
          <Button
            variant="primary"
            disabledReason={
              changes.size === 0 ? "Tick or untick a tag first." : busy ? "Saving…" : undefined
            }
            onClick={() => void commit()}
          >
            {confirming ? "Yes, apply" : "Apply"}
          </Button>
        </div>
      )}
      {single && (
        <div className="tag-picker-footer">
          <Button onClick={onDone}>Done</Button>
        </div>
      )}
    </div>
  );
}
