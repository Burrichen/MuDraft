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
  type Method,
  type NextUpState,
} from "../../services/nextUp";
import { ListenDialog } from "../listening/ListenDialog";
import { loggedMessage } from "../listening/loggedMessage";
import { emptyMessage, fromMethod, type Selection } from "./criteria";
import { MethodChooser } from "./MethodChooser";
import { NextUpResult } from "./NextUpResult";
import "./nextUp.css";

type Load =
  | { kind: "loading" }
  | { kind: "error"; code: string; message: string }
  | { kind: "ready"; state: NextUpState };

type Notice =
  | { kind: "empty"; text: string; listenListLink: boolean }
  | { kind: "exhausted"; poolSize: number }
  | { kind: "ok"; text: string }
  | { kind: "error"; text: string };

function errorInfo(err: unknown) {
  const code = typeof err === "object" && err && "code" in err ? String(err.code) : "unknown";
  return { code, message: err instanceof Error ? err.message : String(err) };
}

/**
 * Next Up. Loading this page only reads the stored pick — it never rolls, so the result
 * survives navigation and restarts until you reroll, change, clear, or listen to it.
 */
export function NextUpPage() {
  const version = useLibraryVersion();
  const [load, setLoad] = useState<Load>({ kind: "loading" });
  const [reload, setReload] = useState(0);
  const [choosing, setChoosing] = useState(false);
  const [guidedOpen, setGuidedOpen] = useState(false);
  const [selection, setSelection] = useState<Selection | null>(null);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [listening, setListening] = useState(false);
  const [undo, setUndo] = useState<Logged | null>(null);

  useEffect(() => {
    let cancelled = false;
    nextUpState().then(
      (state) => {
        if (cancelled) return;
        setLoad({ kind: "ready", state });
        // Start the chooser from the open session's choices (Change Choices, restarts).
        setSelection((s) => s ?? fromMethod(state.session?.method));
        setGuidedOpen((open) => open || state.session?.method.mode === "guided");
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

  const roll = (method: Method) =>
    void act(async () => {
      const out = await rollNextUp(crypto.randomUUID(), method);
      if (out.kind === "picked") {
        setChoosing(false);
        setUndo(null);
      } else if (out.kind === "empty") {
        setNotice({
          kind: "empty",
          text: emptyMessage(out.reason),
          listenListLink: out.reason === "listen_list_empty",
        });
      } else {
        setNotice({ kind: "exhausted", poolSize: out.poolSize });
      }
    });

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

  const { current, session } = load.state;
  const showResult = current !== null && !choosing;

  return (
    <section className="next-up-page">
      <PageHeader
        title="Next Up"
        description="One album to listen to next, picked from your Listen List."
        {...(current && choosing
          ? {
              actions: (
                <Button
                  onClick={() => {
                    setChoosing(false);
                    setNotice(null);
                  }}
                >
                  Back to current pick
                </Button>
              ),
            }
          : {})}
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
                    await resetNextUpPool();
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

      {showResult ? (
        <NextUpResult
          pick={current}
          session={session}
          busy={busy}
          onReroll={() => {
            if (session) roll(session.method);
          }}
          onChangeChoices={() => {
            setChoosing(true);
            setNotice(null);
            setSelection(fromMethod(session?.method));
            setGuidedOpen(session?.method.mode === "guided");
          }}
          onClear={() =>
            void act(async () => {
              await clearNextUp();
              setNotice({
                kind: "ok",
                text: "Next Up cleared. Nothing is picked until you choose again.",
              });
            })
          }
          onMarkListened={() => {
            setListening(true);
          }}
        />
      ) : (
        <>
          {current && (
            <p className="setting-hint">
              “{current.item.title}” stays your Next Up until a new album is picked.
            </p>
          )}
          <MethodChooser
            selection={selection ?? fromMethod(null)}
            onSelectionChange={setSelection}
            guidedOpen={guidedOpen}
            onGuidedOpenChange={setGuidedOpen}
            busy={busy}
            onRoll={roll}
          />
        </>
      )}

      {listening && current && (
        <ListenDialog
          albumTitle={current.item.title}
          editionId={current.editionId}
          attemptId={current.attemptId}
          onClose={() => {
            setListening(false);
          }}
          onLogged={(logged) => {
            setListening(false);
            setNotice(null);
            setUndo(logged);
            setChoosing(false);
            setReload((n) => n + 1);
          }}
        />
      )}
    </section>
  );
}
