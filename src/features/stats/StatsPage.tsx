import { useEffect, useState, type ReactNode } from "react";
import { Link } from "react-router";
import { PageHeader } from "../../components/PageHeader";
import { EmptyState, ErrorState, LoadingState } from "../../components/States";
import { useLibraryVersion } from "../../services/libraryEvents";
import {
  completionRate,
  statsOverview,
  type Best,
  type CollectionStats,
  type ListenListStats,
  type MethodStats,
  type Stats,
} from "../../services/stats";
import { BarTable } from "./BarTable";
import { BestHighlight } from "./BestHighlight";
import { ChartFrame, ColumnChart, StackedBar } from "./charts";
import {
  continuousYears,
  durationParts,
  formatDuration,
  formatStars,
  halfStarBuckets,
  lastTwelveMonths,
  METHOD_LABEL,
  plural,
} from "./format";
import { StatCard } from "./StatCard";
import "./stats.css";

const LISTEN_ASAP = "00000000-0000-7000-8000-000000000001";
const TOP = 10;
const TOP_GENRES = 8;

/** Validated (dark surface) categorical slots, plus a neutral for "not specified". */
const SERIES = ["var(--series-1)", "var(--series-2)", "var(--series-3)"] as const;
const NEUTRAL = "var(--series-neutral)";

type Load =
  | { kind: "loading" }
  | { kind: "error"; code: string; message: string }
  | { kind: "ready"; stats: Stats };

const list = (query: string) => `/listen-list?${query}`;
const collection = (query: string) => `/collection?${query}`;
const q = (key: string, value: string) => `${key}=${encodeURIComponent(value)}`;
const oneDecimal = (n: number) => String(Math.round(n * 10) / 10);

function Block({
  id,
  title,
  description,
  children,
}: {
  id: string;
  title: string;
  description: string;
  children: ReactNode;
}) {
  return (
    <section className="stats-block" aria-labelledby={id}>
      <header className="stats-block-head">
        <h2 id={id} className="stats-block-title">
          {title}
        </h2>
        <p className="setting-hint">{description}</p>
      </header>
      {children}
    </section>
  );
}

function Hero({ s }: { s: CollectionStats }) {
  const r = s.runtime;
  const hint = r.complete
    ? `across ${plural(r.sessions, "listening session")}`
    : `Incomplete: ${[
        r.tracksWithoutLength > 0 && plural(r.tracksWithoutLength, "track") + " of unknown length",
        r.listensWithoutTracklist > 0 &&
          plural(r.listensWithoutTracklist, "listen") + " without a tracklist",
      ]
        .filter(Boolean)
        .join(", ")}`;
  return (
    <div className="hero-figure">
      <span className="hero-eyebrow">
        {r.complete ? "Time listened" : "Time listened (at least)"}
      </span>
      <span className="hero-value">
        {durationParts(r.knownMs).map((p, i) => (
          <span key={p.unit}>
            {i > 0 && " "}
            <span className="hero-number">{p.value}</span>
            <span className="hero-unit"> {p.unit}</span>
          </span>
        ))}
      </span>
      <span className="hero-hint">{hint}</span>
    </div>
  );
}

function RatingValue({ stars }: { stars: number | null }) {
  if (stars === null) return <>No ratings</>;
  return (
    <>
      <span aria-hidden="true">
        {oneDecimal(stars)}
        <span className="stat-star"> ★</span>
      </span>
      <span className="visually-hidden">{formatStars(stars)}</span>
    </>
  );
}

