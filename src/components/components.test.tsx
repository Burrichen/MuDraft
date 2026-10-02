import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { MemoryRouter } from "react-router";
import { FIXTURE_ALBUMS } from "../test/fixtures/albums";
import { AlbumCollection } from "./AlbumCard";
import { Button } from "./Button";
import { Dialog } from "./Dialog";
import { toggleFilter } from "./filterSelection";
import { FilterGroup } from "./Filters";
import { ProgressBar } from "./ProgressBar";
import { RatingInput } from "./Rating";
import { formatRating } from "./ratingFormat";
import { EmptyState, ErrorState, LoadingState } from "./States";
import { Tooltip } from "./Tooltip";

describe("AlbumCollection", () => {
  const renderAlbums = (layout: "grid" | "list") =>
    render(
      <MemoryRouter initialEntries={["/collection"]}>
        <AlbumCollection
          albums={FIXTURE_ALBUMS}
          layout={layout}
          label="Collection"
          empty={<p>empty</p>}
        />
      </MemoryRouter>,
    );

  it.each(["grid", "list"] as const)("renders every album in %s layout", (layout) => {
    renderAlbums(layout);
    const list = screen.getByRole("list", { name: "Collection" });
    expect(list).toHaveAttribute("data-layout", layout);
    expect(screen.getAllByRole("article")).toHaveLength(3);
  });

  it("keeps long names complete for assistive tech and links albums and artists", () => {
    renderAlbums("grid");
    const long = FIXTURE_ALBUMS[1];
    if (!long) throw new Error("fixture missing");
    const link = screen.getByRole("link", { name: long.title });
    expect(link).toHaveAttribute("href", `/albums/${long.albumId}`);
    expect(link.closest("h3")).toHaveAttribute("title", long.title);
    expect(screen.getByRole("link", { name: "Second Fixture Artist" })).toHaveAttribute(
      "href",
      "/artists/0190f5c3-0000-7000-8000-0000000000a3",
    );
    expect(screen.getByText("Year unknown · Deluxe")).toBeInTheDocument();
    expect(screen.getByText("Unknown artist")).toBeInTheDocument();
  });

  it("distinguishes a zero rating from unrated", () => {
    renderAlbums("list");
    expect(screen.getByText("0 of 5 stars")).toBeInTheDocument();
    expect(screen.getByText("3.5 of 5 stars")).toBeInTheDocument();
    expect(screen.getByText("Unrated")).toBeInTheDocument();
  });

  it("shows square, uncropped artwork and falls back to a placeholder", () => {
    renderAlbums("grid");
    const img = document.querySelector<HTMLImageElement>(".artwork-image");
    if (!img) throw new Error("no image");
    expect(img.alt).toBe("");
    expect(screen.getAllByTestId("artwork-placeholder")).toHaveLength(2);
    fireEvent.error(img);
    expect(screen.getAllByTestId("artwork-placeholder")).toHaveLength(3);
  });

  it("renders the empty state when there are no albums", () => {
    render(
      <MemoryRouter>
        <AlbumCollection
          albums={[]}
          layout="grid"
          label="x"
          empty={<EmptyState title="Nothing here" />}
        />
      </MemoryRouter>,
    );
    expect(screen.getByRole("heading", { name: "Nothing here" })).toBeInTheDocument();
    expect(screen.queryByRole("list")).not.toBeInTheDocument();
  });
});

describe("Button", () => {
  it("explains a disabled action with visible, described text and blocks clicks", async () => {
    const onClick = vi.fn();
    render(
      <Button onClick={onClick} disabledReason="Add albums first.">
        Pick
      </Button>,
    );
    const button = screen.getByRole("button", { name: "Pick" });
    expect(button).toHaveAttribute("aria-disabled", "true");
    expect(button).toHaveAccessibleDescription("Add albums first.");
    expect(screen.getByText("Add albums first.")).toBeVisible();
    // Still reachable by keyboard so the reason can be discovered.
    await userEvent.tab();
    expect(button).toHaveFocus();
    await userEvent.click(button);
    await userEvent.keyboard("{Enter}");
    expect(onClick).not.toHaveBeenCalled();
  });

  it("keeps the same element and focus when availability changes", () => {
    const { rerender } = render(<Button disabledReason="Not yet.">Go</Button>);
    const button = screen.getByRole("button", { name: "Go" });
    button.focus();
    rerender(<Button>Go</Button>);
    expect(screen.getByRole("button", { name: "Go" })).toBe(button);
    expect(button).toHaveFocus();
    expect(button).not.toHaveAttribute("aria-disabled");
  });

  it("works normally when available", async () => {
    const onClick = vi.fn();
    render(<Button onClick={onClick}>Go</Button>);
    await userEvent.click(screen.getByRole("button", { name: "Go" }));
    expect(onClick).toHaveBeenCalledOnce();
  });
});

