import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { notifyLibraryChanged } from "../../services/libraryEvents";
import { statsOverview, type Best, type Stats } from "../../services/stats";
import { renderApp } from "../../test/renderApp";

vi.mock("../../services/stats", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/stats")>()),
  statsOverview: vi.fn(),
}));
vi.mock("../../services/preferences", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/preferences")>()),
  updatePreferences: vi.fn((patch: object) =>
    Promise.resolve({
      sidebarCollapsed: false,
      lastRoute: "/stats",
      albumLayout: "grid",
      startPage: "last",
      showArtwork: true,
      ...patch,
    }),
  ),
}));
vi.mock("../../services/health", () => ({
  checkHealth: vi.fn(() => new Promise(() => undefined)),
}));

const ALPHA = "0190f5c3-0000-7000-8000-0000000000a1";
const BETA = "0190f5c3-0000-7000-8000-0000000000a2";

const noBest: Best = { top: [], minAlbums: 3, groups: [], unplacedAlbums: 0 };
const group = (
  key: string,
  label: string,
  meanStars: number,
  albums: number,
  tracks: number | null = null,
) => ({ key, label, meanStars, albums, tracks });

function stats(over: Partial<Stats> = {}): Stats {
  return {
    listenList: {
      entries: 3,
      distinctAlbums: 3,
      distinctArtists: 2,
      artists: [
        { key: ALPHA, label: "Alpha", count: 2 },
        { key: BETA, label: "Beta", count: 1 },
      ],
      years: [{ key: "1994", label: "1994", count: 2 }],
      decades: [{ key: "1990", label: "1990", count: 2 }],
      unknownYears: 1,
      genres: [{ key: "Rock", label: "Rock", count: 2 }],
      albumsWithoutGenre: 1,
      tags: [{ key: "00000000-0000-7000-8000-000000000001", label: "Listen ASAP", count: 1 }],
      recentlyAdded: [],
      oldestWaiting: [
        { albumId: "x", editionId: "e", title: "Old One", addedAt: "2026-09-01", waitingDays: 31 },
      ],
      waiting: { meanDays: 13.7, medianDays: 10, longestDays: 31 },
      runtime: { knownMs: 420_000, editionsKnown: 2, editionsUnknown: 1 },
    },
    collection: {
      albums: 2,
      distinctListenedAlbums: 2,
      listens: {
        dated: 3,
        undated: 1,
        historical: 0,
        first: 2,
        relisten: 1,
        unspecified: 1,
        fullAlbum: 4,
        tracksOnly: 0,
      },
      byMonth: [{ key: "2026-08", label: "2026-08", count: 3 }],
      byYear: [{ key: "2026", label: "2026", count: 3 }],
      ratings: {
        ratedAlbums: 1,
        unratedAlbums: 1,
        averageStars: 0,
        distribution: [{ stars: 0, albums: 1 }],
      },
      bestYear: {
        top: [group("1994", "1994", 0, 3)],
        minAlbums: 3,
        groups: [group("1994", "1994", 0, 3)],
        unplacedAlbums: 2,
      },
      bestDecade: {
        top: [group("1990", "1990s", 0, 3)],
        minAlbums: 3,
        groups: [group("1990", "1990s", 0, 3)],
        unplacedAlbums: 2,
      },
      bestGenre: {
        top: [group("Rock", "Rock", 0, 3)],
        minAlbums: 3,
        groups: [group("Rock", "Rock", 0, 3), group("Jazz", "Jazz", 4, 1)],
        unplacedAlbums: 0,
      },
      // Albums rated zero still rank.
      bestArtist: {
        top: [group(ALPHA, "Alpha", 0, 3)],
        minAlbums: 3,
        groups: [group(ALPHA, "Alpha", 0, 3)],
        unplacedAlbums: 0,
      },
      // By tracks the picture differs: Jazz now has tracks from three albums.
      trackRankings: {
        ratedTracks: 9,
        bestYear: noBest,
        bestDecade: {
          top: [group("1990", "1990s", 2.5, 3, 6)],
          minAlbums: 3,
          groups: [group("1990", "1990s", 2.5, 3, 6)],
          unplacedAlbums: 3,
        },
        bestGenre: {
          top: [group("Jazz", "Jazz", 4.5, 3, 4), group("Rock", "Rock", 1, 3, 5)],
          minAlbums: 3,
          groups: [group("Jazz", "Jazz", 4.5, 3, 4), group("Rock", "Rock", 1, 3, 5)],
          unplacedAlbums: 0,
        },
        bestArtist: {
          top: [group(ALPHA, "Alpha", 2, 3, 7)],
          minAlbums: 3,
          groups: [group(ALPHA, "Alpha", 2, 3, 7)],
          unplacedAlbums: 0,
        },
      },
      favouriteTracks: [],
      runtime: {
        sessions: 3,
        knownMs: 600_000,
        tracksCounted: 4,
        tracksWithoutLength: 1,
        listensWithoutTracklist: 0,
        complete: false,
      },
    },
    selection: [
      { source: "completely_random", shown: 3, skipped: 2, completed: 1, pending: 0 },
      { source: "weighted_random", shown: 0, skipped: 0, completed: 0, pending: 0 },
      { source: "guided", shown: 0, skipped: 0, completed: 0, pending: 0 },
      { source: "manual", shown: 1, skipped: 0, completed: 0, pending: 1 },
      { source: "all", shown: 4, skipped: 2, completed: 1, pending: 1 },
    ],
    ...over,
  };
}