function groupRows(best: Best, href: (key: string) => string) {
  return best.groups.map((g) => ({
    key: g.key,
    label: g.label,
    value: g.meanStars,
    display: `${oneDecimal(g.meanStars)} ★ · ${plural(g.albums, "album")}`,
    href: href(g.key),
  }));
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
  const months = lastTwelveMonths(s.byMonth);
  const monthTotal = months.reduce((t, m) => t + m.count, 0);
  const years = continuousYears(s.byYear);
  const buckets = halfStarBuckets(s.ratings.distribution);
  const decades = [...s.bestDecade.groups].sort((a, b) => Number(a.key) - Number(b.key));
  const genres = [...s.bestGenre.groups]
    .sort((a, b) => b.meanStars - a.meanStars || b.albums - a.albums)
    .slice(0, TOP_GENRES);
  return (
    <>
      <div className="stats-hero">
        <Hero s={s} />
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
            value={<RatingValue stars={s.ratings.averageStars} />}
            hint={`${plural(s.ratings.ratedAlbums, "rated album")} · ${String(s.ratings.unratedAlbums)} unrated`}
            href={collection("sort=rating_highest")}
          />
          <StatCard
            label="Liked tracks"
            value={String(s.favouriteTracks.length)}
            hint="in your Collection"
            href="/liked"
          />
        </ul>
      </div>

      <div className="stats-grid">
        <ColumnChart
          caption="Listening activity"
          subtitle={`${plural(monthTotal, "listen")} in the last 12 months`}
          valueHeading="Listens"
          rows={months.map((m) => ({
            key: m.key,
            label: m.label,
            axisLabel: m.short,
            value: m.count,
          }))}
          empty="No dated listens in the last 12 months."
        />
        <ColumnChart
          caption="Listens by year"
          subtitle={years.length > 0 ? `Since ${years[0]?.key ?? ""}` : undefined}
          valueHeading="Listens"
          rows={years.map((y) => ({ key: y.key, label: y.key, value: y.count }))}
          empty="No dated listens yet."
          note={
            n.undated > 0 ? `${plural(n.undated, "listen")} without a date not shown.` : undefined
          }
        />
        <ColumnChart
          caption="Rating distribution"
          subtitle={
            s.ratings.averageStars === null
              ? "Rated Collection albums"
              : `Average ${formatStars(s.ratings.averageStars)}`
          }
          valueHeading="Albums"
          rows={buckets.map((b) => ({
            key: String(b.stars),
            label:
              b.stars === 5
                ? "5 stars"
                : `${String(b.stars)} to under ${String(b.stars + 0.5)} stars`,
            axisLabel: Number.isInteger(b.stars) ? `${String(b.stars)}★` : "·",
            value: b.albums,
            display: plural(b.albums, "album"),
          }))}
          empty="No rated albums yet."
          note={
            s.ratings.unratedAlbums > 0
              ? `${plural(s.ratings.unratedAlbums, "unrated album")} not shown.`
              : undefined
          }
        />
        <ChartFrame
          caption="How you listen"
          subtitle={plural(n.first + n.relisten + n.unspecified, "listen")}
          empty={n.first + n.relisten + n.unspecified === 0 ? "No listens yet." : undefined}
          chart={
            <div className="stack-group">
              <StackedBar
                label="First or again"
                segments={[
                  { key: "first", label: "First listens", value: n.first, color: SERIES[0] },
                  { key: "relisten", label: "Re-listens", value: n.relisten, color: SERIES[1] },
                  {
                    key: "unspecified",
                    label: "Not specified",
                    value: n.unspecified,
                    color: NEUTRAL,
                  },
                ]}
              />
              <StackedBar
                label="Whole album or tracks"
                segments={[
                  { key: "full", label: "Whole album", value: n.fullAlbum, color: SERIES[0] },
                  { key: "tracks", label: "Tracks only", value: n.tracksOnly, color: SERIES[2] },
                ]}
              />
            </div>
          }
          table={() => (
            <table className="stat-table">
              <thead>
                <tr>
                  <th scope="col">Kind of listen</th>
                  <th scope="col">Listens</th>
                </tr>
              </thead>
              <tbody>
                {[
                  ["First listens", n.first],
                  ["Re-listens", n.relisten],
                  ["Not specified", n.unspecified],
                  ["Whole album", n.fullAlbum],
                  ["Tracks only", n.tracksOnly],
                ].map(([label, value]) => (
                  <tr key={String(label)}>
                    <th scope="row">{label}</th>
                    <td>{value}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        />
      </div>

      <div className="stats-subhead">
        <h3 className="stats-subtitle">Top rated</h3>
        <p className="setting-hint">
          Rated Collection albums only. Each album counts once (its editions’ scores averaged),
          however often you’ve listened; zero is a rating. Years are original release years. Genres
          overlap, so an album counts in each of its genres. Any number of albums can win.
        </p>
      </div>
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
      <div className="stats-grid">
        <BarTable
          caption="Average rating by decade"
          subtitle="Out of 5 stars"
          valueHeading="Average rating"
          max={5}
          rows={groupRows({ ...s.bestDecade, groups: decades }, (k) =>
            collection(q("decade", k)),
          ).map((r) => ({ ...r, label: `${r.label}s` }))}
          empty="No rated albums with a known year yet."
        />
        <BarTable
          caption="Highest-rated genres"
          subtitle={
            s.bestGenre.groups.length > TOP_GENRES
              ? `Top ${String(TOP_GENRES)} of ${String(s.bestGenre.groups.length)}, out of 5 stars`
              : "Out of 5 stars"
          }
          valueHeading="Average rating"
          max={5}
          ranked
          rows={groupRows({ ...s.bestGenre, groups: genres }, (k) => collection(q("genre", k)))}
          empty="No rated albums with a genre yet."
        />
      </div>
    </>
  );
}

function ListenListSection({ s }: { s: ListenListStats }) {
  if (s.entries === 0) {
    return (
      <EmptyState
        icon="stats"
        title="Your Listen List is empty"
        description="Add albums to see decades, genres, artists, and how long albums wait."
      />
    );
  }
  const oldest = s.oldestWaiting[0];
  const decades = [...s.decades].sort((a, b) => Number(a.key) - Number(b.key));
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
      <div className="stats-grid">
        <ColumnChart
          caption="Albums by decade"
          subtitle="Original release years"
          valueHeading="Albums"
          rows={decades.map((d) => ({
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
          ranked
          rows={s.genres.slice(0, TOP).map((g) => ({
            key: g.key,
            label: g.label,
            value: g.count,
            href: list(q("genre", g.label)),
          }))}
          empty="No genres yet."
          note={`${s.genres.length > TOP ? `Top ${String(TOP)} of ${String(s.genres.length)}. ` : ""}Genres overlap: an album counts once in each of its genres.${
            s.albumsWithoutGenre > 0
              ? ` ${plural(s.albumsWithoutGenre, "album")} without a genre.`
              : ""
          }`}
        />
        <BarTable
          caption="Most-listed artists"
          valueHeading="Albums"
          ranked
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
        <WaitingList title="Recently added" items={s.recentlyAdded} />
        <WaitingList title="Waiting longest" items={s.oldestWaiting} />
      </div>
    </>
  );
}

function WaitingList({ title, items }: { title: string; items: ListenListStats["oldestWaiting"] }) {
  return (
    <section className="chart-card" aria-label={title}>
      <h4 className="chart-caption">{title}</h4>
      {items.length === 0 ? (
        <p className="chart-empty">Nothing here yet.</p>
      ) : (
        <ol className="wait-list">
          {items.map((w) => (
            <li key={w.albumId}>
              <Link to={`/albums/${w.albumId}`}>{w.title}</Link>
              <span className="wait-days">
                {w.waitingDays === 0 ? "today" : plural(w.waitingDays, "day")}
              </span>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}

/** Next Up provenance: what each method showed and what came of it. */
function PicksChart({ picks }: { picks: MethodStats[] }) {
  const total = picks.find((p) => p.source === "all");
  if (!total || total.shown === 0) {
    return <p className="setting-hint">No Next Up picks yet.</p>;
  }
  const methods = picks.filter((p) => p.source !== "all" && p.shown > 0);
  const rate = completionRate(total);
  const outcomes = [
    { key: "completed", label: "Listened", color: SERIES[0] },
    { key: "skipped", label: "Skipped", color: SERIES[1] },
    { key: "pending", label: "Current", color: SERIES[2] },
  ] as const;
  return (
    <ChartFrame
      caption="Next Up picks by method"
      subtitle={`${plural(total.shown, "pick")}${
        rate === null ? "" : ` · ${String(Math.round(rate * 100))}% listened of those resolved`
      }`}
      note="From the stored record of each pick, not today’s tags."
      chart={
        <div className="picks">
          <ul className="stack-legend picks-legend" aria-label="Outcomes">
            {outcomes.map((o) => (
              <li key={o.key}>
                <span className="swatch" style={{ background: o.color }} aria-hidden="true" />
                <span className="stack-legend-label">{o.label}</span>
              </li>
            ))}
          </ul>
          {methods.map((m) => (
            <div key={m.source} className="picks-row">
              <span className="picks-label">{METHOD_LABEL[m.source] ?? m.source}</span>
              <div className="stack" aria-hidden="true">
                {outcomes
                  .filter((o) => m[o.key] > 0)
                  .map((o) => (
                    <span
                      key={o.key}
                      className="stack-seg"
                      style={{ flexGrow: m[o.key], background: o.color }}
                      title={`${o.label}: ${String(m[o.key])}`}
                    />
                  ))}
              </div>
              <span className="picks-total">{plural(m.shown, "pick")}</span>
            </div>
          ))}
        </div>
      }
      table={() => (
        // Focusable so keyboard users can scroll it when it is wider than the card.
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
                const r = completionRate(p);
                return (
                  <tr key={p.source}>
                    <th scope="row">{METHOD_LABEL[p.source] ?? p.source}</th>
                    <td>{p.shown}</td>
                    <td>{p.skipped}</td>
                    <td>{p.completed}</td>
                    <td>{p.pending}</td>
                    <td>{r === null ? "—" : `${String(Math.round(r * 100))}%`}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    />
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
    <section className="stats-page">
      <PageHeader title="Stats" description="Your listening, ratings, and what’s waiting." />
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
        <>
          <Block
            id="stats-collection"
            title="Collection"
            description="What you’ve listened to and how you rated it."
          >
            <CollectionSection s={load.stats.collection} />
          </Block>
          <Block
            id="stats-listen-list"
            title="Listen List"
            description="Albums waiting for a first listen."
          >
            <ListenListSection s={load.stats.listenList} />
          </Block>
          <Block
            id="stats-next-up"
            title="Next Up"
            description="How each way of picking your next album has worked out."
          >
            <PicksChart picks={load.stats.selection} />
          </Block>
        </>
      )}
    </section>
  );
}
