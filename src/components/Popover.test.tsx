import { act, fireEvent, render, screen } from "@testing-library/react";
import { useRef, useState } from "react";
import { Popover } from "./Popover";

function Harness({ onTick }: { onTick: () => void }) {
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLButtonElement>(null);
  return (
    // Like the app's `main.content`: focusable (tabIndex -1) and containing the popover.
    <main tabIndex={-1} data-testid="main">
      <button
        ref={anchor}
        type="button"
        onClick={() => {
          setOpen(true);
        }}
      >
        Edit tags
      </button>
      <button type="button">Elsewhere</button>
      <Popover
        open={open}
        label="Tags"
        anchorRef={anchor}
        onClose={() => {
          setOpen(false);
        }}
      >
        <input aria-label="Find" />
        <label>
          <input type="checkbox" onChange={onTick} />
          Chill
        </label>
      </Popover>
    </main>
  );
}

describe("Popover", () => {
  it("stays open when a click inside moves focus to the page behind (macOS WebKit)", () => {
    const onTick = vi.fn();
    render(<Harness onTick={onTick} />);
    fireEvent.click(screen.getByRole("button", { name: "Edit tags" }));
    const popover = screen.getByRole("dialog", { name: "Tags" });
    expect(screen.getByRole("textbox", { name: "Find" })).toHaveFocus();

    // WebKit on macOS: mouse-down on a checkbox doesn't focus it; focus moves to the
    // nearest focusable ancestor, here <main>. The popover must not treat that as leaving.
    const box = screen.getByRole("checkbox", { name: "Chill" });
    fireEvent.mouseDown(box);
    act(() => {
      screen.getByTestId("main").focus();
    });
    expect(screen.getByRole("dialog", { name: "Tags" })).toBe(popover);
    fireEvent.click(box);
    expect(onTick).toHaveBeenCalledTimes(1);

    // The popover itself can hold focus (so WebKit stops there), without being a tab stop.
    expect(popover).toHaveAttribute("tabindex", "-1");
  });

  it("still closes when focus really leaves for something else", () => {
    render(<Harness onTick={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Edit tags" }));
    act(() => {
      screen.getByRole("button", { name: "Elsewhere" }).focus();
    });
    expect(screen.queryByRole("dialog", { name: "Tags" })).not.toBeInTheDocument();
  });
});
