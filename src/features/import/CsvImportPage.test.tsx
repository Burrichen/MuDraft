import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  applyMapping,
  commitImport,
  decideRow,
  discardImport,
  enrichBatch,
  importRows,
  importSummary,
  latestImport,
  openCsv,
  type CommitReport,
  type SessionSummary,
  type StagedRow,
} from "../../services/csvImport";
import { searchAlbums } from "../../services/metadata";
import { renderApp } from "../../test/renderApp";
import { rowStatus } from "./rowStatus";

vi.mock("../../services/csvImport", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/csvImport")>()),
  latestImport: vi.fn(),
  openCsv: vi.fn(),
  applyMapping: vi.fn(),
  importSummary: vi.fn(),
  importRows: vi.fn(),
  enrichBatch: vi.fn(),
  decideRow: vi.fn(),
  commitImport: vi.fn(),
  discardImport: vi.fn(),
  saveTemplate: vi.fn(),
}));
vi.mock("../../services/metadata", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/metadata")>()),
  searchAlbums: vi.fn(() => new Promise(() => undefined)),
}));
vi.mock("../../services/health", () => ({
  checkHealth: vi.fn(() => new Promise(() => undefined)),
}));

const noMapping = {
  album: null,
  artist: null,
  year: null,
  edition: null,
  tags: null,
  releaseGroupId: null,
  releaseId: null,
};
const mapped = { ...noMapping, album: 0, artist: 1, year: 2, tags: 3 };

function summary(over: Partial<SessionSummary> = {}): SessionSummary {
  return {
    sessionId: "0190f5c3-0000-7000-8000-00000000cafe",
    fileName: "albums.csv",
    headers: ["Titel", "Künstler", "Jahr", "Tags"],
    mapping: mapped,
    suggestedMapping: noMapping,
    sample: [["Homogenic", "Björk", "1997", "art pop"]],
    counts: {
      rows: 3,
      ready: 2,
      errors: 1,
      duplicates: 0,
      existing: 0,
      matched: 0,
      manual: 2,
      skipped: 0,
      needsAttention: 0,
      withCandidates: 0,
      lookupPending: 2,
    },
    unknownTags: [],
    status: "open",
    report: null,
    ...over,
  };
}

function row(over: Partial<StagedRow>): StagedRow {
  return {
    rowNumber: 2,
    raw: ["Homogenic", "Björk", "1997", ""],
    fields: {
      album: "Homogenic",
      artist: "Björk",
      year: 1997,
      edition: null,
      tags: [],
      releaseGroupId: null,
      releaseId: null,
    },
    issues: [],
    duplicateOf: null,
    decision: { kind: "manual" },
    enrichment: "not_started",
    candidates: null,
    hasDetails: false,
    readiness: "ready",
    existing: null,
    ...over,
  };
}

const ROWS: StagedRow[] = [
  row({}),
  row({
    rowNumber: 3,
    fields: null,
    raw: ["", "No album", "97", ""],
    issues: [{ field: "album", message: "album is required" }],
    decision: { kind: "skip" },
    readiness: "error",
  }),
];

const REPORT: CommitReport = {
  albumsCreated: 2,
  editionsCreated: 2,
  reusedExisting: 0,
  addedToListenList: 2,
  alreadyOnListenList: 0,
  skipped: 0,
  errors: 1,
  tagsCreated: ["road trip"],
  tagsSkipped: ["café"],
  tagLinksAdded: 1,
  rows: [{ rowNumber: 3, outcome: "error", albumId: null, detail: "album is required" }],
};

beforeEach(() => {
  vi.mocked(latestImport).mockResolvedValue(null);
  vi.mocked(importRows).mockResolvedValue({ rows: ROWS, total: 2 });
  vi.mocked(importSummary).mockResolvedValue(summary());
  vi.mocked(decideRow).mockResolvedValue(row({ decision: { kind: "skip" }, readiness: "skipped" }));
});

