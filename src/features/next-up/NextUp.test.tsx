import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { albumDetail } from "../../services/album";
import { listLibrary, listTags, type ListItem } from "../../services/library";
import { logListen, undoLog, type Logged } from "../../services/listening";
import {
  chooseNextUp,
  clearNextUp,
  nextUpOptions,
  nextUpState,
  resetNextUpPool,
  rollNextUp,
  type CurrentPick,
  type GuidedCriteria,
  type GuidedOptions,
  type Method,
  type NextUpSlot,
  type NextUpState,
  type PoolView,
} from "../../services/nextUp";
import { renderApp } from "../../test/renderApp";
import { explainMatch, fromMethod, toCriteria } from "./criteria";

vi.mock("../../services/nextUp", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/nextUp")>()),
  nextUpState: vi.fn(),
  nextUpOptions: vi.fn(),
  rollNextUp: vi.fn(),
  resetNextUpPool: vi.fn(() => Promise.resolve(2)),
  clearNextUp: vi.fn(() => Promise.resolve(true)),
  chooseNextUp: vi.fn(),
}));
vi.mock("../../services/listening", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/listening")>()),
  logListen: vi.fn(),
  undoLog: vi.fn(() => Promise.resolve()),
}));
vi.mock("../../services/library", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/library")>()),
  listLibrary: vi.fn(),
  listTags: vi.fn(() => Promise.resolve([])),
}));
vi.mock("../../services/album", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/album")>()),
  albumDetail: vi.fn(() => new Promise(() => undefined)),
  artworkPreference: vi.fn(() =>
    Promise.resolve({ allowed: false, cache: { files: 0, bytes: 0 } }),
  ),
}));
vi.mock("../../services/preferences", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/preferences")>()),
  updatePreferences: vi.fn((patch: object) =>
    Promise.resolve({
      sidebarCollapsed: false,
      lastRoute: "/next-up",
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

const ASAP = {
  id: "00000000-0000-7000-8000-000000000001",
  name: "Listen ASAP",
  color: "#d4a017",
  builtin: true,
};
const ROAD = {
  id: "0190f5c3-0000-7000-8000-00000000000b",
  name: "Road trip",
  color: "#1d4ed8",
  builtin: false,
};

function item(over: Partial<ListItem> = {}): ListItem {
  return {
    albumId: "0190f5c3-0000-7000-8000-0000000000a1",
    editionId: "0190f5c3-0000-7000-8000-0000000000e1",
    title: "Kind of Blue",
    artists: [{ id: "0190f5c3-0000-7000-8000-00000000ab01", name: "Miles Davis" }],
    credit: "Miles Davis",
    originalYear: 1959,
    editionName: "Standard",
    editionCount: 1,
    genres: ["Jazz"],
    tags: [ROAD],
    addedAt: "2026-09-12T10:00:00.000Z",
    artwork: null,
    rating: {
      effective: null,
      source: "unrated",
      explicit: null,
      calculated: null,
      ratedTracks: 0,
      totalTracks: 5,
    },
    listening: { lastListened: null, listenCount: 0, undatedListens: 0, earlierUndated: false },
    ...over,
  };
}

const GUIDED: Method = {
  mode: "guided",
  criteria: {
    years: { any_of: [{ decade: 1950 }] },
    genres: "agnostic",
    tagIds: { any_of: [ROAD.id] },
  },
};

function pick(over: Partial<CurrentPick> = {}): CurrentPick {
  const it = item();
  return {
    slot: 1,
    attemptId: "0190f5c3-0000-7000-8000-0000000a7701",
    sessionId: "0190f5c3-0000-7000-8000-0000000a5501",
    albumId: it.albumId,
    editionId: it.editionId,
    source: "guided",
    status: "shown",
    shownAt: "2026-10-01T10:00:00.000Z",
    eligibility: "eligible",
    matched: { method: GUIDED, years: [{ decade: 1950 }], genres: [], tags: [ROAD] },
    item: it,
    ...over,
  };
}

const POOL: PoolView = {
  sessionId: "s",
  method: GUIDED,
  poolCycle: 1,
  poolSize: 3,
  remaining: 2,
  exhausted: false,
};

function state(slots: NextUpSlot[]): NextUpState {
  const used = slots.map((s) => s.slot);
  return {
    slots,
    maxSlots: 5,
    nextFreeSlot: [1, 2, 3, 4, 5].find((n) => !used.includes(n)) ?? null,
  };
}

const NOTHING = state([]);
const WITH_PICK = state([{ slot: 1, current: pick(), session: POOL }]);

/** A second slot: picked by hand (no method), a different album. */
const SECOND: NextUpSlot = {
  slot: 2,
  current: pick({
    slot: 2,
    attemptId: "0190f5c3-0000-7000-8000-0000000a7702",
    sessionId: "manual",
    albumId: "0190f5c3-0000-7000-8000-0000000000a3",
    editionId: "0190f5c3-0000-7000-8000-0000000000e3",
    source: "manual",
    matched: { method: null, years: [], genres: [], tags: [] },
    item: item({
      albumId: "0190f5c3-0000-7000-8000-0000000000a3",
      editionId: "0190f5c3-0000-7000-8000-0000000000e3",
      title: "Bitches Brew",
    }),
  }),
  session: null,
};

const FULL = state(
  [1, 2, 3, 4, 5].map((n) => ({
    slot: n,
    current: pick({
      slot: n,
      attemptId: `0190f5c3-0000-7000-8000-0000000a770${String(n)}`,
      editionId: `0190f5c3-0000-7000-8000-0000000000f${String(n)}`,
      item: item({
        editionId: `0190f5c3-0000-7000-8000-0000000000f${String(n)}`,
        title: `Album ${String(n)}`,
      }),
    }),
    session: POOL,
  })),
);

const slotCard = (name: string) => screen.getByRole("article", { name });

function options(criteria: GuidedCriteria | undefined): GuidedOptions {
  const genres = criteria?.genres;
  const jazzOnly = genres !== undefined && genres !== "agnostic" && genres.any_of.includes("jazz");
  return {
    decades: [
      { value: 1960, count: 2 },
      { value: 2010, count: 1 },
      { value: 2020, count: 1 },
    ],
    currentYear: { value: 2026, count: 1 },
    unknownYears: 1,
    genres: [
      { name: "Jazz", key: "jazz", count: 1 },
      { name: "Rock", key: "rock", count: 3 },
    ],
    tags: [
      { ...ROAD, count: 1, onListenList: 1, anywhere: 1 },
      { ...ASAP, count: 2, onListenList: 2, anywhere: 2 },
      {
        ...ROAD,
        name: "Elsewhere",
        id: "0190f5c3-0000-7000-8000-00000000000e",
        count: 0,
        onListenList: 0,
        anywhere: 4,
      },
      {
        ...ROAD,
        name: "Unused",
        id: "0190f5c3-0000-7000-8000-00000000000f",
        count: 0,
        onListenList: 0,
        anywhere: 0,
      },
    ],
    // Jazz + a decade: an explicit zero-match state.
    matchCount: jazzOnly && criteria?.years !== "agnostic" ? 0 : 4,
    listenAsap: 2,
    listenAsapTag: true,
    total: 5,
  };
}

const lastCriteria = () => vi.mocked(nextUpOptions).mock.calls.at(-1)?.[0];

beforeEach(() => {
  vi.mocked(nextUpState).mockResolvedValue(NOTHING);
  vi.mocked(nextUpOptions).mockImplementation((c) => Promise.resolve(options(c)));
  vi.mocked(rollNextUp).mockImplementation(() => {
    vi.mocked(nextUpState).mockResolvedValue(WITH_PICK);
    return Promise.resolve({
      kind: "picked",
      shown: {
        slot: 1,
        attemptId: "a",
        sessionId: "s",
        albumId: "x",
        editionId: "e",
        source: "completely_random",
      },
      poolSize: 3,
      remaining: 2,
    });
  });
});

const decadeGroup = () => screen.getByRole("group", { name: "Decade" });
const bubble = (group: HTMLElement, name: string | RegExp) =>
  within(group).getByRole("button", { name });

describe("criteria", () => {
  it("round-trips bubbles and explains a guided match", () => {
    const sel = { years: ["d1960", "current"], genres: ["agnostic"], tags: [ROAD.id] };
    const c = toCriteria(sel);
    expect(c).toEqual({
      years: { any_of: [{ decade: 1960 }, "current_year"] },
      genres: "agnostic",
      tagIds: { any_of: [ROAD.id] },
    });
    expect(fromMethod({ mode: "guided", criteria: c })).toEqual(sel);
    expect(explainMatch(pick().matched, 1959)).toBe(
      "Matches your choices — Decade: 1950s · Genre: any · Tag: Road trip.",
    );
    expect(explainMatch({ method: null, years: [], genres: [], tags: [] }, null)).toMatch(
      /yourself/,
    );
  });
});

describe("Next Up page", () => {
  it("offers the three methods; random modes pick immediately", async () => {
    const user = userEvent.setup();
    renderApp("/next-up");
    for (const label of ["Guided Recommendation", "Completely Random", "Weighted Random"]) {
      expect(await screen.findByRole("button", { name: label })).toBeInTheDocument();
    }
    expect(screen.getByRole("button", { name: "Completely Random" })).toHaveAccessibleDescription(
      "Any album on your Listen List · 5 albums",
    );
    await user.click(screen.getByRole("button", { name: "Completely Random" }));
    expect(rollNextUp).toHaveBeenCalledWith(expect.any(String), 1, { mode: "completely_random" });
    expect(await screen.findByRole("heading", { name: "Your Next Up" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Kind of Blue" })).toBeInTheDocument();
    expect(screen.getByText(/Decade: 1950s · Genre: any · Tag: Road trip/)).toBeInTheDocument();
    for (const action of ["Mark Listened", "Reroll", "Change Choices", "Remove"]) {
      expect(screen.getByRole("button", { name: action })).toBeInTheDocument();
    }
    expect(screen.getByRole("link", { name: "Open Album" })).toHaveAttribute(
      "href",
      `/albums/${pick().albumId}`,
    );
  });

  it("expands Guided on the same page with decades, current year, and exclusive Agnostic", async () => {
    const user = userEvent.setup();
    renderApp("/next-up");
    const guided = await screen.findByRole("button", { name: "Guided Recommendation" });
    expect(guided).toHaveAttribute("aria-expanded", "false");
    await user.click(guided);
    expect(guided).toHaveAttribute("aria-expanded", "true");
    const decades = decadeGroup();
    const labels = within(decades)
      .getAllByRole("button")
      .map((b) => b.querySelector(".filter-label")?.textContent);
    expect(labels).toEqual(["Agnostic", "1960s", "2010s", "2020s", "2026"]);
    expect(bubble(decades, /^Agnostic/)).toHaveAttribute("aria-pressed", "true");

    await user.click(bubble(decades, /^1960s/));
    expect(bubble(decades, /^Agnostic/)).toHaveAttribute("aria-pressed", "false");
    // OR within a category: other decades stay available alongside the first choice.
    expect(bubble(decades, /^2020s/)).not.toHaveAttribute("aria-disabled");
    await user.click(bubble(decades, /^2020s/));
    await waitFor(() => {
      expect(lastCriteria()?.years).toEqual({ any_of: [{ decade: 1960 }, { decade: 2020 }] });
    });
    await user.click(bubble(decades, /^1960s/));
    await user.click(bubble(decades, /^2020s/));
    expect(bubble(decades, /^Agnostic/)).toHaveAttribute("aria-pressed", "true");
    await user.click(bubble(decades, /^2026/));
    await user.click(bubble(decades, /^Agnostic/));
    expect(bubble(decades, /^2026/)).toHaveAttribute("aria-pressed", "false");
    expect(screen.getByText("4 albums match")).toBeInTheDocument();
  });

  it("explains unavailable tags and shows an explicit zero-match state", async () => {
    const user = userEvent.setup();
    renderApp("/next-up");
    await user.click(await screen.findByRole("button", { name: "Guided Recommendation" }));
    const tags = screen.getByRole("group", { name: "Tag" });
    await waitFor(() => {
      expect(bubble(tags, /^Unused/)).toHaveAccessibleDescription("Not applied to any album yet");
    });
    expect(bubble(tags, /^Elsewhere/)).toHaveAccessibleDescription(
      "Only on albums outside your Listen List",
    );
    await user.click(bubble(tags, /^Unused/));
    expect(bubble(tags, /^Unused/)).toHaveAttribute("aria-pressed", "false");
    expect(bubble(tags, /^Listen ASAP/)).not.toHaveAttribute("aria-disabled");

    await user.click(bubble(decadeGroup(), /^1960s/));
    await user.click(bubble(screen.getByRole("group", { name: "Genre" }), /^Jazz/));
    expect(
      await screen.findByText(/No albums on your Listen List match all of these choices/),
    ).toBeInTheDocument();
    const find = screen.getByRole("button", { name: "Find an album" });
    expect(find).toHaveAttribute("aria-disabled", "true");
    await user.click(find);
    expect(rollNextUp).not.toHaveBeenCalled();
    // Filters were not relaxed: the selection is unchanged.
    expect(bubble(decadeGroup(), /^1960s/)).toHaveAttribute("aria-pressed", "true");
  });

  it("works with the keyboard from method card to Find an album", async () => {
    const user = userEvent.setup();
    renderApp("/next-up");
    const guided = await screen.findByRole("button", { name: "Guided Recommendation" });
    guided.focus();
    await user.keyboard("{Enter}");
    expect(guided).toHaveAttribute("aria-expanded", "true");
    await user.tab(); // Completely Random
    await user.tab(); // Weighted Random
    await user.tab();
    expect(bubble(decadeGroup(), /^Agnostic/)).toHaveFocus();
    await user.tab();
    expect(bubble(decadeGroup(), /^1960s/)).toHaveFocus();
    await user.keyboard(" ");
    expect(bubble(decadeGroup(), /^1960s/)).toHaveAttribute("aria-pressed", "true");
    const find = screen.getByRole("button", { name: "Find an album" });
    await waitFor(() => {
      expect(find).not.toHaveAttribute("aria-disabled");
    });
    find.focus();
    await user.keyboard("{Enter}");
    expect(rollNextUp).toHaveBeenCalledWith(expect.any(String), 1, {
      mode: "guided",
      criteria: { years: { any_of: [{ decade: 1960 }] }, genres: "agnostic", tagIds: "agnostic" },
    });
    expect(await screen.findByRole("heading", { name: "Your Next Up" })).toBeInTheDocument();
  });

  it("makes Weighted Random unavailable once the Listen ASAP tag is deleted", async () => {
    const user = userEvent.setup();
    vi.mocked(nextUpOptions).mockImplementation((c) =>
      Promise.resolve({ ...options(c), listenAsap: 0, listenAsapTag: false }),
    );
    renderApp("/next-up");
    const weighted = await screen.findByRole("button", { name: "Weighted Random" });
    await waitFor(() => {
      expect(weighted).toHaveAccessibleDescription(
        "Unavailable: you deleted the Listen ASAP tag it picks from",
      );
    });
    expect(weighted).toHaveAttribute("aria-disabled", "true");
    await user.click(weighted);
    expect(rollNextUp).not.toHaveBeenCalled();
  });

  it("explains empty pools instead of falling back", async () => {
    const user = userEvent.setup();
    vi.mocked(rollNextUp)
      .mockResolvedValueOnce({ kind: "empty", reason: "no_listen_asap" })
      .mockResolvedValueOnce({ kind: "empty", reason: "listen_list_empty" });
    renderApp("/next-up");
    await user.click(await screen.findByRole("button", { name: "Weighted Random" }));
    expect(
      await screen.findByText(/tagged Listen ASAP. Weighted Random only picks/),
    ).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Your Next Up" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Completely Random" }));
    expect(await screen.findByRole("link", { name: "Go to Listen List" })).toBeInTheDocument();
  });

  it("keeps the pick on exhaustion until the pool is reset explicitly", async () => {
    const user = userEvent.setup();
    vi.mocked(nextUpState).mockResolvedValue(WITH_PICK);
    vi.mocked(rollNextUp).mockResolvedValue({ kind: "exhausted", poolSize: 3 });
    renderApp("/next-up");
    await user.click(await screen.findByRole("button", { name: "Reroll" }));
    expect(rollNextUp).toHaveBeenCalledWith(expect.any(String), 1, GUIDED);
    expect(await screen.findByText(/seen all 3 matching albums/)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Kind of Blue" })).toBeInTheDocument();
    expect(resetNextUpPool).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Reset pool" }));
    expect(resetNextUpPool).toHaveBeenCalledTimes(1);
    expect(resetNextUpPool).toHaveBeenCalledWith(1);
    expect(await screen.findByText(/Pool reset/)).toBeInTheDocument();
    expect(rollNextUp).toHaveBeenCalledTimes(1);
  });

  it("persists across navigation without rerolling, and Change Choices restores them", async () => {
    const user = userEvent.setup();
    vi.mocked(nextUpState).mockResolvedValue(WITH_PICK);
    const { router } = renderApp("/next-up");
    await user.click(await screen.findByRole("link", { name: "Open Album" }));
    expect(router.state.location.pathname).toBe(`/albums/${pick().albumId}`);
    await user.click(await screen.findByRole("button", { name: "Back to Next Up" }));
    expect(await screen.findByRole("link", { name: "Kind of Blue" })).toBeInTheDocument();
    expect(rollNextUp).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Change Choices" }));
    expect(screen.getByRole("button", { name: "Guided Recommendation" })).toHaveAttribute(
      "aria-expanded",
      "true",
    );
    await waitFor(() => {
      expect(bubble(screen.getByRole("group", { name: "Tag" }), /^Road trip/)).toHaveAttribute(
        "aria-pressed",
        "true",
      );
    });
    expect(
      screen.getByText(/stays in your Next Up until a new album is picked here/),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.getByRole("heading", { name: "Your Next Up" })).toBeInTheDocument();
    expect(rollNextUp).not.toHaveBeenCalled();
  });

  it("marks listened with the pick linked, and removes without picking another", async () => {
    const user = userEvent.setup();
    vi.mocked(nextUpState).mockResolvedValue(WITH_PICK);
    const logged = {
      listenId: "l",
      completedNextUp: true,
      undo: { listenId: "l" },
    } as unknown as Logged;
    vi.mocked(logListen).mockImplementation(() => {
      // The slot keeps its method, ready to pick again.
      vi.mocked(nextUpState).mockResolvedValue(state([{ slot: 1, current: null, session: POOL }]));
      return Promise.resolve(logged);
    });
    renderApp("/next-up");
    await user.click(await screen.findByRole("button", { name: "Mark Listened" }));
    await user.click(screen.getByRole("button", { name: "Mark listened" }));
    expect(vi.mocked(logListen).mock.calls[0]?.[1]).toMatchObject({
      editionId: pick().editionId,
      attemptId: pick().attemptId,
    });
    expect(await screen.findByText(/Next Up pick completed/)).toBeInTheDocument();
    const ready = await screen.findByRole("article", { name: "Ready for another pick" });
    expect(ready).toHaveTextContent("Picks with Guided Recommendation");
    expect(rollNextUp).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Undo" }));
    expect(undoLog).toHaveBeenCalledWith(logged.undo);
    await user.click(within(ready).getByRole("button", { name: "Pick an Album" }));
    expect(rollNextUp).toHaveBeenCalledWith(expect.any(String), 1, GUIDED);
    vi.mocked(rollNextUp).mockClear();

    vi.mocked(nextUpState).mockResolvedValue(WITH_PICK);
    vi.mocked(clearNextUp).mockImplementation(() => {
      vi.mocked(nextUpState).mockResolvedValue(NOTHING);
      return Promise.resolve(true);
    });
    renderApp("/next-up");
    await user.click(await screen.findByRole("button", { name: "Remove" }));
    expect(clearNextUp).toHaveBeenCalledWith(1);
    expect(
      await screen.findByText("“Kind of Blue” removed from Next Up. It stays on your Listen List."),
    ).toBeInTheDocument();
    expect(rollNextUp).not.toHaveBeenCalled();
  });
});

describe("Next Up slots", () => {
  it("gives each slot its own actions and adds another with fresh choices", async () => {
    const user = userEvent.setup();
    vi.mocked(nextUpState).mockResolvedValue(
      state([{ slot: 1, current: pick(), session: POOL }, SECOND]),
    );
    vi.mocked(rollNextUp).mockResolvedValue({
      kind: "picked",
      shown: {
        slot: 1,
        attemptId: "a",
        sessionId: "s",
        albumId: "x",
        editionId: "e",
        source: "guided",
      },
      poolSize: 3,
      remaining: 1,
    });
    renderApp("/next-up");
    const first = await screen.findByRole("article", { name: "Kind of Blue" });
    const second = slotCard("Bitches Brew");
    expect(screen.getByText("2 of 5")).toBeInTheDocument();
    expect(second).toHaveTextContent("You chose this album yourself.");
    // A hand-picked slot has no method to reroll with.
    expect(within(second).getByRole("button", { name: "Reroll" })).toHaveAttribute(
      "aria-disabled",
      "true",
    );
    await user.click(within(first).getByRole("button", { name: "Reroll" }));
    expect(rollNextUp).toHaveBeenCalledWith(expect.any(String), 1, GUIDED);

    // Adding opens fresh choices for the next free slot; the other picks stay.
    await user.click(await screen.findByRole("button", { name: "Add Next Up" }));
    expect(screen.getByText(/Your other Next Up picks stay as they are/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Guided Recommendation" })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(slotCard("Bitches Brew")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Add Next Up" }));
    await user.click(screen.getByRole("button", { name: "Weighted Random" }));
    expect(rollNextUp).toHaveBeenLastCalledWith(expect.any(String), 3, {
      mode: "weighted_random",
    });

    await user.click(
      within(await screen.findByRole("article", { name: "Bitches Brew" })).getByRole("button", {
        name: "Remove",
      }),
    );
    expect(clearNextUp).toHaveBeenCalledWith(2);
  });

  it("offers no Add when full, and explains when every match is already picked", async () => {
    const user = userEvent.setup();
    vi.mocked(nextUpState).mockResolvedValue(FULL);
    vi.mocked(rollNextUp).mockResolvedValue({ kind: "empty", reason: "all_in_next_up" });
    renderApp("/next-up");
    expect(await screen.findByText(/5 of 5 — remove one to add another/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Add Next Up" })).not.toBeInTheDocument();
    await user.click(within(slotCard("Album 3")).getByRole("button", { name: "Reroll" }));
    expect(rollNextUp).toHaveBeenCalledWith(expect.any(String), 3, GUIDED);
    expect(
      await screen.findByText(/Every album that matches is already in your Next Up/),
    ).toBeInTheDocument();
    expect(slotCard("Album 3")).toBeInTheDocument();
  });
});

describe("Make Next Up", () => {
  beforeEach(() => {
    vi.mocked(listTags).mockResolvedValue([]);
    vi.mocked(listLibrary).mockResolvedValue({
      items: [
        item({
          albumId: "0190f5c3-0000-7000-8000-0000000000a2",
          editionId: "0190f5c3-0000-7000-8000-0000000000e2",
          title: "Blue",
        }),
      ],
      total: 1,
      genres: [],
      tags: [],
    });
    vi.mocked(chooseNextUp).mockResolvedValue({
      slot: 2,
      attemptId: "m",
      sessionId: "s",
      albumId: "a",
      editionId: "e",
      source: "manual",
    });
  });

  it("adds to a free slot without asking", async () => {
    const user = userEvent.setup();
    vi.mocked(nextUpState).mockResolvedValue(WITH_PICK);
    renderApp("/listen-list");
    await user.click(await screen.findByRole("button", { name: "Add Blue to Next Up" }));
    expect(await screen.findByText("“Blue” is now in your Next Up.")).toBeInTheDocument();
    expect(chooseNextUp).toHaveBeenCalledWith(
      expect.any(String),
      "0190f5c3-0000-7000-8000-0000000000e2",
      undefined,
    );
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(albumDetail).not.toHaveBeenCalled();
  });

  it("says when the album is already in Next Up", async () => {
    const user = userEvent.setup();
    vi.mocked(nextUpState).mockResolvedValue(
      state([
        {
          slot: 3,
          current: pick({ slot: 3, editionId: "0190f5c3-0000-7000-8000-0000000000e2" }),
          session: null,
        },
      ]),
    );
    renderApp("/listen-list");
    await user.click(await screen.findByRole("button", { name: "Add Blue to Next Up" }));
    expect(await screen.findByText("“Blue” is already in your Next Up.")).toBeInTheDocument();
    expect(chooseNextUp).not.toHaveBeenCalled();
  });

  it("asks which pick to replace when every slot is in use", async () => {
    const user = userEvent.setup();
    vi.mocked(nextUpState).mockResolvedValue(FULL);
    renderApp("/listen-list");
    await user.click(await screen.findByRole("button", { name: "Add Blue to Next Up" }));
    const dialog = await screen.findByRole("dialog", { name: "Your Next Up is full" });
    expect(dialog).toHaveTextContent("All 5 Next Up slots are in use");
    expect(within(dialog).getAllByRole("radio")).toHaveLength(5);
    const replace = within(dialog).getByRole("button", { name: "Replace" });
    expect(replace).toHaveAttribute("aria-disabled", "true");
    await user.click(replace);
    expect(chooseNextUp).not.toHaveBeenCalled();
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(chooseNextUp).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Add Blue to Next Up" }));
    const again = await screen.findByRole("dialog", { name: "Your Next Up is full" });
    await user.click(within(again).getByRole("radio", { name: /Album 4/ }));
    await user.click(within(again).getByRole("button", { name: "Replace" }));
    expect(chooseNextUp).toHaveBeenCalledWith(
      expect.any(String),
      "0190f5c3-0000-7000-8000-0000000000e2",
      4,
    );
    expect(
      await screen.findByText("“Blue” replaced “Album 4” in your Next Up."),
    ).toBeInTheDocument();
  });
});
