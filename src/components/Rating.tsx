import { useId, type KeyboardEvent } from "react";

import { formatRating, type HalfStars } from "./ratingFormat";

const STAR_PATH = "M12 2.5l2.9 6.1 6.6.8-4.9 4.6 1.3 6.6L12 17.3l-5.9 3.3 1.3-6.6-4.9-4.6 6.6-.8z";

function Star({ fill }: { fill: 0 | 0.5 | 1 }) {
  return (
    <span className="star" data-fill={fill} aria-hidden="true">
      <svg viewBox="0 0 24 24" className="star-empty">
        <path d={STAR_PATH} />
      </svg>
      <svg viewBox="0 0 24 24" className="star-full">
        <path d={STAR_PATH} />
      </svg>
    </span>
  );
}

function fillFor(value: HalfStars, index: number): 0 | 0.5 | 1 {
  if (value === null) return 0;
  const remaining = value - index * 2;
  return remaining >= 2 ? 1 : remaining === 1 ? 0.5 : 0;
}

export function RatingDisplay({ value }: { value: HalfStars }) {
  return (
    <span className="rating-display" data-unrated={value === null}>
      <span className="stars" aria-hidden="true">
        {[0, 1, 2, 3, 4].map((i) => (
          <Star key={i} fill={fillFor(value, i)} />
        ))}
      </span>
      <span className="visually-hidden">{formatRating(value)}</span>
    </span>
  );
}

const KEY_STEPS: Record<string, (v: number) => number> = {
  ArrowRight: (v) => v + 1,
  ArrowUp: (v) => v + 1,
  ArrowLeft: (v) => v - 1,
  ArrowDown: (v) => v - 1,
  PageUp: (v) => v + 2,
  PageDown: (v) => v - 2,
  Home: () => 0,
  End: () => 10,
};

/**
 * Accessible half-star rating input. Keyboard: arrows ±½ star, Page Up/Down ±1 star,
 * Home = 0, End = 5, Delete/Backspace = unrated. Pointer: click a star half, or "0".
 * Unavailable states show their reason as visible text.
 */
export function RatingInput({
  label,
  value,
  onChange,
  disabledReason,
  compact = false,
}: {
  label: string;
  value: HalfStars;
  onChange: (value: HalfStars) => void;
  disabledReason?: string | undefined;
  /** Smaller stars with a visually hidden label (e.g. tracklist rows). */
  compact?: boolean;
}) {
  const labelId = useId();
  const hintId = useId();
  const reasonId = useId();
  const unavailable = disabledReason !== undefined;

  const set = (next: HalfStars) => {
    if (unavailable) return;
    const clamped = next === null ? null : Math.min(10, Math.max(0, next));
    if (clamped !== value) onChange(clamped);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Delete" || e.key === "Backspace") {
      e.preventDefault();
      set(null);
      return;
    }
    const step = KEY_STEPS[e.key];
    if (!step) return;
    e.preventDefault();
    // From unrated, steps start at zero (clamped), so Right → ½ star and Left → 0.
    set(step(value ?? 0));
  };

  return (
    <div className="rating-input" data-unavailable={unavailable} data-compact={compact}>
      <span id={labelId} className={compact ? "visually-hidden" : "rating-label"}>
        {label}
      </span>
      <div className="rating-row">
        {/* Zero is a real rating with its own control, distinct from Clear (unrated). */}
        <button
          type="button"
          className="rating-zero"
          aria-pressed={value === 0}
          aria-label={`Set ${label} to 0 stars`}
          aria-disabled={unavailable || undefined}
          onClick={() => {
            set(0);
          }}
        >
          0
        </button>
        <div
          className="rating-slider"
          role="slider"
          tabIndex={0}
          aria-labelledby={labelId}
          aria-describedby={[hintId, unavailable ? reasonId : ""].filter(Boolean).join(" ")}
          aria-valuemin={0}
          aria-valuemax={5}
          aria-valuenow={value === null ? undefined : value / 2}
          aria-valuetext={formatRating(value)}
          aria-disabled={unavailable || undefined}
          onKeyDown={onKeyDown}
        >
          {[0, 1, 2, 3, 4].map((i) => (
            <span key={i} className="rating-star-hit">
              <Star fill={fillFor(value, i)} />
              <span
                className="rating-half rating-half-left"
                data-testid={`rating-half-${String(i * 2 + 1)}`}
                aria-hidden="true"
                onClick={() => {
                  set(i * 2 + 1);
                }}
              />
              <span
                className="rating-half rating-half-right"
                data-testid={`rating-half-${String(i * 2 + 2)}`}
                aria-hidden="true"
                onClick={() => {
                  set(i * 2 + 2);
                }}
              />
            </span>
          ))}
        </div>
        <span className="rating-value" aria-hidden="true">
          {value === null ? "Unrated" : String(value / 2)}
        </span>
        {value !== null && !unavailable && (
          <button
            type="button"
            className="button button-ghost button-small"
            aria-label={`Clear ${label}`}
            onClick={() => {
              set(null);
            }}
          >
            Clear rating
          </button>
        )}
      </div>
      <span id={hintId} className="visually-hidden">
        Arrow keys change by half a star. Home sets zero, End sets five, Delete clears the rating.
      </span>
      {unavailable && (
        <span id={reasonId} className="disabled-reason">
          {disabledReason}
        </span>
      )}
    </div>
  );
}
