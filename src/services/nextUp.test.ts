import { callNative } from "../transport/native";
import { chooseNextUp, clearNextUp, nextUpState, resetNextUpPool, rollNextUp } from "./nextUp";

vi.mock("../transport/native", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../transport/native")>()),
  callNative: vi.fn(),
}));

describe("Next Up service", () => {
  it("sends the slot, method, and request key, and validates outcomes", async () => {
    vi.mocked(callNative).mockResolvedValueOnce({ kind: "empty", reason: "no_listen_asap" });
    await expect(rollNextUp("k1", 2, { mode: "weighted_random" })).resolves.toEqual({
      kind: "empty",
      reason: "no_listen_asap",
    });
    expect(callNative).toHaveBeenCalledWith("next_up_roll", {
      requestId: "k1",
      slot: 2,
      method: { mode: "weighted_random" },
    });

    vi.mocked(callNative).mockResolvedValueOnce({ kind: "surprise" });
    await expect(
      rollNextUp("k2", 1, {
        mode: "guided",
        criteria: { years: { any_of: [{ decade: 1990 }, "current_year"] } },
      }),
    ).rejects.toMatchObject({ code: "invalid_response" });

    vi.mocked(callNative).mockResolvedValueOnce(2);
    await expect(resetNextUpPool(3)).resolves.toBe(2);
    expect(callNative).toHaveBeenLastCalledWith("next_up_reset_pool", { slot: 3 });
    vi.mocked(callNative).mockResolvedValueOnce({ nope: true });
    await expect(chooseNextUp("k3", "e")).rejects.toMatchObject({ code: "invalid_response" });
    // Without a slot, the native side picks the lowest free one.
    expect(callNative).toHaveBeenLastCalledWith("next_up_choose", {
      requestId: "k3",
      editionId: "e",
      slot: null,
    });
    vi.mocked(callNative).mockResolvedValueOnce(true);
    await expect(clearNextUp(4)).resolves.toBe(true);
    expect(callNative).toHaveBeenLastCalledWith("next_up_clear", { slot: 4 });
  });

  it("rejects state without slots", async () => {
    vi.mocked(callNative).mockResolvedValueOnce({ current: null, session: null });
    await expect(nextUpState()).rejects.toMatchObject({ code: "invalid_response" });
    vi.mocked(callNative).mockResolvedValueOnce({ slots: [], maxSlots: 5, nextFreeSlot: 1 });
    await expect(nextUpState()).resolves.toEqual({ slots: [], maxSlots: 5, nextFreeSlot: 1 });
  });
});
