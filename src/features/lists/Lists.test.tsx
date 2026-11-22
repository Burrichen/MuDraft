import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  addEntryToListenList,
  addListEntries,
  addUnlistenedToListenList,
  createList,
  getList,
  linkListEntry,
  listLists,
  previewListImport,
  reviewMissingOnMusicBrainz,
  type ListDetail,
} from "../../services/lists";
import { importRelease, loadEditions, searchAlbums } from "../../services/metadata";
import { renderApp } from "../../test/renderApp";

vi.mock("../../services/lists", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/lists")>()),
  listLists: vi.fn(),
  getList: vi.fn(),
  createList: vi.fn(() => Promise.resolve("0190f5c3-0000-7000-8000-0000000000aa")),
  addListEntries: vi.fn(() => Promise.resolve({ added: 1, duplicates: 0 })),
  addEntryToListenList: vi.fn(() => Promise.resolve({ added: 1, needsMatch: 0 })),
  addUnlistenedToListenList: vi.fn(() => Promise.resolve({ added: 1, needsMatch: 1 })),
  linkListEntry: vi.fn(() => Promise.resolve()),
  reviewMissingOnMusicBrainz: vi.fn(() => Promise.resolve()),
  removeListEntry: vi.fn(() => Promise.resolve()),
  previewListImport: vi.fn(),
}));
vi.mock("../../services/metadata", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/metadata")>()),
  searchAlbums: vi.fn(),
  loadEditions: vi.fn(),
  importRelease: vi.fn(),
}));
vi.mock("../../services/health", () => ({
  checkHealth: vi.fn(() => new Promise(() => undefined)),
}));

function at<T>(items: readonly T[], i: number): NonNullable<T> {
  const item = items[i];
  if (item === undefined || item === null) throw new Error(`no item at ${String(i)}`);
  return item;
}

const LIST = "0190f5c3-0000-7000-8000-000000000001";
const ALBUM = "0190f5c3-0000-7000-8000-000000000002";

function detail(overrides: Partial<ListDetail> = {}): ListDetail {
  return {
    id: LIST,
    name: "100 Best Indie Rock Albums",
    sourceUrl: "https://pitchfork.com/story/the-100-best/",
    progress: { total: 4, listened: 1, onListenList: 1 },
    entries: [
      {
        id: "e1",
        position: 1,
        artistName: "Pavement",
        title: "Crooked Rain, Crooked Rain",
        year: 1994,
        status: "listened",
        albumId: ALBUM,
      },
      {
        id: "e2",
        position: 2,
        artistName: "Liz Phair",
        title: "Exile in Guyville",
        year: 1993,
        status: "listen_list",
        albumId: null,
      },
      {
        id: "e3",
        position: 3,
        artistName: "Sonic Youth",
        title: "Daydream Nation",
        year: 1988,
        status: "in_library",
        albumId: null,
      },
      {
        id: "e4",
        position: 4,
        artistName: "Slint",
        title: "Spiderland",
        year: null,
        status: "missing",
        albumId: null,
      },
    ],
    ...overrides,
  };
}