beforeEach(() => {
  vi.mocked(statsOverview).mockResolvedValue(stats());
});

describe("Stats page", () => {
  it("gives Listen List and Collection their own labelled sections", async () => {
    renderApp("/stats");
    const listen = await screen.findByRole("region", { name: "Listen List" });
    const collection = screen.getByRole("region", { name: "Collection" });
    expect(within(listen).getByRole("list", { name: "Listen List overview" })).toBeInTheDocument();
    expect(
      within(collection).getByRole("list", { name: "Collection overview" }),
    ).toBeInTheDocument();
  });

  it("drills summaries down into filtered albums", async () => {
    renderApp("/stats");
    const listen = await screen.findByRole("region", { name: "Listen List" });
    expect(within(listen).getByRole("link", { name: "1990s" })).toHaveAttribute(
      "href",
      "/listen-list?decade=1990",
    );
    expect(within(listen).getByRole("link", { name: "Rock" })).toHaveAttribute(
      "href",
      "/listen-list?genre=Rock",
    );
    expect(within(listen).getByRole("link", { name: "Alpha" })).toHaveAttribute(
      "href",
      `/listen-list?artist=${ALPHA}`,
    );
    expect(within(listen).queryByText(/Tagged Listen ASAP/)).not.toBeInTheDocument();
    const collection = screen.getByRole("region", { name: "Collection" });
    const bestGenre = within(collection).getByRole("region", { name: "Best genre" });
    expect(within(bestGenre).getByRole("link", { name: "Rock" })).toHaveAttribute(
      "href",
      "/collection?genre=Rock",
    );
    const bestYear = within(collection).getByRole("region", { name: "Best release year" });
    expect(within(bestYear).getByRole("link", { name: "1994" })).toHaveAttribute(
      "href",
      "/collection?year=1994",
    );
  });

  it("renders charts as captioned tables with readable values", async () => {
    renderApp("/stats");
    const chart = await screen.findByRole("figure", { name: "Albums by decade" });
    const table = within(chart).getByRole("table");
    expect(within(table).getByRole("rowheader", { name: "1990s" })).toBeInTheDocument();
    expect(within(table).getByRole("cell", { name: "2" })).toBeInTheDocument();
    const picks = screen.getByRole("figure", { name: "Next Up picks by method" });
    const random = within(picks).getByRole("row", { name: /Completely Random/ });
    // Shown, skipped, listened, current, and listened of resolved (1 of 3).
    expect(
      within(random)
        .getAllByRole("cell")
        .map((c) => c.textContent),
    ).toEqual(["3", "2", "1", "0", "33%"]);
  });

  it("ranks a zero-rated artist with its sample count", async () => {
    renderApp("/stats");
    const best = await screen.findByRole("region", { name: "Best artist" });
    expect(within(best).getByRole("link", { name: "Alpha" })).toHaveAttribute(
      "href",
      `/collection?artist=${ALPHA}`,
    );
    expect(best).toHaveTextContent("0 of 5 stars from 3 rated albums");
    expect(screen.getByText("Time listened (at least)")).toBeInTheDocument();
    expect(screen.getByText("across 3 listening sessions")).toBeInTheDocument();
  });

  it("leads with unique albums and total listens, and leaves out missing-data notes", async () => {
    renderApp("/stats");
    const overview = await screen.findByRole("list", { name: "Collection overview" });
    const cards = within(overview)
      .getAllByRole("listitem")
      .map((c) => c.textContent);
    expect(cards.slice(0, 2)).toEqual([
      "2Unique albums in Collection",
      // Three dated listens plus one undated.
      "4Total listens",
    ]);
    expect(overview).not.toHaveTextContent("Albums listened to");
    expect(
      within(overview).getByRole("link", { name: /Unique albums in Collection/ }),
    ).toHaveAttribute("href", "/collection");
    // The fixture has an undated listen, an unrated album, albums without a genre or year,
    // and tracks of unknown length: none of that is called out.
    const page = screen.getByRole("region", { name: "Collection" }).parentElement;
    expect(page).not.toHaveTextContent(
      /undated|unrated|without a genre|unknown length|Incomplete|not included|not shown|earlier-history/i,
    );
  });

  it("lists the top three with enough rated albums, best first", async () => {
    const GAMMA = "0190f5c3-0000-7000-8000-0000000000a3";
    const ranked: Best = {
      top: [group(ALPHA, "Alpha", 4, 3), group(BETA, "Beta", 3, 4), group(GAMMA, "Gamma", 2, 3)],
      minAlbums: 3,
      groups: [
        group("solo", "Solo", 5, 1),
        group(ALPHA, "Alpha", 4, 3),
        group(BETA, "Beta", 3, 4),
        group(GAMMA, "Gamma", 2, 3),
        group("delta", "Delta", 1, 3),
      ],
      unplacedAlbums: 0,
    };
    vi.mocked(statsOverview).mockResolvedValue(
      stats({ collection: { ...stats().collection, bestArtist: ranked, bestYear: noBest } }),
    );
    renderApp("/stats");
    const best = await screen.findByRole("region", { name: "Best artist" });
    expect(
      within(best)
        .getAllByRole("link")
        .map((a) => a.textContent),
    ).toEqual(["Alpha", "Beta", "Gamma"]);
    expect(best).toHaveTextContent("3 of 5 stars from 4 rated albums");
    expect(screen.getByRole("region", { name: "Best release year" })).toHaveTextContent(
      "Nothing has 3 rated albums yet.",
    );
  });

  it("switches Top rated between album and track ratings", async () => {
    const user = userEvent.setup();
    renderApp("/stats");
    const rankBy = await screen.findByRole("group", { name: "Rank by" });
    const albums = within(rankBy).getByRole("button", { name: "Albums" });
    const tracks = within(rankBy).getByRole("button", { name: "Tracks" });
    expect(albums).toHaveAttribute("aria-pressed", "true");
    const bestGenre = () => screen.getByRole("region", { name: "Best genre" });
    expect(
      within(bestGenre())
        .getAllByRole("link")
        .map((a) => a.textContent),
    ).toEqual(["Rock"]);
    expect(bestGenre()).toHaveTextContent("from 3 rated albums");

    await user.click(tracks);
    expect(tracks).toHaveAttribute("aria-pressed", "true");
    expect(albums).toHaveAttribute("aria-pressed", "false");
    expect(
      within(bestGenre())
        .getAllByRole("link")
        .map((a) => a.textContent),
    ).toEqual(["Jazz", "Rock"]);
    expect(bestGenre()).toHaveTextContent("4.5 of 5 stars from 4 rated tracks on 3 albums");
    expect(screen.getByRole("region", { name: "Best release year" })).toHaveTextContent(
      "Nothing has rated tracks from 3 albums yet.",
    );
    expect(screen.getByText(/averaged track by track/)).toBeInTheDocument();
    // The tables below follow the toggle too.
    const genres = screen.getByRole("figure", { name: "Highest-rated genres" });
    expect(within(genres).getByText("Jazz")).toBeInTheDocument();
    expect(within(genres).getByText(/4\.5 ★ · 4 tracks/)).toBeInTheDocument();

    await user.click(albums);
    expect(
      within(bestGenre())
        .getAllByRole("link")
        .map((a) => a.textContent),
    ).toEqual(["Rock"]);
  });

  it("drops undated notes, the listening-kind chart, and doubled decade suffixes", async () => {
    renderApp("/stats");
    const collection = await screen.findByRole("region", { name: "Collection" });
    expect(collection).not.toHaveTextContent(/without a date|unknown original year/);
    expect(screen.queryByText(/with an unknown release year/)).not.toBeInTheDocument();
    expect(screen.queryByRole("figure", { name: "How you listen" })).not.toBeInTheDocument();
    const decades = screen.getByRole("figure", { name: "Average rating by decade" });
    expect(within(decades).getByText("1990s")).toBeInTheDocument();
    expect(within(decades).queryByText("1990ss")).not.toBeInTheDocument();
    // Genres need three rated albums to be ranked: Jazz (one album) is left out.
    const genres = screen.getByRole("figure", { name: "Highest-rated genres" });
    expect(within(genres).queryByText("Jazz")).not.toBeInTheDocument();
    expect(within(genres).getByText("Rock")).toBeInTheDocument();
  });

  it("charts ratings in half-star buckets and shows any chart as a table on request", async () => {
    const user = userEvent.setup();
    renderApp("/stats");
    const ratings = await screen.findByRole("figure", { name: "Rating distribution" });
    const table = within(ratings).getByRole("table", { hidden: true });
    expect(table.parentElement).toHaveClass("visually-hidden");
    expect(
      within(table).getByRole("row", { name: "0 to under 0.5 stars 1 album" }),
    ).toBeInTheDocument();
    const toggle = within(ratings).getByRole("button", { name: "Table" });
    await user.click(toggle);
    expect(toggle).toHaveAttribute("aria-pressed", "true");
    expect(within(ratings).getByRole("table").parentElement).not.toHaveClass("visually-hidden");
    expect(screen.getByRole("figure", { name: "Listening activity" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Liked tracks/ })).toHaveAttribute("href", "/liked");
  });

  it("is honest about empty data", async () => {
    const empty = stats();
    vi.mocked(statsOverview).mockResolvedValue({
      ...empty,
      listenList: { ...empty.listenList, entries: 0, distinctAlbums: 0 },
      collection: { ...empty.collection, albums: 0, distinctListenedAlbums: 0 },
      selection: empty.selection.map((m) => ({
        ...m,
        shown: 0,
        skipped: 0,
        completed: 0,
        pending: 0,
      })),
    });
    renderApp("/stats");
    expect(await screen.findByText("Your Listen List is empty")).toBeInTheDocument();
    expect(screen.getByText("Your Collection is empty")).toBeInTheDocument();
    expect(screen.getByText("No Next Up picks yet.")).toBeInTheDocument();
    expect(screen.queryByText(/NaN/)).not.toBeInTheDocument();
  });

  it("updates immediately after data changes", async () => {
    renderApp("/stats");
    await screen.findByRole("region", { name: "Listen List" });
    vi.mocked(statsOverview).mockResolvedValue(
      stats({ listenList: { ...stats().listenList, distinctAlbums: 4 } }),
    );
    act(() => {
      notifyLibraryChanged();
    });
    await waitFor(() => {
      expect(screen.getByRole("link", { name: /^4\s*Albums waiting/ })).toBeInTheDocument();
    });
  });
});
