import { useEffect, useState } from "react";
import { useParams } from "react-router";
import { useBack } from "../../app/useBack";
import { Button } from "../../components/Button";
import { FilterBubble } from "../../components/Filters";
import { PageHeader } from "../../components/PageHeader";
import { ProgressBar } from "../../components/ProgressBar";
import { ErrorState, LoadingState } from "../../components/States";
import {
  addDiscoveredToListenList,
  artistCatalogue,
  cancelCatalogueRequest,
  fetchArtistCatalogue,
  loadArtistTracklists,
  setCatalogueScope,
  type ArtistCatalogue,
  type CatalogueEntry,
  type Scope,
} from "../../services/discography";
import { useLibraryVersion } from "../../services/libraryEvents";
import { formatStars, PRIMARY_TYPES, SECONDARY_TYPES } from "./artistFormat";
import { EntryRow } from "./EntryRow";
import "./artist.css";

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

type Load =
  | { kind: "loading" }
  | { kind: "error"; code: string; message: string }
  | { kind: "ready"; data: ArtistCatalogue };

type Job = { kind: "fetch" | "tracklists"; requestId: string };

function errorInfo(err: unknown) {
  const code = typeof err === "object" && err && "code" in err ? String(err.code) : "unknown";
  return { code, message: err instanceof Error ? err.message : String(err) };
}

