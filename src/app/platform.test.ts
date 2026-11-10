import { navItemsFor } from "./navigation";
import { DEFAULT_PREFERENCES, startPagesFor, startRoute } from "../services/preferences";

describe("Android build without Stats", () => {
  it("keeps Stats on the desktop and swaps it for Liked on Android", () => {
    expect(navItemsFor(true).map((n) => n.path)).toEqual([
      "/listen-list",
      "/next-up",
      "/collection",
      "/stats",
      "/settings",
    ]);
    const android = navItemsFor(false).map((n) => n.path);
    expect(android).not.toContain("/stats");
    expect(android).toContain("/liked");
    expect(android).toHaveLength(5);
  });

  it("never opens or offers Stats without it, but leaves stored values alone", () => {
    const stored = { ...DEFAULT_PREFERENCES, startPage: "/stats" as const };
    expect(startRoute(stored, true)).toBe("/stats");
    expect(startRoute(stored, false)).toBe("/listen-list");
    expect(startRoute({ ...DEFAULT_PREFERENCES, lastRoute: "/stats" }, false)).toBe("/listen-list");
    expect(startRoute({ ...DEFAULT_PREFERENCES, lastRoute: "/stats" }, true)).toBe("/stats");
    expect(startPagesFor(true)).toContain("/stats");
    expect(startPagesFor(false)).not.toContain("/stats");
  });
});
