import { callNative } from "../transport/native";
import { chooseNextUp, resetNextUpPool, rollNextUp } from "./nextUp";

vi.mock("../transport/native", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../transport/native")>()),
  callNative: vi.fn(),
}));

describe("Next Up service", () => {
  it("sends the method and request key, and validates outcomes", async () => {
    vi.mocked(callNative).mockResolvedValueOnce({ kind: "empty", reason: "no_listen_asap" });
    await expect(rollNextUp("k1", { mode: "weighted_random" })).resolves.toEqual({
      kind: "empty",
      reason: "no_listen_asap",
    });
    expect(callNative).toHaveBeenCalledWith("next_up_roll", {
      requestId: "k1",
      method: { mode: "weighted_random" },
    });

    vi.mocked(callNative).mockResolvedValueOnce({ kind: "surprise" });
    await expect(
      rollNextUp("k2", {
        mode: "guided",
        criteria: { years: { any_of: [{ decade: 1990 }, "current_year"] } },
      }),
    ).rejects.toMatchObject({ code: "invalid_response" });

    vi.mocked(callNative).mockResolvedValueOnce(2);
    await expect(resetNextUpPool()).resolves.toBe(2);
    vi.mocked(callNative).mockResolvedValueOnce({ nope: true });
    await expect(chooseNextUp("k3", "e")).rejects.toMatchObject({ code: "invalid_response" });
  });
});
