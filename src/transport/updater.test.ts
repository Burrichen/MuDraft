import { callNative } from "./native";
import { checkForUpdate } from "./updater";

vi.mock("../app/platform", () => ({ IS_ANDROID: true }));
vi.mock("./native", async (original) => ({
  ...(await original<typeof import("./native")>()),
  callNative: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: vi.fn() }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: vi.fn() }));

describe("Android update check", () => {
  it("is null when this is the newest version", async () => {
    vi.mocked(callNative).mockResolvedValueOnce(null);
    expect(await checkForUpdate()).toBeNull();
    expect(callNative).toHaveBeenCalledWith("app_update_check");
  });

  it("offers the newer version and opens its download on install", async () => {
    vi.mocked(callNative)
      .mockResolvedValueOnce({ version: "1.5.0", currentVersion: "1.4.0", notes: "- Faster" })
      .mockResolvedValueOnce(null);
    const update = await checkForUpdate();
    expect(update).toMatchObject({ version: "1.5.0", notes: "- Faster", manual: true });
    await update?.install(() => undefined);
    expect(callNative).toHaveBeenLastCalledWith("app_update_open", { version: "1.5.0" });
  });

  it("rejects a malformed answer", async () => {
    vi.mocked(callNative).mockResolvedValueOnce({ version: 5 });
    await expect(checkForUpdate()).rejects.toMatchObject({ code: "invalid_response" });
  });
});
