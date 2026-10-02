import { useCallback, useEffect, useId, useRef, useState, type SyntheticEvent } from "react";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { ErrorState, LoadingState } from "../../components/States";
import {
  creditText,
  loadEditions,
  searchAlbums,
  type EditionCandidate,
  type EditionList,
  type Fetched,
  type ManualAlbum,
  type ReleaseGroupCandidate,
  type SearchPage,
  type SearchQuery,
} from "../../services/metadata";
import "./match.css";

export type MatchChoice =
  | {
      kind: "musicbrainz";
      releaseGroupId: string;
      releaseId: string;
      editionName?: string;
      candidate: ReleaseGroupCandidate;
      edition: EditionCandidate;
    }
  | { kind: "manual"; album: ManualAlbum };

interface Props {
  open: boolean;
  onClose: () => void;
  /** Persist the choice. Rejections are shown in the dialog; resolve to finish. */
  onConfirm: (choice: MatchChoice) => Promise<void>;
  title?: string;
  confirmLabel?: string;
  initialQuery?: Partial<SearchQuery>;
  /** Search immediately with `initialQuery` when first shown (e.g. reviewing an import row). */
  autoSearch?: boolean;
}

type Load<T> =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "error"; code: string; message: string }
  | { kind: "ready"; data: T };

function errorInfo(err: unknown): { code: string; message: string } {
  const code = typeof err === "object" && err && "code" in err ? String(err.code) : "unknown";
  return { code, message: err instanceof Error ? err.message : String(err) };
}

const isCancelled = (err: unknown) => errorInfo(err).code === "cancelled";

function yearLabel(c: { originalDate: { year: number | null } }): string {
  return c.originalDate.year !== null ? String(c.originalDate.year) : "Year unknown";
}

function parseYear(raw: string): number | undefined | "invalid" {
  const v = raw.trim();
  if (v === "") return undefined;
  return /^\d{4}$/.test(v) ? Number(v) : "invalid";
}

function FetchNote({ fetched }: { fetched: Fetched<unknown> }) {
  if (fetched.source !== "stale_cache") return null;
  const when = new Date(fetched.fetchedAt).toLocaleString();
  return (
    <p className="match-note match-note-warning" role="status">
      MusicBrainz can’t be reached. Showing results saved on {when}.
    </p>
  );
}

function CandidateOption({
  candidate,
  name,
  checked,
  onSelect,
}: {
  candidate: ReleaseGroupCandidate;
  name: string;
  checked: boolean;
  onSelect: () => void;
}) {
  const types = [candidate.primaryType ?? "Type unknown", ...candidate.secondaryTypes].join(" · ");
  const artist = creditText(candidate.artistCredit) || "Unknown artist";
  const disambiguations = candidate.artistCredit
    .filter((p) => p.disambiguation)
    .map((p) => `${p.artistName}: ${p.disambiguation ?? ""}`);
  return (
    <label className="match-option" data-checked={checked}>
      <input type="radio" name={name} checked={checked} onChange={onSelect} />
      <span className="match-option-body">
        <span className="match-option-title">
          {candidate.title}
          {candidate.disambiguation && (
            <span className="match-muted"> ({candidate.disambiguation})</span>
          )}
        </span>
        <span className="match-option-line">{artist}</span>
        <span className="match-option-meta">
          {yearLabel(candidate)} · {types}
        </span>
        {disambiguations.length > 0 && (
          <span className="match-option-meta">{disambiguations.join("; ")}</span>
        )}
        <span className="match-chips">
          <span className="match-chip">Relevance {candidate.score}</span>
          {candidate.exactTitle && <span className="match-chip">Exact title</span>}
          {candidate.yearMatches === true && <span className="match-chip">Year matches</span>}
          {candidate.yearMatches === false && (
            <span className="match-chip match-chip-warn">Different year</span>
          )}
        </span>
      </span>
    </label>
  );
}

function EditionOption({
  edition,
  name,
  checked,
  onSelect,
}: {
  edition: EditionCandidate;
  name: string;
  checked: boolean;
  onSelect: () => void;
}) {
  const facts = [
    edition.date.value ?? "Date unknown",
    edition.country,
    edition.formats.join(" + ") || null,
    `${String(edition.trackCount)} tracks`,
    edition.status,
  ].filter(Boolean);
  return (
    <label className="match-option" data-checked={checked}>
      <input type="radio" name={name} checked={checked} onChange={onSelect} />
      <span className="match-option-body">
        <span className="match-option-title">
          {edition.title}
          {edition.disambiguation && (
            <span className="match-muted"> ({edition.disambiguation})</span>
          )}
        </span>
        <span className="match-option-meta">{facts.join(" · ")}</span>
      </span>
    </label>
  );
}

