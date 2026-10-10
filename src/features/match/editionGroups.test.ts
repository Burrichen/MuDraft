import type { EditionCandidate } from "../../services/metadata";
import { collapseEditions, editionLabel } from "./editionGroups";

let n = 0;
function rel(over: Partial<EditionCandidate>): EditionCandidate {
  n += 1;
  return {
    id: `id-${String(n)}`,
    title: "Blonde",
    disambiguation: null,
    date: { value: "2016-08-20", precision: "day", year: 2016 },
    country: "XW",
    status: "Official",
    formats: ["Digital Media"],
    trackCount: 17,
    ...over,
  };
}

describe("editionLabel", () => {
  it("drops format words but keeps the edition", () => {
    expect(editionLabel("Deluxe, 2LP 180g vinyl")).toBe("Deluxe");
    expect(editionLabel("digital")).toBe("");
    expect(editionLabel("Expanded Edition")).toBe("Expanded Edition");
    expect(editionLabel(null)).toBe("");
  });
});

describe("collapseEditions", () => {
  it("shows one entry for the same edition on CD, vinyl and digital", () => {
    const digital = rel({ date: { value: "2016-08-20", precision: "day", year: 2016 } });
    const out = collapseEditions([
      rel({
        formats: ["CD"],
        country: "GB",
        date: { value: "2016-08-20", precision: "day", year: 2016 },
      }),
      rel({ formats: ['12" Vinyl', '12" Vinyl'], disambiguation: "vinyl", country: "US" }),
      digital,
      rel({ formats: ["Cassette"] }),
    ]);
    expect(out).toEqual([digital]);
  });

  it("keeps editions with different content apart", () => {
    const out = collapseEditions([
      rel({}),
      rel({ disambiguation: "Deluxe edition", trackCount: 23 }),
      rel({ disambiguation: "deluxe, vinyl", formats: ["Vinyl"], trackCount: 23 }),
    ]);
    expect(out.map((e) => e.trackCount)).toEqual([17, 23]);
  });

  it("drops bootlegs when an official release exists, and keeps preferred entries", () => {
    const owned = rel({ formats: ["CD"] });
    const out = collapseEditions(
      [rel({}), owned, rel({ status: "Bootleg", trackCount: 9 })],
      (e) => e === owned,
    );
    expect(out).toEqual([owned]);
  });
});
