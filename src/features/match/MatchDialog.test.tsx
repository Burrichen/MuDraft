import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  loadEditions,
  searchAlbums,
  type EditionList,
  type Fetched,
  type SearchPage,
} from "../../services/metadata";
import { NativeError } from "../../transport/native";
import { MatchDialog, type MatchChoice } from "./MatchDialog";

vi.mock("../../services/metadata", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/metadata")>()),
  searchAlbums: vi.fn(),
  loadEditions: vi.fn(),
}));

const RG = "b1392450-e666-3926-a536-22c65f834433";
const REL = "11111111-1111-4111-8111-111111111111";
const credit = (name: string, disambiguation: string | null = null) => ({
  artistId: `id-${name}`,
  artistName: name,
  sortName: null,
  disambiguation,
  creditedName: null,
  joinPhrase: "",
});

function fetched<T>(value: T, source: Fetched<T>["source"] = "network"): Fetched<T> {
  return { value, fetchedAt: "2026-09-01T10:00:00.000Z", source };
}

const SEARCH: SearchPage = {
  total: 2,
  offset: 0,
  candidates: [
    {
      id: RG,
      title: "OK Computer",
      disambiguation: null,
      primaryType: "Album",
      secondaryTypes: [],
      originalDate: { value: "1997-05-21", precision: "day", year: 1997 },
      artistCredit: [credit("Radiohead")],
      score: 100,
      exactTitle: true,
      yearMatches: null,
    },
    {
      id: "0a1b2c3d-0000-4000-8000-000000000003",
      title: "OK Computer",
      disambiguation: "tribute",
      primaryType: null,
      secondaryTypes: [],
      originalDate: { value: null, precision: "unknown", year: null },
      artistCredit: [credit("Radiohead", "tribute band")],
      score: 80,
      exactTitle: true,
      yearMatches: null,
    },
  ],
};

const EDITIONS: EditionList = {
  total: 1,
  truncated: false,
  editions: [
    {
      id: REL,
      title: "OK Computer",
      disambiguation: null,
      date: { value: "1997-06", precision: "month", year: 1997 },
      country: "GB",
      status: "Official",
      formats: ["CD"],
      trackCount: 12,
    },
  ],
};

function setup(onConfirm = vi.fn<(choice: MatchChoice) => Promise<void>>(() => Promise.resolve())) {
  const onClose = vi.fn();
  const utils = render(<MatchDialog open onClose={onClose} onConfirm={onConfirm} />);
  return { onConfirm, onClose, ...utils };
}

async function search(title = "OK Computer") {
  await userEvent.type(screen.getByLabelText("Album title"), title);
  await userEvent.click(screen.getByRole("button", { name: "Search" }));
}

beforeEach(() => {
  vi.mocked(searchAlbums).mockResolvedValue(fetched(SEARCH));
  vi.mocked(loadEditions).mockResolvedValue(fetched(EDITIONS));
});

