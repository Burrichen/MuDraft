import { useCallback, useEffect, useState } from "react";
import { useBack } from "../../app/useBack";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { PageHeader } from "../../components/PageHeader";
import { LoadingState } from "../../components/States";
import {
  applyMapping,
  discardImport,
  importSummary,
  latestImport,
  openCsv,
  saveTemplate,
  TAG_DELIMITER,
  type ColumnMapping,
  type CommitReport,
  type SessionSummary,
} from "../../services/csvImport";
import { ImportReport } from "./ImportReport";
import { MappingStep } from "./MappingStep";
import { ReviewStep } from "./ReviewStep";
import "./import.css";

function message(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

function FormatHelp() {
  return (
    <details className="import-help">
      <summary>CSV format</summary>
      <ul>
        <li>First row: column names. Required columns: Album and Artist.</li>
        <li>
          Optional: Year, Edition, Tags, MusicBrainz Release Group ID, MusicBrainz Release ID.
        </li>
        <li>
          Separate several tags with a semicolon (<code>{TAG_DELIMITER}</code>), e.g.{" "}
          <code>Listen ASAP; road trip</code>. Tags may contain commas.
        </li>
        <li>Artist is used exactly as written — MuDraft never splits it on “&” or commas.</li>
        <li>Wrap values containing commas, quotes, or line breaks in double quotes.</li>
        <li>Save as UTF-8 (“CSV UTF-8” in spreadsheet apps). Up to 20,000 rows, 10 MB.</li>
      </ul>
    </details>
  );
}

/** Staged CSV import into the Listen List. Nothing changes until the final Import step. */
export function CsvImportPage() {
  const { label, goBack } = useBack("/listen-list");
  const [summary, setSummary] = useState<SessionSummary | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [mapping, setMapping] = useState(false);
  const [report, setReport] = useState<CommitReport | null>(null);
  const [confirmDiscard, setConfirmDiscard] = useState(false);

  useEffect(() => {
    latestImport().then(
      (s) => {
        setSummary(s);
        setLoading(false);
      },
      (err: unknown) => {
        setError(message(err));
        setLoading(false);
      },
    );
  }, []);

  const run = async (task: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await task();
    } catch (err) {
      setError(message(err));
    } finally {
      setBusy(false);
    }
  };

  const refresh = useCallback(async () => {
    if (summary) setSummary(await importSummary(summary.sessionId));
  }, [summary]);

  const choose = () =>
    run(async () => {
      const s = await openCsv();
      if (!s) return;
      setSummary(s);
      setReport(null);
      setMapping(s.mapping === null);
    });

  const apply = (m: ColumnMapping) =>
    run(async () => {
      if (!summary) return;
      setSummary(await applyMapping(summary.sessionId, m));
      setMapping(false);
    });

  const discard = () =>
    run(async () => {
      if (summary) await discardImport(summary.sessionId);
      setSummary(null);
      setConfirmDiscard(false);
      setNote("Import discarded. Nothing was added.");
    });

  return (
    <section>
      <PageHeader
        title="Import CSV"
        description="Add albums to your Listen List from a spreadsheet."
        back={{ label, onBack: goBack }}
        {...(summary && !report
          ? {
              actions: (
                <Button
                  variant="danger"
                  onClick={() => {
                    setConfirmDiscard(true);
                  }}
                >
                  Discard import
                </Button>
              ),
            }
          : {})}
      />
      {error && (
        <p className="match-note match-note-error import-banner" role="alert">
          {error}
        </p>
      )}
      {note && (
        <p className="banner banner-ok" role="status">
          {note}
        </p>
      )}

      {loading ? (
        <LoadingState label="Checking for an unfinished import…" />
      ) : report ? (
        <>
          <ImportReport report={report} />
          <div className="import-footer">
            <Button variant="primary" onClick={goBack}>
              Back to Listen List
            </Button>
          </div>
        </>
      ) : !summary ? (
        <section className="panel section" aria-labelledby="import-start">
          <h2 id="import-start" className="section-title">
            Choose a file
          </h2>
          <p className="setting-hint">
            You’ll see every row before anything is added. Importing works offline; MusicBrainz
            lookups are optional.
          </p>
          <div className="page-actions">
            <Button
              variant="primary"
              disabledReason={busy ? "Opening…" : undefined}
              onClick={() => void choose()}
            >
              Choose CSV file…
            </Button>
            <Button
              onClick={() =>
                void run(async () => {
                  const saved = await saveTemplate();
                  if (saved) setNote(`Template saved as ${saved}.`);
                })
              }
            >
              Download template
            </Button>
          </div>
          <FormatHelp />
        </section>
      ) : mapping || summary.mapping === null ? (
        <MappingStep summary={summary} busy={busy} onApply={(m) => void apply(m)} />
      ) : (
        <ReviewStep
          summary={summary}
          onRefresh={refresh}
          onChangeColumns={() => {
            setMapping(true);
          }}
          onCommitted={(r) => {
            setReport(r);
            setSummary(null);
          }}
        />
      )}

      <Dialog
        open={confirmDiscard}
        title="Discard this import?"
        onClose={() => {
          setConfirmDiscard(false);
        }}
        actions={
          <>
            <Button
              onClick={() => {
                setConfirmDiscard(false);
              }}
            >
              Keep reviewing
            </Button>
            <Button variant="danger" onClick={() => void discard()}>
              Discard
            </Button>
          </>
        }
      >
        The staged rows and any lookups are deleted. Nothing in your library changes.
      </Dialog>
    </section>
  );
}
