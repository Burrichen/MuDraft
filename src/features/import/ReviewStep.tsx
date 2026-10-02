import { useEffect, useRef, useState } from "react";
import { Button } from "../../components/Button";
import { FilterBubble } from "../../components/Filters";
import { ProgressBar } from "../../components/ProgressBar";
import { ErrorState, LoadingState } from "../../components/States";
import {
  commitImport,
  decideRow,
  enrichBatch,
  importRows,
  type CommitReport,
  type DecisionInput,
  type RowFilter,
  type SessionSummary,
  type StagedRow,
} from "../../services/csvImport";
import { MatchDialog, type MatchChoice } from "../match/MatchDialog";
import { rowStatus } from "./rowStatus";

const PAGE = 50;
const BATCH = 10;

const FILTERS: { value: RowFilter; label: string; count?: (s: SessionSummary) => number }[] = [
  { value: "all", label: "All rows", count: (s) => s.counts.rows },
  { value: "attention", label: "Needs review" },
  { value: "errors", label: "Problems", count: (s) => s.counts.errors },
  { value: "duplicates", label: "Duplicates", count: (s) => s.counts.duplicates },
  { value: "existing", label: "Already in library", count: (s) => s.counts.existing },
  { value: "candidates", label: "Possible matches", count: (s) => s.counts.withCandidates },
  { value: "skipped", label: "Skipped", count: (s) => s.counts.skipped },
];

function errorInfo(err: unknown) {
  const code = typeof err === "object" && err && "code" in err ? String(err.code) : "unknown";
  return { code, message: err instanceof Error ? err.message : String(err) };
}

