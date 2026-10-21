import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  checkSync,
  configureSync,
  disconnectSync,
  getSyncStatus,
  initializeSync,
  onSyncProgress,
  removeSyncToken,
  saveSyncToken,
  syncNow,
  type RemoteCheck,
  type SyncConflict,
  type SyncCounts,
  type SyncProgress,
  type SyncStatus,
} from "../../services/sync";
import { NativeError } from "../../transport/native";
import { SyncSettings } from "./SyncSettings";

vi.mock("../../services/sync", () => ({
  getSyncStatus: vi.fn(),
  configureSync: vi.fn(),
  disconnectSync: vi.fn(),
  saveSyncToken: vi.fn(),
  removeSyncToken: vi.fn(),
  checkSync: vi.fn(),
  initializeSync: vi.fn(),
  syncNow: vi.fn(),
  onSyncProgress: vi.fn(),
}));

const TOKEN = "github_pat_11SECRET_never_shown_0123456789abcdefghijklmnop";
const EMPTY = { albums: 0, listens: 0, ratings: 0, reviews: 0, tags: 0, artwork: 0, other: 0 };
const COUNTS: SyncCounts = {
  artists: 2,
  albums: 3,
  editions: 3,
  listens: 5,
  ratedTracks: 4,
  reviews: 1,
  tags: 2,
  listenList: 1,
  collection: 2,
  artwork: 0,
};
const NONE: SyncCounts = {
  ...COUNTS,
  artists: 0,
  albums: 0,
  editions: 0,
  listens: 0,
  ratedTracks: 0,
  reviews: 0,
  tags: 0,
  listenList: 0,
  collection: 0,
};

const UNCONFIGURED: SyncStatus = {
  configured: false,
  owner: null,
  repo: null,
  branch: null,
  hasToken: false,
  linked: false,
  revision: null,
  lastSyncedAt: null,
  pendingLocalChanges: null,
  pending: null,
  unresolvedConflicts: 0,
  lastError: null,
};
const CONNECTED: SyncStatus = {
  ...UNCONFIGURED,
  configured: true,
  owner: "me",
  repo: "MuDraft-Sync",
  branch: "main",
  hasToken: true,
};
const LINKED: SyncStatus = {
  ...CONNECTED,
  linked: true,
  revision: "r1",
  lastSyncedAt: "2026-10-20T10:00:00Z",
  pendingLocalChanges: false,
  pending: EMPTY,
};

function ready(overrides: Partial<RemoteCheck> = {}): RemoteCheck {
  return {
    branch: "main",
    kind: "ready",
    sameSync: null,
    remote: { ...COUNTS, albums: 7 },
    local: COUNTS,
    remoteChanges: null,
    revision: "r9",
    ...overrides,
  };
}

const TITLE_CONFLICT: SyncConflict = {
  id: "field:1",
  kind: "field",
  record: "OK Computer",
  table: "album",
  key: ["a"],
  columns: ["title"],
  base: { title: "OK Computer" },
  local: { title: "Mac title" },
  remote: { title: "Windows title" },
};
const RATING_CONFLICT: SyncConflict = {
  id: "field:2",
  kind: "field",
  record: "Rating of “Airbag” on OK Computer (Standard)",
  table: "track_rating",
  key: ["t"],
  columns: ["rating"],
  base: { rating: 8 },
  local: { rating: 0 },
  remote: { rating: null },
};

let progress: (p: SyncProgress) => void = () => undefined;

beforeEach(() => {
  vi.mocked(onSyncProgress).mockImplementation((handler) => {
    progress = handler;
    return Promise.resolve(() => undefined);
  });
});

const section = () => screen.getByRole("region", { name: "GitHub Sync" });

