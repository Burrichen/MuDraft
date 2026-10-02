import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { artworkPreference, clearArtworkCache, setArtworkPreference } from "../../services/album";
import { renderApp } from "../../test/renderApp";

vi.mock("../../services/album", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/album")>()),
  artworkPreference: vi.fn(),
  setArtworkPreference: vi.fn(() => Promise.resolve()),
  clearArtworkCache: vi.fn(),
}));
vi.mock("../../services/library", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/library")>()),
  listTags: vi.fn(() => Promise.resolve([])),
  listLibrary: vi.fn(() => Promise.resolve({ items: [], total: 0, genres: [], tags: [] })),
}));
vi.mock("../../services/preferences", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/preferences")>()),
  updatePreferences: vi.fn((patch: object) =>
    Promise.resolve({
      sidebarCollapsed: false,
      lastRoute: "/listen-list",
      albumLayout: "grid",
      ...patch,
    }),
  ),
}));
vi.mock("../../services/health", () => ({
  checkHealth: vi.fn(() => new Promise(() => undefined)),
}));

describe("artwork consent", () => {
  it("asks once, explains what is sent, and records the answer", async () => {
    vi.mocked(artworkPreference).mockResolvedValue({
      allowed: null,
      cache: { files: 0, bytes: 0 },
    });
    renderApp("/listen-list");
    const dialog = await screen.findByRole("dialog", { name: "Download album artwork?" });
    expect(dialog).toHaveTextContent("coverartarchive.org");
    expect(dialog).toHaveTextContent("No account or personal details are sent");
    await userEvent.click(within(dialog).getByRole("button", { name: "Not now" }));
    expect(setArtworkPreference).toHaveBeenCalledWith(false);
  });

  it("doesn't ask again once answered", async () => {
    vi.mocked(artworkPreference).mockResolvedValue({
      allowed: true,
      cache: { files: 0, bytes: 0 },
    });
    renderApp("/listen-list");
    await waitFor(() => {
      expect(artworkPreference).toHaveBeenCalled();
    });
    expect(
      screen.queryByRole("dialog", { name: "Download album artwork?" }),
    ).not.toBeInTheDocument();
  });
});

describe("artwork settings", () => {
  it("toggles downloads and clears the cache after confirming", async () => {
    vi.mocked(artworkPreference).mockResolvedValue({
      allowed: true,
      cache: { files: 3, bytes: 3 * 1024 * 1024 },
    });
    vi.mocked(clearArtworkCache).mockResolvedValue({ files: 3, bytes: 3 * 1024 * 1024 });
    renderApp("/settings");
    const toggle = await screen.findByRole("switch", {
      name: "Download artwork from Cover Art Archive",
    });
    await waitFor(() => {
      expect(toggle).toBeChecked();
    });
    expect(toggle).toHaveAccessibleDescription("When off, MuDraft makes no artwork requests.");
    expect(screen.getByText("3 images, 3.0 MB")).toBeInTheDocument();
    await userEvent.click(toggle);
    expect(setArtworkPreference).toHaveBeenCalledWith(false);

    await userEvent.click(screen.getByRole("button", { name: "Clear downloaded artwork" }));
    expect(clearArtworkCache).not.toHaveBeenCalled();
    await userEvent.click(
      within(screen.getByRole("dialog", { name: "Clear downloaded artwork?" })).getByRole(
        "button",
        { name: "Clear" },
      ),
    );
    expect(clearArtworkCache).toHaveBeenCalled();
    expect(
      await screen.findByText(
        /Removed 3 downloaded images \(3\.0 MB\)\. Your own images are kept\./,
      ),
    ).toBeInTheDocument();
  });
});