export function ReviewStep({
  summary,
  onRefresh,
  onCommitted,
  onChangeColumns,
}: {
  summary: SessionSummary;
  onRefresh: () => Promise<void>;
  onCommitted: (report: CommitReport) => void;
  onChangeColumns: () => void;
}) {
  const id = summary.sessionId;
  const [filter, setFilter] = useState<RowFilter>("all");
  const [offset, setOffset] = useState(0);
  const [page, setPage] = useState<{ rows: StagedRow[]; total: number } | null>(null);
  const [pageError, setPageError] = useState<{ code: string; message: string } | null>(null);
  const [lookup, setLookup] = useState<{ running: boolean; message: string | null; done: number }>({
    running: false,
    message: null,
    done: 0,
  });
  const lookupAbort = useRef<AbortController | null>(null);
  const [matchRow, setMatchRow] = useState<StagedRow | null>(null);
  const [rowError, setRowError] = useState<string | null>(null);
  const [createTags, setCreateTags] = useState<Set<string>>(new Set());
  const [committing, setCommitting] = useState(false);
  const [commitError, setCommitError] = useState<string | null>(null);

  const [reload, setReload] = useState(0);
  useEffect(() => {
    let cancelled = false;
    importRows(id, filter, offset, PAGE).then(
      (p) => {
        if (cancelled) return;
        setPage(p);
        setPageError(null);
      },
      (err: unknown) => {
        if (!cancelled) setPageError(errorInfo(err));
      },
    );
    return () => {
      cancelled = true;
    };
  }, [id, filter, offset, reload]);

  useEffect(
    () => () => {
      lookupAbort.current?.abort();
    },
    [],
  );

  const refreshAll = async () => {
    await onRefresh();
    setReload((n) => n + 1);
  };

  const startLookup = async () => {
    const controller = new AbortController();
    lookupAbort.current = controller;
    setLookup({ running: true, message: null, done: 0 });
    let done = 0;
    try {
      for (;;) {
        const progress = await enrichBatch(id, BATCH, controller.signal);
        done += progress.processed;
        setLookup({ running: true, message: null, done });
        await refreshAll();
        if (progress.stopped) {
          const message =
            progress.stopped.code === "cancelled"
              ? "Lookups paused. Resume any time — progress is saved."
              : progress.stopped.code === "offline"
                ? "You’re offline. Lookups are paused; you can still import now — rows without a match become manual entries."
                : `${progress.stopped.message}. Lookups are paused; progress is saved.`;
          setLookup({ running: false, message, done });
          return;
        }
        if (progress.remaining === 0 || progress.processed === 0) break;
      }
      setLookup({ running: false, message: "Lookups finished.", done });
    } catch (err) {
      setLookup({ running: false, message: errorInfo(err).message, done });
    }
  };

  const decide = async (row: StagedRow, decision: DecisionInput) => {
    setRowError(null);
    try {
      await decideRow(id, row.rowNumber, decision);
      await refreshAll();
    } catch (err) {
      setRowError(`Row ${String(row.rowNumber)}: ${errorInfo(err).message}`);
      throw err;
    }
  };

  const onMatch = async (choice: MatchChoice) => {
    if (!matchRow) return;
    await decide(
      matchRow,
      choice.kind === "musicbrainz"
        ? {
            kind: "match",
            releaseGroupId: choice.releaseGroupId,
            releaseId: choice.releaseId,
            editionName: choice.editionName ?? matchRow.fields?.edition ?? null,
          }
        : { kind: "manual" },
    );
    setMatchRow(null);
  };

  const commit = async () => {
    setCommitting(true);
    setCommitError(null);
    try {
      const create = summary.unknownTags.map((t) => t.name).filter((t) => createTags.has(t));
      const skip = summary.unknownTags.map((t) => t.name).filter((t) => !createTags.has(t));
      onCommitted(await commitImport(id, { create, skip }));
    } catch (err) {
      setCommitError(errorInfo(err).message);
    } finally {
      setCommitting(false);
    }
  };

  const c = summary.counts;
  const lookupTotal = lookup.done + c.lookupPending;
  const commitReason = committing
    ? "Importing…"
    : lookup.running
      ? "Pause the MusicBrainz lookups first."
      : c.needsAttention > 0
        ? `${String(c.needsAttention)} rows need a MusicBrainz lookup or an edition choice. Look them up, keep them as manual entries, or skip them.`
        : c.ready === 0
          ? "No rows are ready to import."
          : undefined;

  return (
    <div className="settings-list">
      <section className="panel section" aria-labelledby="import-summary">
        <div className="toolbar">
          <h2 id="import-summary" className="section-title">
            Preview · {summary.fileName}
          </h2>
          <Button variant="ghost" onClick={onChangeColumns}>
            Change columns
          </Button>
        </div>
        <p className="setting-hint">
          Nothing has been added yet. Review the rows, then import. Rows with problems or duplicates
          are skipped.
        </p>
        <dl className="facts">
          <dt>Ready to import</dt>
          <dd>{c.ready}</dd>
          <dt>Manual entries</dt>
          <dd>{c.manual}</dd>
          <dt>Matched on MusicBrainz</dt>
          <dd>{c.matched}</dd>
          <dt>Already in library</dt>
          <dd>{c.existing}</dd>
          <dt>Need attention</dt>
          <dd>{c.needsAttention}</dd>
          <dt>Problems / duplicates</dt>
          <dd>
            {c.errors} / {c.duplicates}
          </dd>
        </dl>
      </section>

      <section className="panel section" aria-labelledby="import-lookup">
        <h2 id="import-lookup" className="section-title">
          MusicBrainz lookup (optional)
        </h2>
        <p className="setting-hint">
          Looks rows up one at a time (about one per second) to confirm IDs and suggest matches.
          Suggestions are never accepted for you. You can import without it.
        </p>
        {(lookup.running || lookup.done > 0) && lookupTotal > 0 && (
          <ProgressBar label="Rows looked up" value={lookup.done} max={lookupTotal} />
        )}
        {lookup.message && (
          <p className="match-note" role="status">
            {lookup.message}
          </p>
        )}
        <div>
          {lookup.running ? (
            <Button
              onClick={() => {
                lookupAbort.current?.abort();
              }}
            >
              Pause lookups
            </Button>
          ) : (
            <Button
              disabledReason={
                c.lookupPending === 0
                  ? "Every row has been looked up or doesn’t need it."
                  : undefined
              }
              onClick={() => {
                void startLookup();
              }}
            >
              {lookup.done > 0 ? "Resume lookups" : `Look up ${String(c.lookupPending)} rows`}
            </Button>
          )}
        </div>
      </section>

      <section className="panel section" aria-labelledby="import-rows">
        <h2 id="import-rows" className="section-title">
          Rows
        </h2>
        <div className="filter-group" role="group" aria-label="Show rows">
          <div className="filter-bubbles">
            {FILTERS.map((f) => (
              <FilterBubble
                key={f.value}
                label={f.label}
                count={f.count?.(summary)}
                pressed={filter === f.value}
                onToggle={() => {
                  setFilter(f.value);
                  setOffset(0);
                }}
              />
            ))}
          </div>
        </div>
        {rowError && (
          <p className="match-note match-note-error" role="alert">
            {rowError}
          </p>
        )}
        {pageError && (
          <ErrorState
            title="Couldn’t load rows"
            message={pageError.message}
            code={pageError.code}
          />
        )}
        {!page && !pageError && <LoadingState label="Loading rows…" />}
        {page && (
          <>
            <div className="import-table-wrap">
              <table className="import-table">
                <thead>
                  <tr>
                    <th scope="col">Row</th>
                    <th scope="col">Album</th>
                    <th scope="col">Artist</th>
                    <th scope="col">Year</th>
                    <th scope="col">Edition</th>
                    <th scope="col">Tags</th>
                    <th scope="col">Status</th>
                    <th scope="col">
                      <span className="visually-hidden">Actions</span>
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {page.rows.map((row) => {
                    const status = rowStatus(row);
                    const f = row.fields;
                    return (
                      <tr key={row.rowNumber}>
                        <td>{row.rowNumber}</td>
                        <td>{f?.album ?? row.raw[0] ?? ""}</td>
                        <td>{f?.artist ?? ""}</td>
                        <td>{f?.year ?? ""}</td>
                        <td>{f?.edition ?? ""}</td>
                        <td>{f?.tags.join("; ") ?? ""}</td>
                        <td>
                          <span className="import-status" data-tone={status.tone}>
                            {status.label}
                          </span>
                        </td>
                        <td>
                          {row.readiness !== "error" && (
                            <div className="import-actions">
                              <Button
                                variant="ghost"
                                className="button-small"
                                aria-label={`Change match for row ${String(row.rowNumber)}`}
                                onClick={() => {
                                  setMatchRow(row);
                                }}
                              >
                                Change match…
                              </Button>
                              {row.decision.kind !== "manual" && (
                                <Button
                                  variant="ghost"
                                  className="button-small"
                                  aria-label={`Keep row ${String(row.rowNumber)} as manual`}
                                  onClick={() => {
                                    void decide(row, { kind: "manual" }).catch(() => undefined);
                                  }}
                                >
                                  Keep as manual
                                </Button>
                              )}
                            </div>
                          )}
                          {row.readiness !== "skipped" && (
                            <Button
                              variant="ghost"
                              className="button-small"
                              aria-label={`Skip row ${String(row.rowNumber)}`}
                              onClick={() => {
                                void decide(row, { kind: "skip" }).catch(() => undefined);
                              }}
                            >
                              Skip
                            </Button>
                          )}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
            {page.total === 0 ? (
              <p className="setting-hint">No rows in this view.</p>
            ) : (
              <div className="toolbar">
                <span className="setting-hint" role="status">
                  Rows {offset + 1}–{Math.min(offset + PAGE, page.total)} of {page.total}
                </span>
                <div className="page-actions">
                  <Button
                    variant="ghost"
                    disabledReason={offset === 0 ? "Already on the first page." : undefined}
                    onClick={() => {
                      setOffset(Math.max(0, offset - PAGE));
                    }}
                  >
                    Previous
                  </Button>
                  <Button
                    variant="ghost"
                    disabledReason={
                      offset + PAGE >= page.total ? "Already on the last page." : undefined
                    }
                    onClick={() => {
                      setOffset(offset + PAGE);
                    }}
                  >
                    Next
                  </Button>
                </div>
              </div>
            )}
          </>
        )}
      </section>

      {summary.unknownTags.length > 0 && (
        <section className="panel section" aria-labelledby="import-tags">
          <h2 id="import-tags" className="section-title">
            New tags
          </h2>
          <p className="setting-hint">
            These tags don’t exist yet. Tick the ones to create; unticked tags are left off.
          </p>
          <div className="import-tags">
            {summary.unknownTags.map((t) => (
              <label key={t.name} className="import-tag">
                <input
                  type="checkbox"
                  checked={createTags.has(t.name)}
                  onChange={(e) => {
                    const next = new Set(createTags);
                    if (e.target.checked) next.add(t.name);
                    else next.delete(t.name);
                    setCreateTags(next);
                  }}
                />
                {t.name} <span className="setting-hint">({t.rows} rows)</span>
              </label>
            ))}
          </div>
        </section>
      )}

      <section className="panel section" aria-labelledby="import-commit">
        <h2 id="import-commit" className="section-title">
          Import
        </h2>
        {commitError && (
          <p className="match-note match-note-error" role="alert">
            {commitError}
          </p>
        )}
        <div>
          <Button
            variant="primary"
            disabledReason={commitReason}
            onClick={() => {
              void commit();
            }}
          >
            Import {c.ready} rows to Listen List
          </Button>
        </div>
      </section>

      {matchRow?.fields && (
        <MatchDialog
          key={matchRow.rowNumber}
          open
          autoSearch
          title={`Match row ${String(matchRow.rowNumber)}`}
          confirmLabel="Use this match"
          initialQuery={{
            title: matchRow.fields.album,
            artist: matchRow.fields.artist,
            ...(matchRow.fields.year !== null ? { year: matchRow.fields.year } : {}),
          }}
          onClose={() => {
            setMatchRow(null);
          }}
          onConfirm={onMatch}
        />
      )}
    </div>
  );
}
