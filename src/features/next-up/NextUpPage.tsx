import { useEffect, useState } from "react";
import { Link } from "react-router";
import { Button } from "../../components/Button";
import { PageHeader } from "../../components/PageHeader";
import { ErrorState, LoadingState } from "../../components/States";
import { UndoBanner } from "../../components/UndoBanner";
import { useLibraryVersion } from "../../services/libraryEvents";
import { undoLog, type Logged } from "../../services/listening";
import {
  clearNextUp,
  nextUpState,
  resetNextUpPool,
  rollNextUp,
  type CurrentPick,
  type Method,
  type NextUpSlot,
  type NextUpState,
} from "../../services/nextUp";
import { ListenDialog } from "../listening/ListenDialog";
import { loggedMessage } from "../listening/loggedMessage";
import { ALL_AGNOSTIC, emptyMessage, fromMethod, type Selection } from "./criteria";
import { MethodChooser } from "./MethodChooser";
import { NextUpResult } from "./NextUpResult";
import "./nextUp.css";

type Load =
  | { kind: "loading" }
  | { kind: "error"; code: string; message: string }
  | { kind: "ready"; state: NextUpState };

type Notice =
  | { kind: "empty"; text: string; listenListLink: boolean }
  | { kind: "exhausted"; poolSize: number; slot: number }
  | { kind: "ok"; text: string }
  | { kind: "error"; text: string };

function errorInfo(err: unknown) {
  const code = typeof err === "object" && err && "code" in err ? String(err.code) : "unknown";
  return { code, message: err instanceof Error ? err.message : String(err) };
}

/**
 * Next Up: up to `maxSlots` albums, each slot picked its own way with its own reroll pool.
 * Loading this page only reads the stored picks — it never rolls, so they survive
 * navigation and restarts until you reroll, change, remove, or listen to them.
 */
