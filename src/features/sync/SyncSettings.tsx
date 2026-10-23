import { useCallback, useEffect, useId, useState } from "react";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { ProgressBar } from "../../components/ProgressBar";
import {
  checkSync,
  configureSync,
  disconnectSync,
  getSyncStatus,
  initializeSync,
  onSyncProgress,
  removeSyncToken,
  saveSyncToken,
  syncNow,
  type FirstSync,
  type RemoteCheck,
  type Resolution,
  type Side,
  type SyncConflict,
  type SyncOutcome,
  type SyncProgress,
  type SyncStatus,
} from "../../services/sync";
import { NativeError } from "../../transport/native";
import { ConflictDialog } from "./ConflictDialog";
import { FirstSyncDialog, type FirstChoice } from "./FirstSyncDialog";
import {
  deriveStatus,
  formatWhen,
  isEmptySummary,
  plural,
  progressText,
  recovery,
  summaryText,
  type Recovery,
} from "./syncText";
import "./sync.css";

type Busy = "connect" | "check" | "sync" | "token" | "disconnect";

type Notice = { kind: "ok"; text: string } | { kind: "error"; recovery: Recovery; code: string };

const DEFAULT_REPO = "MuDraft-Sync";

function toError(err: unknown): NativeError {
  if (err instanceof NativeError) return err;
  return new NativeError("unknown", err instanceof Error ? err.message : String(err));
}

function Steps() {
  return (
    <details className="sync-steps" open>
      <summary>How to set it up</summary>
      <ol>
        <li>
          On github.com, create a <strong>new repository</strong> named <code>{DEFAULT_REPO}</code>{" "}
          and choose <strong>Private</strong>. Ticking “Add a README file” is fine. Use it only for
          MuDraft, never for anything else.
        </li>
        <li>
          Create a <strong>fine-grained personal access token</strong>: your profile picture →
          Settings → Developer settings → Personal access tokens → Fine-grained tokens → Generate
          new token. Under Repository access choose “Only select repositories” and pick{" "}
          <code>{DEFAULT_REPO}</code>. Under Repository permissions set <strong>Contents</strong> to
          “Read and write” (Metadata stays “Read-only”). Grant nothing else. Copy the token; it
          starts with <code>github_pat_</code>.
        </li>
        <li>Enter the owner (your GitHub user name), the repository, and the token below.</li>
        <li>
          On your other computer, make a separate token the same way and connect it to the same
          repository. Each computer keeps its own token; tokens are never synced.
        </li>
      </ol>
      <p className="setting-hint">
        MuDraft never asks for your GitHub password. A private repository is visible only to you and
        people you invite, but it isn’t end-to-end encrypted: GitHub can read what’s stored there.
      </p>
    </details>
  );
}

