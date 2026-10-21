import { callNative, listenNative } from "../transport/native";
import {
  checkSync,
  getSyncStatus,
  initializeSync,
  onSyncProgress,
  saveSyncToken,
  syncNow,
} from "./sync";

vi.mock("../transport/native", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../transport/native")>()),
  callNative: vi.fn(),
  listenNative: vi.fn(),
}));

const STATUS = {
  configured: true,
  owner: "me",
  repo: "MuDraft-Sync",
  branch: "main",
  hasToken: true,
  linked: true,
  revision: "abc",
  lastSyncedAt: "2026-10-20T10:00:00Z",
  pendingLocalChanges: false,
  pending: null,
  unresolvedConflicts: 0,
  lastError: null,
};

describe("sync service", () => {
  it("sends the token once and validates status", async () => {
    vi.mocked(callNative).mockResolvedValueOnce(STATUS);
    await expect(saveSyncToken("github_pat_x")).resolves.toEqual(STATUS);
    expect(callNative).toHaveBeenCalledWith("sync_set_token", { token: "github_pat_x" });
    vi.mocked(callNative).mockResolvedValueOnce({ token: "github_pat_x" });
    await expect(getSyncStatus()).rejects.toMatchObject({ code: "invalid_response" });
  });

  it("passes the first-sync choice and resolutions, and rejects odd outcomes", async () => {
    vi.mocked(callNative).mockResolvedValueOnce({ status: "upToDate", revision: "r" });
    await syncNow({ firstSync: "merge", resolutions: [{ id: "c1", choice: "local" }] });
    expect(callNative).toHaveBeenLastCalledWith("sync_now", {
      request: { firstSync: "merge", resolutions: [{ id: "c1", choice: "local" }] },
    });
    vi.mocked(callNative).mockResolvedValueOnce({ status: "upToDate", revision: "r" });
    await syncNow();
    expect(callNative).toHaveBeenLastCalledWith("sync_now", { request: { resolutions: [] } });
    vi.mocked(callNative).mockResolvedValueOnce({ status: "done" });
    await expect(syncNow()).rejects.toMatchObject({ code: "invalid_response" });
    vi.mocked(callNative).mockResolvedValueOnce({ status: "conflicts" });
    await expect(syncNow()).rejects.toMatchObject({ code: "invalid_response" });

    vi.mocked(callNative).mockResolvedValueOnce({ status: "upToDate", revision: "r" });
    await initializeSync(true);
    expect(callNative).toHaveBeenLastCalledWith("sync_initialize", { confirm: true });
    vi.mocked(callNative).mockResolvedValueOnce({ kind: "ready" });
    await expect(checkSync()).rejects.toMatchObject({ code: "invalid_response" });
  });

  it("forwards only well-formed progress events", async () => {
    let deliver: (payload: unknown) => void = () => undefined;
    vi.mocked(listenNative).mockImplementation((_event, handler) => {
      deliver = handler;
      return Promise.resolve(() => undefined);
    });
    const seen: unknown[] = [];
    await onSyncProgress((p) => seen.push(p));
    expect(listenNative).toHaveBeenCalledWith("sync-progress", expect.any(Function));
    deliver({ phase: "uploading", done: 1, total: 3 });
    deliver({ phase: "exploding" });
    deliver("reading");
    expect(seen).toEqual([{ phase: "uploading", done: 1, total: 3 }]);
  });
});