/**
 * Search MusicBrainz, review candidates, and pick a canonical album plus the edition to
 * track — or enter an unmatched album by hand. Nothing is chosen automatically, even for
 * a perfect score: the user always selects both album and edition before confirming.
 */
export function MatchDialog({
  open,
  onClose,
  onConfirm,
  title = "Find an album",
  confirmLabel = "Add album",
  initialQuery,
  autoSearch = false,
}: Props) {
  const ids = {
    title: useId(),
    artist: useId(),
    year: useId(),
    editionName: useId(),
    groups: useId(),
    editions: useId(),
  };
  const [mode, setMode] = useState<"search" | "manual">("search");
  const [form, setForm] = useState({
    title: initialQuery?.title ?? "",
    artist: initialQuery?.artist ?? "",
    year: initialQuery?.year !== undefined ? String(initialQuery.year) : "",
  });
  const [lastQuery, setLastQuery] = useState<SearchQuery | null>(null);
  const [results, setResults] = useState<Load<Fetched<SearchPage>>>({ kind: "idle" });
  const [groupId, setGroupId] = useState<string | null>(null);
  const [editions, setEditions] = useState<Load<Fetched<EditionList>>>({ kind: "idle" });
  const [releaseId, setReleaseId] = useState<string | null>(null);
  const [editionName, setEditionName] = useState("");
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<{ code: string; message: string } | null>(null);
  const searchAbort = useRef<AbortController | null>(null);
  const editionsAbort = useRef<AbortController | null>(null);

  // Closing (or unmounting) cancels anything still running.
  useEffect(() => {
    if (open) return;
    searchAbort.current?.abort();
    editionsAbort.current?.abort();
  }, [open]);
  useEffect(
    () => () => {
      searchAbort.current?.abort();
      editionsAbort.current?.abort();
    },
    [],
  );

  const year = parseYear(form.year);
  const yearInvalid = year === "invalid";

  const runSearch = useCallback((query: SearchQuery, offset: number) => {
    searchAbort.current?.abort();
    const controller = new AbortController();
    searchAbort.current = controller;
    if (offset === 0) {
      setResults({ kind: "loading" });
      setGroupId(null);
      setEditions({ kind: "idle" });
      setReleaseId(null);
    }
    searchAlbums(query, { signal: controller.signal, offset }).then(
      (page) => {
        if (controller.signal.aborted) return;
        setResults((prev) =>
          offset > 0 && prev.kind === "ready"
            ? {
                kind: "ready",
                data: {
                  ...page,
                  value: {
                    ...page.value,
                    candidates: [...prev.data.value.candidates, ...page.value.candidates],
                  },
                },
              }
            : { kind: "ready", data: page },
        );
      },
      (err: unknown) => {
        if (controller.signal.aborted || isCancelled(err)) return;
        setResults({ kind: "error", ...errorInfo(err) });
      },
    );
  }, []);

  // One-shot search on first open; remount (new `key`) to search again for another row.
  const autoSearched = useRef(false);
  useEffect(() => {
    if (!autoSearch || !open || autoSearched.current || !initialQuery?.title) return;
    autoSearched.current = true;
    const query: SearchQuery = {
      title: initialQuery.title,
      ...(initialQuery.artist ? { artist: initialQuery.artist } : {}),
      ...(initialQuery.year !== undefined ? { year: initialQuery.year } : {}),
    };
    setLastQuery(query);
    runSearch(query, 0);
  }, [autoSearch, open, initialQuery?.title, initialQuery?.artist, initialQuery?.year, runSearch]);

  const onSearch = (e: SyntheticEvent<HTMLFormElement>) => {
    e.preventDefault();
    if (form.title.trim() === "" || yearInvalid) return;
    const query: SearchQuery = {
      title: form.title.trim(),
      ...(form.artist.trim() ? { artist: form.artist.trim() } : {}),
      ...(typeof year === "number" ? { year } : {}),
    };
    setLastQuery(query);
    runSearch(query, 0);
  };

  const selectGroup = (id: string) => {
    setGroupId(id);
    setReleaseId(null);
    setSaveError(null);
    editionsAbort.current?.abort();
    const controller = new AbortController();
    editionsAbort.current = controller;
    setEditions({ kind: "loading" });
    loadEditions(id, { signal: controller.signal }).then(
      (list) => {
        if (!controller.signal.aborted) setEditions({ kind: "ready", data: list });
      },
      (err: unknown) => {
        if (controller.signal.aborted || isCancelled(err)) return;
        setEditions({ kind: "error", ...errorInfo(err) });
      },
    );
  };

  const candidates = results.kind === "ready" ? results.data.value.candidates : [];
  const candidate = candidates.find((c) => c.id === groupId);
  const editionList = editions.kind === "ready" ? editions.data.value.editions : [];
  const edition = editionList.find((e) => e.id === releaseId);

  const manualReady = form.title.trim() !== "" && form.artist.trim() !== "" && !yearInvalid;
  const confirmReason =
    mode === "manual"
      ? manualReady
        ? undefined
        : yearInvalid
          ? "Enter the year as four digits, or leave it empty."
          : "Enter an album title and an artist."
      : !candidate
        ? "Choose an album from the search results first."
        : !edition
          ? "Choose the edition you have or plan to hear."
          : undefined;

  const confirm = async () => {
    let choice: MatchChoice;
    if (mode === "manual") {
      choice = {
        kind: "manual",
        album: {
          title: form.title.trim(),
          artistName: form.artist.trim(),
          ...(typeof year === "number" ? { year } : {}),
        },
      };
    } else if (candidate && edition) {
      choice = {
        kind: "musicbrainz",
        releaseGroupId: candidate.id,
        releaseId: edition.id,
        candidate,
        edition,
        ...(editionName.trim() ? { editionName: editionName.trim() } : {}),
      };
    } else {
      return;
    }
    setSaving(true);
    setSaveError(null);
    try {
      await onConfirm(choice);
    } catch (err) {
      setSaveError(errorInfo(err));
    } finally {
      setSaving(false);
    }
  };

  const hasMore =
    results.kind === "ready" && results.data.value.candidates.length < results.data.value.total;

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={title}
      size="wide"
      actions={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            disabledReason={saving ? "Saving…" : confirmReason}
            onClick={() => {
              void confirm();
            }}
          >
            {mode === "manual" ? "Add without matching" : confirmLabel}
          </Button>
        </>
      }
    >
      <div className="match">
        <div className="filter-group" role="group" aria-label="How to add">
          <div className="filter-bubbles">
            <button
              type="button"
              className="filter-bubble"
              aria-pressed={mode === "search"}
              onClick={() => {
                setMode("search");
              }}
            >
              Search MusicBrainz
            </button>
            <button
              type="button"
              className="filter-bubble"
              aria-pressed={mode === "manual"}
              onClick={() => {
                setMode("manual");
              }}
            >
              Enter manually
            </button>
          </div>
        </div>

        <form className="match-form" onSubmit={onSearch} noValidate>
          <div className="match-field match-field-wide">
            <label htmlFor={ids.title}>Album title</label>
            <input
              id={ids.title}
              value={form.title}
              required
              autoComplete="off"
              onChange={(e) => {
                setForm({ ...form, title: e.target.value });
              }}
            />
          </div>
          <div className="match-field">
            <label htmlFor={ids.artist}>Artist{mode === "search" ? " (optional)" : ""}</label>
            <input
              id={ids.artist}
              value={form.artist}
              autoComplete="off"
              onChange={(e) => {
                setForm({ ...form, artist: e.target.value });
              }}
            />
          </div>
          <div className="match-field match-field-year">
            <label htmlFor={ids.year}>Year (optional)</label>
            <input
              id={ids.year}
              value={form.year}
              inputMode="numeric"
              maxLength={4}
              aria-invalid={yearInvalid || undefined}
              aria-describedby={yearInvalid ? `${ids.year}-error` : undefined}
              onChange={(e) => {
                setForm({ ...form, year: e.target.value });
              }}
            />
            {yearInvalid && (
              <span id={`${ids.year}-error`} className="match-field-error">
                Use four digits, like 1997.
              </span>
            )}
          </div>
          {mode === "search" && (
            <div className="match-field match-field-action">
              <Button
                type="submit"
                variant="secondary"
                disabledReason={
                  form.title.trim() === ""
                    ? "Enter an album title to search."
                    : yearInvalid
                      ? "Fix the year first."
                      : undefined
                }
              >
                Search
              </Button>
            </div>
          )}
        </form>

        {mode === "manual" ? (
          <p className="match-note">
            The album is added as you typed it, with a new artist entry — it isn’t linked to
            MusicBrainz or merged with existing artists that share the name.
          </p>
        ) : (
          <>
            <section className="match-section" aria-labelledby={ids.groups}>
              <h3 id={ids.groups} className="match-heading">
                Album
              </h3>
              {results.kind === "idle" && (
                <p className="match-muted">Search by title, optionally with artist and year.</p>
              )}
              {results.kind === "loading" && <LoadingState label="Searching MusicBrainz…" />}
              {results.kind === "error" && (
                <ErrorState
                  title="Search failed"
                  message={results.message}
                  code={results.code}
                  {...(lastQuery
                    ? {
                        onRetry: () => {
                          runSearch(lastQuery, 0);
                        },
                      }
                    : {})}
                />
              )}
              {results.kind === "ready" && (
                <>
                  <FetchNote fetched={results.data} />
                  <p className="match-muted" role="status">
                    {results.data.value.total === 0
                      ? "No matches. Try a shorter title, drop the artist, or enter the album manually."
                      : `${String(results.data.value.total)} matches. Review and choose one — nothing is selected for you.`}
                  </p>
                  <div className="match-options" role="radiogroup" aria-labelledby={ids.groups}>
                    {candidates.map((c) => (
                      <CandidateOption
                        key={c.id}
                        candidate={c}
                        name={ids.groups}
                        checked={c.id === groupId}
                        onSelect={() => {
                          selectGroup(c.id);
                        }}
                      />
                    ))}
                  </div>
                  {hasMore && lastQuery && (
                    <Button
                      variant="ghost"
                      onClick={() => {
                        runSearch(lastQuery, candidates.length);
                      }}
                    >
                      Show more results
                    </Button>
                  )}
                </>
              )}
            </section>

            {candidate && (
              <section className="match-section" aria-labelledby={ids.editions}>
                <h3 id={ids.editions} className="match-heading">
                  Edition of “{candidate.title}”
                </h3>
                {editions.kind === "loading" && <LoadingState label="Loading editions…" />}
                {editions.kind === "error" && (
                  <ErrorState
                    title="Couldn’t load editions"
                    message={editions.message}
                    code={editions.code}
                    onRetry={() => {
                      selectGroup(candidate.id);
                    }}
                  />
                )}
                {editions.kind === "ready" && (
                  <>
                    <FetchNote fetched={editions.data} />
                    {editionList.length === 0 ? (
                      <p className="match-muted">MusicBrainz lists no editions for this album.</p>
                    ) : (
                      <div
                        className="match-options"
                        role="radiogroup"
                        aria-labelledby={ids.editions}
                      >
                        {editionList.map((e) => (
                          <EditionOption
                            key={e.id}
                            edition={e}
                            name={ids.editions}
                            checked={e.id === releaseId}
                            onSelect={() => {
                              setReleaseId(e.id);
                              setSaveError(null);
                            }}
                          />
                        ))}
                      </div>
                    )}
                    {editions.data.value.truncated && (
                      <p className="match-muted">
                        Showing the first {editionList.length} of {editions.data.value.total}{" "}
                        editions.
                      </p>
                    )}
                  </>
                )}
                {edition && (
                  <div className="match-field">
                    <label htmlFor={ids.editionName}>Edition name in MuDraft (optional)</label>
                    <input
                      id={ids.editionName}
                      value={editionName}
                      placeholder="Standard, Deluxe, …"
                      aria-describedby={`${ids.editionName}-hint`}
                      onChange={(e) => {
                        setEditionName(e.target.value);
                      }}
                    />
                    <span id={`${ids.editionName}-hint`} className="match-muted">
                      Leave empty to use a name based on the edition.
                    </span>
                  </div>
                )}
              </section>
            )}
          </>
        )}

        {saveError && (
          <p className="match-note match-note-error" role="alert">
            {saveError.message}
          </p>
        )}
      </div>
    </Dialog>
  );
}
