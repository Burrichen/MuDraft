import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { checkHealth } from "../services/health";
import { listLibrary } from "../services/library";
import { updatePreferences } from "../services/preferences";
import { renderApp } from "../test/renderApp";
import { showErrorDialog } from "../transport/dialogs";
import { NativeError } from "../transport/native";

vi.mock("../services/stats", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../services/stats")>()),
  statsOverview: vi.fn(() => new Promise(() => undefined)),
}));
vi.mock("../services/metadata", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../services/metadata")>()),
  metadataCacheStats: vi.fn(() => Promise.resolve({ entries: 0, bytes: 0, expired: 0 })),
}));
vi.mock("../services/health", () => ({ checkHealth: vi.fn() }));
vi.mock("../transport/dialogs", () => ({ showErrorDialog: vi.fn(() => Promise.resolve()) }));
vi.mock("../services/library", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../services/library")>()),
  listLibrary: vi.fn(),
}));
vi.mock("../services/preferences", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../services/preferences")>()),
  updatePreferences: vi.fn(),
}));

const ALBUM = "/albums/0190f5c3-2b7a-7c3e-9a1b-3c4d5e6f7a8b";

function mockMatchMedia(matches: boolean) {
  vi.stubGlobal(
    "matchMedia",
    vi.fn((query: string) => ({
      matches,
      media: query,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })),
  );
}

beforeEach(() => {
  vi.unstubAllGlobals();
  vi.mocked(listLibrary).mockResolvedValue({
    total: 1,
    genres: [],
    tags: [],
    items: [
      {
        albumId: "0190f5c3-0000-7000-8000-000000000001",
        editionId: "0190f5c3-0000-7000-8000-0000000000e1",
        title: "Fixture Album",
        artists: [{ id: "0190f5c3-0000-7000-8000-0000000000a1", name: "Fixture Artist" }],
        credit: "Fixture Artist",
        originalYear: 1997,
        editionName: "Standard",
        editionCount: 1,
        genres: [],
        tags: [],
        addedAt: "2026-09-30T12:00:00.000Z",
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
      },
    ],
  });
  vi.mocked(checkHealth).mockResolvedValue({
    appVersion: "0.1.0",
    profile: "development",
    dataDir: "/tmp/mudraft",
    schemaVersion: 2,
    sqliteVersion: "3.50.0",
  });
  vi.mocked(updatePreferences).mockImplementation((patch) =>
    Promise.resolve({
      sidebarCollapsed: false,
      lastRoute: "/listen-list",
      albumLayout: "grid",
      startPage: "last",
      showArtwork: true,
      ...patch,
    }),
  );
});

describe("navigation", () => {
  it("offers exactly the five primary sections", () => {
    renderApp("/listen-list");
    const nav = screen.getByRole("navigation", { name: "Main" });
    expect(
      within(nav)
        .getAllByRole("link")
        .map((l) => l.textContent),
    ).toEqual(["Listen List", "Next Up", "Collection", "Stats", "Settings"]);
  });

  it("redirects unknown paths, navigates, marks the current page, and persists the route", async () => {
    const { router } = renderApp("/nowhere");
    expect(
      await screen.findByRole("heading", { level: 1, name: "Listen List" }),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("link", { name: "Stats" }));
    expect(router.state.location.pathname).toBe("/stats");
    expect(screen.getByRole("link", { name: "Stats" })).toHaveAttribute("aria-current", "page");
    await waitFor(() => {
      expect(updatePreferences).toHaveBeenCalledWith({ lastRoute: "/stats" });
    });
    // Focus moves to the new page heading for keyboard and screen-reader users.
    expect(screen.getByRole("heading", { level: 1, name: "Stats" })).toHaveFocus();
  });

  it("does not persist routes Rust would reject", async () => {
    renderApp("/albums/not-an-id");
    expect(await screen.findByText(/This link doesn.t point to an album/)).toBeInTheDocument();
    expect(updatePreferences).not.toHaveBeenCalled();
  });

  it("goes back through history when a nested page was opened in-app", async () => {
    const { router } = renderApp("/collection");
    await router.navigate(ALBUM, { state: { from: "/collection" } });
    await userEvent.click(await screen.findByRole("button", { name: "Back to Collection" }));
    expect(router.state.location.pathname).toBe("/collection");
    expect(router.state.historyAction).toBe("POP");
  });

  it("falls back to a section when a nested page was opened directly", async () => {
    const { router } = renderApp(ALBUM);
    await userEvent.click(await screen.findByRole("button", { name: "Back to Listen List" }));
    expect(router.state.location.pathname).toBe("/listen-list");
    expect(router.state.historyAction).toBe("REPLACE");
  });

  it("renders artist pages", async () => {
    renderApp("/artists/0190f5c3-2b7a-7c3e-9a1b-3c4d5e6f7a8b");
    expect(await screen.findByRole("heading", { level: 1, name: "Artist" })).toBeInTheDocument();
  });

  it("skips to the page heading without changing the hash route", async () => {
    const { router } = renderApp("/stats");
    await userEvent.click(screen.getByRole("link", { name: "Skip to content" }));
    expect(screen.getByRole("heading", { level: 1, name: "Stats" })).toHaveFocus();
    expect(router.state.location.pathname).toBe("/stats");
  });
});