/** Artist page: identity, honest completion, personal ranking, and the discography. */
export function ArtistPage() {
  const { artistId = "" } = useParams();
  const { label, goBack } = useBack("/collection");
  const version = useLibraryVersion();
  const [load, setLoad] = useState<Load>({ kind: "loading" });
  const [reload, setReload] = useState(0);
  const [job, setJob] = useState<Job | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [adding, setAdding] = useState<string | null>(null);
  const valid = UUID.test(artistId);

  useEffect(() => {
    if (!valid) return;
    let cancelled = false;
    artistCatalogue(artistId).then(
      (data) => {
        if (!cancelled) setLoad({ kind: "ready", data });
      },
      (err: unknown) => {
        if (!cancelled) setLoad({ kind: "error", ...errorInfo(err) });
      },
    );
    return () => {
      cancelled = true;
    };
  }, [artistId, valid, version, reload]);

  // Visible progress: re-read what has been stored while a run is going.
  useEffect(() => {
    if (!job) return;
    const timer = setInterval(() => {
      setReload((n) => n + 1);
    }, 1000);
    return () => {
      clearInterval(timer);
    };
  }, [job]);

  const runJob = async (kind: Job["kind"]) => {
    const requestId = crypto.randomUUID();
    setJob({ kind, requestId });
    setStatus(null);
    setActionError(null);
    try {
      if (kind === "fetch") {
        const run = await fetchArtistCatalogue(requestId, artistId);
        setStatus(
          `Catalogue ${run.complete ? "fetched" : "partly fetched"}${run.usedOfflineCache ? " (offline: from the saved copy)" : ""}.`,
        );
      } else {
        const run = await loadArtistTracklists(requestId, artistId);
        const parts = [`Loaded ${String(run.loaded)} tracklist${run.loaded === 1 ? "" : "s"}.`];
        if (run.failed.length > 0)
          parts.push(`${String(run.failed.length)} couldn’t be loaded and stay unknown.`);
        if (run.stopped) parts.push("Stopped early — try again to resume.");
        setStatus(parts.join(" "));
      }
    } catch (err) {
      const info = errorInfo(err);
      setActionError(
        info.code === "cancelled"
          ? "Stopped. What was already loaded is kept; run it again to resume."
          : `${info.message} What was already loaded is kept; run it again to resume.`,
      );
    } finally {
      setJob(null);
      setReload((n) => n + 1);
    }
  };

  if (!valid) {
    return (
      <section>
        <PageHeader title="Artist" back={{ label, onBack: goBack }} />
        <ErrorState
          title="This link doesn’t point to an artist"
          message={`“${artistId}” isn’t a valid artist ID.`}
        />
      </section>
    );
  }
  if (load.kind !== "ready") {
    return (
      <section>
        <PageHeader title="Artist" back={{ label, onBack: goBack }} />
        {load.kind === "loading" ? (
          <LoadingState label="Loading artist…" />
        ) : (
          <ErrorState
            title={load.code === "not_found" ? "Artist not found" : "Couldn’t load this artist"}
            message={load.message}
            code={load.code}
            onRetry={() => {
              setReload((n) => n + 1);
            }}
          />
        )}
      </section>
    );
  }

  const d = load.data;
  const c = d.coverage;
  const p = d.progress;
  // Anything short of a finished MusicBrainz fetch is provisional.
  const provisional = c.status !== "complete";
  const online = c.kind === "online";
  const ranked = d.entries
    .filter((e) => e.rank !== null)
    .sort((a, b) => (a.rank ?? 0) - (b.rank ?? 0));
  const onLists = d.entries.filter((e) => e.counted && (e.onListenList || e.inCollection));
  const discovered = d.entries.filter((e) => e.counted && !e.onListenList && !e.inCollection);
  const notCounted = d.entries.filter((e) => !e.counted);

  const toggleType = (key: keyof Scope, value: string) => {
    const current = d.scope[key];
    const next = current.includes(value) ? current.filter((v) => v !== value) : [...current, value];
    setActionError(null);
    setCatalogueScope(artistId, { ...d.scope, [key]: next }).then(
      () => {
        setReload((n) => n + 1);
      },
      (err: unknown) => {
        setActionError(errorInfo(err).message);
      },
    );
  };

  const addToList = (entry: CatalogueEntry) => {
    if (!entry.id) return;
    setAdding(entry.id);
    setActionError(null);
    addDiscoveredToListenList(crypto.randomUUID(), entry.id).then(
      () => {
        setAdding(null);
        setStatus(`Added “${entry.title}” to your Listen List.`);
      },
      (err: unknown) => {
        setAdding(null);
        setActionError(errorInfo(err).message);
      },
    );
  };

  const rows = (entries: CatalogueEntry[], withAdd: boolean) => (
    <ul className="artist-entries">
      {entries.map((e) => (
        <EntryRow
          key={e.id ?? e.albumId ?? e.title}
          entry={e}
          artistId={artistId}
          {...(withAdd && e.id && e.source === "musicbrainz"
            ? {
                onAdd: () => {
                  addToList(e);
                },
                adding: adding === e.id,
              }
            : {})}
        />
      ))}
    </ul>
  );

  const tracklistsMissing = c.tracklistsNeeded - c.tracklistsKnown;

  return (
    <section className="artist-page">
      <PageHeader title={d.artistName} back={{ label, onBack: goBack }} />
      <p className="artist-identity">
        {[
          d.disambiguation,
          d.sortName && d.sortName !== d.artistName ? `Sorted as ${d.sortName}` : null,
          d.musicbrainzId ? `MusicBrainz ${d.musicbrainzId}` : "Not linked to MusicBrainz",
        ]
          .filter(Boolean)
          .join(" · ")}
      </p>
      {status && (
        <p className="banner banner-ok" role="status">
          {status}
        </p>
      )}
      {actionError && (
        <p className="match-note match-note-error library-note" role="alert">
          {actionError}
        </p>
      )}

      <section className="panel section artist-progress" aria-labelledby="artist-progress">
        <div className="toolbar">
          <h2 id="artist-progress" className="section-title">
            Your progress
          </h2>
          {provisional && <span className="edition-badge">Provisional</span>}
        </div>
        <p className="setting-hint">{c.note}</p>
        {p.albumsCounted > 0 ? (
          <ProgressBar
            label="Albums listened in full"
            value={p.albumsListened}
            max={p.albumsCounted}
          />
        ) : (
          <p className="artist-unknown">
            Albums listened in full: unknown — no albums are known in this scope yet.
          </p>
        )}
        {p.tracksTotal > 0 ? (
          <ProgressBar
            label="Tracks heard (known tracklists only)"
            value={p.tracksListened}
            max={p.tracksTotal}
          />
        ) : (
          <p className="artist-unknown">Tracks heard: unknown — no tracklists are loaded yet.</p>
        )}
        {p.albumsCounted > 0 && (
          <p className="setting-hint">
            Tracklists loaded for {c.tracklistsKnown}/{c.tracklistsNeeded} albums
            {tracklistsMissing > 0
              ? ", so track progress is not full-discography completion."
              : "."}
          </p>
        )}
        <p className="artist-average">
          {d.average.stars === null
            ? "No rated albums yet."
            : `Your average: ${formatStars(d.average.stars)} across ${String(d.average.ratedAlbums)} rated album${d.average.ratedAlbums === 1 ? "" : "s"}.`}
        </p>
        <details className="artist-explain">
          <summary>How this is counted</summary>
          <p>
            An album counts once however many editions you have. It’s complete when you’ve logged a
            full listen of any edition or confirmed you listened before; listening to individual
            tracks doesn’t complete it. Each track counts once per recording heard, against one
            reference edition per album; re-listens don’t add more, and bonus tracks need their own
            listens. An album’s score averages its rated editions; your average counts each album
            once, with no minimum.
          </p>
          <p>{d.denominator}</p>
          <p>Reference edition when you have none: {d.representativeRule}</p>
        </details>
        <div className="page-actions">
          {online && (
            <Button
              disabledReason={job ? "Already running…" : undefined}
              onClick={() => void runJob("fetch")}
            >
              {c.nextOffset > 0
                ? "Resume fetching catalogue"
                : c.status === "complete"
                  ? "Refresh catalogue"
                  : "Fetch catalogue"}
            </Button>
          )}
          {online && (
            <Button
              disabledReason={
                job
                  ? "Already running…"
                  : tracklistsMissing <= 0
                    ? "All counted tracklists are loaded."
                    : undefined
              }
              onClick={() => void runJob("tracklists")}
            >
              Load tracklists ({tracklistsMissing > 0 ? String(tracklistsMissing) : "0"})
            </Button>
          )}
          {job && (
            <Button
              variant="ghost"
              onClick={() => {
                void cancelCatalogueRequest(job.requestId);
              }}
            >
              Cancel
            </Button>
          )}
        </div>
        {job && (
          <p className="setting-hint" role="status" aria-live="polite">
            {job.kind === "fetch"
              ? `Fetching… ${String(c.fetchedEntries)}${c.providerTotal !== null ? ` of ${String(c.providerTotal)}` : ""} release groups stored.`
              : `Loading tracklists… ${String(c.tracklistsKnown)}/${String(c.tracklistsNeeded)} albums have one.`}
          </p>
        )}
        <div className="artist-scope" role="group" aria-labelledby="artist-scope-label">
          <span id="artist-scope-label" className="filter-group-label">
            Counted types
          </span>
          <div className="filter-bubbles">
            {PRIMARY_TYPES.map((t) => (
              <FilterBubble
                key={t}
                label={t}
                pressed={d.scope.primaryTypes.includes(t)}
                onToggle={() => {
                  toggleType("primaryTypes", t);
                }}
              />
            ))}
            {SECONDARY_TYPES.map((t) => (
              <FilterBubble
                key={t}
                label={`+ ${t}`}
                pressed={d.scope.secondaryTypes.includes(t)}
                onToggle={() => {
                  toggleType("secondaryTypes", t);
                }}
              />
            ))}
          </div>
        </div>
      </section>

      <section className="panel section" aria-labelledby="artist-ranking">
        <h2 id="artist-ranking" className="section-title">
          Your ranking
        </h2>
        {ranked.length > 0 ? (
          rows(ranked, false)
        ) : (
          <p className="setting-hint">Rate an album to rank it here.</p>
        )}
      </section>

      <section className="panel section" aria-labelledby="artist-lists">
        <h2 id="artist-lists" className="section-title">
          On your Listen List or in your Collection
        </h2>
        {onLists.length > 0 ? (
          rows(onLists, false)
        ) : (
          <p className="setting-hint">None of this artist’s counted albums are on your lists.</p>
        )}
      </section>

      <section className="panel section" aria-labelledby="artist-discovered">
        <h2 id="artist-discovered" className="section-title">
          Discovered, not on your lists
        </h2>
        {discovered.length > 0 ? (
          rows(discovered, true)
        ) : (
          <p className="setting-hint">
            {online
              ? "Nothing else in the counted catalogue — or it hasn’t been fetched yet."
              : "This artist isn’t linked to MusicBrainz, so nothing can be discovered."}
          </p>
        )}
      </section>

      {notCounted.length > 0 && (
        <details className="panel section artist-not-counted">
          <summary>
            Not counted ({notCounted.length}):{" "}
            {d.types
              .filter((t) => !t.counted)
              .map((t) => `${t.label} ${String(t.count)}`)
              .join(" · ")}
          </summary>
          {rows(notCounted, true)}
        </details>
      )}
    </section>
  );
}
