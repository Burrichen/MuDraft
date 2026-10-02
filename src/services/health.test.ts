import { callNative } from "../transport/native";
import { checkHealth } from "./health";

vi.mock("../transport/native", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../transport/native")>()),
  callNative: vi.fn(),
}));

const valid = {
  appVersion: "0.1.0",
  profile: "development",
  dataDir: "/tmp/mudraft",
  schemaVersion: 1,
  sqliteVersion: "3.50.0",
};

describe("checkHealth", () => {
  it("returns a validated report", async () => {
    vi.mocked(callNative).mockResolvedValueOnce(valid);
    await expect(checkHealth()).resolves.toEqual(valid);
  });

  it("rejects malformed responses instead of trusting them", async () => {
    vi.mocked(callNative).mockResolvedValueOnce({ ...valid, profile: "staging" });
    await expect(checkHealth()).rejects.toMatchObject({ code: "invalid_response" });
  });
});
