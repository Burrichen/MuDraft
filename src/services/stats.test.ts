import { callNative } from "../transport/native";
import { completionRate, statsOverview } from "./stats";

vi.mock("../transport/native", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../transport/native")>()),
  callNative: vi.fn(),
}));

describe("stats service", () => {
  it("validates the overview shape", async () => {
    vi.mocked(callNative).mockResolvedValueOnce({ listenList: {}, collection: {}, selection: [] });
    await expect(statsOverview()).resolves.toMatchObject({ selection: [] });
    vi.mocked(callNative).mockResolvedValueOnce({ listenList: {} });
    await expect(statsOverview()).rejects.toMatchObject({ code: "invalid_response" });
  });

  it("computes completion over resolved recommendations only", () => {
    const m = { source: "guided" as const, shown: 5, skipped: 2, completed: 2, pending: 1 };
    expect(completionRate(m)).toBe(0.5);
    expect(completionRate({ ...m, skipped: 0, completed: 0 })).toBeNull();
  });
});
