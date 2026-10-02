import { callNative } from "../transport/native";
import { isPersistableRoute, loadPreferences, startRoute, updatePreferences } from "./preferences";

vi.mock("../transport/native", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../transport/native")>()),
  callNative: vi.fn(),
}));

const stored = {
  sidebarCollapsed: true,
  lastRoute: "/stats",
  albumLayout: "list" as const,
  startPage: "last" as const,
  showArtwork: true,
};

describe("preferences service", () => {
  it("loads validated preferences", async () => {
    vi.mocked(callNative).mockResolvedValueOnce(stored);
    await expect(loadPreferences()).resolves.toEqual(stored);
    expect(callNative).toHaveBeenCalledWith("get_ui_preferences");
  });

  it("sends partial patches under the `patch` argument", async () => {
    vi.mocked(callNative).mockResolvedValueOnce(stored);
    await updatePreferences({ sidebarCollapsed: true });
    expect(callNative).toHaveBeenCalledWith("update_ui_preferences", {
      patch: { sidebarCollapsed: true },
    });
  });

  it("rejects malformed responses", async () => {
    vi.mocked(callNative).mockResolvedValueOnce({ ...stored, albumLayout: "masonry" });
    await expect(loadPreferences()).rejects.toMatchObject({ code: "invalid_response" });
  });

  it("opens the chosen start page, or the last page when set to last", () => {
    expect(startRoute(stored)).toBe("/stats");
    expect(startRoute({ ...stored, startPage: "/next-up" })).toBe("/next-up");
  });

  it("rejects unknown start pages", async () => {
    vi.mocked(callNative).mockResolvedValueOnce({ ...stored, startPage: "/diary" });
    await expect(loadPreferences()).rejects.toMatchObject({ code: "invalid_response" });
  });

  it("matches the Rust route rule", () => {
    expect(isPersistableRoute("/collection")).toBe(true);
    expect(isPersistableRoute("/albums/0190f5c3-2b7a-7c3e-9a1b-3c4d5e6f7a8b")).toBe(true);
    expect(isPersistableRoute("/albums/42")).toBe(false);
    expect(isPersistableRoute("/diary")).toBe(false);
    expect(isPersistableRoute("/")).toBe(false);
  });
});
