import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { checkForUpdate, type AvailableUpdate } from "../transport/updater";
import { UpdateContext, type UpdateStatus, type UpdateValue } from "./updateContext";

type Install =
  | { kind: "idle" }
  | { kind: "running"; fraction: number | null }
  | { kind: "error"; message: string };

const message = (err: unknown) => (err instanceof Error ? err.message : String(err));

/** Release notes as short paragraphs and bullet points (bold markers dropped). */
export function ReleaseNotes({ text }: { text: string }) {
  const blocks: ({ kind: "p"; text: string } | { kind: "ul"; items: string[] })[] = [];
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.replace(/\*\*/g, "").trim();
    if (!line || line.startsWith("#")) continue;
    const item = /^[-*]\s+(.*)$/.exec(line);
    const last = blocks.at(-1);
    if (item?.[1] !== undefined) {
      if (last?.kind === "ul") last.items.push(item[1]);
      else blocks.push({ kind: "ul", items: [item[1]] });
    } else {
      blocks.push({ kind: "p", text: line });
    }
  }
  return (
    <div className="release-notes">
      {blocks.map((b, i) =>
        b.kind === "p" ? (
          <p key={i}>{b.text}</p>
        ) : (
          <ul key={i}>
            {b.items.map((t, j) => (
              <li key={j}>{t}</li>
            ))}
          </ul>
        ),
      )}
    </div>
  );
}

function UpdateDialog({ update, onLater }: { update: AvailableUpdate; onLater: () => void }) {
  const [install, setInstall] = useState<Install>({ kind: "idle" });
  const running = install.kind === "running";
  const start = () => {
    setInstall({ kind: "running", fraction: null });
    update
      .install((fraction) => {
        setInstall({ kind: "running", fraction });
      })
      .catch((err: unknown) => {
        setInstall({ kind: "error", message: message(err) });
      });
  };
  const percent = running && install.fraction !== null ? Math.round(install.fraction * 100) : null;
  return (
    <Dialog
      open
      title={`MuDraft ${update.version} is available`}
      onClose={() => {
        if (!running) onLater();
      }}
      actions={
        <>
          <Button disabledReason={running ? "Updating…" : undefined} onClick={onLater}>
            Later
          </Button>
          <Button
            variant="primary"
            disabledReason={running ? "Updating…" : undefined}
            onClick={start}
          >
            {install.kind === "error" ? "Try Again" : "Update and Restart"}
          </Button>
        </>
      }
    >
      <p>
        You have MuDraft {update.currentVersion}. Your library is kept, and it’s backed up before
        the new version changes anything.
      </p>
      {update.notes && (
        <>
          <h3 className="release-notes-title">What’s new</h3>
          <ReleaseNotes text={update.notes} />
        </>
      )}
      {running && (
        <div className="update-progress" role="status">
          <span>
            {percent === null
              ? "Downloading…"
              : percent < 100
                ? `Downloading… ${String(percent)}%`
                : "Installing… MuDraft will restart."}
          </span>
          <progress max={100} value={percent ?? undefined} aria-label="Update download" />
        </div>
      )}
      {install.kind === "error" && (
        <p className="match-note match-note-error" role="alert">
          Couldn’t update: {install.message}. Nothing was changed. Try again, or download it from
          the MuDraft releases page on GitHub.
        </p>
      )}
    </Dialog>
  );
}

/**
 * Checks once at startup for a newer release (silently skipped when offline) and offers to
 * install it with its patch notes. Settings can check again on demand.
 */
export function UpdateProvider({
  children,
  autoCheck = import.meta.env.PROD,
  unavailable = import.meta.env.DEV ? "Updates are turned off in development builds." : null,
}: {
  children: ReactNode;
  autoCheck?: boolean;
  unavailable?: string | null;
}) {
  const [status, setStatus] = useState<UpdateStatus>({ kind: "idle" });
  const [prompt, setPrompt] = useState(false);
  const started = useRef(false);

  const run = useCallback((manual: boolean) => {
    setStatus({ kind: "checking" });
    checkForUpdate().then(
      (update) => {
        if (update) {
          setStatus({ kind: "available", update });
          setPrompt(true);
        } else {
          setStatus({ kind: "current" });
        }
      },
      (err: unknown) => {
        // A startup check that fails (offline, GitHub down) stays quiet.
        setStatus(manual ? { kind: "error", message: message(err) } : { kind: "idle" });
      },
    );
  }, []);

  useEffect(() => {
    if (!autoCheck || unavailable || started.current) return;
    started.current = true;
    run(false);
  }, [autoCheck, unavailable, run]);

  const value = useMemo<UpdateValue>(
    () => ({
      status,
      unavailable,
      checkNow: () => {
        if (!unavailable) run(true);
      },
    }),
    [status, unavailable, run],
  );

  return (
    <UpdateContext.Provider value={value}>
      {children}
      {prompt && status.kind === "available" && (
        <UpdateDialog
          update={status.update}
          onLater={() => {
            setPrompt(false);
          }}
        />
      )}
    </UpdateContext.Provider>
  );
}