export function NextUpPage() {
  const version = useLibraryVersion();
  const [load, setLoad] = useState<Load>({ kind: "loading" });
  const [reload, setReload] = useState(0);
  /** The slot whose choices are open: an existing slot, or the free one being added. */
  const [editing, setEditing] = useState<number | null>(null);
  const [guidedOpen, setGuidedOpen] = useState(false);
  const [selection, setSelection] = useState<Selection>(ALL_AGNOSTIC);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [listening, setListening] = useState<CurrentPick | null>(null);
  const [undo, setUndo] = useState<Logged | null>(null);

  useEffect(() => {
    let cancelled = false;
    nextUpState().then(
      (state) => {
        if (!cancelled) setLoad({ kind: "ready", state });
      },
      (err: unknown) => {
        if (!cancelled) setLoad({ kind: "error", ...errorInfo(err) });
      },
    );
    return () => {
      cancelled = true;
    };
  }, [version, reload]);

  const act = async (task: () => Promise<void>) => {
    setBusy(true);
    setNotice(null);
    try {
      await task();
    } catch (err) {
      setNotice({ kind: "error", text: errorInfo(err).message });
    } finally {
      setBusy(false);
      setReload((n) => n + 1);
    }
  };

  const roll = (slot: number, method: Method) =>
    void act(async () => {
      const out = await rollNextUp(crypto.randomUUID(), slot, method);
      if (out.kind === "picked") {
        setEditing(null);
        setUndo(null);
      } else if (out.kind === "empty") {
        setNotice({
          kind: "empty",
          text: emptyMessage(out.reason),
          listenListLink: out.reason === "listen_list_empty",
        });
      } else {
        setNotice({ kind: "exhausted", poolSize: out.poolSize, slot });
      }
    });

  const choose = (slot: number, method: Method | null | undefined) => {
    setEditing(slot);
    setNotice(null);
    setSelection(fromMethod(method));
    setGuidedOpen(method?.mode === "guided");
  };

  if (load.kind === "loading") {
    return (
      <section>
        <PageHeader title="Next Up" />
        <LoadingState label="Loading Next Up…" />
      </section>
    );
  }
  if (load.kind === "error") {
    return (
      <section>
        <PageHeader title="Next Up" />
        <ErrorState
          title="Couldn’t load Next Up"
          message={load.message}
          code={load.code}
          onRetry={() => {
            setReload((n) => n + 1);
          }}
        />
      </section>
    );
  }

  const { slots, maxSlots, nextFreeSlot } = load.state;
  // With nothing in Next Up, the choices are the page.
  const chooserSlot = editing ?? (slots.length === 0 ? nextFreeSlot : null);
  const editingSlot: NextUpSlot | undefined = slots.find((s) => s.slot === chooserSlot);
  const full = nextFreeSlot === null;

  let actions = null;
  if (chooserSlot !== null && slots.length > 0) {
    actions = (
      <Button
        onClick={() => {
          setEditing(null);
          setNotice(null);
        }}
      >
        Cancel
      </Button>
    );
  } else if (chooserSlot === null && nextFreeSlot !== null) {
    actions = (
      <Button
        variant="primary"
        onClick={() => {
          choose(nextFreeSlot, null);
        }}
      >
        Add Next Up
      </Button>
    );
  }

  return (
    <section className="next-up-page">
      <PageHeader
        title="Next Up"
        description={`Up to ${String(maxSlots)} albums to listen to next, each picked its own way from your Listen List.`}
        {...(actions ? { actions } : {})}
      />
      {undo && (
        <UndoBanner
          message={loggedMessage(undo)}
          onUndo={() => undoLog(undo.undo)}
          onDismiss={() => {
            setUndo(null);
          }}
        />
      )}
      {notice && (
        <div
          className={`banner ${notice.kind === "ok" ? "banner-ok" : ""} next-up-notice`}
          role={notice.kind === "error" ? "alert" : "status"}
        >
          {notice.kind === "exhausted" ? (
            <>
              <span className="banner-text">
                You’ve seen all {notice.poolSize} matching album{notice.poolSize === 1 ? "" : "s"}{" "}
                this round. Reset the pool to let them come up again, or change your choices.
              </span>
              <Button
                disabledReason={busy ? "Working…" : undefined}
                onClick={() =>
                  void act(async () => {
                    await resetNextUpPool(notice.slot);
                    setNotice({
                      kind: "ok",
                      text: "Pool reset. Every matching album can come up again.",
                    });
                  })
                }
              >
                Reset pool
              </Button>
            </>
          ) : (
            <span className="banner-text">
              {notice.text}
              {notice.kind === "empty" && notice.listenListLink && (
                <>
                  {" "}
                  <Link to="/listen-list">Go to Listen List</Link>
                </>
              )}
            </span>
          )}
        </div>
      )}

      {chooserSlot !== null ? (
        <>
          {editingSlot?.current ? (
            <p className="setting-hint">
              “{editingSlot.current.item.title}” stays in your Next Up until a new album is picked
              here.
            </p>
          ) : (
            slots.length > 0 &&
            !editingSlot && (
              <p className="setting-hint">
                Choose how to pick another album. Your other Next Up picks stay as they are.
              </p>
            )
          )}
          <MethodChooser
            selection={selection}
            onSelectionChange={setSelection}
            guidedOpen={guidedOpen}
            onGuidedOpenChange={setGuidedOpen}
            busy={busy}
            onRoll={(method) => {
              roll(chooserSlot, method);
            }}
          />
        </>
      ) : (
        <section className="next-up-result" aria-labelledby="next-up-current">
          <div className="next-up-list-head">
            <h2 id="next-up-current" className="section-title">
              Your Next Up
            </h2>
            <span className="setting-hint">
              {slots.length} of {maxSlots}
              {full ? " — remove one to add another" : ""}
            </span>
          </div>
          <ol className="next-up-slots">
            {slots.map((s) => (
              <li key={s.slot}>
                <NextUpResult
                  slot={s}
                  busy={busy}
                  onRoll={() => {
                    if (s.session) roll(s.slot, s.session.method);
                  }}
                  onChangeChoices={() => {
                    choose(s.slot, s.session?.method);
                  }}
                  onRemove={() =>
                    void act(async () => {
                      await clearNextUp(s.slot);
                      setNotice({
                        kind: "ok",
                        text: s.current
                          ? `“${s.current.item.title}” removed from Next Up. It stays on your Listen List.`
                          : "Removed from Next Up.",
                      });
                    })
                  }
                  onMarkListened={() => {
                    setListening(s.current);
                  }}
                />
              </li>
            ))}
          </ol>
        </section>
      )}

      {listening && (
        <ListenDialog
          albumTitle={listening.item.title}
          editionId={listening.editionId}
          attemptId={listening.attemptId}
          onClose={() => {
            setListening(null);
          }}
          onLogged={(logged) => {
            setListening(null);
            setNotice(null);
            setUndo(logged);
            setEditing(null);
            setReload((n) => n + 1);
          }}
        />
      )}
    </section>
  );
}