describe("GitHub Sync settings", () => {
  it("explains setup, connects without ever showing the token, and previews the first sync", async () => {
    const user = userEvent.setup();
    vi.mocked(getSyncStatus).mockResolvedValue(UNCONFIGURED);
    vi.mocked(configureSync).mockResolvedValue({ ...CONNECTED, hasToken: false });
    vi.mocked(saveSyncToken).mockResolvedValue(CONNECTED);
    vi.mocked(checkSync).mockResolvedValue(ready());
    render(<SyncSettings />);
    expect(await within(section()).findByText("Not Connected")).toBeInTheDocument();
    const steps = within(section()).getByText("How to set it up").closest("details");
    expect(steps).toHaveTextContent("Private");
    expect(steps).toHaveTextContent("Only select repositories");
    expect(steps).toHaveTextContent("Contents to “Read and write”");
    expect(steps).toHaveTextContent("never asks for your GitHub password");
    expect(steps).toHaveTextContent("isn’t end-to-end encrypted");

    await user.type(screen.getByLabelText("Repository owner"), "me");
    expect(section()).toHaveTextContent(
      "Data will go only to the sync/ folder of github.com/me/MuDraft-Sync",
    );
    await user.click(screen.getByRole("button", { name: "Connect" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Paste the fine-grained token");
    expect(configureSync).not.toHaveBeenCalled();

    const token = screen.getByLabelText("Access token for this computer");
    expect(token).toHaveAttribute("type", "password");
    expect(token).toHaveAttribute("autocomplete", "off");
    await user.type(token, TOKEN);
    await user.click(screen.getByRole("button", { name: "Connect" }));
    expect(configureSync).toHaveBeenCalledWith("me", "MuDraft-Sync");
    expect(saveSyncToken).toHaveBeenCalledWith(TOKEN);

    const dialog = await screen.findByRole("dialog", { name: "First sync on this computer" });
    const table = within(dialog).getByRole("table");
    expect(within(table).getByRole("row", { name: /Albums 3 7/ })).toBeInTheDocument();
    expect(within(table).getByRole("row", { name: /In the Collection 2 2/ })).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Merge both" })).toBeInTheDocument();
    expect(
      within(dialog).queryByRole("button", { name: "Upload this library" }),
    ).not.toBeInTheDocument();
    expect(syncNow).not.toHaveBeenCalled();
    expect(document.body).not.toHaveTextContent(TOKEN);
  });

  it("asks twice before downloading over an existing library, and backs up first", async () => {
    const user = userEvent.setup();
    vi.mocked(getSyncStatus).mockResolvedValue(CONNECTED);
    vi.mocked(checkSync).mockResolvedValue(ready());
    vi.mocked(syncNow).mockResolvedValue({
      status: "synced",
      revision: "r9",
      uploadedFiles: 0,
      downloadedFiles: 12,
      localChanges: 40,
      received: { ...EMPTY, albums: 7, listens: 9 },
      sent: EMPTY,
      backupFileName: "mudraft-20261021-pre-sync.sqlite3",
    });
    render(<SyncSettings />);
    await user.click(await screen.findByRole("button", { name: "Set up first sync…" }));
    const dialog = await screen.findByRole("dialog", { name: "First sync on this computer" });
    await user.click(within(dialog).getByRole("button", { name: "Download remote library…" }));
    expect(syncNow).not.toHaveBeenCalled();
    expect(within(dialog).getByRole("alert")).toHaveTextContent(
      "replaces this computer’s library — 3 albums and 2 in the Collection",
    );
    expect(within(dialog).getByRole("alert")).toHaveTextContent("A backup is saved first");
    await user.click(within(dialog).getByRole("button", { name: "Replace and download" }));
    expect(syncNow).toHaveBeenCalledWith({ firstSync: "download" });
    expect(
      await within(section()).findByText(/Received: 7 albums, 9 listens\./),
    ).toBeInTheDocument();
    expect(section()).toHaveTextContent("saved as “mudraft-20261021-pre-sync.sqlite3”");
  });

  it("offers upload for an empty repository and download for an empty computer", async () => {
    const user = userEvent.setup();
    vi.mocked(getSyncStatus).mockResolvedValue(CONNECTED);
    vi.mocked(checkSync).mockResolvedValueOnce(ready({ kind: "uninitialised", remote: null }));
    vi.mocked(initializeSync).mockResolvedValue({
      status: "synced",
      revision: "r1",
      uploadedFiles: 30,
      downloadedFiles: 0,
      localChanges: 0,
      received: EMPTY,
      sent: { ...EMPTY, albums: 3 },
      backupFileName: null,
    });
    render(<SyncSettings />);
    await user.click(await screen.findByRole("button", { name: "Set up first sync…" }));
    let dialog = await screen.findByRole("dialog", { name: "First sync on this computer" });
    expect(within(dialog).queryByRole("button", { name: /Download/ })).not.toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Upload this library" }));
    expect(initializeSync).toHaveBeenCalledWith(true);
    expect(await within(section()).findByText(/Sent: 3 albums\./)).toBeInTheDocument();

    vi.mocked(checkSync).mockResolvedValueOnce(ready({ local: NONE }));
    await user.click(screen.getByRole("button", { name: "Set up first sync…" }));
    dialog = await screen.findByRole("dialog", { name: "First sync on this computer" });
    expect(
      within(dialog).getByRole("button", { name: "Download remote library" }),
    ).toBeInTheDocument();
    expect(within(dialog).queryByRole("button", { name: "Merge both" })).not.toBeInTheDocument();
  });

  it("shows status, progress, and success only after the sync completes", async () => {
    const user = userEvent.setup();
    vi.mocked(getSyncStatus).mockResolvedValue({
      ...LINKED,
      pendingLocalChanges: true,
      pending: { ...EMPTY, albums: 1, ratings: 2 },
    });
    let finish: (v: Awaited<ReturnType<typeof syncNow>>) => void = () => undefined;
    vi.mocked(syncNow).mockReturnValue(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    render(<SyncSettings />);
    expect(await within(section()).findByText("Changes Pending")).toBeInTheDocument();
    expect(section()).toHaveTextContent("github.com/me/MuDraft-Sync (private), branch main");
    expect(section()).toHaveTextContent("1 album, 2 ratings");
    expect(section()).toHaveTextContent("Saved in this computer’s secure credential store");

    await user.click(screen.getByRole("button", { name: "Sync Now" }));
    expect(within(section()).getAllByRole("status")[0]).toHaveTextContent("Syncing");
    progress({ phase: "uploading", done: 2, total: 5 });
    expect(await screen.findByRole("progressbar", { name: "Uploading" })).toHaveAttribute(
      "aria-valuenow",
      "2",
    );
    expect(section()).toHaveTextContent("Uploading changes…");
    expect(section()).not.toHaveTextContent("Synced with");

    vi.mocked(getSyncStatus).mockResolvedValue(LINKED);
    finish({
      status: "synced",
      revision: "r2",
      uploadedFiles: 3,
      downloadedFiles: 1,
      localChanges: 1,
      received: { ...EMPTY, listens: 1 },
      sent: { ...EMPTY, albums: 1, ratings: 2 },
      backupFileName: null,
    });
    expect(
      await within(section()).findByText(
        "Synced with github.com/me/MuDraft-Sync. Received: 1 listen. Sent: 1 album, 2 ratings.",
      ),
    ).toBeInTheDocument();
    expect(await within(section()).findByText("Up to Date")).toBeInTheDocument();
  });

  it("resolves conflicts one by one, keeps choices on cancel, and works by keyboard", async () => {
    const user = userEvent.setup();
    vi.mocked(getSyncStatus).mockResolvedValue(LINKED);
    const conflicts = {
      status: "conflicts" as const,
      revision: "r2",
      conflicts: [TITLE_CONFLICT, RATING_CONFLICT],
    };
    vi.mocked(syncNow).mockResolvedValue(conflicts);
    render(<SyncSettings />);
    await user.click(await screen.findByRole("button", { name: "Sync Now" }));
    let dialog = await screen.findByRole("dialog", { name: "Resolve 2 conflicts" });
    const title = within(dialog).getByRole("group", { name: /OK Computer.*Title/ });
    expect(title).toHaveTextContent("Keep this computer’s versionMac title");
    expect(title).toHaveTextContent("Keep GitHub’s versionWindows title");
    const rating = within(dialog).getByRole("group", { name: /Rating of “Airbag”/ });
    expect(rating).toHaveTextContent("0 stars (rated zero)");
    expect(rating).toHaveTextContent("Unrated");
    expect(dialog).not.toHaveTextContent(/\bMac\b(?! title)|Windows(?! title)/);

    const apply = within(dialog).getByRole("button", { name: "Apply choices and sync" });
    expect(apply).toHaveAttribute("aria-disabled", "true");
    expect(dialog).toHaveTextContent("Choose a version for 2 conflicts");
    await user.click(within(title).getByRole("radio", { name: /Keep GitHub’s version/ }));
    expect(dialog).toHaveTextContent("Choose a version for 1 conflict");

    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog", { name: /Resolve/ })).not.toBeInTheDocument();
    expect(section()).toHaveTextContent("Nothing changed. Your changes stay pending");
    expect(syncNow).toHaveBeenCalledTimes(1);

    // Reopened: the earlier choice is still there. Finish by keyboard.
    await user.click(screen.getByRole("button", { name: "Sync Now" }));
    dialog = await screen.findByRole("dialog", { name: "Resolve 2 conflicts" });
    expect(
      within(dialog)
        .getByRole("group", { name: /^OK Computer/ })
        .querySelector("input[value=remote]"),
    ).toBeChecked();
    const keepLocal = within(within(dialog).getByRole("group", { name: /Rating of/ })).getByRole(
      "radio",
      {
        name: /Keep this computer’s version/,
      },
    );
    keepLocal.focus();
    await user.keyboard(" ");
    expect(keepLocal).toBeChecked();
    vi.mocked(syncNow).mockResolvedValueOnce({
      status: "synced",
      revision: "r3",
      uploadedFiles: 2,
      downloadedFiles: 1,
      localChanges: 1,
      received: { ...EMPTY, albums: 1 },
      sent: { ...EMPTY, ratings: 1 },
      backupFileName: "b.sqlite3",
    });
    within(dialog).getByRole("button", { name: "Apply choices and sync" }).focus();
    await user.keyboard("{Enter}");
    expect(syncNow).toHaveBeenLastCalledWith({
      resolutions: [
        { id: "field:1", choice: "remote" },
        { id: "field:2", choice: "local" },
      ],
    });
    expect(
      await within(section()).findByText(/Received: 1 album\. Sent: 1 rating\./),
    ).toBeInTheDocument();
  });

  it.each([
    [
      "auth_required",
      "GitHub rejected the access token (expired or revoked).",
      "GitHub didn’t accept a token",
      "Enter a new token",
    ],
    ["sync_offline", "Can't reach GitHub.", "Can’t reach GitHub", "Try again"],
    ["rate_limited", "GitHub is limiting requests", "GitHub is limiting requests", "Try again"],
    [
      "disk_full",
      "syncing needs about 300 MB free; 12 MB is free. Nothing was changed",
      "Not enough free disk space",
      "Try again",
    ],
    [
      "sync_setup",
      "the sync data comes from a newer MuDraft; update MuDraft on this computer",
      "Made by a newer MuDraft",
      null,
    ],
    [
      "sync_setup",
      "the repository wasn't found, or the token can't access it",
      "Repository not available",
      "Check repository details",
    ],
    [
      "storage_unavailable",
      "disk I/O error. Your library on this computer wasn't changed. Sync again to try once more.",
      "Couldn’t apply the changes here",
      "Try again",
    ],
  ])("explains %s and offers a safe next step", async (code, message, title, action) => {
    const user = userEvent.setup();
    vi.mocked(getSyncStatus).mockResolvedValue(LINKED);
    vi.mocked(syncNow).mockRejectedValue(new NativeError(code, message));
    render(<SyncSettings />);
    await user.click(await screen.findByRole("button", { name: "Sync Now" }));
    const alert = await within(section()).findByRole("alert");
    expect(alert).toHaveTextContent(title);
    expect(alert).toHaveTextContent(`Error code: ${code}`);
    if (action) expect(within(alert).getByRole("button", { name: action })).toBeInTheDocument();
    else expect(within(alert).queryByRole("button")).not.toBeInTheDocument();
    if (code === "auth_required") {
      await user.click(within(alert).getByRole("button", { name: "Enter a new token" }));
      expect(screen.getByLabelText("Access token for this computer")).toHaveValue("");
    }
  });

  it("tests the connection without syncing and reports GitHub's changes", async () => {
    const user = userEvent.setup();
    vi.mocked(getSyncStatus).mockResolvedValue(LINKED);
    vi.mocked(checkSync).mockResolvedValue(
      ready({ sameSync: true, remoteChanges: { ...EMPTY, listens: 2 }, revision: "r5" }),
    );
    render(<SyncSettings />);
    await user.click(await screen.findByRole("button", { name: "Test Connection" }));
    expect(
      await within(section()).findByText(
        /Changes on GitHub since this computer last synced: 2 listens\./,
      ),
    ).toBeInTheDocument();
    expect(within(section()).getByText("Changes Pending")).toBeInTheDocument();
    expect(syncNow).not.toHaveBeenCalled();
  });

  it("removes the token or disconnects without touching the library", async () => {
    const user = userEvent.setup();
    vi.mocked(getSyncStatus).mockResolvedValue({ ...LINKED, pendingLocalChanges: true });
    vi.mocked(removeSyncToken).mockResolvedValue({ ...LINKED, hasToken: false });
    vi.mocked(disconnectSync).mockResolvedValue({ ...UNCONFIGURED, hasToken: true });
    render(<SyncSettings />);
    await user.click(await screen.findByRole("button", { name: "Disconnect…" }));
    const dialog = screen.getByRole("dialog", { name: "Disconnect this computer?" });
    expect(dialog).toHaveTextContent("including changes not yet synced");
    await user.click(within(dialog).getByRole("checkbox", { name: /Also remove the saved token/ }));
    await user.click(within(dialog).getByRole("button", { name: "Disconnect" }));
    await waitFor(() => {
      expect(disconnectSync).toHaveBeenCalled();
    });
    expect(removeSyncToken).toHaveBeenCalled();
    expect(
      await within(section()).findByText(
        /no longer syncs\. Your library here and the data on GitHub are unchanged/,
      ),
    ).toBeInTheDocument();
  });
});
