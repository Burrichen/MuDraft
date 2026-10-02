import { contrastRatio, parseHex, readableTextOn } from "./color";

describe("tag colour contrast", () => {
  it("picks readable text for any colour", () => {
    // Sample the whole RGB cube coarsely, including the hardest mid-greys.
    for (let r = 0; r <= 255; r += 17)
      for (let g = 0; g <= 255; g += 17)
        for (let b = 0; b <= 255; b += 17) {
          const hex = `#${[r, g, b].map((c) => c.toString(16).padStart(2, "0")).join("")}`;
          expect(contrastRatio(hex, readableTextOn(hex))).toBeGreaterThanOrEqual(4.5);
        }
    expect(readableTextOn("#d4a017")).toBe("#000000");
    expect(readableTextOn("#1d4ed8")).toBe("#ffffff");
  });

  it("parses only #rrggbb", () => {
    expect(parseHex("#10B981")).toEqual([16, 185, 129]);
    expect(parseHex("#abc")).toBeNull();
    expect(parseHex("blue")).toBeNull();
  });
});