/** Settings → GitHub Sync. Everything is manual: nothing syncs until Sync Now. */
export function SyncSettings() {
  const ownerId = useId();
  const repoId = useId();
  const tokenId = useId();
  const tokenHint = useId();
  const alsoTokenId = useId();
  const [status, setStatus] = useState<SyncStatus | null>(null);
  const [loadError, setLoadError] = useState<NativeError | null>(null);
  const [check, setCheck] = useState<RemoteCheck | null>(null);
  const [busy, setBusy] = useState<Busy | null>(null);
  const [progress, setProgress] = useState<SyncProgress | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [firstSync, setFirstSync] = useState<RemoteCheck | null>(null);
  const [pendingFirst, setPendingFirst] = useState<FirstSync | undefined>(undefined);
  const [conflicts, setConflicts] = useState<SyncConflict[]>([]);
  const [choices, setChoices] = useState<Record<string, Side>>({});
  const [editing, setEditing] = useState(false);
  const [replacingToken, setReplacingToken] = useState(false);
  const [confirmDisconnect, setConfirmDisconnect] = useState(false);
  const [alsoRemoveToken, setAlsoRemoveToken] = useState(false);
  const [owner, setOwner] = useState("");
  const [repo, setRepo] = useState(DEFAULT_REPO);
  const [token, setToken] = useState("");
  const [formError, setFormError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const s = await getSyncStatus();
      setStatus(s);
      setLoadError(null);
      return s;
    } catch (err) {
      setLoadError(toError(err));
      return null;
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    getSyncStatus().then(
      (s) => {
        if (!cancelled) setStatus(s);
      },
      (err: unknown) => {
        if (!cancelled) setLoadError(toError(err));
      },
    );
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let stopped = false;
    onSyncProgress(setProgress).then(
      (u) => {
        if (stopped) u();
        else unlisten = u;
      },
      () => undefined,
    );
    return () => {
      stopped = true;
      unlisten?.();
    };
  }, []);

  const repository =
    status?.owner && status.repo ? `github.com/${status.owner}/${status.repo}` : "";
  const fail = (err: unknown) => {
    const e = toError(err);
    setNotice({ kind: "error", code: e.code, recovery: recovery(e.code, e.message) });
  };

  const describeCheck = (c: RemoteCheck, s: SyncStatus | null): Notice => {
    const where = s?.owner && s.repo ? `github.com/${s.owner}/${s.repo}` : "the repository";
    if (c.kind === "unrelated")
      return { kind: "error", code: "sync_setup", recovery: recovery("sync_setup", "other files") };
    if (c.sameSync === false)
      return {
        kind: "error",
        code: "sync_setup",
        recovery: recovery("sync_setup", "different sync data"),
      };
    if (c.kind === "empty" || c.kind === "uninitialised")
      return {
        kind: "ok",
        text: `Connected to ${where} (private). It’s ready for its first upload.`,
      };
    if (c.sameSync === null)
      return {
        kind: "ok",
        text: `Connected to ${where} (private). It holds a MuDraft library with ${plural(c.remote?.albums ?? 0, "album")}. Run the first sync to connect this computer.`,
      };
    return {
      kind: "ok",
      text: `Connected to ${where} (private). ${
        isEmptySummary(c.remoteChanges)
          ? "No new changes on GitHub."
          : `Changes on GitHub since this computer last synced: ${summaryText(c.remoteChanges)}.`
      }`,
    };
  };

  const handleOutcome = async (out: SyncOutcome) => {
    switch (out.status) {
      case "upToDate":
        setNotice({ kind: "ok", text: "Up to date: nothing to sync." });
        setConflicts([]);
        setCheck(null);
        break;
      case "synced": {
        const got = summaryText(out.received);
        const sent = summaryText(out.sent);
        setNotice({
          kind: "ok",
          text: [
            `Synced with ${repository || "GitHub"}.`,
            got ? `Received: ${got}.` : "Nothing new to receive.",
            sent ? `Sent: ${sent}.` : "Nothing new to send.",
            out.backupFileName
              ? `A backup from before the sync was saved as “${out.backupFileName}”.`
              : "",
          ]
            .filter(Boolean)
            .join(" "),
        });
        setConflicts([]);
        setChoices({});
        setPendingFirst(undefined);
        setCheck(null);
        break;
      }
      case "conflicts":
        setNotice(null);
        setConflicts(out.conflicts);
        break;
      case "needsConfirmation":
        setFirstSync(await checkSync());
        break;
    }
  };

  const runSync = async (options: { firstSync?: FirstSync; resolutions?: Resolution[] }) => {
    setBusy("sync");
    setProgress(null);
    setNotice(null);
    try {
      await handleOutcome(await syncNow(options));
    } catch (err) {
      fail(err);
    } finally {
      setBusy(null);
      setProgress(null);
      await refresh();
    }
  };

  /** Sync Now: a computer that has never synced previews both libraries first. */
  const startSync = async () => {
    if (status?.linked) {
      await runSync({});
      return;
    }
    setBusy("check");
    setNotice(null);
    try {
      setFirstSync(await checkSync());
    } catch (err) {
      fail(err);
    } finally {
      setBusy(null);
      await refresh();
    }
  };

  const chooseFirst = async (choice: FirstChoice) => {
    setFirstSync(null);
    if (choice === "upload") {
      setBusy("sync");
      setNotice(null);
      try {
        await handleOutcome(await initializeSync(true));
      } catch (err) {
        fail(err);
      } finally {
        setBusy(null);
        setProgress(null);
        await refresh();
      }
      return;
    }
    setPendingFirst(choice);
    await runSync({ firstSync: choice });
  };

  const applyChoices = async () => {
    const resolutions = conflicts
      .filter((c) => choices[c.id])
      .map((c) => ({ id: c.id, choice: choices[c.id] as Side }));
    const first = status?.linked ? undefined : pendingFirst;
    await runSync(first ? { firstSync: first, resolutions } : { resolutions });
  };

  const cancelConflicts = () => {
    setConflicts([]);
    setNotice({
      kind: "ok",
      text: "Nothing changed. Your changes stay pending until the conflicts are resolved.",
    });
  };

  const connect = async () => {
    setFormError(null);
    const needToken = !status?.hasToken || replacingToken;
    if (!owner.trim() || !repo.trim()) {
      setFormError("Enter the repository owner and name.");
      return;
    }
    if (needToken && !token.trim()) {
      setFormError("Paste the fine-grained token you created for this computer.");
      return;
    }
    setBusy("connect");
    setNotice(null);
    try {
      let s = await configureSync(owner, repo);
      if (token.trim()) {
        s = await saveSyncToken(token);
        setToken("");
      }
      setStatus(s);
      setEditing(false);
      setReplacingToken(false);
      const c = await checkSync();
      setCheck(c);
      setNotice(describeCheck(c, s));
      if (!s.linked && c.kind !== "unrelated") setFirstSync(c);
    } catch (err) {
      fail(err);
    } finally {
      setBusy(null);
      await refresh();
    }
  };

  const testConnection = async () => {
    setBusy("check");
    setNotice(null);
    try {
      const c = await checkSync();
      setCheck(c);
      setNotice(describeCheck(c, status));
    } catch (err) {
      fail(err);
    } finally {
      setBusy(null);
      await refresh();
    }
  };

  const saveToken = async () => {
    if (!token.trim()) {
      setFormError("Paste the new token first.");
      return;
    }
    setFormError(null);
    setBusy("token");
    try {
      setStatus(await saveSyncToken(token));
      setToken("");
      setReplacingToken(false);
      setNotice({
        kind: "ok",
        text: "New token saved in this computer’s secure credential store.",
      });
    } catch (err) {
      fail(err);
    } finally {
      setBusy(null);
    }
  };

  const removeToken = async () => {
    setBusy("token");
    try {
      setStatus(await removeSyncToken());
      setNotice({
        kind: "ok",
        text: "Token removed from this computer. To revoke it completely, delete it on GitHub under Settings → Developer settings → Fine-grained tokens.",
      });
    } catch (err) {
      fail(err);
    } finally {
      setBusy(null);
    }
  };

  const disconnect = async () => {
    setBusy("disconnect");
    try {
      let s = await disconnectSync();
      if (alsoRemoveToken) s = await removeSyncToken();
      setStatus(s);
      setCheck(null);
      setConflicts([]);
      setChoices({});
      setPendingFirst(undefined);
      setConfirmDisconnect(false);
      setNotice({
        kind: "ok",
        text: "This computer no longer syncs. Your library here and the data on GitHub are unchanged.",
      });
    } catch (err) {
      fail(err);
    } finally {
      setBusy(null);
    }
  };

  const view = deriveStatus(status, busy === "sync", check);
  const working = busy ? "Working…" : undefined;
  const showForm = !status?.configured || editing;
  const tokenField = (
    <div className="setting-row">
      <div className="setting-text">
        <label className="setting-label" htmlFor={tokenId}>
          Access token for this computer
        </label>
        <span id={tokenHint} className="setting-hint">
          Starts with github_pat_. Saved only in this computer’s secure credential store (Keychain
          on macOS, Credential Manager on Windows) and never shown again.
        </span>
      </div>
      <input
        id={tokenId}
        className="import-select sync-input"
        type="password"
        autoComplete="off"
        spellCheck={false}
        aria-describedby={tokenHint}
        value={token}
        onChange={(e) => {
          setToken(e.target.value);
        }}
      />
    </div>
  );

  return (
    <section className="panel section" aria-labelledby="settings-sync">
      <h2 id="settings-sync" className="section-title">
        GitHub Sync
      </h2>
      <p className="setting-hint">
        Keep this library the same on your computers through a private GitHub repository you own.
        MuDraft syncs only when you choose Sync Now; it never syncs in the background.
      </p>

      <div className="sync-status" role="status">
        <span className="sync-badge" data-tone={view.tone}>
          {view.label}
        </span>
        <span className="setting-hint">
          {busy === "sync" ? progressText(progress) : view.detail}
        </span>
      </div>
      {busy === "sync" &&
        (progress?.phase === "uploading" || progress?.phase === "downloading") &&
        progress.total > 0 && (
          <ProgressBar
            label={progress.phase === "uploading" ? "Uploading" : "Downloading"}
            value={progress.done}
            max={progress.total}
          />
        )}
      {loadError && (
        <p className="match-note match-note-error" role="alert">
          Couldn’t read the sync settings: {loadError.message}
        </p>
      )}

      {showForm ? (
        <>
          <Steps />
          <form
            className="sync-form"
            onSubmit={(e) => {
              e.preventDefault();
              void connect();
            }}
            noValidate
          >
            <div className="setting-row">
              <div className="setting-text">
                <label className="setting-label" htmlFor={ownerId}>
                  Repository owner
                </label>
                <span className="setting-hint">Your GitHub user or organisation name.</span>
              </div>
              <input
                id={ownerId}
                className="import-select sync-input"
                autoComplete="off"
                spellCheck={false}
                value={owner}
                onChange={(e) => {
                  setOwner(e.target.value);
                }}
              />
            </div>
            <div className="setting-row">
              <div className="setting-text">
                <label className="setting-label" htmlFor={repoId}>
                  Repository name
                </label>
                <span className="setting-hint">The private repository you made for MuDraft.</span>
              </div>
              <input
                id={repoId}
                className="import-select sync-input"
                autoComplete="off"
                spellCheck={false}
                value={repo}
                onChange={(e) => {
                  setRepo(e.target.value);
                }}
              />
            </div>
            {(!status?.hasToken || replacingToken) && tokenField}
            {owner.trim() && repo.trim() && (
              <p className="setting-hint">
                Data will go only to the <code>sync/</code> folder of{" "}
                <strong>
                  github.com/{owner.trim()}/{repo.trim()}
                </strong>
                .
              </p>
            )}
            {formError && (
              <p className="match-note match-note-error" role="alert">
                {formError}
              </p>
            )}
            <div className="page-actions">
              <Button type="submit" variant="primary" disabledReason={working}>
                {busy === "connect" ? "Connecting…" : "Connect"}
              </Button>
              {editing && (
                <Button
                  onClick={() => {
                    setEditing(false);
                  }}
                >
                  Cancel
                </Button>
              )}
            </div>
          </form>
        </>
      ) : (
        <>
          <dl className="facts">
            <dt>Repository</dt>
            <dd>
              <strong>{repository}</strong> (private)
              {status.branch ? `, branch ${status.branch}` : ""}. MuDraft writes only to its{" "}
              <code>sync/</code> folder.
            </dd>
            <dt>Token</dt>
            <dd>
              {status.hasToken
                ? "Saved in this computer’s secure credential store"
                : "None saved on this computer"}
            </dd>
            <dt>Last successful sync</dt>
            <dd>{formatWhen(status.lastSyncedAt)}</dd>
            <dt>Changes on this computer</dt>
            <dd>
              {!status.linked
                ? "Not synced yet"
                : isEmptySummary(status.pending)
                  ? "None"
                  : summaryText(status.pending)}
            </dd>
            <dt>Changes on GitHub</dt>
            <dd>
              {check?.sameSync
                ? isEmptySummary(check.remoteChanges)
                  ? "None"
                  : summaryText(check.remoteChanges)
                : "Not checked yet (Test Connection looks without changing anything)"}
            </dd>
          </dl>

          {replacingToken && (
            <form
              className="sync-form"
              onSubmit={(e) => {
                e.preventDefault();
                void saveToken();
              }}
              noValidate
            >
              {tokenField}
              {formError && (
                <p className="match-note match-note-error" role="alert">
                  {formError}
                </p>
              )}
              <div className="page-actions">
                <Button type="submit" variant="primary" disabledReason={working}>
                  Save token
                </Button>
                <Button
                  onClick={() => {
                    setReplacingToken(false);
                    setToken("");
                  }}
                >
                  Cancel
                </Button>
              </div>
            </form>
          )}

          <div className="page-actions">
            <Button
              variant="primary"
              disabledReason={!status.hasToken ? "Add a token for this computer first" : working}
              onClick={() => void startSync()}
            >
              {busy === "sync"
                ? "Syncing…"
                : status.unresolvedConflicts > 0
                  ? "Resolve conflicts…"
                  : status.linked
                    ? "Sync Now"
                    : "Set up first sync…"}
            </Button>
            <Button
              disabledReason={!status.hasToken ? "Add a token first" : working}
              onClick={() => void testConnection()}
            >
              {busy === "check" ? "Testing…" : "Test Connection"}
            </Button>
            {!replacingToken && (
              <Button
                disabledReason={working}
                onClick={() => {
                  setReplacingToken(true);
                  setFormError(null);
                }}
              >
                {status.hasToken ? "Replace token…" : "Add token…"}
              </Button>
            )}
            {status.hasToken && (
              <Button variant="ghost" disabledReason={working} onClick={() => void removeToken()}>
                Remove token from this computer
              </Button>
            )}
            <Button
              variant="ghost"
              disabledReason={working}
              onClick={() => {
                setOwner(status.owner ?? "");
                setRepo(status.repo ?? DEFAULT_REPO);
                setEditing(true);
              }}
            >
              Change repository…
            </Button>
            <Button
              variant="danger"
              disabledReason={working}
              onClick={() => {
                setAlsoRemoveToken(false);
                setConfirmDisconnect(true);
              }}
            >
              Disconnect…
            </Button>
          </div>
        </>
      )}

      {notice?.kind === "ok" && (
        <p className="banner banner-ok sync-notice" role="status">
          {notice.text}
        </p>
      )}
      {notice?.kind === "error" && (
        <div className="match-note match-note-error sync-notice" role="alert">
          <p className="sync-notice-title">{notice.recovery.title}</p>
          <p>{notice.recovery.advice}</p>
          <div className="page-actions">
            {notice.recovery.action === "retry" && status?.configured && (
              <Button disabledReason={working} onClick={() => void startSync()}>
                Try again
              </Button>
            )}
            {notice.recovery.action === "token" && !replacingToken && status?.configured && (
              <Button
                onClick={() => {
                  setReplacingToken(true);
                }}
              >
                Enter a new token
              </Button>
            )}
            {notice.recovery.action === "repository" && status?.configured && (
              <Button
                onClick={() => {
                  setOwner(status.owner ?? "");
                  setRepo(status.repo ?? DEFAULT_REPO);
                  setEditing(true);
                }}
              >
                Check repository details
              </Button>
            )}
            {notice.recovery.action === "disconnect" && (
              <Button
                variant="danger"
                onClick={() => {
                  setConfirmDisconnect(true);
                }}
              >
                Disconnect…
              </Button>
            )}
          </div>
          <p className="setting-hint">Error code: {notice.code}</p>
        </div>
      )}

      <FirstSyncDialog
        check={firstSync}
        repository={repository}
        busy={busy !== null}
        onChoose={(c) => void chooseFirst(c)}
        onClose={() => {
          setFirstSync(null);
        }}
      />
      <ConflictDialog
        conflicts={conflicts}
        choices={choices}
        busy={busy !== null}
        onChoose={(id, side) => {
          setChoices((prev) => ({ ...prev, [id]: side }));
        }}
        onApply={() => void applyChoices()}
        onCancel={cancelConflicts}
      />
      <Dialog
        open={confirmDisconnect}
        title="Disconnect this computer?"
        onClose={() => {
          setConfirmDisconnect(false);
        }}
        actions={
          <>
            <Button
              onClick={() => {
                setConfirmDisconnect(false);
              }}
            >
              Cancel
            </Button>
            <Button variant="danger" disabledReason={working} onClick={() => void disconnect()}>
              Disconnect
            </Button>
          </>
        }
      >
        <p>
          This computer stops syncing with {repository || "the repository"}. Your library here and
          the data on GitHub stay as they are
          {status?.pendingLocalChanges ? ", including changes not yet synced" : ""}. Connecting
          again later asks how to combine the libraries.
        </p>
        <label className="sync-checkbox" htmlFor={alsoTokenId}>
          <input
            id={alsoTokenId}
            type="checkbox"
            checked={alsoRemoveToken}
            onChange={(e) => {
              setAlsoRemoveToken(e.target.checked);
            }}
          />
          Also remove the saved token from this computer
        </label>
      </Dialog>
    </section>
  );
}
