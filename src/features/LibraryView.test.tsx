import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  listLibrary,
  listTags,
  removeFromListenList,
  updateAlbumTags,
  type ListItem,
  type ListResult,
  type TagInfo,
} from "../services/library";
import { notifyLibraryChanged } from "../services/libraryEvents";
import { logListen, undoLog } from "../services/listening";
import { addManualAlbum } from "../services/metadata";
import { renderApp } from "../test/renderApp";

vi.mock("../services/library", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../services/library")>()),
  listLibrary: vi.fn(),
  listTags: vi.fn(),
  removeFromListenList: vi.fn(),
  updateAlbumTags: vi.fn(),
  createTag: vi.fn(),
}));
vi.mock("../services/preferences", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../services/preferences")>()),
  updatePreferences: vi.fn((patch: object) =>
    Promise.resolve({
      sidebarCollapsed: false,
      lastRoute: "/listen-list",
      albumLayout: "grid",
      ...patch,
    }),
  ),
}));
vi.mock("../services/listening", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../services/listening")>()),
  logListen: vi.fn(),
  undoLog: vi.fn(() => Promise.resolve()),
}));
vi.mock("../services/metadata", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../services/metadata")>()),
  addManualAlbum: vi.fn(() => Promise.resolve()),
}));
vi.mock("../services/health", () => ({ checkHealth: vi.fn(() => new Promise(() => undefined)) }));

const ASAP = {
  id: "00000000-0000-7000-8000-000000000001",
  name: "Listen ASAP",
  color: "#f59e0b",
  builtin: true,
};
const ROAD = {
  id: "0190f5c3-0000-7000-8000-00000000000b",
  name: "Road trip",
  color: "#1d4ed8",
  builtin: false,
};
const TAGS: TagInfo[] = [
  { ...ASAP, albumCount: 1 },
  { ...ROAD, albumCount: 2 },
  {
    id: "0190f5c3-0000-7000-8000-00000000000c",
    name: "Unused",
    color: "#34d399",
    builtin: false,
    albumCount: 0,
  },
];

function item(n: number, over: Partial<ListItem> = {}): ListItem {
  return {
    albumId: `0190f5c3-0000-7000-8000-00000000000${String(n)}`,
    editionId: `0190f5c3-0000-7000-8000-0000000000e${String(n)}`,
    title: `Album ${String(n)}`,
    artists: [{ id: "0190f5c3-0000-7000-8000-0000000000a1", name: "Björk" }],
    credit: "Björk",
    originalYear: 1997,
    editionName: "Standard",
    editionCount: 1,
    genres: ["Electronic", "Pop"],
    tags: [],
    addedAt: "2026-09-12T10:00:00.000Z",
    artwork: null,
    listening: { lastListened: null, listenCount: 0, undatedListens: 0, earlierUndated: false },
    rating: {
      effective: null,
      source: "unrated" as const,
      explicit: null,
      calculated: null,
      ratedTracks: 0,
      totalTracks: 0,
    },
    ...over,
  };
}

const ITEMS = [
  item(1, {
    title: "Homogenic",
    editionName: "Deluxe",
    editionCount: 2,
    tags: [ASAP, ROAD],
    rating: {
      effective: 7,
      source: "calculated",
      explicit: null,
      calculated: 7,
      ratedTracks: 3,
      totalTracks: 10,
    },
  }),
  item(2, { title: "Vespertine", originalYear: null, genres: [], tags: [ROAD] }),
];

function result(items: ListItem[], total = items.length): ListResult {
  return {
    items,
    total,
    genres: [
      { name: "Electronic", count: 1 },
      { name: "Pop", count: 1 },
    ],
    tags: [
      { ...ASAP, count: 1 },
      { ...ROAD, count: 2 },
    ],
  };
}

function lastQuery() {
  return vi.mocked(listLibrary).mock.calls.at(-1)?.[1];
}

beforeEach(() => {
  vi.mocked(listLibrary).mockResolvedValue(result(ITEMS));
  vi.mocked(listTags).mockResolvedValue(TAGS);
  vi.mocked(updateAlbumTags).mockImplementation(() => {
    notifyLibraryChanged();
    return Promise.resolve({ added: 1, removed: 0 });
  });
  vi.mocked(removeFromListenList).mockResolvedValue(1);
});

