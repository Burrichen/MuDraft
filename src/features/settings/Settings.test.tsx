import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { checkHealth } from "../../services/health";
import { clearMetadataCache, metadataCacheStats } from "../../services/metadata";
import { updatePreferences } from "../../services/preferences";
import {
  cancelRestore,
  chooseProfileToRestore,
  confirmRestore,
  exportProfile,
  type ProfileCounts,
  type ProfilePreview,
} from "../../services/profile";
import { NativeError } from "../../transport/native";
import { renderApp } from "../../test/renderApp";

vi.mock("../../services/preferences", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/preferences")>()),
  updatePreferences: vi.fn((patch: object) =>
    Promise.resolve({
      sidebarCollapsed: false,
      lastRoute: "/settings",
      albumLayout: "grid",
      startPage: "last",
      showArtwork: true,
      ...patch,
    }),
  ),
}));
vi.mock("../../services/metadata", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/metadata")>()),
  metadataCacheStats: vi.fn(),
  clearMetadataCache: vi.fn(() => Promise.resolve(2)),
}));
vi.mock("../../services/listening", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/listening")>()),
  listeningSettings: vi.fn(() => Promise.resolve({ fullListenRemovesFromListenList: true })),
}));
vi.mock("../../services/album", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/album")>()),
  artworkPreference: vi.fn(() => Promise.resolve({ allowed: true, cache: { files: 0, bytes: 0 } })),
}));
vi.mock("../../services/library", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/library")>()),
  listTags: vi.fn(() => Promise.resolve([])),
}));
vi.mock("../../services/health", () => ({ checkHealth: vi.fn() }));
vi.mock("../../services/profile", () => ({
  exportProfile: vi.fn(),
  chooseProfileToRestore: vi.fn(),
  confirmRestore: vi.fn(),
  cancelRestore: vi.fn(() => Promise.resolve()),
}));

const COUNTS: ProfileCounts = {
  artists: 2,
  albums: 3,
  editions: 4,
  tracks: 20,
  listenList: 1,
  collection: 2,
  listens: 7,
  ratedTracks: 2,
  albumReviews: 1,
  tags: 2,
  selectionAttempts: 1,
  catalogueEntries: 0,
  artworkFiles: 0,
};
const PREVIEW: ProfilePreview = {
  formatVersion: 1,
  schemaVersion: 9,
  currentSchemaVersion: 10,
  appVersion: "0.1.0",
  createdAt: "2026-10-01T12:00:00.000Z",
  artwork: "included",
  counts: COUNTS,
  archiveBytes: 2048,
  sha256: "abc123",
};

beforeEach(() => {
  vi.mocked(metadataCacheStats).mockResolvedValue({ entries: 2, bytes: 2048, expired: 1 });
  vi.mocked(checkHealth).mockResolvedValue({
    appVersion: "0.1.0",
    profile: "development",
    dataDir: "/Users/someone/Library/Application Support/app.mudraft.desktop.dev",
    schemaVersion: 10,
    sqliteVersion: "3.53.2",
  });
});

