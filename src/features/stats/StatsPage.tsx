import { useEffect, useState } from "react";
import { Link } from "react-router";
import { PageHeader } from "../../components/PageHeader";
import { EmptyState, ErrorState, LoadingState } from "../../components/States";
import { useLibraryVersion } from "../../services/libraryEvents";
import {
  completionRate,
  statsOverview,
  type CollectionStats,
  type ListenListStats,
  type MethodStats,
  type Stats,
} from "../../services/stats";
import { BarTable } from "./BarTable";
import { BestHighlight } from "./BestHighlight";
import { formatDuration, formatMonth, formatStars, METHOD_LABEL, plural } from "./format";
import { StatCard } from "./StatCard";
import "./stats.css";

const LISTEN_ASAP = "00000000-0000-7000-8000-000000000001";
const TOP = 10;

type Load =
  | { kind: "loading" }
  | { kind: "error"; code: string; message: string }
  | { kind: "ready"; stats: Stats };

const list = (query: string) => `/listen-list?${query}`;
const collection = (query: string) => `/collection?${query}`;
const q = (key: string, value: string) => `${key}=${encodeURIComponent(value)}`;

function ListenListSection({ s, picks }: { s: ListenListStats; picks: MethodStats[] }) {
  if (s.entries === 0) {
    return (
      <>
        <EmptyState
          icon="stats"
          title="Your Listen List is empty"
          description="Add albums to see decades, genres, artists, and how long albums wait."
        />
        <PicksTable picks={picks} />
      </>
    );
  }
  const oldest = s.oldestWaiting[0];
  return (
    <>
      <ul className="stat-cards" aria-label="Listen List overview">
        <StatCard label="Albums waiting" value={String(s.distinctAlbums)} href="/listen-list" />
        <StatCard label="Artists" value={String(s.distinctArtists)} />
        <StatCard
          label="Tagged Listen ASAP"
          value={String(s.listenAsap)}
          href={list(q("tag", LISTEN_ASAP))}
        />
        <StatCard
          label="Longest wait"
          value={plural(s.waiting?.longestDays ?? 0, "day")}
          hint={
            s.waiting
              ? `Median ${plural(s.waiting.medianDays, "day")}${oldest ? ` · ${oldest.title}` : ""}`
              : undefined
          }
          href={list("sort=added_oldest")}
        />
        <StatCard
          label="Known runtime"
          value={formatDuration(s.runtime.knownMs)}
          hint={
            s.runtime.editionsUnknown > 0
              ? `${plural(s.runtime.editionsUnknown, "album")} with unknown length not included`
              : "Every album’s length is known"
          }
        />
      </ul>
      <BarTable
        caption="Albums by decade"
        valueHeading="Albums"
        rows={s.decades.map((d) => ({
          key: d.key,
          label: `${d.label}s`,
          value: d.count,
          href: list(q("decade", d.key)),
        }))}
        empty="No release years are known yet."
        note={
          s.unknownYears > 0 ? (
            <Link to={list("yearUnknown=1")}>
              {plural(s.unknownYears, "album")} with an unknown release year
            </Link>
          ) : undefined
        }
      />
      <BarTable
        caption="Albums by genre"
        valueHeading="Albums"
        rows={s.genres.map((g) => ({
          key: g.key,
          label: g.label,
          value: g.count,
          href: list(q("genre", g.label)),
        }))}
        empty="No genres yet."
        note={`Genres overlap: an album counts once in each of its genres.${
          s.albumsWithoutGenre > 0
            ? ` ${plural(s.albumsWithoutGenre, "album")} without a genre.`
            : ""
        }`}
      />
      <BarTable
        caption="Most-listed artists"
        valueHeading="Albums"
        rows={s.artists.slice(0, TOP).map((a) => ({
          key: a.key,
          label: a.label,
          value: a.count,
          href: list(q("artist", a.key)),
        }))}
        empty="No artists yet."
        note={
          s.artists.length > TOP
            ? `Top ${String(TOP)} of ${String(s.artists.length)}. Collaborations count for each artist.`
            : "Collaborations count for each artist."
        }
      />
      <BarTable
        caption="Tags"
        valueHeading="Albums"
        rows={s.tags.map((t) => ({
          key: t.key,
          label: t.label,
          value: t.count,
          href: list(q("tag", t.key)),
        }))}
        empty="No tags on Listen List albums."
      />
      <div className="stat-lists">
        <WaitingList title="Recently added" items={s.recentlyAdded} />
        <WaitingList title="Waiting longest" items={s.oldestWaiting} />
      </div>
      <PicksTable picks={picks} />
    </>
  );
}

