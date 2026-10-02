import type { CSSProperties } from "react";
import { readableTextOn } from "../theme/color";

/** A tag in its colour, with text colour chosen for contrast. */
export function TagChip({
  name,
  color,
  builtin = false,
}: {
  name: string;
  color: string;
  builtin?: boolean;
}) {
  return (
    <span
      className="tag-chip"
      style={{ "--tag-bg": color, "--tag-fg": readableTextOn(color) } as CSSProperties}
      title={builtin ? `${name} (built-in)` : name}
    >
      {name}
    </span>
  );
}