describe("Settings", () => {
  it("persists the start page and artwork display", async () => {
    const user = userEvent.setup();
    renderApp("/settings");
    const start = screen.getByLabelText("Start page");
    expect(start).toHaveValue("last");
    await user.selectOptions(start, "Stats");
    expect(updatePreferences).toHaveBeenCalledWith({ startPage: "/stats" });
    expect(start).toHaveValue("/stats");

    const show = screen.getByRole("switch", { name: "Show artwork" });
    expect(show).toBeChecked();
    await user.click(show);
    expect(updatePreferences).toHaveBeenCalledWith({ showArtwork: false });
    expect(show).not.toBeChecked();
    // Existing controls stay connected.
    expect(screen.getByRole("switch", { name: "Collapse sidebar" })).toBeInTheDocument();
    expect(
      await screen.findByRole("switch", { name: /Remove albums from your Listen List/ }),
    ).toBeChecked();
    expect(screen.getByText(/0 to 5 stars in half-star steps/)).toBeInTheDocument();
  });

  it("exports a lightweight profile and reports what was saved", async () => {
    const user = userEvent.setup();
    vi.mocked(exportProfile).mockResolvedValueOnce(null).mockResolvedValueOnce({
      fileName: "mudraft-profile-2026-10-02.mudraft",
      bytes: 4096,
      artwork: "omitted",
      counts: COUNTS,
      artworkMissing: 0,
    });
    renderApp("/settings");
    const profile = screen.getByRole("region", { name: "Profile" });
    await user.click(within(profile).getByRole("button", { name: "Export profile…" }));
    expect(exportProfile).toHaveBeenLastCalledWith(true);
    expect(within(profile).queryByRole("status")).not.toBeInTheDocument(); // cancelled
    await user.click(within(profile).getByRole("switch", { name: "Include artwork" }));
    await user.click(within(profile).getByRole("button", { name: "Export profile…" }));
    expect(exportProfile).toHaveBeenLastCalledWith(false);
    expect(await within(profile).findByRole("status")).toHaveTextContent(
      "Saved “mudraft-profile-2026-10-02.mudraft” (4 KB): 3 albums, 7 listens, 2 rated tracks, 2 tags, 0 images, without artwork.",
    );
  });

  it("previews a profile, requires confirmation, and names the backup", async () => {
    const user = userEvent.setup();
    vi.mocked(chooseProfileToRestore).mockResolvedValue(PREVIEW);
    vi.mocked(confirmRestore).mockResolvedValue({
      backupFileName: "before-restore-2026-10-02.mudraft",
      preview: PREVIEW,
    });
    renderApp("/settings");
    const profile = screen.getByRole("region", { name: "Profile" });
    await user.click(within(profile).getByRole("button", { name: "Restore from a profile…" }));
    const dialog = await screen.findByRole("dialog", { name: "Replace your profile?" });
    expect(dialog).toHaveTextContent("3 albums, 7 listens");
    expect(dialog).toHaveTextContent("v9, upgraded to v10 when restored");
    expect(dialog).toHaveTextContent("backed up first");
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(cancelRestore).toHaveBeenCalled();
    expect(confirmRestore).not.toHaveBeenCalled();

    await user.click(within(profile).getByRole("button", { name: "Restore from a profile…" }));
    await user.click(
      within(await screen.findByRole("dialog", { name: "Replace your profile?" })).getByRole(
        "button",
        { name: "Replace my profile" },
      ),
    );
    expect(confirmRestore).toHaveBeenCalledWith("abc123");
    expect(await screen.findByRole("dialog", { name: "Profile restored" })).toHaveTextContent(
      "before-restore-2026-10-02.mudraft",
    );
  });

  it("shows why an archive can't be restored, changing nothing", async () => {
    const user = userEvent.setup();
    vi.mocked(chooseProfileToRestore).mockRejectedValue(
      new NativeError("validation", "it was made by a newer version of MuDraft"),
    );
    renderApp("/settings");
    await user.click(screen.getByRole("button", { name: "Restore from a profile…" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("newer version of MuDraft");
    expect(screen.queryByRole("dialog", { name: "Replace your profile?" })).not.toBeInTheDocument();
    expect(confirmRestore).not.toHaveBeenCalled();
  });

  it("clears saved MusicBrainz responses only after confirming", async () => {
    const user = userEvent.setup();
    renderApp("/settings");
    const section = screen.getByRole("region", { name: "Metadata" });
    expect(
      await within(section).findByText(/2 saved \(2 KB\), 1 older than a week/),
    ).toBeInTheDocument();
    await user.click(within(section).getByRole("button", { name: "Clear saved responses" }));
    expect(clearMetadataCache).not.toHaveBeenCalled();
    const dialog = screen.getByRole("dialog", { name: "Clear saved MusicBrainz responses?" });
    await user.click(within(dialog).getByRole("button", { name: "Clear" }));
    expect(clearMetadataCache).toHaveBeenCalledTimes(1);
    expect(
      await screen.findByText(/Removed 2 saved responses. Your library is unchanged./),
    ).toBeInTheDocument();
  });

  it("shows the app version, storage location, and data sources", async () => {
    renderApp("/settings");
    expect(await screen.findByText("MuDraft 0.1.0 · SQLite 3.53.2")).toBeInTheDocument();
    expect(screen.getByText(/app\.mudraft\.desktop\.dev/)).toBeInTheDocument();
    const sources = screen.getByRole("region", { name: "Data sources" });
    expect(sources).toHaveTextContent("MusicBrainz");
    expect(sources).toHaveTextContent("Cover Art Archive");
    await waitFor(() => {
      expect(metadataCacheStats).toHaveBeenCalled();
    });
  });
});
