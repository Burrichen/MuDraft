import {
  continuousYears,
  durationParts,
  halfStarBuckets,
  lastTwelveMonths,
  niceMax,
} from "./format";

describe("stats chart data", () => {
  it("shows the last 12 calendar months, zero-filled", () => {
    const months = lastTwelveMonths(
      [
        { key: "2026-08", count: 3 },
        { key: "2025-01", count: 9 },
      ],
      new Date(2026, 9, 10),
    );
    expect(months).toHaveLength(12);
    expect(months[0]?.key).toBe("2025-11");
    expect(months[11]?.key).toBe("2026-10");
    expect(months.find((m) => m.key === "2026-08")?.count).toBe(3);
    expect(months.reduce((n, m) => n + m.count, 0)).toBe(3);
  });

  it("fills gaps between listening years", () => {
    expect(
      continuousYears([
        { key: "2024", count: 2 },
        { key: "2026", count: 5 },
      ]),
    ).toEqual([
      { key: "2024", count: 2 },
      { key: "2025", count: 0 },
      { key: "2026", count: 5 },
    ]);
    expect(continuousYears([])).toEqual([]);
  });

  it("bins exact album scores into half-star buckets", () => {
    const buckets = halfStarBuckets([
      { stars: 0, albums: 1 },
      { stars: 3.75, albums: 2 },
      { stars: 3.5, albums: 1 },
      { stars: 11 / 3, albums: 1 },
      { stars: 5, albums: 4 },
    ]);
    expect(buckets).toHaveLength(11);
    expect(buckets[0]).toEqual({ stars: 0, albums: 1 });
    expect(buckets[7]).toEqual({ stars: 3.5, albums: 4 });
    expect(buckets[10]).toEqual({ stars: 5, albums: 4 });
  });

  it("rounds axis maxima to clean whole numbers", () => {
    expect(niceMax(0)).toBe(1);
    expect(niceMax(3)).toBe(4);
    expect(niceMax(7)).toBe(8);
    expect(niceMax(23)).toBe(25);
    expect(niceMax(100)).toBe(100);
  });

  it("splits durations into number and unit", () => {
    expect(durationParts(600_000)).toEqual([{ value: "10", unit: "min" }]);
    expect(durationParts(3_720_000)).toEqual([
      { value: "1", unit: "h" },
      { value: "2", unit: "min" },
    ]);
  });
});
