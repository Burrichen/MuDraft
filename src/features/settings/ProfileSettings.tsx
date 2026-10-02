import { useId, useState } from "react";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import {
  cancelRestore,
  chooseProfileToRestore,
  confirmRestore,
  exportProfile,
  type ProfileCounts,
  type ProfilePreview,
} from "../../services/profile";
import { formatBytes } from "../artwork/formatBytes";

const plural = (n: number, one: string, many = `${one}s`) => `${String(n)} ${n === 1 ? one : many}`;

function summary(c: ProfileCounts): string {
  return [
    plural(c.albums, "album"),
    plural(c.listens, "listen"),
    plural(c.ratedTracks, "rated track"),
    plural(c.tags, "tag"),
    plural(c.artworkFiles, "image"),
  ].join(", ");
}

function errorText(err: unknown) {
  return err instanceof Error ? err.message : String(err);
}

/** Settings → Profile: export to a .mudraft file, or replace this profile with one. */
export function ProfileSettings() {
  const artworkId = useId();
  const [includeArtwork, setIncludeArtwork] = useState(true);
  const [busy, setBusy] = useState<"export" | "choose" | "restore" | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [preview, setPreview] = useState<ProfilePreview | null>(null);
  const [restored, setRestored] = useState<string | null>(null);

  const run = async (kind: "export" | "choose", task: () => Promise<void>) => {
    setBusy(kind);
    setStatus(null);
    setError(null);
    try {
      await task();
    } catch (err) {
      setError(errorText(err));
    } finally {
      setBusy(null);
    }
  };

  const doExport = () =>
    void run("export", async () => {
      const out = await exportProfile(includeArtwork);
      if (!out) return;
      setStatus(
        `Saved “${out.fileName}” (${formatBytes(out.bytes)}): ${summary(out.counts)}${
          out.artwork === "omitted" ? ", without artwork" : ""
        }.${
          out.artworkMissing > 0
            ? ` ${plural(out.artworkMissing, "image")} missing from this computer ${out.artworkMissing === 1 ? "was" : "were"} left out.`
            : ""
        }`,
      );
    });

  const doChoose = () =>
    void run("choose", async () => {
      const p = await chooseProfileToRestore();
      if (p) setPreview(p);
    });

  const closePreview = () => {
    setPreview(null);
    void cancelRestore().catch(() => undefined);
  };

  const doRestore = async () => {
    if (!preview) return;
    setBusy("restore");
    setError(null);
    try {
      const out = await confirmRestore(preview.sha256);
      setPreview(null);
      setRestored(out.backupFileName);
    } catch (err) {
      setError(errorText(err));
      setPreview(null);
    } finally {
      setBusy(null);
    }
  };

  const created = preview
    ? new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" }).format(
        new Date(preview.createdAt),
      )
    : "";

  return (
    <section className="panel section" aria-labelledby="settings-profile">
      <h2 id="settings-profile" className="section-title">
        Profile
      </h2>
      <p className="setting-hint">
        A profile file holds your whole library — albums, lists, listens, ratings, reviews, tags,
        Next Up history, catalogues, and settings — and works on any computer running MuDraft.
      </p>
      <div className="setting-row">
        <div className="setting-text">
          <label className="setting-label" htmlFor={artworkId}>
            Include artwork
          </label>
          <span id={`${artworkId}-hint`} className="setting-hint">
            Off makes a lightweight file; images can be downloaded again later.
          </span>
        </div>
        <input
          id={artworkId}
          type="checkbox"
          role="switch"
          className="switch"
          aria-describedby={`${artworkId}-hint`}
          checked={includeArtwork}
          onChange={(e) => {
            setIncludeArtwork(e.target.checked);
          }}
        />
      </div>
      <div className="page-actions">
        <Button disabledReason={busy ? "Working…" : undefined} onClick={doExport}>
          {busy === "export" ? "Exporting…" : "Export profile…"}
        </Button>
        <Button disabledReason={busy ? "Working…" : undefined} onClick={doChoose}>
          {busy === "choose" ? "Checking file…" : "Restore from a profile…"}
        </Button>
      </div>
      {status && (
        <p className="setting-hint" role="status">
          {status}
        </p>
      )}
      {error && (
        <p className="match-note match-note-error" role="alert">
          {error}
        </p>
      )}

      <Dialog
        open={preview !== null}
        title="Replace your profile?"
        onClose={closePreview}
        actions={
          <>
            <Button onClick={closePreview}>Cancel</Button>
            <Button
              variant="danger"
              disabledReason={busy === "restore" ? "Restoring…" : undefined}
              onClick={() => void doRestore()}
            >
              Replace my profile
            </Button>
          </>
        }
      >
        {preview && (
          <div className="profile-preview">
            <p>
              This replaces everything in MuDraft with the profile below. Your current profile is
              backed up first, so you can restore it later.
            </p>
            <dl className="facts">
              <dt>Created</dt>
              <dd>
                {created} with MuDraft {preview.appVersion}
              </dd>
              <dt>Contents</dt>
              <dd>{summary(preview.counts)}</dd>
              <dt>Lists</dt>
              <dd>
                {plural(preview.counts.listenList, "album")} on the Listen List,{" "}
                {plural(preview.counts.collection, "edition")} in the Collection
              </dd>
              <dt>Artwork</dt>
              <dd>{preview.artwork === "included" ? "Included" : "Not included"}</dd>
              <dt>Data version</dt>
              <dd>
                {preview.schemaVersion < preview.currentSchemaVersion
                  ? `v${String(preview.schemaVersion)}, upgraded to v${String(preview.currentSchemaVersion)} when restored`
                  : `v${String(preview.schemaVersion)}`}
              </dd>
              <dt>File size</dt>
              <dd>{formatBytes(preview.archiveBytes)}</dd>
            </dl>
          </div>
        )}
      </Dialog>

      <Dialog
        open={restored !== null}
        title="Profile restored"
        onClose={() => {
          window.location.reload();
        }}
        actions={
          <Button
            variant="primary"
            onClick={() => {
              window.location.reload();
            }}
          >
            Reload MuDraft
          </Button>
        }
      >
        Your previous profile was saved as “{restored}” in the data folder’s backups. MuDraft will
        reload to show the restored profile.
      </Dialog>
    </section>
  );
}