describe("filters", () => {
  it("keeps the exclusive option exclusive and as the empty fallback", () => {
    expect(toggleFilter(["agnostic"], "1990s", "agnostic")).toEqual(["1990s"]);
    expect(toggleFilter(["1990s"], "2000s", "agnostic")).toEqual(["1990s", "2000s"]);
    expect(toggleFilter(["1990s", "2000s"], "agnostic", "agnostic")).toEqual(["agnostic"]);
    expect(toggleFilter(["1990s"], "1990s", "agnostic")).toEqual(["agnostic"]);
    expect(toggleFilter(["a"], "a")).toEqual([]);
  });

  it("exposes pressed state on labelled bubbles", async () => {
    function Harness() {
      const [selected, setSelected] = useState(["agnostic"]);
      return (
        <FilterGroup
          label="Decade"
          exclusive="agnostic"
          selected={selected}
          onChange={setSelected}
          options={[
            { value: "agnostic", label: "Agnostic" },
            { value: "1990s", label: "1990s", count: 4 },
          ]}
        />
      );
    }
    render(<Harness />);
    expect(screen.getByRole("group", { name: "Decade" })).toBeInTheDocument();
    const nineties = screen.getByRole("button", { name: /1990s/ });
    await userEvent.click(nineties);
    expect(nineties).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: "Agnostic" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
  });
});

describe("ProgressBar", () => {
  it("reports counts and percentage", () => {
    render(<ProgressBar label="Albums heard" value={3} max={8} />);
    const bar = screen.getByRole("progressbar", { name: "Albums heard" });
    expect(bar).toHaveAttribute("aria-valuetext", "3 of 8 (38%)");
    expect(screen.getByText("3 / 8 · 38%")).toBeInTheDocument();
  });

  it("handles an empty catalogue and clamps", () => {
    render(<ProgressBar label="Tracks" value={5} max={0} />);
    expect(screen.getByRole("progressbar")).toHaveAttribute("aria-valuetext", "0 of 0 (0%)");
  });
});

describe("RatingInput", () => {
  function Harness({
    initial = null as number | null,
    disabledReason,
  }: {
    initial?: number | null;
    disabledReason?: string;
  }) {
    const [value, setValue] = useState<number | null>(initial);
    return (
      <>
        <RatingInput
          label="Album rating"
          value={value}
          onChange={setValue}
          {...(disabledReason ? { disabledReason } : {})}
        />
        <output data-testid="value">{value === null ? "null" : String(value)}</output>
      </>
    );
  }

  it("is a labelled slider that starts unrated", () => {
    render(<Harness />);
    const slider = screen.getByRole("slider", { name: "Album rating" });
    expect(slider).toHaveAttribute("aria-valuetext", "Unrated");
    expect(slider).not.toHaveAttribute("aria-valuenow");
    expect(slider).toHaveAccessibleDescription(/Arrow keys change by half a star/);
  });

  it("supports half-star keyboard control, zero, and clearing", async () => {
    render(<Harness />);
    const slider = screen.getByRole("slider");
    slider.focus();
    await userEvent.keyboard("{Home}");
    expect(screen.getByTestId("value")).toHaveTextContent("0");
    expect(slider).toHaveAttribute("aria-valuetext", "0 of 5 stars");
    await userEvent.keyboard("{ArrowRight}{ArrowRight}{ArrowRight}");
    expect(slider).toHaveAttribute("aria-valuenow", "1.5");
    await userEvent.keyboard("{End}{ArrowRight}");
    expect(screen.getByTestId("value")).toHaveTextContent("10");
    await userEvent.keyboard("{PageDown}");
    expect(screen.getByTestId("value")).toHaveTextContent("8");
    await userEvent.keyboard("{Delete}");
    expect(screen.getByTestId("value")).toHaveTextContent("null");
  });

  it("sets half stars by pointer and clears with a labelled button", async () => {
    render(<Harness initial={4} />);
    await userEvent.click(screen.getByTestId("rating-half-7"));
    expect(screen.getByTestId("value")).toHaveTextContent("7");
    await userEvent.click(screen.getByRole("button", { name: "Clear Album rating" }));
    expect(screen.getByTestId("value")).toHaveTextContent("null");
    expect(screen.queryByRole("button", { name: /Clear/ })).not.toBeInTheDocument();
  });

  it("offers zero as its own control, distinct from clearing", async () => {
    render(<Harness initial={7} />);
    const zero = screen.getByRole("button", { name: "Set Album rating to 0 stars" });
    expect(zero).toHaveAttribute("aria-pressed", "false");
    await userEvent.click(zero);
    expect(screen.getByTestId("value")).toHaveTextContent("0");
    expect(zero).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("slider")).toHaveAttribute("aria-valuetext", "0 of 5 stars");
    await userEvent.click(screen.getByRole("button", { name: "Clear Album rating" }));
    expect(screen.getByTestId("value")).toHaveTextContent("null");
    expect(zero).toHaveAttribute("aria-pressed", "false");
  });

  it("explains why it is unavailable and ignores input", async () => {
    render(<Harness initial={6} disabledReason="Log a listen before rating." />);
    const slider = screen.getByRole("slider");
    expect(slider).toHaveAttribute("aria-disabled", "true");
    expect(slider).toHaveAccessibleDescription(/Log a listen before rating\./);
    expect(screen.getByText("Log a listen before rating.")).toBeVisible();
    slider.focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(screen.getByTestId("value")).toHaveTextContent("6");
  });

  it("formats ratings", () => {
    expect(formatRating(null)).toBe("Unrated");
    expect(formatRating(0)).toBe("0 of 5 stars");
    expect(formatRating(9)).toBe("4.5 of 5 stars");
  });
});

