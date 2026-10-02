import { act, screen, waitFor, within } from "@testing-library/react";
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

const noBest: Best = { winners: [], groups: [], unplacedAlbums: 0 };
const group = (key: string, label: string, meanStars: number, albums: number) => ({
  key,
  label,
  meanStars,
  albums,
});

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
      listenAsap: 1,
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
        winners: [group("1994", "1994", 0, 1)],
        groups: [group("1994", "1994", 0, 1)],
        unplacedAlbums: 0,
      },
      bestDecade: noBest,
      bestGenre: {
        winners: [group("Rock", "Rock", 0, 1)],
        groups: [group("Rock", "Rock", 0, 1)],
        unplacedAlbums: 0,
      },
      // A single album rated zero is a legitimate winner.
      bestArtist: {
        winners: [group(ALPHA, "Alpha", 0, 1)],
        groups: [group(ALPHA, "Alpha", 0, 1)],
        unplacedAlbums: 0,
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
    expect(
      within(listen).getByRole("link", { name: "1 album with an unknown release year" }),
    ).toHaveAttribute("href", "/listen-list?yearUnknown=1");
    expect(within(listen).getByRole("link", { name: /Tagged Listen ASAP/ })).toHaveAttribute(
      "href",
      "/listen-list?tag=00000000-0000-7000-8000-000000000001",
    );
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

  it("shows a zero-rated single-album winner with its neutral sample count", async () => {
    renderApp("/stats");
    const best = await screen.findByRole("region", { name: "Best artist" });
    expect(within(best).getByRole("link", { name: "Alpha" })).toHaveAttribute(
      "href",
      `/collection?artist=${ALPHA}`,
    );
    expect(best).toHaveTextContent("0 of 5 stars from 1 rated album.");
    expect(screen.getByText("Time listened (at least)")).toBeInTheDocument();
    expect(screen.getByText(/Incomplete: 1 track of unknown length/)).toBeInTheDocument();
  });

  it("names every tied winner", async () => {
    const tied: Best = {
      winners: [group(ALPHA, "Alpha", 3, 2), group(BETA, "Beta", 3, 1)],
      groups: [group(ALPHA, "Alpha", 3, 2), group(BETA, "Beta", 3, 1)],
      unplacedAlbums: 0,
    };
    vi.mocked(statsOverview).mockResolvedValue(
      stats({ collection: { ...stats().collection, bestArtist: tied } }),
    );
    renderApp("/stats");
    const best = await screen.findByRole("region", { name: "Best artist" });
    expect(best).toHaveTextContent("Alpha and Beta");
    expect(best).toHaveTextContent("3 of 5 stars each — tied, 2 rated albums and 1 rated album.");
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
