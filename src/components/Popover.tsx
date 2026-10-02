import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";

interface Props {
  open: boolean;
  onClose: () => void;
  /** The button that opened it: used for placement and to return focus. */
  anchorRef: RefObject<HTMLElement | null>;
  label: string;
  children: ReactNode;
}

/**
 * Non-modal popover anchored to a button. Moves focus inside on open; Escape, an outside
 * click, or focus leaving closes it and returns focus to the anchor. Positioned in the
 * viewport (fixed) so it is never clipped by cards or small windows.
 */
export function Popover({ open, onClose, anchorRef, label, children }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);

  useLayoutEffect(() => {
    if (!open) return;
    const place = () => {
      const anchor = anchorRef.current?.getBoundingClientRect();
      const box = ref.current?.getBoundingClientRect();
      if (!anchor || !box) return;
      const margin = 8;
      const left = Math.min(Math.max(margin, anchor.left), window.innerWidth - box.width - margin);
      const below = anchor.bottom + 4;
      const top =
        below + box.height > window.innerHeight - margin && anchor.top - box.height - 4 > margin
          ? anchor.top - box.height - 4
          : Math.min(below, Math.max(margin, window.innerHeight - box.height - margin));
      setPos({ top, left: Math.max(margin, left) });
    };
    place();
    // Content can grow after opening (e.g. tags finish loading): keep it in view.
    const observer = typeof ResizeObserver === "function" ? new ResizeObserver(place) : null;
    if (ref.current) observer?.observe(ref.current);
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      observer?.disconnect();
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [open, anchorRef]);

  useEffect(() => {
    if (!open) return;
    const popover = ref.current;
    popover?.querySelector<HTMLElement>("input, button, [tabindex='0']")?.focus();
    const close = () => {
      onClose();
      anchorRef.current?.focus();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        close();
      }
    };
    const onPointer = (e: MouseEvent) => {
      const t = e.target as Node;
      if (!popover?.contains(t) && !anchorRef.current?.contains(t)) onClose();
    };
    const onFocus = (e: FocusEvent) => {
      const t = e.target as Node;
      if (!popover?.contains(t) && !anchorRef.current?.contains(t)) onClose();
    };
    document.addEventListener("keydown", onKey, true);
    document.addEventListener("mousedown", onPointer);
    document.addEventListener("focusin", onFocus);
    return () => {
      document.removeEventListener("keydown", onKey, true);
      document.removeEventListener("mousedown", onPointer);
      document.removeEventListener("focusin", onFocus);
    };
  }, [open, onClose, anchorRef]);

  if (!open) return null;
  return (
    <div
      ref={ref}
      className="popover"
      role="dialog"
      aria-label={label}
      // Transparent (not hidden) while measuring, so focus can move inside immediately.
      style={pos ? { top: pos.top, left: pos.left } : { opacity: 0, top: 0, left: 0 }}
    >
      {children}
    </div>
  );
}
