import { callNative } from "../transport/native";
import { addManualAlbum, importRelease, loadEditions, searchAlbums } from "./metadata";

vi.mock("../transport/native", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../transport/native")>()),
  callNative: vi.fn(),
}));

const page = {
  value: { candidates: [], total: 0, offset: 0 },
  fetchedAt: "2026-09-30T12:00:00.000Z",
  source: "network",
};

describe("metadata service", () => {
  it("sends a request ID and explicit nulls for omitted fields", async () => {
    vi.mocked(callNative).mockResolvedValueOnce(page);
    await searchAlbums({ title: "OK Computer" });
    const [command, args] = vi.mocked(callNative).mock.calls[0] ?? [];
    expect(command).toBe("metadata_search");
    expect(args).toMatchObject({
      query: { title: "OK Computer", artist: null, year: null },
      offset: 0,
    });
    expect(String(args?.requestId)).toMatch(/^[0-9a-f-]{36}$/);
  });

  it("cancels the same request in Rust when the signal aborts", async () => {
    let resolve: (v: unknown) => void = () => undefined;
    vi.mocked(callNative).mockImplementation((command) =>
      command === "metadata_cancel" ? Promise.resolve(true) : new Promise((r) => (resolve = r)),
    );
    const controller = new AbortController();
    const pending = loadEditions("b1392450-e666-3926-a536-22c65f834433", {
      signal: controller.signal,
    });
    controller.abort();
    const calls = vi.mocked(callNative).mock.calls;
    const requestId = calls[0]?.[1]?.requestId;
    expect(calls[1]).toEqual(["metadata_cancel", { requestId }]);
    resolve({ ...page, value: { editions: [], total: 0, truncated: false } });
    await pending;
  });

  it("refuses to start an already-aborted request", async () => {
    const controller = new AbortController();
    controller.abort();
    await expect(searchAlbums({ title: "x" }, { signal: controller.signal })).rejects.toMatchObject(
      {
        code: "cancelled",
      },
    );
    expect(callNative).not.toHaveBeenCalled();
  });

  it("maps import and manual arguments and validates responses", async () => {
    vi.mocked(callNative).mockResolvedValueOnce({ albumId: "a", editionId: "e" });
    await importRelease({ releaseGroupId: "rg", releaseId: "r", addToListenList: true });
    expect(vi.mocked(callNative).mock.calls[0]?.[1]).toMatchObject({
      releaseGroupId: "rg",
      releaseId: "r",
      editionName: null,
      destination: { listenList: true, collection: false },
    });
    vi.mocked(callNative).mockResolvedValueOnce({ albumId: "a", editionId: "e", artistId: "x" });
    await addManualAlbum({ title: "Demo", artistName: "Band" }, false, true);
    expect(vi.mocked(callNative).mock.calls[1]?.[1]).toMatchObject({
      album: { title: "Demo", artistName: "Band", artistId: null, year: null, editionName: null },
      destination: { listenList: false, collection: true },
    });
    vi.mocked(callNative).mockResolvedValueOnce({ nope: true });
    await expect(searchAlbums({ title: "x" })).rejects.toMatchObject({ code: "invalid_response" });
  });
});
