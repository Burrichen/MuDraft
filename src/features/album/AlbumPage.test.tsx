import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  addEdition,
  albumDetail,
  artworkPreference,
  editAlbum,
  editionCandidates,
  fetchArtwork,
  previewEditionSwitch,
  refreshAlbum,
  removeArtwork,
  switchEdition,
  unlockField,
  type AlbumDetail,
} from "../../services/album";
import { renderApp } from "../../test/renderApp";
import { NativeError } from "../../transport/native";
import { setAlbumRating, setFavourite, setReview, setTrackRating } from "../../services/ratings";
import { runtimeText } from "./runtime";

vi.mock("../../services/album", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/album")>()),
  albumDetail: vi.fn(),
  artworkPreference: vi.fn(),
  fetchArtwork: vi.fn(),
  editAlbum: vi.fn(),
  unlockField: vi.fn(),
  refreshAlbum: vi.fn(),
  editionCandidates: vi.fn(),
  addEdition: vi.fn(),
  previewEditionSwitch: vi.fn(),
  switchEdition: vi.fn(),
  removeArtwork: vi.fn(),
  restoreArtwork: vi.fn(),
  replaceArtwork: vi.fn(),
  artworkUrl: vi.fn((ref: unknown) => (ref ? "artwork://localhost/x" : null)),
}));
vi.mock("../../services/ratings", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/ratings")>()),
  setAlbumRating: vi.fn(() => Promise.resolve()),
  setTrackRating: vi.fn(() => Promise.resolve()),
  setFavourite: vi.fn(() => Promise.resolve()),
  setReview: vi.fn(() => Promise.resolve()),
}));
vi.mock("../../services/preferences", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/preferences")>()),
  updatePreferences: vi.fn((patch: object) =>
    Promise.resolve({
      sidebarCollapsed: false,
      lastRoute: "/listen-list",
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

const ALBUM = "0190f5c3-0000-7000-8000-0000000000a1";
const STD = "0190f5c3-0000-7000-8000-0000000000e1";
const DLX = "0190f5c3-0000-7000-8000-0000000000e2";

function track(n: number, title: string, lengthMs: number | null, extra = {}) {
  return {
    id: `t${String(n)}`,
    position: n,
    title,
    lengthMs,
    credit: null,
    recordingId: null,
    rating: null,
    favourite: false,
    listened: false,
    ...extra,
  };
}

function detail(over: Partial<AlbumDetail> = {}): AlbumDetail {
  return {
    albumId: ALBUM,
    title: "OK Computer",
    credits: [
      {
        artistId: "0190f5c3-0000-7000-8000-00000000aaaa",
        name: "Radiohead",
        creditedName: null,
        joinPhrase: "",
      },
    ],
    originalDate: { value: "1997-05-21", precision: "day", year: 1997 },
    musicbrainzReleaseGroupId: "b1392450-e666-3926-a536-22c65f834433",
    genres: ["Rock", "Electronic"],
    tags: [
      {
        id: "00000000-0000-7000-8000-000000000001",
        name: "Listen ASAP",
        color: "#d4a017",
        builtin: true,
      },
    ],
    description: { text: "Third studio album.", source: "musicbrainz" },
    summary: "“OK Computer” by Radiohead, first released 1997-05-21.",
    lockedFields: [],
    editions: [
      {
        id: STD,
        name: "Standard",
        releaseDate: { value: "1997", precision: "year", year: 1997 },
        musicbrainzReleaseId: "r1",
        trackCount: 3,
        onListenList: true,
        inCollection: false,
      },
      {
        id: DLX,
        name: "Deluxe",
        releaseDate: { value: "2017", precision: "year", year: 2017 },
        musicbrainzReleaseId: "r2",
        trackCount: 5,
        onListenList: false,
        inCollection: true,
      },
    ],
    editionId: DLX,
    discs: [
      {
        number: 1,
        tracks: [
          track(1, "Airbag", 284_000, { rating: 8, favourite: true }),
          track(2, "Lucky", null, { listened: true }),
        ],
      },
      { number: 2, tracks: [track(1, "Lucky (live)", 270_000)] },
    ],
    runtime: { knownMs: 554_000, unknownTracks: 1, trackCount: 3 },
    artwork: { current: null, removed: false },
    rating: {
      effective: 8,
      source: "calculated",
      explicit: null,
      calculated: 8,
      ratedTracks: 1,
      totalTracks: 3,
    },
    review: null,
    listening: {
      summary: { lastListened: null, listenCount: 0, undatedListens: 0, earlierUndated: false },
      listens: [],
    },
    ...over,
  };
}

beforeEach(() => {
  vi.mocked(albumDetail).mockResolvedValue(detail());
  vi.mocked(artworkPreference).mockResolvedValue({ allowed: false, cache: { files: 0, bytes: 0 } });
  vi.mocked(fetchArtwork).mockResolvedValue({ current: null, removed: false });
});

describe("Album page", () => {
  it("shows credits, badges, genres, tags, sourced description, and the chosen edition", async () => {
    renderApp(`/albums/${ALBUM}?edition=${DLX}`);
    expect(
      await screen.findByRole("heading", { level: 1, name: "OK Computer" }),
    ).toBeInTheDocument();
    expect(albumDetail).toHaveBeenCalledWith(ALBUM, DLX);
    expect(screen.getByRole("link", { name: "Radiohead" })).toHaveAttribute(
      "href",
      "/artists/0190f5c3-0000-7000-8000-00000000aaaa",
    );
    expect(screen.getByText("1997")).toHaveClass("edition-badge");
    expect(screen.getByText("Deluxe · 2017")).toHaveClass("edition-badge");
    expect(screen.getByText("Rock · Electronic")).toBeInTheDocument();
    expect(screen.getByRole("list", { name: "Tags on OK Computer" })).toHaveTextContent(
      "Listen ASAP",
    );
    expect(screen.getByText("Third studio album.")).toBeInTheDocument();
    expect(screen.getByText("From the MusicBrainz annotation.")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Tracklist · Deluxe" })).toBeInTheDocument();
  });

  it("renders multi-disc tracklists with unknown lengths left unknown", async () => {
    renderApp(`/albums/${ALBUM}`);
    const disc1 = await screen.findByRole("table", { name: "Disc 1 of 2" });
    const lucky = within(disc1).getByText("Lucky").closest("tr") as HTMLElement;
    expect(within(lucky).getByText("Unknown")).toHaveClass("visually-hidden");
    expect(within(lucky).queryByText("0:00")).not.toBeInTheDocument();
    expect(within(lucky).getByText(/Listened/)).toBeInTheDocument();
    expect(within(disc1).getByText("4:44")).toBeInTheDocument();
    expect(screen.getByRole("table", { name: "Disc 2 of 2" })).toHaveTextContent("Lucky (live)");
    expect(
      screen.getAllByText("3 tracks · at least 9:14 (1 track of unknown length)").length,
    ).toBeGreaterThan(0);
  });

  it("works without artwork or description, using a factual summary", async () => {
    vi.mocked(albumDetail).mockResolvedValue(detail({ description: null }));
    renderApp(`/albums/${ALBUM}`);
    expect(
      await screen.findByText("“OK Computer” by Radiohead, first released 1997-05-21."),
    ).toBeInTheDocument();
    expect(screen.getByText(/A summary of facts from your library/)).toBeInTheDocument();
    expect(screen.getByText("No artwork.")).toBeInTheDocument();
    expect(screen.getByTestId("artwork-placeholder")).toBeInTheDocument();
    await waitFor(() => {
      expect(artworkPreference).toHaveBeenCalled();
    });
    expect(fetchArtwork).not.toHaveBeenCalled();
  });

  it("downloads missing artwork only after consent, and labels the canonical fallback", async () => {
    vi.mocked(artworkPreference).mockResolvedValue({
      allowed: true,
      cache: { files: 0, bytes: 0 },
    });
    const art = {
      current: {
        ownerType: "album" as const,
        ownerId: ALBUM,
        source: "cover_art_archive" as const,
        canonicalFallback: true,
        version: "v1",
      },
      removed: false,
    };
    vi.mocked(fetchArtwork).mockResolvedValue(art);
    vi.mocked(albumDetail)
      .mockResolvedValueOnce(detail())
      .mockResolvedValue(detail({ artwork: art }));
    renderApp(`/albums/${ALBUM}`);
    await waitFor(() => {
      expect(fetchArtwork).toHaveBeenCalledWith(DLX, expect.any(AbortSignal));
    });
    expect(await screen.findByText(/not specific to this edition/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Remove" }));
    expect(removeArtwork).toHaveBeenCalledWith(DLX);
  });

  it("edits only changed fields and can hand a field back to MusicBrainz", async () => {
    vi.mocked(albumDetail).mockResolvedValue(detail({ lockedFields: ["description"] }));
    renderApp(`/albums/${ALBUM}`);
    await userEvent.click(await screen.findByRole("button", { name: "Edit details" }));
    const dialog = screen.getByRole("dialog", { name: "Edit album details" });
    const title = within(dialog).getByLabelText("Title");
    await userEvent.clear(title);
    await userEvent.type(title, "OK Computer (mine)");
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Let MusicBrainz update description again" }),
    );
    expect(unlockField).toHaveBeenCalledWith(ALBUM, "description");
    await userEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(editAlbum).toHaveBeenCalledWith(ALBUM, { title: "OK Computer (mine)" });
  });

  it("refreshes from MusicBrainz and reports kept edits", async () => {
    vi.mocked(refreshAlbum).mockResolvedValue({
      albumId: ALBUM,
      editionId: DLX,
      albumCreated: false,
      editionCreated: false,
      lockedFieldsKept: ["album.title"],
      retainedTracks: 1,
      genres: [],
    });
    renderApp(`/albums/${ALBUM}`);
    await userEvent.click(await screen.findByRole("button", { name: "Refresh from MusicBrainz" }));
    expect(refreshAlbum).toHaveBeenCalledWith(ALBUM, DLX);
    expect(await screen.findByText(/Refreshed from MusicBrainz\./)).toHaveTextContent(
      "Your edits were kept (album.title). 1 rated or listened tracks",
    );
  });

  it("previews an edition switch and copies only matched recordings when chosen", async () => {
    vi.mocked(previewEditionSwitch).mockResolvedValue({
      from: { id: STD, name: "Standard" },
      to: { id: DLX, name: "Deluxe" },
      carry: [
        {
          from: { trackId: "a", title: "Airbag", disc: 1, position: 1 },
          to: { trackId: "b", title: "Airbag", disc: 1, position: 2 },
          rating: 8,
          favourite: false,
          targetHasData: false,
        },
      ],
      stay: [
        {
          track: { trackId: "c", title: "Lucky", disc: 1, position: 3 },
          rating: null,
          favourite: false,
          listened: true,
        },
      ],
      listensKept: 1,
    });
    vi.mocked(switchEdition).mockResolvedValue(1);
    renderApp(`/albums/${ALBUM}`);
    await userEvent.click(await screen.findByRole("button", { name: "Use on Listen List…" }));
    const dialog = await screen.findByRole("dialog", { name: "Use “Deluxe” on your Listen List?" });
    expect(dialog).toHaveTextContent("Airbag → Airbag (disc 1, track 2): 4 of 5 stars");
    expect(dialog).toHaveTextContent("Lucky: listened");
    expect(dialog).toHaveTextContent("Its 1 listen stay with it");
    const copy = within(dialog).getByRole("checkbox", { name: /Copy 1 rating/ });
    expect(copy).toBeChecked();
    await userEvent.click(within(dialog).getByRole("button", { name: "Switch edition" }));
    expect(switchEdition).toHaveBeenCalledWith(ALBUM, DLX, true);
    expect(await screen.findByText(/Copied 1 ratings to matching recordings/)).toBeInTheDocument();
  });

  it("adds a deliberate edition and explains refused ordinary pressings", async () => {
    vi.mocked(editionCandidates).mockResolvedValue({
      fetchedAt: "2026-10-01T00:00:00.000Z",
      source: "network",
      value: [
        {
          id: "r1",
          title: "OK Computer",
          disambiguation: null,
          date: { value: "1997", precision: "year", year: 1997 },
          country: "GB",
          status: "Official",
          formats: ["CD"],
          trackCount: 12,
          inLibraryAs: "Standard",
        },
        {
          id: "r3",
          title: "OK Computer",
          disambiguation: "US pressing",
          date: { value: "1997", precision: "year", year: 1997 },
          country: "US",
          status: "Official",
          formats: ["CD"],
          trackCount: 12,
          inLibraryAs: null,
        },
      ],
    });
    vi.mocked(addEdition).mockRejectedValue(
      new NativeError(
        "conflict",
        "it has exactly the same recordings as your “Standard” edition — an ordinary pressing isn’t added again",
      ),
    );
    renderApp(`/albums/${ALBUM}`);
    await userEvent.click(await screen.findByRole("button", { name: "Add another edition…" }));
    const dialog = await screen.findByRole("dialog", { name: "Add an edition of “OK Computer”" });
    const existing = await within(dialog).findByRole("radio", { name: /GB/ });
    expect(existing).toBeDisabled();
    expect(existing).toHaveAccessibleDescription("Already in your library as “Standard”.");
    await userEvent.click(within(dialog).getByRole("radio", { name: /US pressing/ }));
    expect(within(dialog).getByLabelText("Edition name")).toHaveValue("US pressing");
    await userEvent.click(within(dialog).getByRole("button", { name: "Add edition" }));
    expect(addEdition).toHaveBeenCalledWith(ALBUM, "r3", "US pressing");
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("ordinary pressing");
  });

  it("explains a missing album", async () => {
    vi.mocked(albumDetail).mockRejectedValue(
      new NativeError("not_found", "album x does not exist"),
    );
    renderApp(`/albums/${ALBUM}`);
    expect(await screen.findByRole("heading", { name: "Album not found" })).toBeInTheDocument();
  });
});

describe("ratings on the album page", () => {
  it("labels the calculated rating and sets, zeroes, and clears the explicit one", async () => {
    renderApp(`/albums/${ALBUM}`);
    expect(await screen.findByText("Calculated from 1/3 rated tracks")).toBeInTheDocument();
    const panel = screen.getByRole("region", { name: /Your rating and notes/ });
    await userEvent.click(
      within(panel).getByRole("button", { name: "Set Your album rating to 0 stars" }),
    );
    expect(setAlbumRating).toHaveBeenCalledWith(DLX, 0);
    expect(
      within(panel).queryByRole("button", { name: "Clear Your album rating" }),
    ).not.toBeInTheDocument();
  });

  it("shows an explicit zero as a rating and clears it separately", async () => {
    vi.mocked(albumDetail).mockResolvedValue(
      detail({
        rating: {
          effective: 0,
          source: "explicit",
          explicit: 0,
          calculated: 8,
          ratedTracks: 1,
          totalTracks: 3,
        },
      }),
    );
    renderApp(`/albums/${ALBUM}`);
    const panel = await screen.findByRole("region", { name: /Your rating and notes/ });
    expect(
      within(panel).getByText("Your rating (tracks alone would give 4 from 1/3 rated tracks)"),
    ).toBeInTheDocument();
    expect(within(panel).getByRole("slider", { name: "Your album rating" })).toHaveAttribute(
      "aria-valuetext",
      "0 of 5 stars",
    );
    await userEvent.click(within(panel).getByRole("button", { name: "Clear Your album rating" }));
    expect(setAlbumRating).toHaveBeenCalledWith(DLX, null);
  });

  it("saves notes and rates or favourites tracks without marking them listened", async () => {
    vi.mocked(albumDetail).mockResolvedValue(detail({ review: "Old note" }));
    renderApp(`/albums/${ALBUM}`);
    const notes = await screen.findByLabelText("Review and notes");
    expect(notes).toHaveValue("Old note");
    await userEvent.clear(notes);
    await userEvent.type(notes, "Better on vinyl.");
    await userEvent.click(screen.getByRole("button", { name: "Save notes" }));
    expect(setReview).toHaveBeenCalledWith(DLX, "Better on vinyl.");

    const lucky = screen.getByRole("slider", { name: "Rating for Lucky" });
    lucky.focus();
    await userEvent.keyboard("{Home}");
    expect(setTrackRating).toHaveBeenCalledWith("t2", 0);
    const fav = screen.getByRole("button", { name: "Favourite: Airbag" });
    expect(fav).toHaveAttribute("aria-pressed", "true");
    await userEvent.click(fav);
    expect(setFavourite).toHaveBeenCalledWith("t1", false);
  });
});

describe("runtimeText", () => {
  it("never treats unknown lengths as zero", () => {
    expect(runtimeText({ knownMs: 0, unknownTracks: 2, trackCount: 2 })).toBe(
      "2 tracks · length unknown",
    );
    expect(runtimeText({ knownMs: 4_500_000, unknownTracks: 0, trackCount: 150 })).toBe(
      "150 tracks · 1:15:00",
    );
    expect(runtimeText({ knownMs: 0, unknownTracks: 0, trackCount: 0 })).toBe("No tracklist");
  });
});
