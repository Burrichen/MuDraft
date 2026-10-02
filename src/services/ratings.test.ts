import { describeRating, type RatingSummary } from "./ratings";

const base: RatingSummary = {
  effective: null,
  source: "unrated",
  explicit: null,
  calculated: null,
  ratedTracks: 0,
  totalTracks: 12,
};

describe("describeRating", () => {
  it("labels derived, explicit, and unrated ratings", () => {
    expect(
      describeRating({
        ...base,
        effective: 7,
        source: "calculated",
        calculated: 7,
        ratedTracks: 3,
      }),
    ).toBe("Calculated from 3/12 rated tracks");
    expect(
      describeRating({
        ...base,
        effective: 0,
        source: "explicit",
        explicit: 0,
        calculated: 7,
        ratedTracks: 3,
      }),
    ).toBe("Your rating (tracks alone would give 3.5 from 3/12 rated tracks)");
    expect(describeRating({ ...base, effective: 0, source: "explicit", explicit: 0 })).toBe(
      "Your rating",
    );
    expect(describeRating(base)).toBe("Not rated · 0/12 rated tracks");
    expect(describeRating({ ...base, totalTracks: 0 })).toBe("Not rated");
  });
});
