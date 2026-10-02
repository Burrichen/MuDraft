import { callNative } from "../transport/native";
import { addCatalogueEntry, artistCatalogue, fetchArtistCatalogue } from "./discography";

vi.mock("../transport/native", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../transport/native")>()),
  callNative: vi.fn(),
}));

describe("discography service", () => {
  it("sends request IDs and validates responses", async () => {
    vi.mocked(callNative).mockResolvedValueOnce({
      pagesFetched: 2,
      complete: true,
      usedOfflineCache: false,
    });
    await expect(fetchArtistCatalogue("k", "a")).resolves.toMatchObject({ complete: true });
    expect(callNative).toHaveBeenCalledWith("artist_catalogue_fetch", {
      requestId: "k",
      artistId: "a",
    });
    vi.mocked(callNative).mockResolvedValueOnce({ entries: "nope" });
    await expect(artistCatalogue("a")).rejects.toMatchObject({ code: "invalid_response" });
    vi.mocked(callNative).mockResolvedValueOnce("id");
    await addCatalogueEntry("a", { title: "Demo", year: 2003, primaryType: "Album" });
    expect(vi.mocked(callNative).mock.calls.at(-1)?.[1]).toEqual({
      artistId: "a",
      entry: { title: "Demo", year: 2003, primaryType: "Album", secondaryTypes: [] },
    });
  });
});
