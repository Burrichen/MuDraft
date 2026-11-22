import { useEffect, useState } from "react";
import { Link, useLocation, useNavigate, useParams } from "react-router";
import { useBack } from "../../app/useBack";
import { Button } from "../../components/Button";
import { PageHeader } from "../../components/PageHeader";
import { ProgressBar } from "../../components/ProgressBar";
import { EmptyState, ErrorState, LoadingState } from "../../components/States";
import { useLibraryVersion } from "../../services/libraryEvents";
import { addManualAlbum, importRelease } from "../../services/metadata";
import {
  addEntryToListenList,
  addListEntries,
  addUnlistenedToListenList,
  deleteList,
  getList,
  linkListEntry,
  removeListEntry,
  renameList,
  reviewMissingOnMusicBrainz,
  type EntryStatus,
  type ListDetail,
  type ListEntry,
} from "../../services/lists";
import { MatchDialog, type MatchChoice } from "../match/MatchDialog";
import { ImportListDialog } from "./ImportListDialog";
import { AddAlbumDialog, DeleteListDialog, NameListDialog } from "./ListDialogs";
import "./lists.css";

type Load =
  | { kind: "loading" }
  | { kind: "error"; code: string; message: string }
  | { kind: "ready"; list: ListDetail };

type DialogKind = "add" | "import" | "rename" | "delete";

const STATUS_LABEL: Record<EntryStatus, string> = {
  listened: "Listened",
  listen_list: "On Listen List",
  in_library: "In library",
  missing: "Not in library",
};

function errorInfo(err: unknown) {
  const code = typeof err === "object" && err && "code" in err ? String(err.code) : "unknown";
  return { code, message: err instanceof Error ? err.message : String(err) };
}

function host(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, "");
  } catch {
    return url;
  }
}

/** Entries that "Add unlistened to Listen List" would add (not listened, not already on it). */
function addable(entry: ListEntry): boolean {
  return entry.status === "missing" || entry.status === "in_library";
}

function Entry({
  entry,
  from,
  onAdd,
  onRemove,
}: {
  entry: ListEntry;
  from: string;
  onAdd: () => void;
  onRemove: () => void;
}) {
  const label = `${entry.title} by ${entry.artistName}`;
  return (
    <li className="list-entry" data-status={entry.status}>
      <span className="list-entry-rank">{entry.position}</span>
      <span className="list-entry-main">
        {entry.albumId ? (
          <Link className="list-entry-title" to={`/albums/${entry.albumId}`} state={{ from }}>
            {entry.title}
          </Link>
        ) : (
          <span className="list-entry-title">{entry.title}</span>
        )}
        <span className="list-muted">
          {entry.artistName}
          {entry.year !== null && ` · ${String(entry.year)}`}
        </span>
      </span>
      <span className="list-entry-status" data-status={entry.status}>
        {STATUS_LABEL[entry.status]}
      </span>
      <span className="list-entry-actions">
        {addable(entry) && (
          <Button aria-label={`Add to Listen List: ${label}`} onClick={onAdd}>
            {entry.status === "missing" ? "Add to Listen List…" : "Add to Listen List"}
          </Button>
        )}
        <Button variant="ghost" aria-label={`Remove from list: ${label}`} onClick={onRemove}>
          Remove
        </Button>
      </span>
    </li>
  );
}

