import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  addDiscoveredToListenList,
  artistCatalogue,
  cancelCatalogueRequest,
  fetchArtistCatalogue,
  setCatalogueScope,
  type ArtistCatalogue,
  type CatalogueEntry,
} from "../../services/discography";
import { notifyLibraryChanged } from "../../services/libraryEvents";
import { renderApp } from "../../test/renderApp";
import { NativeError } from "../../transport/native";

vi.mock("../../services/discography", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/discography")>()),
  artistCatalogue: vi.fn(),
  fetchArtistCatalogue: vi.fn(),
  loadArtistTracklists: vi.fn(),
  cancelCatalogueRequest: vi.fn(() => Promise.resolve(true)),
  setCatalogueScope: vi.fn((_: string, s: unknown) => Promise.resolve(s)),
  addDiscoveredToListenList: vi.fn(),
}));
vi.mock("../../services/preferences", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/preferences")>()),
  updatePreferences: vi.fn((patch: object) =>
    Promise.resolve({
      sidebarCollapsed: false,
      lastRoute: "/collection",
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

const ARTIST = "0190f5c3-0000-7000-8000-0000000000a1";
const PARTNER = "0190f5c3-0000-7000-8000-0000000000a2";

function entry(over: Partial<CatalogueEntry>): CatalogueEntry {
  return {
    id: "0190f5c3-0000-7000-8000-0000000000c1",
    credits: [{ name: "Björk", joinPhrase: "", artistId: ARTIST }],
    score: null,
    rank: null,
    source: "musicbrainz",
    releaseGroupId: "rg",
    albumId: null,
    title: "Debut",
    disambiguation: null,
    credit: "Björk",
    collaboration: false,
    originalYear: 1993,
    primaryType: "Album",
    secondaryTypes: [],
    typeLabel: "Album",
    excluded: false,
    counted: true,
    onListenList: false,
    inCollection: false,
    listened: false,
    reference: {
      kind: "representative",
      editionId: null,
      musicbrainzReleaseId: "r",
      name: "Debut",
      date: "1993",
      country: "GB",
      formats: ["CD"],
      trackCount: 11,
      note: "",
      lastError: null,
    },
    tracks: { total: 11, listened: 0 },
    fetchedAt: "2026-10-01T10:00:00.000Z",
    ...over,
  };
}

function catalogue(over: Partial<ArtistCatalogue> = {}): ArtistCatalogue {
  return {
    artistId: ARTIST,
    artistName: "Björk",
    sortName: "Björk",
    disambiguation: "Icelandic singer",
    musicbrainzId: "87c5dedd-371d-4a53-9f7f-80522fb7f3cb",
    average: { stars: 2, ratedAlbums: 1 },
    coverage: {
      kind: "online",
      status: "partial",
      musicbrainzArtistId: "87c5dedd-371d-4a53-9f7f-80522fb7f3cb",
      providerTotal: 150,
      fetchedEntries: 100,
      nextOffset: 100,
      startedAt: null,
      pageFetchedAt: null,
      completedAt: null,
      lastError: null,
      manualEntries: 0,
      tracklistsKnown: 2,
      tracklistsNeeded: 3,
      note: "Only part of the MusicBrainz catalogue has been fetched (100 of 150).",
    },
    scope: { primaryTypes: ["Album"], secondaryTypes: [] },
    denominator: "Counting MusicBrainz release groups of type Album with no secondary type.",
    representativeRule: "The earliest-dated official release that has a tracklist.",
    entries: [
      entry({
        id: "0190f5c3-0000-7000-8000-0000000000c2",
        title: "Homogenic",
        albumId: "0190f5c3-0000-7000-8000-0000000000b2",
        source: "musicbrainz",
        onListenList: true,
        listened: true,
        rank: 1,
        score: {
          stars: 2,
          editions: [
            { editionId: "e1", name: "Standard", rating: 0, source: "explicit" },
            { editionId: "e2", name: "Deluxe", rating: 8, source: "explicit" },
          ],
        },
        tracks: { total: 13, listened: 10 },
        reference: { ...entry({}).reference, kind: "your_edition", name: "Deluxe" },
      }),
      entry({
        title: "Medúlla",
        collaboration: true,
        credits: [
          { name: "Björk", joinPhrase: " & ", artistId: ARTIST },
          { name: "Partner", joinPhrase: "", artistId: PARTNER },
        ],
      }),
      entry({
        id: "0190f5c3-0000-7000-8000-0000000000c3",
        title: "Vespertine",
        tracks: null,
        reference: { ...entry({}).reference, trackCount: null, kind: "none", name: null },
      }),
      entry({
        id: "0190f5c3-0000-7000-8000-0000000000c4",
        title: "Live Box",
        typeLabel: "Album · Live",
        counted: false,
      }),
    ],
    types: [
      { label: "Album", count: 3, counted: true },
      { label: "Album · Live", count: 1, counted: false },
    ],
    progress: {
      albumsCounted: 3,
      albumsListened: 1,
      tracksTotal: 24,
      tracksListened: 10,
      albumsWithoutTracklist: 1,
    },
    ...over,
  };
}

beforeEach(() => {
  vi.mocked(artistCatalogue).mockResolvedValue(catalogue());
});

describe("Artist page", () => {
  it("shows identity, both progress bars, provisional coverage, and the ranking", async () => {
    renderApp(`/artists/${ARTIST}`);
    expect(await screen.findByRole("heading", { level: 1, name: "Björk" })).toBeInTheDocument();
    expect(screen.getByText(/Icelandic singer · MusicBrainz 87c5/)).toBeInTheDocument();
    expect(screen.getByText("Provisional")).toBeInTheDocument();
    expect(screen.getByRole("progressbar", { name: "Albums listened in full" })).toHaveAttribute(
      "aria-valuetext",
      "1 of 3 (33%)",
    );
    expect(
      screen.getByRole("progressbar", { name: "Tracks heard (known tracklists only)" }),
    ).toHaveAttribute("aria-valuetext", "10 of 24 (42%)");
    expect(screen.getByText("1 / 3 · 33%")).toBeInTheDocument();
    expect(
      screen.getByText(
        /Tracklists loaded for 2\/3 albums, so track progress is not full-discography completion/,
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Your average: 2 of 5 stars across 1 rated album."),
    ).toBeInTheDocument();

    const ranking = screen.getByRole("region", { name: "Your ranking" });
    expect(within(ranking).getByRole("link", { name: "Homogenic" })).toHaveAttribute(
      "href",
      "/albums/0190f5c3-0000-7000-8000-0000000000b2",
    );
    const editions = within(ranking).getByRole("list", { name: "Edition scores for Homogenic" });
    expect(editions).toHaveTextContent("Standard: 0 of 5 stars (your rating)");
    expect(editions).toHaveTextContent("Deluxe: 4 of 5 stars (your rating)");

    // Collaborators stay independently navigable.
    expect(screen.getByRole("link", { name: "Partner" })).toHaveAttribute(
      "href",
      `/artists/${PARTNER}`,
    );
    expect(screen.getByText(/Not counted \(1\): Album · Live 1/)).toBeInTheDocument();
  });

  it("never shows an unknown denominator as 0%", async () => {
    vi.mocked(artistCatalogue).mockResolvedValue(
      catalogue({
        entries: [],
        coverage: {
          ...catalogue().coverage,
          status: "not_fetched",
          note: "Not fetched yet.",
          tracklistsKnown: 0,
          tracklistsNeeded: 0,
        },
        progress: {
          albumsCounted: 0,
          albumsListened: 0,
          tracksTotal: 0,
          tracksListened: 0,
          albumsWithoutTracklist: 0,
        },
        average: { stars: null, ratedAlbums: 0 },
      }),
    );
    renderApp(`/artists/${ARTIST}`);
    expect(await screen.findByText(/Albums listened in full: unknown/)).toBeInTheDocument();
    expect(screen.getByText(/Tracks heard: unknown/)).toBeInTheDocument();
    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();
    expect(screen.queryByText(/0%/)).not.toBeInTheDocument();
    expect(screen.getByText("No rated albums yet.")).toBeInTheDocument();
  });

  it("updates immediately after listens or ratings change", async () => {
    renderApp(`/artists/${ARTIST}`);
    await screen.findByRole("heading", { level: 1, name: "Björk" });
    vi.mocked(artistCatalogue).mockResolvedValue(
      catalogue({ progress: { ...catalogue().progress, albumsListened: 2 } }),
    );
    act(() => {
      notifyLibraryChanged();
    });
    await waitFor(() => {
      expect(screen.getByRole("progressbar", { name: "Albums listened in full" })).toHaveAttribute(
        "aria-valuetext",
        "2 of 3 (67%)",
      );
    });
  });

  it("adds a discovered album to the Listen List only when asked", async () => {
    const user = userEvent.setup();
    vi.mocked(addDiscoveredToListenList).mockResolvedValue({
      albumId: "a",
      editionId: "e",
      imported: true,
    });
    renderApp(`/artists/${ARTIST}`);
    const discovered = await screen.findByRole("region", { name: "Discovered, not on your lists" });
    expect(addDiscoveredToListenList).not.toHaveBeenCalled();
    await user.click(
      within(discovered).getByRole("button", { name: "Add Vespertine to Listen List" }),
    );
    expect(addDiscoveredToListenList).toHaveBeenCalledWith(
      expect.any(String),
      "0190f5c3-0000-7000-8000-0000000000c3",
    );
    expect(await screen.findByText("Added “Vespertine” to your Listen List.")).toBeInTheDocument();
  });

  it("resumes and cancels catalogue fetching, keeping what was stored", async () => {
    const user = userEvent.setup();
    let reject: (e: unknown) => void = () => undefined;
    vi.mocked(fetchArtistCatalogue).mockReturnValue(
      new Promise((_, r) => {
        reject = r;
      }),
    );
    renderApp(`/artists/${ARTIST}`);
    await user.click(await screen.findByRole("button", { name: "Resume fetching catalogue" }));
    expect(
      await screen.findByText(/Fetching… 100 of 150 release groups stored/),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    const requestId = vi.mocked(fetchArtistCatalogue).mock.calls[0]?.[0];
    expect(cancelCatalogueRequest).toHaveBeenCalledWith(requestId);
    act(() => {
      reject(new NativeError("cancelled", "Cancelled"));
    });
    expect(await screen.findByRole("alert")).toHaveTextContent(
      /Stopped. What was already loaded is kept/,
    );
  });

  it("changes the counted types explicitly", async () => {
    const user = userEvent.setup();
    renderApp(`/artists/${ARTIST}`);
    const scope = await screen.findByRole("group", { name: "Counted types" });
    expect(within(scope).getByRole("button", { name: "Album" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(within(scope).getByRole("button", { name: "EP" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
    await user.click(within(scope).getByRole("button", { name: "EP" }));
    expect(setCatalogueScope).toHaveBeenCalledWith(ARTIST, {
      primaryTypes: ["Album", "EP"],
      secondaryTypes: [],
    });
  });
});
