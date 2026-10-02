import { installTabNavigation, nextTabStop, tabbableElements } from "./tabNavigation";

function setup(html: string) {
  document.body.innerHTML = html;
  return installTabNavigation(document);
}

function tab(shift = false) {
  const event = new KeyboardEvent("keydown", {
    key: "Tab",
    shiftKey: shift,
    bubbles: true,
    cancelable: true,
  });
  (document.activeElement ?? document.body).dispatchEvent(event);
  return event;
}

const PAGE = `
  <a href="#/skip" id="skip">Skip</a>
  <button id="toggle">Toggle</button>
  <nav><a href="#/listen-list" id="l1">Listen List</a><a href="#/stats" id="l2">Stats</a></nav>
  <button disabled id="off">Off</button>
  <div hidden><button id="hid">Hidden</button></div>
  <div inert><a href="#/x" id="inert">Inert</a></div>
  <div role="slider" tabindex="0" id="slider"></div>
  <main tabindex="-1" id="main"><h1 tabindex="-1">Title</h1><button id="after">After</button></main>
  <dialog id="dlg"><button id="in-dialog">Close</button></dialog>
`;

describe("tab navigation", () => {
  let cleanup: () => void;
  afterEach(() => {
    cleanup();
    document.body.innerHTML = "";
  });

  it("includes links and skips disabled, hidden, inert, negative-tabindex, and closed-dialog content", () => {
    cleanup = setup(PAGE);
    expect(tabbableElements(document).map((e) => e.id)).toEqual([
      "skip",
      "toggle",
      "l1",
      "l2",
      "slider",
      "after",
    ]);
  });

  it("moves forward and backward through links in DOM order, wrapping", () => {
    cleanup = setup(PAGE);
    const ids: string[] = [];
    for (let i = 0; i < 6; i++) {
      expect(tab().defaultPrevented).toBe(true);
      ids.push(document.activeElement?.id ?? "");
    }
    expect(ids).toEqual(["skip", "toggle", "l1", "l2", "slider", "after"]);
    tab();
    expect(document.activeElement?.id).toBe("skip");
    tab(true);
    expect(document.activeElement?.id).toBe("after");
  });

  it("continues from a programmatically focused heading", () => {
    cleanup = setup(PAGE);
    document.querySelector<HTMLElement>("h1")?.focus();
    // The heading is not a stop: Tab continues after it, Shift+Tab before it.
    expect(nextTabStop(document, document.activeElement, false)?.id).toBe("after");
    expect(nextTabStop(document, document.activeElement, true)?.id).toBe("slider");
  });

  it("keeps focus inside an open modal dialog", () => {
    cleanup = setup(PAGE);
    const dialog = document.getElementById("dlg") as HTMLDialogElement;
    dialog.showModal();
    tab();
    expect(document.activeElement?.id).toBe("in-dialog");
    tab();
    expect(document.activeElement?.id).toBe("in-dialog");
  });

  it("leaves modified Tab alone", () => {
    cleanup = setup(PAGE);
    const event = new KeyboardEvent("keydown", {
      key: "Tab",
      ctrlKey: true,
      bubbles: true,
      cancelable: true,
    });
    document.body.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(false);
  });
});