function WaitingList({ title, items }: { title: string; items: ListenListStats["oldestWaiting"] }) {
  return (
    <section aria-label={title}>
      <h4 className="best-title">{title}</h4>
      <ol className="stat-list">
        {items.map((w) => (
          <li key={w.albumId}>
            <Link to={`/albums/${w.albumId}`}>{w.title}</Link>{" "}
            <span className="setting-hint">· {plural(w.waitingDays, "day")}</span>
          </li>
        ))}
      </ol>
    </section>
  );
}

/** Next Up provenance: what each method showed and what came of it. */
function PicksTable({ picks }: { picks: MethodStats[] }) {
  const total = picks.find((p) => p.source === "all");
  if (!total || total.shown === 0) {
    return <p className="setting-hint">No Next Up picks yet.</p>;
  }
  return (
    <figure className="bar-table" aria-labelledby="stats-picks">
      <figcaption id="stats-picks" className="bar-table-caption">
        Next Up picks by method
      </figcaption>
      {/* Focusable so keyboard users can scroll it when it is wider than the column. */}
      <div className="table-scroll" tabIndex={0} role="region" aria-label="Next Up picks table">
        <table className="stat-table">
          <thead>
            <tr>
              <th scope="col">Method</th>
              <th scope="col">Shown</th>
              <th scope="col">Skipped</th>
              <th scope="col">Listened</th>
              <th scope="col">Current</th>
              <th scope="col">Listened of resolved</th>
            </tr>
          </thead>
          <tbody>
            {picks.map((p) => {
              const rate = completionRate(p);
              return (
                <tr key={p.source}>
                  <th scope="row">{METHOD_LABEL[p.source] ?? p.source}</th>
                  <td>{p.shown}</td>
                  <td>{p.skipped}</td>
                  <td>{p.completed}</td>
                  <td>{p.pending}</td>
                  <td>{rate === null ? "—" : `${String(Math.round(rate * 100))}%`}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      <p className="setting-hint">From the stored record of each pick, not today’s tags.</p>
    </figure>
  );
}

function CollectionSection({ s }: { s: CollectionStats }) {
  const n = s.listens;
  if (s.albums === 0 && s.distinctListenedAlbums === 0) {
    return (
      <EmptyState
        icon="stats"
        title="Your Collection is empty"
        description="Log listens and rate albums to see your best years, genres, and artists."
      />
    );
  }
  const recentMonths = s.byMonth.slice(-12);
  return (
    <>
      <ul className="stat-cards" aria-label="Collection overview">
        <StatCard label="Albums in Collection" value={String(s.albums)} href="/collection" />
        <StatCard
          label="Albums listened to"
          value={String(s.distinctListenedAlbums)}
          href={collection("sort=recently_listened")}
        />
        <StatCard
          label="Dated listens"
          value={String(n.dated)}
          hint={`${plural(n.undated, "undated listen")} · ${plural(n.historical, "earlier-history note")}`}
        />
        <StatCard
          label="Average rating"
          value={
            s.ratings.averageStars === null ? "No ratings" : formatStars(s.ratings.averageStars)
          }
          hint={`${plural(s.ratings.ratedAlbums, "rated album")} · ${String(s.ratings.unratedAlbums)} unrated`}
          href={collection("sort=rating_highest")}
        />
        <StatCard
          label={s.runtime.complete ? "Time listened" : "Time listened (at least)"}
          value={formatDuration(s.runtime.knownMs)}
          hint={
            s.runtime.complete
              ? plural(s.runtime.sessions, "session")
              : `Incomplete: ${[
                  s.runtime.tracksWithoutLength > 0 &&
                    plural(s.runtime.tracksWithoutLength, "track") + " of unknown length",
                  s.runtime.listensWithoutTracklist > 0 &&
                    plural(s.runtime.listensWithoutTracklist, "listen") + " without a tracklist",
                ]
                  .filter(Boolean)
                  .join(", ")}`
          }
        />
      </ul>

      <div className="best-grid">
        <BestHighlight
          title="Best release year"
          best={s.bestYear}
          href={(k) => collection(q("year", k))}
          unplaced="with an unknown original year"
        />
        <BestHighlight
          title="Best decade"
          best={s.bestDecade}
          href={(k) => collection(q("decade", k))}
          unplaced="with an unknown original year"
        />
        <BestHighlight
          title="Best genre"
          best={s.bestGenre}
          href={(k) => collection(q("genre", k))}
          unplaced="without a genre"
        />
        <BestHighlight
          title="Best artist"
          best={s.bestArtist}
          href={(k) => collection(q("artist", k))}
        />
      </div>
      <p className="setting-hint">
        Rated Collection albums only. Each album counts once (its editions’ scores averaged),
        however often you’ve listened; zero is a rating. Years are original release years. Genres
        overlap, so an album counts in each of its genres. Any number of albums can win.
      </p>

      <BarTable
        caption="Rating distribution"
        valueHeading="Albums"
        rows={s.ratings.distribution.map((b) => ({
          key: String(b.stars),
          label: formatStars(b.stars),
          value: b.albums,
        }))}
        empty="No rated albums yet."
        note={
          s.ratings.unratedAlbums > 0
            ? `${plural(s.ratings.unratedAlbums, "unrated album")} not shown.`
            : undefined
        }
      />
      <BarTable
        caption="Listens by year"
        valueHeading="Listens"
        rows={s.byYear.map((y) => ({ key: y.key, label: y.label, value: y.count }))}
        empty="No dated listens yet."
        note={
          n.undated > 0 ? `${plural(n.undated, "listen")} without a date not shown.` : undefined
        }
      />
      <BarTable
        caption={
          s.byMonth.length > 12 ? "Listens by month (last 12 with listens)" : "Listens by month"
        }
        valueHeading="Listens"
        rows={recentMonths.map((m) => ({ key: m.key, label: formatMonth(m.key), value: m.count }))}
        empty="No dated listens yet."
      />
      <BarTable
        caption="Kinds of listen"
        valueHeading="Listens"
        rows={[
          { key: "first", label: "First listens", value: n.first },
          { key: "relisten", label: "Re-listens", value: n.relisten },
          { key: "unspecified", label: "Not specified", value: n.unspecified },
          { key: "full", label: "Whole album", value: n.fullAlbum },
          { key: "tracks", label: "Tracks only", value: n.tracksOnly },
        ]}
        empty="No listens yet."
      />
      <section aria-label="Favourite tracks">
        <h4 className="best-title">Favourite tracks</h4>
        {s.favouriteTracks.length === 0 ? (
          <p className="setting-hint">No favourite tracks in your Collection yet.</p>
        ) : (
          <ul className="stat-list">
            {s.favouriteTracks.map((t) => (
              <li key={t.trackId}>
                {t.title} <span className="setting-hint">· </span>
                <Link to={`/albums/${t.albumId}`}>{t.albumTitle}</Link>
                <span className="setting-hint"> ({t.editionName})</span>
              </li>
            ))}
          </ul>
        )}
      </section>
    </>
  );
}

export function StatsPage() {
  const version = useLibraryVersion();
  const [load, setLoad] = useState<Load>({ kind: "loading" });
  const [retry, setRetry] = useState(0);

  useEffect(() => {
    let cancelled = false;
    statsOverview().then(
      (stats) => {
        if (!cancelled) setLoad({ kind: "ready", stats });
      },
      (err: unknown) => {
        if (cancelled) return;
        const code = typeof err === "object" && err && "code" in err ? String(err.code) : "unknown";
        setLoad({ kind: "error", code, message: err instanceof Error ? err.message : String(err) });
      },
    );
    return () => {
      cancelled = true;
    };
  }, [version, retry]);

  return (
    <section>
      <PageHeader title="Stats" description="Listen List and Collection, side by side." />
      {load.kind === "loading" && <LoadingState label="Counting…" />}
      {load.kind === "error" && (
        <ErrorState
          title="Couldn’t calculate stats"
          message={load.message}
          code={load.code}
          onRetry={() => {
            setRetry((n) => n + 1);
          }}
        />
      )}
      {load.kind === "ready" && (
        <div className="stats-columns">
          <section className="panel section stats-section" aria-labelledby="stats-listen-list">
            <h2 id="stats-listen-list" className="section-title">
              Listen List
            </h2>
            <ListenListSection s={load.stats.listenList} picks={load.stats.selection} />
          </section>
          <section className="panel section stats-section" aria-labelledby="stats-collection">
            <h2 id="stats-collection" className="section-title">
              Collection
            </h2>
            <CollectionSection s={load.stats.collection} />
          </section>
        </div>
      )}
    </section>
  );
}