describe("Listen List", () => {
  it("shows artwork placeholder, linked artist, year, edition badge, genres, tags, and date added", async () => {
    renderApp("/listen-list");
    const card = (await screen.findByRole("link", { name: "Homogenic" })).closest("article");
    if (!card) throw new Error("no card");
    const c = within(card);
    expect(c.getByRole("link", { name: "Björk" })).toHaveAttribute(
      "href",
      "/artists/0190f5c3-0000-7000-8000-0000000000a1",
    );
    expect(c.getByText("1997")).toBeInTheDocument();
    expect(c.getByText("Deluxe")).toHaveClass("edition-badge");
    expect(c.getByText("Electronic · Pop")).toBeInTheDocument();
    expect(c.getByRole("list", { name: "Tags on Homogenic" })).toHaveTextContent(
      "Listen ASAPRoad trip",
    );
    expect(c.getByText(/^Added /)).toBeInTheDocument();
    expect(c.getByTestId("artwork-placeholder")).toBeInTheDocument();
    expect(c.getByText("3.5 of 5 stars")).toBeInTheDocument();
    expect(c.getByText("Calculated from 3/10 rated tracks")).toBeInTheDocument();
    const other = screen.getByRole("link", { name: "Vespertine" }).closest("article");
    expect(within(other as HTMLElement).getByText("Year unknown")).toBeInTheDocument();
    expect(within(other as HTMLElement).getByText("No genres")).toBeInTheDocument();
    expect(within(other as HTMLElement).queryByText("Standard")).not.toBeInTheDocument();
  });

  it("searches, sorts, and filters with genre and tag bubbles", async () => {
    renderApp("/listen-list");
    await userEvent.type(await screen.findByLabelText("Search Listen List"), "björk");
    await waitFor(() => {
      expect(lastQuery()).toMatchObject({ search: "björk" });
    });
    await userEvent.selectOptions(screen.getByLabelText("Sort"), "Year (oldest)");
    expect(lastQuery()).toMatchObject({ sort: "year_oldest" });
    await userEvent.selectOptions(screen.getByLabelText("Sort"), "Rating (highest)");
    expect(lastQuery()).toMatchObject({ sort: "rating_highest" });
    await userEvent.click(screen.getByRole("button", { name: /^Pop/ }));
    await userEvent.click(
      within(screen.getByRole("group", { name: "Tags" })).getByRole("button", {
        name: /Road trip/,
      }),
    );
    expect(lastQuery()).toMatchObject({ genres: ["Pop"], tagIds: [ROAD.id] });
  });

  it("makes zero results useful and lets filters be cleared", async () => {
    renderApp("/listen-list");
    await userEvent.click(await screen.findByRole("button", { name: /^Pop/ }));
    vi.mocked(listLibrary).mockResolvedValue(result([], 2));
    await userEvent.type(screen.getByLabelText("Search Listen List"), "zzz");
    expect(await screen.findByRole("heading", { name: "No albums match" })).toBeInTheDocument();
    expect(screen.getByText(/Nothing matches “zzz” · genre: Pop/)).toBeInTheDocument();
    expect(screen.getByText(/^0 of 2 albums/)).toBeInTheDocument();
    vi.mocked(listLibrary).mockResolvedValue(result(ITEMS));
    await userEvent.click(screen.getByRole("button", { name: "Clear search and filters" }));
    await waitFor(() => {
      expect(lastQuery()).toMatchObject({ genres: [], tagIds: [] });
    });
    expect(lastQuery()).not.toHaveProperty("search");
  });

  it("guides an empty list to adding or importing", async () => {
    vi.mocked(listLibrary).mockResolvedValue(result([], 0));
    renderApp("/listen-list");
    const empty = (
      await screen.findByRole("heading", { name: "Your Listen List is empty" })
    ).closest("div");
    expect(
      within(empty as HTMLElement).getByRole("button", { name: "Add album" }),
    ).toBeInTheDocument();
    expect(within(empty as HTMLElement).getByRole("link", { name: "Import CSV" })).toHaveAttribute(
      "href",
      "/listen-list/import",
    );
    expect(screen.queryByLabelText("Search Listen List")).not.toBeInTheDocument();
  });

  it("tags a single album from the plus beside its genres, updating the view", async () => {
    renderApp("/listen-list");
    const plus = await screen.findByRole("button", { name: "Edit tags for Vespertine" });
    await userEvent.click(plus);
    const pop = screen.getByRole("dialog", { name: "Tags for Vespertine" });
    const asap = await within(pop).findByRole("checkbox", { name: /Listen ASAP/ });
    expect(asap).not.toBeChecked();
    expect(within(pop).getByRole("checkbox", { name: /Road trip/ })).toBeChecked();
    const calls = vi.mocked(listLibrary).mock.calls.length;
    await userEvent.click(asap);
    expect(updateAlbumTags).toHaveBeenCalledWith([ITEMS[1]?.albumId], {
      add: [ASAP.id],
      remove: [],
    });
    await waitFor(() => {
      expect(vi.mocked(listLibrary).mock.calls.length).toBeGreaterThan(calls);
    });
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "Tags for Vespertine" })).not.toBeInTheDocument();
    expect(plus).toHaveFocus();
  });

  it("removes one album only after confirming, keeping its history", async () => {
    renderApp("/listen-list");
    await userEvent.click(
      await screen.findByRole("button", { name: "Remove Homogenic from Listen List" }),
    );
    const dialog = screen.getByRole("dialog", { name: "Remove from Listen List?" });
    expect(dialog).toHaveTextContent("tags, ratings, reviews, and listening history");
    expect(removeFromListenList).not.toHaveBeenCalled();
    await userEvent.click(within(dialog).getByRole("button", { name: "Remove" }));
    expect(removeFromListenList).toHaveBeenCalledWith([ITEMS[0]?.albumId]);
    expect(
      await screen.findByText(/Removed “Homogenic” from your Listen List/),
    ).toBeInTheDocument();
  });

  it("bulk tags selected albums and confirms tag removal", async () => {
    renderApp("/listen-list");
    await userEvent.click(await screen.findByRole("button", { name: "Select" }));
    await userEvent.click(screen.getByRole("checkbox", { name: "Select Homogenic" }));
    await userEvent.click(screen.getByRole("checkbox", { name: "Select Vespertine" }));
    expect(screen.getByText("2 selected")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Tag…" }));
    const pop = screen.getByRole("dialog", { name: "Tags for 2 albums" });
    const asap = await within(pop).findByRole("checkbox", { name: /Listen ASAP/ });
    expect(asap).toHaveAttribute("aria-checked", "mixed");
    await userEvent.click(within(pop).getByRole("checkbox", { name: /Road trip/ }));
    await userEvent.click(within(pop).getByRole("button", { name: "Apply" }));
    expect(within(pop).getByRole("alert")).toHaveTextContent("Remove 1 tag from 2 albums?");
    expect(updateAlbumTags).not.toHaveBeenCalled();
    await userEvent.click(within(pop).getByRole("button", { name: "Yes, apply" }));
    expect(updateAlbumTags).toHaveBeenCalledWith([ITEMS[0]?.albumId, ITEMS[1]?.albumId], {
      add: [],
      remove: [ROAD.id],
    });
  });

  it("bulk removes after confirmation", async () => {
    renderApp("/listen-list");
    await userEvent.click(await screen.findByRole("button", { name: "Select" }));
    await userEvent.click(screen.getByRole("button", { name: "Select all shown" }));
    await userEvent.click(screen.getByRole("button", { name: "Remove from Listen List" }));
    await userEvent.click(
      within(screen.getByRole("dialog", { name: "Remove from Listen List?" })).getByRole("button", {
        name: "Remove",
      }),
    );
    expect(removeFromListenList).toHaveBeenCalledWith([ITEMS[0]?.albumId, ITEMS[1]?.albumId]);
  });

  it("recovers from load errors", async () => {
    vi.mocked(listLibrary).mockRejectedValueOnce(new Error("database locked"));
    renderApp("/listen-list");
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("database locked");
    await userEvent.click(within(alert).getByRole("button", { name: "Try again" }));
    expect(await screen.findByRole("link", { name: "Homogenic" })).toBeInTheDocument();
  });
});