describe("sidebar", () => {
  it("starts from the stored preference and persists toggling", async () => {
    renderApp("/stats", { sidebarCollapsed: true });
    const toggle = screen.getByRole("button", { name: "Expand sidebar" });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    // Labels stay available to assistive tech while visually hidden.
    expect(screen.getByRole("link", { name: "Collection" })).toBeInTheDocument();

    await userEvent.click(toggle);
    expect(screen.getByRole("button", { name: "Collapse sidebar" })).toHaveAttribute(
      "aria-expanded",
      "true",
    );
    expect(updatePreferences).toHaveBeenCalledWith({ sidebarCollapsed: false });
  });

  it("is forced compact in narrow windows and hides the pointless toggle", () => {
    mockMatchMedia(true);
    renderApp("/stats", { sidebarCollapsed: false });
    expect(screen.queryByRole("button", { name: /sidebar/ })).not.toBeInTheDocument();
    expect(document.querySelector(".shell")).toHaveAttribute("data-compact", "true");
    expect(screen.getByRole("link", { name: "Settings" })).toBeInTheDocument();
  });

  it("does not write a preference that is already set", async () => {
    renderApp("/listen-list");
    await userEvent.click(await screen.findByRole("button", { name: "Grid" }));
    expect(updatePreferences).not.toHaveBeenCalled();
  });

  it("reverts and explains when a preference cannot be saved", async () => {
    vi.mocked(updatePreferences).mockRejectedValueOnce(
      new NativeError("storage_unavailable", "disk full"),
    );
    renderApp("/listen-list"); // already the stored route, so the click is the first write
    await userEvent.click(screen.getByRole("button", { name: "Collapse sidebar" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Couldn't save interface settings: disk full",
    );
    expect(screen.getByRole("button", { name: "Collapse sidebar" })).toBeInTheDocument();
  });

  it("does not retry a failed route save in a loop", async () => {
    vi.mocked(updatePreferences).mockRejectedValue(
      new NativeError("storage_unavailable", "disk full"),
    );
    renderApp("/stats");
    expect(await screen.findByRole("alert")).toHaveTextContent("disk full");
    await new Promise((r) => setTimeout(r, 50));
    expect(updatePreferences).toHaveBeenCalledTimes(1);
  });

  it("uses defaults without writing when preferences failed to load", async () => {
    renderApp("/stats", {}, "database locked");
    expect(screen.getByRole("alert")).toHaveTextContent("could not be loaded (database locked)");
    await userEvent.click(screen.getByRole("button", { name: "Collapse sidebar" }));
    expect(screen.getByRole("button", { name: "Expand sidebar" })).toBeInTheDocument();
    expect(updatePreferences).not.toHaveBeenCalled();
  });
});

describe("pages and settings", () => {
  it("persists the album layout from the toolbar", async () => {
    renderApp("/collection");
    await userEvent.click(await screen.findByRole("button", { name: "List" }));
    expect(screen.getByRole("button", { name: "List" })).toHaveAttribute("aria-pressed", "true");
    expect(updatePreferences).toHaveBeenCalledWith({ albumLayout: "list" });
  });

  it("adds albums straight to the Collection", async () => {
    renderApp("/collection");
    const add = screen.getByRole("button", { name: "Add album" });
    expect(add).not.toHaveAttribute("aria-disabled");
    await userEvent.click(add);
    expect(screen.getByRole("dialog", { name: "Add to Collection" })).toHaveAttribute("open");
  });

  it("opens the match dialog from the Listen List", async () => {
    renderApp("/listen-list");
    const add = screen.getByRole("button", { name: "Add album" });
    expect(add).not.toHaveAttribute("aria-disabled");
    await userEvent.click(add);
    expect(screen.getByRole("dialog", { name: "Add to Listen List" })).toHaveAttribute("open");
  });

  it("shows interface settings and storage facts", async () => {
    renderApp("/settings", { sidebarCollapsed: true });
    expect(screen.getByRole("switch", { name: "Collapse sidebar" })).toBeChecked();
    expect(await screen.findByText("/tmp/mudraft")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("switch", { name: "Collapse sidebar" }));
    expect(updatePreferences).toHaveBeenCalledWith({ sidebarCollapsed: false });
  });

  it("surfaces structured storage errors inline and in a native dialog", async () => {
    vi.mocked(checkHealth).mockRejectedValue(
      new NativeError("schema_too_new", "Database was created by a newer MuDraft"),
    );
    renderApp("/listen-list");
    expect(await screen.findByText("Storage error: schema_too_new")).toBeInTheDocument();
    await waitFor(() => {
      expect(showErrorDialog).toHaveBeenCalledWith(
        "MuDraft storage problem",
        "Database was created by a newer MuDraft (schema_too_new)",
      );
    });
  });
});
