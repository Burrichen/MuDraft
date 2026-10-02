import { useState } from "react";
import { Icon } from "./Icon";

interface Props {
  src?: string | null | undefined;
  title: string;
  size?: "card" | "row" | "hero";
}

function initials(title: string): string {
  const words = title.trim().split(/\s+/).filter(Boolean);
  return words
    .slice(0, 2)
    .map((w) => Array.from(w)[0] ?? "")
    .join("")
    .toUpperCase();
}

/** Stable hue per title so placeholders are distinguishable but stay in the slate/blue family. */
function hue(title: string): number {
  let h = 0;
  for (const ch of title) h = (h * 31 + (ch.codePointAt(0) ?? 0)) % 360;
  return 200 + (h % 60); // 200–259: cyan through indigo
}

/**
 * Always a square frame. Images are contained, not cropped, so non-square artwork is
 * letterboxed instead of being cut like a film poster. Broken or missing art falls back
 * to a placeholder. Decorative: the album title is always shown alongside.
 */
export function Artwork({ src, title, size = "card" }: Props) {
  const [failed, setFailed] = useState(false);
  const showImage = Boolean(src) && !failed;
  return (
    <div className="artwork" data-size={size}>
      {showImage ? (
        <img
          className="artwork-image"
          src={src ?? undefined}
          alt=""
          loading="lazy"
          decoding="async"
          draggable={false}
          onError={() => {
            setFailed(true);
          }}
        />
      ) : (
        <div
          className="artwork-placeholder"
          data-testid="artwork-placeholder"
          style={{ "--placeholder-hue": hue(title) } as React.CSSProperties}
          aria-hidden="true"
        >
          <Icon name="music" size={size === "row" ? 16 : 28} />
          {size !== "row" && <span className="artwork-initials">{initials(title)}</span>}
        </div>
      )}
    </div>
  );
}