describe("MatchDialog", () => {
  it("never pre-selects a candidate, even a perfect match", async () => {
    setup();
    await search();
    const group = await screen.findByRole("radiogroup", { name: "Album" });
    const options = within(group).getAllByRole("radio");
    expect(options).toHaveLength(2);
    for (const o of options) expect(o).not.toBeChecked();
    const add = screen.getByRole("button", { name: "Add album" });
    expect(add).toHaveAttribute("aria-disabled", "true");
    expect(add).toHaveAccessibleDescription("Choose an album from the search results first.");
  });

  it("shows artist credit, year, type, disambiguation, and missing data honestly", async () => {
    setup();
    await search();
    expect(await screen.findByText("1997 · Album")).toBeInTheDocument();
    expect(screen.getByText("Year unknown · Type unknown")).toBeInTheDocument();
    expect(screen.getByText("Radiohead: tribute band")).toBeInTheDocument();
    expect(screen.getByText("Relevance 100")).toBeInTheDocument();
  });

  it("requires an explicit album and edition, then confirms with both IDs", async () => {
    const { onConfirm } = setup();
    await search();
    await userEvent.click(await screen.findByRole("radio", { name: /1997 · Album/ }));
    expect(loadEditions).toHaveBeenCalledWith(RG, expect.anything());
    const add = screen.getByRole("button", { name: "Add album" });
    expect(await screen.findByRole("radiogroup", { name: /Edition of/ })).toBeInTheDocument();
    expect(add).toHaveAccessibleDescription("Choose the edition you have or plan to hear.");

    await userEvent.click(screen.getByRole("radio", { name: /GB · CD · 12 tracks/ }));
    await userEvent.type(screen.getByLabelText("Edition name in MuDraft (optional)"), "UK CD");
    expect(add).not.toHaveAttribute("aria-disabled");
    await userEvent.click(add);
    expect(onConfirm).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: "musicbrainz",
        releaseGroupId: RG,
        releaseId: REL,
        editionName: "UK CD",
      }),
    );
  });

  it("explains offline results served from the cache", async () => {
    vi.mocked(searchAlbums).mockResolvedValue(fetched(SEARCH, "stale_cache"));
    setup();
    await search();
    expect(
      await screen.findByText(/MusicBrainz can’t be reached\. Showing results saved on/),
    ).toBeInTheDocument();
  });

  it("shows rate limiting and other errors with a retry", async () => {
    vi.mocked(searchAlbums)
      .mockRejectedValueOnce(
        new NativeError(
          "rate_limited",
          "MusicBrainz is limiting requests right now; try again shortly",
        ),
      )
      .mockResolvedValueOnce(fetched(SEARCH));
    setup();
    await search();
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("limiting requests");
    expect(alert).toHaveTextContent("rate_limited");
    await userEvent.click(within(alert).getByRole("button", { name: "Try again" }));
    expect(await screen.findByRole("radiogroup", { name: "Album" })).toBeInTheDocument();
  });

  it("cancels a superseded search and ignores its late result", async () => {
    const signals: AbortSignal[] = [];
    let finishFirst: (v: Fetched<SearchPage>) => void = () => undefined;
    vi.mocked(searchAlbums)
      .mockImplementationOnce((_q, opts) => {
        if (opts?.signal) signals.push(opts.signal);
        return new Promise((r) => (finishFirst = r));
      })
      .mockImplementationOnce((_q, opts) => {
        if (opts?.signal) signals.push(opts.signal);
        return Promise.resolve(
          fetched({ ...SEARCH, candidates: SEARCH.candidates.slice(0, 1), total: 1 }),
        );
      });
    setup();
    await search("OK");
    await userEvent.click(screen.getByRole("button", { name: "Search" }));
    expect(signals[0]?.aborted).toBe(true);
    await screen.findByText(/1 matches/);
    finishFirst(fetched(SEARCH));
    await waitFor(() => {
      expect(screen.getAllByRole("radio")).toHaveLength(1);
    });
  });

  it("cancels in-flight work when closed", async () => {
    let signal: AbortSignal | undefined;
    vi.mocked(searchAlbums).mockImplementationOnce((_q, opts) => {
      signal = opts?.signal;
      return new Promise(() => undefined);
    });
    const onConfirm = vi.fn(() => Promise.resolve());
    const { rerender } = render(<MatchDialog open onClose={vi.fn()} onConfirm={onConfirm} />);
    await search();
    rerender(<MatchDialog open={false} onClose={vi.fn()} onConfirm={onConfirm} />);
    expect(signal?.aborted).toBe(true);
  });

  it("validates the year before searching", async () => {
    setup();
    await userEvent.type(screen.getByLabelText("Album title"), "OK Computer");
    await userEvent.type(screen.getByLabelText("Year (optional)"), "97");
    expect(screen.getByText("Use four digits, like 1997.")).toBeInTheDocument();
    const searchButton = screen.getByRole("button", { name: "Search" });
    expect(searchButton).toHaveAccessibleDescription("Fix the year first.");
    await userEvent.click(searchButton);
    expect(searchAlbums).not.toHaveBeenCalled();
  });

  it("supports manual entries without matching", async () => {
    const { onConfirm } = setup();
    await userEvent.click(screen.getByRole("button", { name: "Enter manually" }));
    const add = screen.getByRole("button", { name: "Add without matching" });
    expect(add).toHaveAccessibleDescription("Enter an album title and an artist.");
    await userEvent.type(screen.getByLabelText("Album title"), "Basement Demo");
    await userEvent.type(screen.getByLabelText("Artist"), "Local Band");
    await userEvent.type(screen.getByLabelText("Year (optional)"), "2003");
    await userEvent.click(add);
    expect(onConfirm).toHaveBeenCalledWith({
      kind: "manual",
      album: { title: "Basement Demo", artistName: "Local Band", year: 2003 },
    });
    expect(screen.getByText(/isn’t linked to MusicBrainz or merged/)).toBeInTheDocument();
  });

  it("keeps the dialog open and explains save failures such as name collisions", async () => {
    const onConfirm = vi.fn(() =>
      Promise.reject(
        new NativeError(
          "conflict",
          'This album already has an edition named "Standard"; choose another name',
        ),
      ),
    );
    setup(onConfirm);
    await search();
    await userEvent.click(await screen.findByRole("radio", { name: /1997 · Album/ }));
    await userEvent.click(await screen.findByRole("radio", { name: /12 tracks/ }));
    await userEvent.click(screen.getByRole("button", { name: "Add album" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      'already has an edition named "Standard"',
    );
    expect(screen.getByRole("dialog")).toHaveAttribute("open");
  });
});