/** One list: progress, every album with its listened state, and the Listen List actions. */
export function ListPage() {
  const { listId = "" } = useParams();
  const version = useLibraryVersion();
  const location = useLocation();
  const navigate = useNavigate();
  const back = useBack("/lists");
  const [load, setLoad] = useState<Load>({ kind: "loading" });
  const [retry, setRetry] = useState(0);
  const [dialog, setDialog] = useState<{ kind: DialogKind | null; nonce: number }>({
    kind: null,
    nonce: 0,
  });
  const [status, setStatus] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [matching, setMatching] = useState<ListEntry | null>(null);

  useEffect(() => {
    let cancelled = false;
    getList(listId).then(
      (list) => {
        if (!cancelled) setLoad({ kind: "ready", list });
      },
      (err: unknown) => {
        if (!cancelled) setLoad({ kind: "error", ...errorInfo(err) });
      },
    );
    return () => {
      cancelled = true;
    };
  }, [listId, version, retry]);

  const show = (kind: DialogKind) => {
    setDialog((d) => ({ kind, nonce: d.nonce + 1 }));
  };
  const close = () => {
    setDialog((d) => ({ ...d, kind: null }));
  };

  async function run(action: () => Promise<string | null>) {
    setStatus(null);
    setActionError(null);
    try {
      setStatus(await action());
    } catch (err) {
      setActionError(errorInfo(err).message);
    }
  }

  const plural = (n: number) => `${String(n)} album${n === 1 ? "" : "s"}`;
  const listenListMessage = ({ added, needsMatch }: { added: number; needsMatch: number }) => {
    const pending =
      needsMatch > 0
        ? ` ${plural(needsMatch)} aren’t in your library yet: find them on MusicBrainz to add them.`
        : "";
    if (added > 0) return `Added ${plural(added)} to your Listen List.${pending}`;
    return needsMatch > 0
      ? pending.trim()
      : "Nothing new to add: those albums are already listened to or on your Listen List.";
  };
  const findMissing = () =>
    run(async () => {
      await reviewMissingOnMusicBrainz(listId);
      void navigate("/listen-list/import", { state: { from: location.pathname } });
      return null;
    });
  const addChoice = async (entry: ListEntry, choice: MatchChoice) => {
    const created =
      choice.kind === "manual"
        ? await addManualAlbum(choice.album, true)
        : await importRelease({
            releaseGroupId: choice.releaseGroupId,
            releaseId: choice.releaseId,
            addToListenList: true,
            ...(choice.editionName ? { editionName: choice.editionName } : {}),
          });
    await linkListEntry(entry.id, created.albumId);
    setMatching(null);
    setStatus(`Added “${entry.title}” to your Listen List.`);
  };

  if (load.kind === "loading") return <LoadingState label="Loading list…" />;
  if (load.kind === "error") {
    return (
      <ErrorState
        title={load.code === "not_found" ? "That list doesn’t exist" : "Couldn’t load this list"}
        message={load.message}
        code={load.code}
        {...(load.code === "not_found"
          ? {}
          : {
              onRetry: () => {
                setRetry((n) => n + 1);
              },
            })}
      />
    );
  }

  const { list } = load;
  const { total, listened, onListenList } = list.progress;
  const toAdd = list.entries.filter(addable).length;
  const missing = list.entries.filter((e) => e.status === "missing").length;

  return (
    <section>
      <PageHeader
        title={list.name}
        description={
          list.sourceUrl
            ? `${String(total)} albums · from ${host(list.sourceUrl)}`
            : `${String(total)} album${total === 1 ? "" : "s"}`
        }
        back={{ label: back.label, onBack: back.goBack }}
        actions={
          <>
            <Button
              variant="primary"
              disabledReason={
                toAdd === 0
                  ? total === 0
                    ? "This list has no albums yet."
                    : "Every album is already listened to or on your Listen List."
                  : undefined
              }
              onClick={() => {
                void run(async () => listenListMessage(await addUnlistenedToListenList(list.id)));
              }}
            >
              Add unlistened to Listen List
            </Button>
            <Button
              onClick={() => {
                show("add");
              }}
            >
              Add album
            </Button>
            <Button
              onClick={() => {
                show("import");
              }}
            >
              Import from web page
            </Button>
            <Button
              variant="ghost"
              onClick={() => {
                show("rename");
              }}
            >
              Rename
            </Button>
            <Button
              variant="ghost"
              onClick={() => {
                show("delete");
              }}
            >
              Delete list
            </Button>
          </>
        }
      />
      <div className="panel list-progress">
        <ProgressBar label="Listened" value={listened} max={total} />
        <p className="list-muted">
          Albums in your Collection count as listened. Albums only on your Listen List
          {onListenList > 0 ? ` (${String(onListenList)} here)` : ""} don’t count yet.
        </p>
        {missing > 0 && (
          <div className="list-missing">
            <p className="list-muted">
              {plural(missing)} aren’t in your library yet. They’re added only after you pick the
              right MusicBrainz match, so nothing is duplicated.
            </p>
            <Button onClick={() => void findMissing()}>
              Find {plural(missing)} on MusicBrainz…
            </Button>
          </div>
        )}
      </div>
      {actionError && (
        <p className="match-note match-note-error list-note" role="alert">
          {actionError}
        </p>
      )}
      {status && (
        <p className="list-note" role="status">
          {status}
        </p>
      )}
      {list.entries.length === 0 ? (
        <EmptyState
          icon="lists"
          title="No albums in this list yet"
          description="Add an album by hand, or import a whole ranked list from a web page."
        />
      ) : (
        <ol className="list-entries" aria-label={`Albums in ${list.name}`}>
          {list.entries.map((entry) => (
            <Entry
              key={entry.id}
              entry={entry}
              from={location.pathname}
              onAdd={() => {
                if (entry.status === "missing") {
                  setMatching(entry);
                  return;
                }
                void run(async () => listenListMessage(await addEntryToListenList(entry.id)));
              }}
              onRemove={() => {
                void run(async () => {
                  await removeListEntry(entry.id);
                  return `Removed “${entry.title}” from this list.`;
                });
              }}
            />
          ))}
        </ol>
      )}

      {matching && (
        <MatchDialog
          key={matching.id}
          open
          autoSearch
          title={`Find “${matching.title}”`}
          confirmLabel="Add to Listen List"
          initialQuery={{
            title: matching.title,
            artist: matching.artistName,
            ...(matching.year !== null ? { year: matching.year } : {}),
          }}
          onClose={() => {
            setMatching(null);
          }}
          onConfirm={(choice) => addChoice(matching, choice)}
        />
      )}
      <AddAlbumDialog
        key={`add-${String(dialog.nonce)}`}
        open={dialog.kind === "add"}
        onClose={close}
        onSubmit={async (album) => {
          const { added } = await addListEntries(list.id, [album]);
          close();
          setStatus(
            added === 0 ? "That album is already in this list." : `Added “${album.title}”.`,
          );
        }}
      />
      <ImportListDialog
        key={`import-${String(dialog.nonce)}`}
        open={dialog.kind === "import"}
        onClose={close}
        target={{ id: list.id, name: list.name }}
        onImported={(_id, summary) => {
          close();
          setStatus(summary);
        }}
      />
      <NameListDialog
        key={`rename-${String(dialog.nonce)}`}
        open={dialog.kind === "rename"}
        onClose={close}
        title="Rename list"
        confirmLabel="Save name"
        initialName={list.name}
        onSubmit={async (name) => {
          await renameList(list.id, name);
          close();
        }}
      />
      <DeleteListDialog
        key={`delete-${String(dialog.nonce)}`}
        open={dialog.kind === "delete"}
        onClose={close}
        name={list.name}
        onConfirm={async () => {
          await deleteList(list.id);
          void navigate("/lists", { replace: true });
        }}
      />
    </section>
  );
}
