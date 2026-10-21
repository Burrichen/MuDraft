import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import axe from "axe-core";
import {
  checkSync,
  getSyncStatus,
  onSyncProgress,
  syncNow,
  type SyncCounts,
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
  onSyncProgress: vi.fn(() => Promise.resolve(() => undefined)),
}));

const COUNTS: SyncCounts = {
  artists: 1,
  albums: 2,
  editions: 2,
  listens: 3,
  ratedTracks: 1,
  reviews: 0,
  tags: 1,
  listenList: 1,
  collection: 1,
  artwork: 0,
};
const STATUS: SyncStatus = {
  configured: true,
  owner: "me",
  repo: "MuDraft-Sync",
  branch: "main",
  hasToken: true,
  linked: true,
  revision: "r1",
  lastSyncedAt: "2026-10-20T10:00:00Z",
  pendingLocalChanges: false,
  pending: null,
  unresolvedConflicts: 0,
  lastError: null,
};

/** Page-level and rendering rules don't apply to one section in jsdom. */
async function violations(root: Element = document.body) {
  const result = await axe.run(root, {
    rules: { "color-contrast": { enabled: false }, region: { enabled: false } },
  });
  return result.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.html).join(" | ")}`);
}

beforeEach(() => {
  vi.mocked(onSyncProgress).mockResolvedValue(() => undefined);
});

describe("GitHub Sync accessibility", () => {
  it("really checks (an unlabelled field is caught)", async () => {
    const { container } = render(<input type="text" />);
    expect((await violations(container)).join()).toMatch(/^label:/);
  });

  it("has no axe violations before connecting", async () => {
    vi.mocked(getSyncStatus).mockResolvedValue({
      ...STATUS,
      configured: false,
      linked: false,
      hasToken: false,
    });
    render(<SyncSettings />);
    await screen.findByLabelText("Access token for this computer");
    expect(await violations()).toEqual([]);
  });

  it("has no axe violations when connected, failed, or resolving conflicts", async () => {
    const user = userEvent.setup();
    vi.mocked(getSyncStatus).mockResolvedValue(STATUS);
    vi.mocked(syncNow).mockRejectedValueOnce(
      new NativeError("sync_offline", "Can't reach GitHub."),
    );
    render(<SyncSettings />);
    await user.click(await screen.findByRole("button", { name: "Sync Now" }));
    await screen.findByRole("alert");
    expect(await violations()).toEqual([]);

    vi.mocked(syncNow).mockResolvedValueOnce({
      status: "conflicts",
      revision: "r2",
      conflicts: [
        {
          id: "field:1",
          kind: "field",
          record: "Review of OK Computer (Standard)",
          table: "album_review",
          key: ["e"],
          columns: ["review"],
          base: { review: "a" },
          local: { review: "b" },
          remote: { review: "c" },
        },
      ],
    });
    await user.click(screen.getByRole("button", { name: "Sync Now" }));
    const dialog = await screen.findByRole("dialog", { name: "Resolve 1 conflict" });
    expect(await violations(dialog)).toEqual([]);
    // Every choice is a labelled radio in a named group.
    expect(within(dialog).getAllByRole("radio")).toHaveLength(2);
  });

  it("has no axe violations in the first-sync preview", async () => {
    const user = userEvent.setup();
    vi.mocked(getSyncStatus).mockResolvedValue({ ...STATUS, linked: false, revision: null });
    vi.mocked(checkSync).mockResolvedValue({
      branch: "main",
      kind: "ready",
      sameSync: null,
      remote: COUNTS,
      local: COUNTS,
      remoteChanges: null,
      revision: "r9",
    });
    render(<SyncSettings />);
    await user.click(await screen.findByRole("button", { name: "Set up first sync…" }));
    const dialog = await screen.findByRole("dialog", { name: "First sync on this computer" });
    expect(await violations(dialog)).toEqual([]);
  });
});