describe("Collection", () => {
  it("shows the same album-level tags without Listen List removal", async () => {
    renderApp("/collection");
    expect(await screen.findByRole("list", { name: "Tags on Homogenic" })).toHaveTextContent(
      "Road trip",
    );
    expect(vi.mocked(listLibrary).mock.calls[0]?.[0]).toBe("collection");
    expect(
      screen.queryByRole("button", { name: /Remove .* from Listen List/ }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Edit tags for Homogenic" })).toBeInTheDocument();
  });

  it("shows listening dates and counts, transparently, with a Recently listened sort", async () => {
    vi.mocked(listLibrary).mockResolvedValue(
      result([
        item(1, {
          title: "Homogenic",
          listening: {
            lastListened: "2024-03-02",
            listenCount: 3,
            undatedListens: 0,
            earlierUndated: false,
          },
        }),
        item(2, {
          title: "Vespertine",
          listening: {
            lastListened: null,
            listenCount: 1,
            undatedListens: 1,
            earlierUndated: false,
          },
        }),
        item(3, { title: "Debut" }),
      ]),
    );
    const user = userEvent.setup();
    renderApp("/collection");
    const card = (await screen.findByRole("link", { name: "Homogenic" })).closest("article");
    expect(card).toHaveTextContent(/Last listened .*2024 · 3 listens/);
    expect(screen.getByRole("link", { name: "Vespertine" }).closest("article")).toHaveTextContent(
      "Listened · date unknown · 1 listen",
    );
    expect(screen.getByRole("link", { name: "Debut" }).closest("article")).toHaveTextContent(
      "Not listened yet",
    );
    await user.selectOptions(screen.getByLabelText("Sort"), "Recently listened");
    await waitFor(() => {
      expect(lastQuery()?.sort).toBe("recently_listened");
    });
  });

  it("adds an album directly, without a listening date", async () => {
    const user = userEvent.setup();
    renderApp("/collection");
    await user.click(await screen.findByRole("button", { name: "Add album" }));
    const dialog = screen.getByRole("dialog", { name: "Add to Collection" });
    await user.click(within(dialog).getByRole("button", { name: "Enter manually" }));
    await user.type(within(dialog).getByLabelText("Album title"), "Basement Demo");
    await user.type(within(dialog).getByLabelText("Artist"), "Local Band");
    await user.click(within(dialog).getByRole("button", { name: "Add without matching" }));
    await waitFor(() => {
      expect(addManualAlbum).toHaveBeenCalledWith(
        expect.objectContaining({ title: "Basement Demo" }),
        false,
        true,
      );
    });
    expect(await screen.findByText(/Added “Basement Demo” to your Collection/)).toBeInTheDocument();
  });

  it("marks an album listened from its card, with undo", async () => {
    const logged = {
      listenId: "l1",
      isFull: true,
      coverage: "tracks" as const,
      coveredTracks: 10,
      addedToCollection: false,
      removedFromListenList: false,
      completedNextUp: false,
      undo: {
        listenId: "l1",
        priorHistoryId: null,
        restoreListenList: null,
        createdCollectionFor: null,
        completedAttempt: null,
      },
    };
    vi.mocked(logListen).mockResolvedValue(logged);
    const user = userEvent.setup();
    renderApp("/collection");
    await user.click(await screen.findByRole("button", { name: "Mark Homogenic listened" }));
    await user.click(screen.getByRole("button", { name: "Mark listened" }));
    expect(vi.mocked(logListen).mock.calls[0]?.[1]).toMatchObject({
      editionId: ITEMS[0]?.editionId,
      trackIds: null,
    });
    await user.click(await screen.findByRole("button", { name: "Undo" }));
    expect(undoLog).toHaveBeenCalledWith(logged.undo);
  });
});
