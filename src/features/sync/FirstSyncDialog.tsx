import { useState } from "react";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import type { RemoteCheck, SyncCounts } from "../../services/sync";
import { isEmptyLibrary, plural } from "./syncText";

export type FirstChoice = "upload" | "download" | "merge";

interface Props {
  check: RemoteCheck | null;
  repository: string;
  busy: boolean;
  onChoose: (choice: FirstChoice) => void;
  onClose: () => void;
}

const ROWS: [keyof SyncCounts, string][] = [
  ["albums", "Albums"],
  ["collection", "In the Collection"],
  ["listenList", "On the Listen List"],
  ["listens", "Listens"],
  ["ratedTracks", "Rated tracks"],
  ["reviews", "Reviews"],
  ["tags", "Tags"],
  ["artwork", "Custom artwork"],
];

function Preview({ local, remote }: { local: SyncCounts; remote: SyncCounts | null }) {
  return (
    <table className="sync-preview">
      <caption className="visually-hidden">Libraries before the first sync</caption>
      <thead>
        <tr>
          <th scope="col">Library</th>
          <th scope="col">This computer</th>
          <th scope="col">GitHub</th>
        </tr>
      </thead>
      <tbody>
        {ROWS.map(([key, label]) => (
          <tr key={key}>
            <th scope="row">{label}</th>
            <td>{local[key]}</td>
            <td>{remote ? remote[key] : "Empty"}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** First sync on this computer: preview both libraries, then choose explicitly. */
export function FirstSyncDialog({ check, repository, busy, onChoose, onClose }: Props) {
  const [confirmReplace, setConfirmReplace] = useState(false);
  const close = () => {
    setConfirmReplace(false);
    onClose();
  };
  const working = busy ? "Syncing…" : undefined;
  let body = null;
  let actions = <Button onClick={close}>Cancel</Button>;

  if (check) {
    const localEmpty = isEmptyLibrary(check.local);
    if (check.kind === "unrelated") {
      body = (
        <p role="alert">
          {repository} holds other files, so MuDraft won’t write to it. Create a new private
          repository just for MuDraft sync.
        </p>
      );
    } else if (check.kind === "empty" || check.kind === "uninitialised") {
      body = (
        <p>
          {repository} has no MuDraft library yet. Uploading publishes this computer’s library
          there; your other computers then download or merge with it.
          {localEmpty && " This computer’s library is empty, so the synced library starts empty."}
        </p>
      );
      actions = (
        <>
          <Button onClick={close}>Cancel</Button>
          <Button
            variant="primary"
            disabledReason={working}
            onClick={() => {
              onChoose("upload");
            }}
          >
            Upload this library
          </Button>
        </>
      );
    } else if (localEmpty) {
      body = (
        <p>
          This computer’s library is empty. Downloading copies the library from {repository} here.
        </p>
      );
      actions = (
        <>
          <Button onClick={close}>Cancel</Button>
          <Button
            variant="primary"
            disabledReason={working}
            onClick={() => {
              onChoose("download");
            }}
          >
            Download remote library
          </Button>
        </>
      );
    } else if (confirmReplace) {
      body = (
        <p role="alert">
          Downloading replaces this computer’s library — {plural(check.local.albums, "album")} and{" "}
          {String(check.local.collection)} in the Collection — with the library from {repository}.
          Anything only on this computer is removed here. A backup is saved first, in the data
          folder’s backups.
        </p>
      );
      actions = (
        <>
          <Button
            onClick={() => {
              setConfirmReplace(false);
            }}
          >
            Back
          </Button>
          <Button
            variant="danger"
            disabledReason={working}
            onClick={() => {
              onChoose("download");
            }}
          >
            Replace and download
          </Button>
        </>
      );
    } else {
      body = (
        <>
          <p>
            Both this computer and {repository} have a library. Merging keeps everything from both
            and asks you about any record that differs. Nothing changes until you choose.
          </p>
          <p className="setting-hint">
            Or download to replace this computer’s library with GitHub’s (backed up first).
          </p>
        </>
      );
      actions = (
        <>
          <Button onClick={close}>Cancel</Button>
          <Button
            variant="danger"
            disabledReason={working}
            onClick={() => {
              setConfirmReplace(true);
            }}
          >
            Download remote library…
          </Button>
          <Button
            variant="primary"
            disabledReason={working}
            onClick={() => {
              onChoose("merge");
            }}
          >
            Merge both
          </Button>
        </>
      );
    }
  }

  return (
    <Dialog
      open={check !== null}
      title="First sync on this computer"
      onClose={close}
      actions={actions}
    >
      {check && (
        <div className="sync-first">
          <Preview local={check.local} remote={check.kind === "ready" ? check.remote : null} />
          {body}
        </div>
      )}
    </Dialog>
  );
}