describe("Tooltip", () => {
  it("shows on focus and hover, describes the trigger, and closes on Escape", async () => {
    render(
      <Tooltip content="Expand sidebar">
        <button type="button" aria-label="Expand">
          »
        </button>
      </Tooltip>,
    );
    const trigger = screen.getByRole("button", { name: "Expand" });
    expect(screen.getByRole("tooltip", { hidden: true })).not.toBeVisible();
    await userEvent.tab();
    expect(screen.getByRole("tooltip")).toBeVisible();
    expect(trigger).toHaveAccessibleDescription("Expand sidebar");
    await userEvent.keyboard("{Escape}");
    expect(screen.getByRole("tooltip", { hidden: true })).not.toBeVisible();
    await userEvent.hover(trigger);
    expect(screen.getByRole("tooltip")).toBeVisible();
    await userEvent.unhover(trigger);
    expect(screen.getByRole("tooltip", { hidden: true })).not.toBeVisible();
  });
});

describe("Dialog", () => {
  it("opens modally, closes on Escape and the close button, and restores focus", async () => {
    function Harness() {
      const [open, setOpen] = useState(false);
      return (
        <>
          <button
            type="button"
            onClick={() => {
              setOpen(true);
            }}
          >
            Open
          </button>
          <Dialog
            open={open}
            title="Remove album?"
            onClose={() => {
              setOpen(false);
            }}
          >
            This cannot be undone.
          </Dialog>
        </>
      );
    }
    render(<Harness />);
    const opener = screen.getByRole("button", { name: "Open" });
    await userEvent.click(opener);
    const dialog = screen.getByRole("dialog", { name: "Remove album?" });
    expect(dialog).toHaveAttribute("open");
    await userEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(dialog).not.toHaveAttribute("open");
    expect(opener).toHaveFocus();

    await userEvent.click(opener);
    fireEvent(dialog, new Event("cancel", { cancelable: true }));
    expect(dialog).not.toHaveAttribute("open");
  });
});

describe("states", () => {
  it("announces loading and errors", async () => {
    const onRetry = vi.fn();
    render(
      <>
        <LoadingState label="Loading albums…" />
        <ErrorState
          title="Couldn't load"
          message="Disk unavailable"
          code="storage_unavailable"
          onRetry={onRetry}
        />
      </>,
    );
    expect(screen.getByRole("status")).toHaveTextContent("Loading albums…");
    expect(screen.getByRole("alert")).toHaveTextContent("Error code: storage_unavailable");
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(onRetry).toHaveBeenCalledOnce();
  });
});
