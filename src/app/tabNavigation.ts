/**
 * Deterministic Tab order. macOS WebKit (WKWebView) skips links on Tab unless the user
 * enables "Press Tab to highlight each item", which would leave MuDraft's navigation and
 * album cards unreachable by keyboard. This moves focus through every tabbable element —
 * links included — in DOM order, which is also what Chromium/WebView2 does by default, so
 * behaviour is the same on Windows and macOS. Open modal dialogs keep their focus trap.
 */

const CANDIDATES = [
  "a[href]",
  "button",
  "input:not([type='hidden'])",
  "select",
  "textarea",
  "summary",
  "[tabindex]",
  "[contenteditable='true']",
].join(",");

function isHidden(el: Element): boolean {
  for (let node: Element | null = el; node; node = node.parentElement) {
    if (node.hasAttribute("hidden") || node.hasAttribute("inert")) return true;
    if (node instanceof HTMLDialogElement && !node.open) return true;
    const style = getComputedStyle(node);
    if (style.display === "none" || style.visibility === "hidden") return true;
  }
  return false;
}

export function tabbableElements(root: ParentNode): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(CANDIDATES)).filter(
    (el) => el.tabIndex >= 0 && !(el as HTMLButtonElement).disabled && !isHidden(el),
  );
}

/**
 * The next element Tab (or Shift+Tab) should focus, wrapping at the ends. When focus is
 * on a non-stop (e.g. the page heading focused after navigation), continue from its
 * position in the document, as browsers do.
 */
export function nextTabStop(
  root: ParentNode,
  current: Element | null,
  backwards: boolean,
): HTMLElement | null {
  const stops = tabbableElements(root);
  if (stops.length === 0) return null;
  const first = stops[0] ?? null;
  const last = stops.at(-1) ?? null;
  const index = current instanceof HTMLElement ? stops.indexOf(current) : -1;
  if (index !== -1) {
    const next = (index + (backwards ? -1 : 1) + stops.length) % stops.length;
    return stops[next] ?? null;
  }
  if (!current || current === current.ownerDocument.body) return backwards ? last : first;
  const follows = (el: HTMLElement) =>
    (current.compareDocumentPosition(el) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0;
  if (backwards) return stops.filter((el) => !follows(el)).at(-1) ?? last;
  return stops.find(follows) ?? first;
}

export function installTabNavigation(doc: Document = document): () => void {
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key !== "Tab" || e.altKey || e.ctrlKey || e.metaKey || e.defaultPrevented) return;
    const modal = doc.querySelector<HTMLDialogElement>("dialog[open]");
    const target = nextTabStop(modal ?? doc, doc.activeElement, e.shiftKey);
    if (!target) return;
    e.preventDefault();
    target.focus();
  };
  doc.addEventListener("keydown", onKeyDown);
  return () => {
    doc.removeEventListener("keydown", onKeyDown);
  };
}
