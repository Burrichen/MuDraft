import { invoke } from "@tauri-apps/api/core";
import { callNative, NativeError, toNativeError } from "./native";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

describe("toNativeError", () => {
  it("keeps structured Rust error codes", () => {
    const err = toNativeError({ code: "storage_unavailable", message: "disk full" });
    expect(err).toBeInstanceOf(NativeError);
    expect(err.code).toBe("storage_unavailable");
    expect(err.message).toBe("disk full");
  });

  it("wraps plain strings and unknown values as transport errors", () => {
    expect(toNativeError("boom").code).toBe("transport");
    expect(toNativeError(42).message).toBe("Unknown native error");
  });
});

describe("callNative", () => {
  it("forwards the command name and rethrows NativeError", async () => {
    vi.mocked(invoke).mockRejectedValueOnce({ code: "schema_too_new", message: "newer" });
    await expect(callNative("health_check")).rejects.toMatchObject({ code: "schema_too_new" });
    expect(invoke).toHaveBeenCalledWith("health_check", undefined);
  });
});
