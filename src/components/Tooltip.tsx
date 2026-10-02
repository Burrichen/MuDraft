import {
  cloneElement,
  useEffect,
  useId,
  useState,
  type FocusEvent,
  type MouseEvent,
  type ReactElement,
} from "react";

interface TriggerProps {
  "aria-describedby"?: string | undefined;
  onFocus?: (e: FocusEvent<HTMLElement>) => void;
  onBlur?: (e: FocusEvent<HTMLElement>) => void;
  onMouseEnter?: (e: MouseEvent<HTMLElement>) => void;
  onMouseLeave?: (e: MouseEvent<HTMLElement>) => void;
}

interface Props {
  /** Supplementary text only. Essential information (e.g. why a control is disabled) must be visible. */
  content: string;
  children: ReactElement<TriggerProps>;
  placement?: "right" | "bottom";
  /** When false, the tooltip never shows (e.g. the label is already visible). */
  enabled?: boolean;
}

/** Shows on hover and keyboard focus; Escape dismisses; described-by keeps it available to AT. */
export function Tooltip({ content, children, placement = "bottom", enabled = true }: Props) {
  const id = useId();
  const [open, setOpen] = useState(false);
  const visible = enabled && open;

  useEffect(() => {
    if (!visible) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  }, [visible]);

  const p = children.props;
  const trigger = cloneElement(children, {
    "aria-describedby": enabled
      ? [p["aria-describedby"], id].filter(Boolean).join(" ")
      : p["aria-describedby"],
    onFocus: (e) => {
      p.onFocus?.(e);
      setOpen(true);
    },
    onBlur: (e) => {
      p.onBlur?.(e);
      setOpen(false);
    },
    onMouseEnter: (e) => {
      p.onMouseEnter?.(e);
      setOpen(true);
    },
    onMouseLeave: (e) => {
      p.onMouseLeave?.(e);
      setOpen(false);
    },
  });

  return (
    <span className="tooltip-anchor">
      {trigger}
      {enabled && (
        <span
          id={id}
          role="tooltip"
          className="tooltip"
          data-placement={placement}
          hidden={!visible}
        >
          {content}
        </span>
      )}
    </span>
  );
}
