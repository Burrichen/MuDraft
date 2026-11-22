import { useEffect, useState } from "react";
import { Link, useLocation, useNavigate } from "react-router";
import { Button } from "../../components/Button";
import { PageHeader } from "../../components/PageHeader";
import { ProgressBar } from "../../components/ProgressBar";
import { EmptyState, ErrorState, LoadingState } from "../../components/States";
import { useLibraryVersion } from "../../services/libraryEvents";
import { createList, listLists, type ListSummary } from "../../services/lists";
import { ImportListDialog } from "./ImportListDialog";
import { NameListDialog } from "./ListDialogs";
import "./lists.css";

type Load =
  | { kind: "loading" }
  | { kind: "error"; code: string; message: string }
  | { kind: "ready"; lists: ListSummary[] };

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

function ListCard({ list, from }: { list: ListSummary; from: string }) {
  const { total, listened, onListenList } = list.progress;
  return (
    <li>
      <Link className="list-card" to={`/lists/${list.id}`} state={{ from }}>
        <span className="list-card-title">{list.name}</span>
        <span className="list-muted">
          {total} album{total === 1 ? "" : "s"}
          {list.sourceUrl && ` · from ${host(list.sourceUrl)}`}
        </span>
        <ProgressBar label="Listened" value={listened} max={total} />
        {onListenList > 0 && (
          <span className="list-muted">{onListenList} more on your Listen List</span>
        )}
      </Link>
    </li>
  );
}

/** Every list, each with how much of it is in the Collection. */
export function ListsPage() {
  const version = useLibraryVersion();
  const location = useLocation();
  const navigate = useNavigate();
  const [load, setLoad] = useState<Load>({ kind: "loading" });
  const [retry, setRetry] = useState(0);
  const [dialog, setDialog] = useState<{ kind: "new" | "import" | null; nonce: number }>({
    kind: null,
    nonce: 0,
  });

  useEffect(() => {
    let cancelled = false;
    listLists().then(
      (lists) => {
        if (!cancelled) setLoad({ kind: "ready", lists });
      },
      (err: unknown) => {
        if (!cancelled) setLoad({ kind: "error", ...errorInfo(err) });
      },
    );
    return () => {
      cancelled = true;
    };
  }, [version, retry]);

  const show = (kind: "new" | "import") => {
    setDialog((d) => ({ kind, nonce: d.nonce + 1 }));
  };
  const close = () => {
    setDialog((d) => ({ ...d, kind: null }));
  };
  const open = (id: string) => {
    void navigate(`/lists/${id}`, { state: { from: location.pathname } });
  };

  const actions = (
    <>
      <Button
        onClick={() => {
          show("import");
        }}
      >
        Import from web page
      </Button>
      <Button
        variant="primary"
        onClick={() => {
          show("new");
        }}
      >
        New list
      </Button>
    </>
  );

  return (
    <section>
      <PageHeader
        title="Lists"
        description="Curated album lists. Albums in your Collection count as listened to."
        actions={actions}
      />
      {load.kind === "loading" && <LoadingState label="Loading lists…" />}
      {load.kind === "error" && (
        <ErrorState
          title="Couldn’t load lists"
          message={load.message}
          code={load.code}
          onRetry={() => {
            setRetry((n) => n + 1);
          }}
        />
      )}
      {load.kind === "ready" &&
        (load.lists.length === 0 ? (
          <EmptyState
            icon="lists"
            title="No lists yet"
            description="Make a list of albums to work through, or import a ranked list such as Pitchfork’s 100 best indie rock albums from its web page."
          />
        ) : (
          <ul className="lists-grid" aria-label="Lists">
            {load.lists.map((list) => (
              <ListCard key={list.id} list={list} from={location.pathname} />
            ))}
          </ul>
        ))}
      <NameListDialog
        key={`new-${String(dialog.nonce)}`}
        open={dialog.kind === "new"}
        onClose={close}
        title="New list"
        confirmLabel="Create list"
        onSubmit={async (name) => {
          open(await createList(name, null));
        }}
      />
      <ImportListDialog
        key={`import-${String(dialog.nonce)}`}
        open={dialog.kind === "import"}
        onClose={close}
        onImported={(id) => {
          close();
          open(id);
        }}
      />
    </section>
  );
}