describe("CSV import page", () => {
  it("maps columns before showing anything else, requiring album and artist", async () => {
    vi.mocked(openCsv).mockResolvedValue(summary({ mapping: null }));
    vi.mocked(applyMapping).mockResolvedValue(summary());
    renderApp("/listen-list/import");
    await userEvent.click(await screen.findByRole("button", { name: "Choose CSV file…" }));
    const use = await screen.findByRole("button", { name: "Use these columns" });
    expect(use).toHaveAccessibleDescription("Choose the Album and Artist columns.");
    expect(screen.getByRole("table")).toHaveTextContent("Björk");

    await userEvent.selectOptions(screen.getByLabelText("Album (required)"), "Titel");
    await userEvent.selectOptions(screen.getByLabelText("Artist (required)"), "Künstler");
    await userEvent.selectOptions(screen.getByLabelText("Year"), "Titel");
    expect(use).toHaveAccessibleDescription("Each column can be used for one field only.");
    await userEvent.selectOptions(screen.getByLabelText("Year"), "Jahr");
    await userEvent.click(use);
    expect(applyMapping).toHaveBeenCalledWith(
      "0190f5c3-0000-7000-8000-00000000cafe",
      expect.objectContaining({ album: 0, artist: 1, year: 2 }),
    );
    expect(
      await screen.findByRole("heading", { name: /Preview · albums.csv/ }),
    ).toBeInTheDocument();
  });

  it("previews rows with plain-language problems before any change", async () => {
    vi.mocked(latestImport).mockResolvedValue(summary());
    renderApp("/listen-list/import");
    const table = await screen.findByRole("table");
    expect(within(table).getByText("album is required")).toBeInTheDocument();
    expect(screen.getByText(/Nothing has been added yet/)).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Change match for row 3" }),
    ).not.toBeInTheDocument();
    expect(commitImport).not.toHaveBeenCalled();
  });

  it("blocks import while rows need a lookup, and explains why", async () => {
    vi.mocked(latestImport).mockResolvedValue(
      summary({ counts: { ...summary().counts, needsAttention: 2 } }),
    );
    renderApp("/listen-list/import");
    const importButton = await screen.findByRole("button", { name: /Import 2 rows/ });
    expect(importButton).toHaveAccessibleDescription(
      /2 rows need a MusicBrainz lookup or an edition choice/,
    );
  });

  it("runs bounded lookups until stopped offline, and says importing still works", async () => {
    vi.mocked(latestImport).mockResolvedValue(summary());
    vi.mocked(enrichBatch)
      .mockResolvedValueOnce({ processed: 10, remaining: 5, stopped: null })
      .mockResolvedValueOnce({
        processed: 0,
        remaining: 5,
        stopped: { code: "offline", message: "Can't reach MusicBrainz" },
      });
    renderApp("/listen-list/import");
    await userEvent.click(await screen.findByRole("button", { name: "Look up 2 rows" }));
    expect(
      await screen.findByText(/You’re offline\. Lookups are paused; you can still import now/),
    ).toBeInTheDocument();
    expect(enrichBatch).toHaveBeenCalledTimes(2);
    expect(enrichBatch).toHaveBeenCalledWith(
      "0190f5c3-0000-7000-8000-00000000cafe",
      10,
      expect.any(AbortSignal),
    );
    expect(screen.getByRole("button", { name: "Resume lookups" })).toBeInTheDocument();
  });

  it("pauses lookups by cancelling the batch", async () => {
    vi.mocked(latestImport).mockResolvedValue(summary());
    let signal: AbortSignal | undefined;
    vi.mocked(enrichBatch).mockImplementationOnce((_s, _n, sig) => {
      signal = sig;
      return new Promise((resolve) => {
        sig?.addEventListener("abort", () => {
          resolve({
            processed: 0,
            remaining: 2,
            stopped: { code: "cancelled", message: "Request cancelled" },
          });
        });
      });
    });
    renderApp("/listen-list/import");
    await userEvent.click(await screen.findByRole("button", { name: "Look up 2 rows" }));
    const importButton = screen.getByRole("button", { name: /Import 2 rows/ });
    expect(importButton).toHaveAccessibleDescription("Pause the MusicBrainz lookups first.");
    await userEvent.click(screen.getByRole("button", { name: "Pause lookups" }));
    expect(signal?.aborted).toBe(true);
    expect(await screen.findByText(/Lookups paused\. Resume any time/)).toBeInTheDocument();
  });

  it("offers skip, keep as manual, and change match per row", async () => {
    vi.mocked(latestImport).mockResolvedValue(summary());
    vi.mocked(importRows).mockResolvedValue({
      rows: [
        row({
          decision: { kind: "use_existing", albumId: "a", editionId: "e", reason: "same_name" },
          existing: { albumTitle: "Homogenic", editionName: "Standard" },
        }),
      ],
      total: 1,
    });
    renderApp("/listen-list/import");
    expect(
      await screen.findByText("Possible duplicate of “Homogenic” (Standard) — check"),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Keep row 2 as manual" }));
    expect(decideRow).toHaveBeenCalledWith("0190f5c3-0000-7000-8000-00000000cafe", 2, {
      kind: "manual",
    });
    await userEvent.click(screen.getByRole("button", { name: "Skip row 2" }));
    expect(decideRow).toHaveBeenLastCalledWith("0190f5c3-0000-7000-8000-00000000cafe", 2, {
      kind: "skip",
    });

    await userEvent.click(screen.getByRole("button", { name: "Change match for row 2" }));
    expect(screen.getByRole("dialog", { name: "Match row 2" })).toHaveAttribute("open");
    await waitFor(() => {
      expect(searchAlbums).toHaveBeenCalledWith(
        { title: "Homogenic", artist: "Björk", year: 1997 },
        expect.anything(),
      );
    });
  });

  it("requires explicit confirmation of new tags and shows the report", async () => {
    vi.mocked(latestImport).mockResolvedValue(
      summary({
        unknownTags: [
          { name: "road trip", rows: 2 },
          { name: "café", rows: 1 },
        ],
      }),
    );
    vi.mocked(commitImport).mockResolvedValue(REPORT);
    renderApp("/listen-list/import");
    const roadTrip = await screen.findByRole("checkbox", { name: /road trip/ });
    expect(roadTrip).not.toBeChecked();
    await userEvent.click(roadTrip);
    await userEvent.click(screen.getByRole("button", { name: /Import 2 rows/ }));
    expect(commitImport).toHaveBeenCalledWith("0190f5c3-0000-7000-8000-00000000cafe", {
      create: ["road trip"],
      skip: ["café"],
    });
    expect(await screen.findByRole("heading", { name: "Import complete" })).toBeInTheDocument();
    expect(screen.getByText("New tags: road trip")).toBeInTheDocument();
    expect(screen.getByRole("table")).toHaveTextContent("album is required");
  });

  it("shows commit errors without leaving the review", async () => {
    vi.mocked(latestImport).mockResolvedValue(summary());
    vi.mocked(commitImport).mockRejectedValue(new Error("row 3: disk full"));
    renderApp("/listen-list/import");
    await userEvent.click(await screen.findByRole("button", { name: /Import 2 rows/ }));
    expect(await screen.findByRole("alert")).toHaveTextContent("row 3: disk full");
    expect(screen.getByRole("heading", { name: /Preview/ })).toBeInTheDocument();
  });

  it("discards only after confirmation", async () => {
    vi.mocked(latestImport).mockResolvedValue(summary());
    vi.mocked(discardImport).mockResolvedValue();
    renderApp("/listen-list/import");
    await userEvent.click(await screen.findByRole("button", { name: "Discard import" }));
    expect(discardImport).not.toHaveBeenCalled();
    await userEvent.click(
      within(screen.getByRole("dialog", { name: "Discard this import?" })).getByRole("button", {
        name: "Discard",
      }),
    );
    expect(discardImport).toHaveBeenCalledWith("0190f5c3-0000-7000-8000-00000000cafe");
    expect(await screen.findByText("Import discarded. Nothing was added.")).toBeInTheDocument();
  });
});

describe("rowStatus", () => {
  it("describes every state", () => {
    expect(
      rowStatus(row({ readiness: "skipped", duplicateOf: 2, decision: { kind: "skip" } })).label,
    ).toBe("Duplicate of row 2");
    expect(rowStatus(row({ readiness: "needs_edition" })).label).toBe("Choose an edition");
    expect(
      rowStatus(
        row({ readiness: "needs_lookup", candidates: { kind: "failed", message: "No such ID." } }),
      ).label,
    ).toBe("No such ID.");
    expect(
      rowStatus(
        row({
          decision: {
            kind: "match",
            releaseGroupId: "g",
            releaseId: "r",
            editionName: null,
            chosenBy: "csv_ids",
          },
        }),
      ).label,
    ).toBe("Matched by ID");
    expect(rowStatus(row({ candidates: { kind: "groups", candidates: [] } })).label).toBe(
      "Manual entry",
    );
  });
});