describe("Lists", () => {
  it("is in the sidebar and shows each list with its progress", async () => {
    vi.mocked(listLists).mockResolvedValue([
      {
        id: LIST,
        name: "100 Best Indie Rock Albums",
        sourceUrl: "https://www.pitchfork.com/story/the-100-best/",
        progress: { total: 100, listened: 37, onListenList: 12 },
      },
    ]);
    renderApp("/lists");
    expect(screen.getByRole("link", { name: "Lists" })).toHaveAttribute("aria-current", "page");
    const card = await screen.findByRole("link", { name: /100 Best Indie Rock Albums/ });
    expect(card).toHaveAttribute("href", `/lists/${LIST}`);
    expect(within(card).getByText("100 albums · from pitchfork.com")).toBeInTheDocument();
    expect(within(card).getByRole("progressbar")).toHaveAttribute("aria-valuenow", "37");
    expect(within(card).getByText("12 more on your Listen List")).toBeInTheDocument();
  });

  it("explains an empty page and creates a list by name", async () => {
    vi.mocked(listLists).mockResolvedValue([]);
    vi.mocked(getList).mockResolvedValue(
      detail({ entries: [], progress: { total: 0, listened: 0, onListenList: 0 } }),
    );
    renderApp("/lists");
    expect(await screen.findByText("No lists yet")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "New list" }));
    const dialog = await screen.findByRole("dialog", { name: "New list" });
    await userEvent.type(within(dialog).getByLabelText("List name"), "Summer");
    await userEvent.click(within(dialog).getByRole("button", { name: "Create list" }));
    expect(createList).toHaveBeenCalledWith("Summer", null);
    expect(
      await screen.findByRole("heading", { level: 1, name: "100 Best Indie Rock Albums" }),
    ).toBeInTheDocument();
  });

  it("shows every album's state; only the Collection counts toward progress", async () => {
    vi.mocked(getList).mockResolvedValue(detail());
    renderApp(`/lists/${LIST}`);
    const entries = await screen.findByRole("list", {
      name: "Albums in 100 Best Indie Rock Albums",
    });
    const rows = within(entries).getAllByRole("listitem");
    expect(rows).toHaveLength(4);
    expect(within(at(rows, 0)).getByText("Listened")).toBeInTheDocument();
    expect(
      within(at(rows, 0)).getByRole("link", { name: "Crooked Rain, Crooked Rain" }),
    ).toHaveAttribute("href", `/albums/${ALBUM}`);
    expect(within(at(rows, 1)).getByText("On Listen List")).toBeInTheDocument();
    expect(within(at(rows, 2)).getByText("In library")).toBeInTheDocument();
    expect(within(at(rows, 3)).getByText("Not in library")).toBeInTheDocument();
    expect(screen.getByRole("progressbar", { name: "Listened" })).toHaveAttribute(
      "aria-valuetext",
      "1 of 4 (25%)",
    );
    // Only albums that aren't listened to or already on the Listen List offer the button.
    expect(within(at(rows, 0)).queryByRole("button", { name: /Add to Listen List/ })).toBeNull();
    expect(within(at(rows, 1)).queryByRole("button", { name: /Add to Listen List/ })).toBeNull();
    expect(
      within(at(rows, 2)).getByRole("button", { name: /Add to Listen List/ }),
    ).toBeInTheDocument();
  });

  it("adds an album that is already in the library straight away", async () => {
    vi.mocked(getList).mockResolvedValue(detail());
    renderApp(`/lists/${LIST}`);
    const row = at([(await screen.findByText("Daydream Nation")).closest("li")], 0);
    await userEvent.click(within(row).getByRole("button", { name: /Add to Listen List/ }));
    expect(addEntryToListenList).toHaveBeenCalledWith("e3");
    expect(await screen.findByText("Added 1 album to your Listen List.")).toHaveAttribute(
      "role",
      "status",
    );
    expect(searchAlbums).not.toHaveBeenCalled();
  });

  it("adds the unlistened albums in the library and says the rest need a match", async () => {
    vi.mocked(getList).mockResolvedValue(detail());
    renderApp(`/lists/${LIST}`);
    await userEvent.click(
      await screen.findByRole("button", { name: "Add unlistened to Listen List" }),
    );
    expect(addUnlistenedToListenList).toHaveBeenCalledWith(LIST);
    expect(
      await screen.findByText(
        /Added 1 album to your Listen List. 1 album aren’t in your library yet/,
      ),
    ).toBeInTheDocument();
  });

  it("finds albums that aren't in the library on MusicBrainz for review, creating nothing", async () => {
    vi.mocked(getList).mockResolvedValue(detail());
    const { router } = renderApp(`/lists/${LIST}`);
    await userEvent.click(
      await screen.findByRole("button", { name: "Find 1 album on MusicBrainz…" }),
    );
    expect(reviewMissingOnMusicBrainz).toHaveBeenCalledWith(LIST);
    expect(router.state.location.pathname).toBe("/listen-list/import");
    expect(importRelease).not.toHaveBeenCalled();
  });

  it("matches one album that isn't in the library with review, then links it", async () => {
    vi.mocked(getList).mockResolvedValue(detail());
    const RG = "b1392450-e666-3926-a536-22c65f834433";
    const REL = "11111111-1111-4111-8111-111111111111";
    vi.mocked(searchAlbums).mockResolvedValue({
      value: {
        total: 1,
        offset: 0,
        candidates: [
          {
            id: RG,
            title: "Spiderland",
            disambiguation: null,
            primaryType: "Album",
            secondaryTypes: [],
            originalDate: { value: "1991-03-27", precision: "day", year: 1991 },
            artistCredit: [
              {
                artistId: "a1",
                artistName: "Slint",
                sortName: null,
                disambiguation: null,
                creditedName: null,
                joinPhrase: "",
              },
            ],
            score: 100,
            exactTitle: true,
            yearMatches: null,
          },
        ],
      },
      fetchedAt: "2026-09-01T10:00:00.000Z",
      source: "network",
    });
    vi.mocked(loadEditions).mockResolvedValue({
      value: {
        total: 1,
        truncated: false,
        editions: [
          {
            id: REL,
            title: "Spiderland",
            disambiguation: null,
            date: { value: "1991", precision: "year", year: 1991 },
            country: "US",
            status: "Official",
            formats: ["CD"],
            trackCount: 6,
          },
        ],
      },
      fetchedAt: "2026-09-01T10:00:00.000Z",
      source: "network",
    });
    vi.mocked(importRelease).mockResolvedValue({
      albumId: ALBUM,
      editionId: "e-new",
    } as Awaited<ReturnType<typeof importRelease>>);
    renderApp(`/lists/${LIST}`);
    const row = at([(await screen.findByText("Spiderland")).closest("li")], 0);
    await userEvent.click(within(row).getByRole("button", { name: /Add to Listen List/ }));
    const dialog = await screen.findByRole("dialog", { name: "Find “Spiderland”" });
    // Prefilled and searched, but nothing is chosen for the user.
    expect(searchAlbums).toHaveBeenCalledWith(
      { title: "Spiderland", artist: "Slint" },
      expect.anything(),
    );
    expect(importRelease).not.toHaveBeenCalled();
    await userEvent.click(await within(dialog).findByRole("radio", { name: /1991 · Album/ }));
    await userEvent.click(within(dialog).getByRole("button", { name: "Add to Listen List" }));
    expect(importRelease).toHaveBeenCalledWith(
      expect.objectContaining({ releaseGroupId: RG, releaseId: REL, addToListenList: true }),
    );
    expect(linkListEntry).toHaveBeenCalledWith("e4", ALBUM);
    expect(await screen.findByText("Added “Spiderland” to your Listen List.")).toBeInTheDocument();
  });

  it("disables the bulk button with a visible reason when nothing is left to add", async () => {
    vi.mocked(getList).mockResolvedValue(
      detail({
        entries: detail().entries.slice(0, 2),
        progress: { total: 2, listened: 1, onListenList: 1 },
      }),
    );
    renderApp(`/lists/${LIST}`);
    const button = await screen.findByRole("button", { name: "Add unlistened to Listen List" });
    expect(button).toHaveAttribute("aria-disabled", "true");
    expect(
      screen.getByText("Every album is already listened to or on your Listen List."),
    ).toBeInTheDocument();
  });

  it("adds an album by hand", async () => {
    vi.mocked(getList).mockResolvedValue(detail());
    renderApp(`/lists/${LIST}`);
    await userEvent.click(await screen.findByRole("button", { name: "Add album" }));
    const dialog = await screen.findByRole("dialog", { name: "Add an album to this list" });
    await userEvent.type(within(dialog).getByLabelText("Artist"), "Bedhead");
    await userEvent.type(within(dialog).getByLabelText("Album title"), "WhatFunLifeWas");
    await userEvent.type(within(dialog).getByLabelText("Year (optional)"), "1994");
    await userEvent.click(within(dialog).getByRole("button", { name: "Add album" }));
    expect(addListEntries).toHaveBeenCalledWith(LIST, [
      { artistName: "Bedhead", title: "WhatFunLifeWas", year: 1994 },
    ]);
  });

  it("reads a web page, shows what it found, and only then creates the list", async () => {
    vi.mocked(listLists).mockResolvedValue([]);
    vi.mocked(getList).mockResolvedValue(detail());
    vi.mocked(previewListImport).mockResolvedValue({
      name: "The 100 Best Indie Rock Albums of All Time",
      sourceUrl: "https://pitchfork.com/story/the-100-best/",
      entries: [
        { artistName: "Pavement", title: "Crooked Rain, Crooked Rain", year: 1994, position: 1 },
        { artistName: "Liz Phair", title: "Exile in Guyville", year: 1993, position: 2 },
      ],
    });
    renderApp("/lists");
    await userEvent.click(await screen.findByRole("button", { name: "Import from web page" }));
    const dialog = await screen.findByRole("dialog", { name: "Import a list from a web page" });
    await userEvent.type(
      within(dialog).getByLabelText("Web address"),
      "https://pitchfork.com/story/the-100-best/",
    );
    await userEvent.click(within(dialog).getByRole("button", { name: "Read page" }));
    expect(previewListImport).toHaveBeenCalledWith("https://pitchfork.com/story/the-100-best/");
    expect(await within(dialog).findByText(/Found 2 albums/)).toBeInTheDocument();
    expect(createList).not.toHaveBeenCalled();
    expect(within(dialog).getByLabelText("List name")).toHaveValue(
      "The 100 Best Indie Rock Albums of All Time",
    );
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Create list with 2 albums" }),
    );
    expect(createList).toHaveBeenCalledWith(
      "The 100 Best Indie Rock Albums of All Time",
      "https://pitchfork.com/story/the-100-best/",
      expect.arrayContaining([
        expect.objectContaining({ title: "Exile in Guyville", position: 2 }),
      ]),
    );
  });

  it("reports an address that can't be imported and saves nothing", async () => {
    vi.mocked(listLists).mockResolvedValue([]);
    const { NativeError } = await import("../../transport/native");
    vi.mocked(previewListImport).mockRejectedValue(
      new NativeError("validation", "That address isn’t supported."),
    );
    renderApp("/lists");
    await userEvent.click(await screen.findByRole("button", { name: "Import from web page" }));
    const dialog = await screen.findByRole("dialog", { name: "Import a list from a web page" });
    await userEvent.type(within(dialog).getByLabelText("Web address"), "https://example.com/x");
    await userEvent.click(within(dialog).getByRole("button", { name: "Read page" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("isn’t supported");
    expect(createList).not.toHaveBeenCalled();
  });
});
