import { useEffect, useId, useState } from "react";
import { Button } from "../../components/Button";
import { FilterBubble } from "../../components/Filters";
import { toggleFilter } from "../../components/filterSelection";
import { useLibraryVersion } from "../../services/libraryEvents";
import { nextUpOptions, type GuidedOptions, type Method } from "../../services/nextUp";
import {
  AGNOSTIC,
  CURRENT_YEAR,
  decadeLabel,
  decadeValue,
  METHOD_LABEL,
  toCriteria,
  type Selection,
} from "./criteria";

interface Bubble {
  value: string;
  label: string;
  count?: number;
  disabledReason?: string;
}

function BubbleGroup({
  label,
  bubbles,
  selected,
  onChange,
}: {
  label: string;
  bubbles: Bubble[];
  selected: string[];
  onChange: (next: string[]) => void;
}) {
  const labelId = useId();
  return (
    <div className="filter-group next-up-group" role="group" aria-labelledby={labelId}>
      <span id={labelId} className="filter-group-label">
        {label}
      </span>
      <div className="filter-bubbles">
        {[{ value: AGNOSTIC, label: "Agnostic" }, ...bubbles].map((b) => {
          const pressed = selected.includes(b.value);
          return (
            <FilterBubble
              key={b.value}
              label={b.label}
              count={b.count}
              pressed={pressed}
              // A chosen option always stays removable, even if it is now unavailable.
              disabledReason={pressed ? undefined : b.disabledReason}
              onToggle={() => {
                onChange(toggleFilter(selected, b.value, AGNOSTIC));
              }}
            />
          );
        })}
      </div>
    </div>
  );
}

/** Keep chosen values visible (and removable) even when they are no longer offered. */
function withSelected(bubbles: Bubble[], selected: string[], label: (v: string) => string) {
  const missing = selected
    .filter((v) => v !== AGNOSTIC && !bubbles.some((b) => b.value === v))
    .map((v) => ({ value: v, label: label(v), count: 0 }));
  return [...bubbles, ...missing];
}

const plural = (n: number) => `${String(n)} album${n === 1 ? "" : "s"}`;

/**
 * Picking method cards. The random methods pick immediately; Guided Recommendation opens
 * its choices below on the same page. Counts are live and filters are never relaxed.
 */
export function MethodChooser({
  selection,
  onSelectionChange,
  guidedOpen,
  onGuidedOpenChange,
  busy,
  onRoll,
}: {
  selection: Selection;
  onSelectionChange: (s: Selection) => void;
  guidedOpen: boolean;
  onGuidedOpenChange: (open: boolean) => void;
  busy: boolean;
  onRoll: (method: Method) => void;
}) {
  const version = useLibraryVersion();
  const ids = { guided: useId(), random: useId(), weighted: useId(), panel: useId() };
  const [options, setOptions] = useState<GuidedOptions | null>(null);
  const [error, setError] = useState<string | null>(null);
  const criteria = toCriteria(selection);
  const key = JSON.stringify(criteria);

  useEffect(() => {
    let cancelled = false;
    nextUpOptions(JSON.parse(key) as typeof criteria).then(
      (o) => {
        if (cancelled) return;
        setOptions(o);
        setError(null);
      },
      (err: unknown) => {
        if (cancelled) return;
        const code = typeof err === "object" && err && "code" in err ? String(err.code) : "";
        if (code === "not_found") {
          // A chosen tag was deleted; say so instead of quietly widening the search.
          setError("A tag you chose no longer exists, so Tag was set back to Agnostic.");
          onSelectionChange({ ...selection, tags: [AGNOSTIC] });
        } else {
          setError(err instanceof Error ? err.message : String(err));
        }
      },
    );
    return () => {
      cancelled = true;
    };
    // `selection` is captured only for the deleted-tag reset; `key` drives refetches.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, version]);

  const decades: Bubble[] = withSelected(
    [
      ...(options?.decades ?? []).map((d) => ({
        value: decadeValue(d.value),
        label: decadeLabel(d.value),
        count: d.count,
      })),
      ...(options?.currentYear
        ? [
            {
              value: CURRENT_YEAR,
              label: String(options.currentYear.value),
              count: options.currentYear.count,
            },
          ]
        : []),
    ],
    selection.years,
    (v) => (v === CURRENT_YEAR ? "This year" : decadeLabel(Number(v.slice(1)))),
  );
  const genres: Bubble[] = withSelected(
    (options?.genres ?? []).map((g) => ({ value: g.key, label: g.name, count: g.count })),
    selection.genres,
    (v) => v,
  );
  const tags: Bubble[] = (options?.tags ?? []).map((t) => ({
    value: t.id,
    label: t.name,
    count: t.count,
    ...(t.onListenList === 0
      ? {
          disabledReason:
            t.anywhere === 0
              ? "Not applied to any album yet"
              : "Only on albums outside your Listen List",
        }
      : {}),
  }));

  const matchCount = options?.matchCount ?? null;
  const findReason = busy
    ? "Picking…"
    : matchCount === null
      ? "Counting matches…"
      : matchCount === 0
        ? "No albums match these choices."
        : undefined;

  return (
    <section className="next-up-chooser" aria-labelledby="next-up-how">
      <h2 id="next-up-how" className="section-title">
        How should Next Up pick?
      </h2>
      <div className="method-cards" role="group" aria-labelledby="next-up-how">
        <button
          type="button"
          className="method-card"
          aria-label={METHOD_LABEL.guided}
          aria-describedby={ids.guided}
          aria-expanded={guidedOpen}
          aria-controls={ids.panel}
          onClick={() => {
            onGuidedOpenChange(!guidedOpen);
          }}
        >
          <span className="method-title">{METHOD_LABEL.guided}</span>
          <span id={ids.guided} className="method-hint">
            Narrow by decade, genre, and tag, then pick at random from the matches.
          </span>
        </button>
        <button
          type="button"
          className="method-card"
          aria-label={METHOD_LABEL.completely_random}
          aria-describedby={ids.random}
          aria-disabled={busy || undefined}
          onClick={() => {
            if (!busy) onRoll({ mode: "completely_random" });
          }}
        >
          <span className="method-title">{METHOD_LABEL.completely_random}</span>
          <span id={ids.random} className="method-hint">
            Any album on your Listen List
            {options ? ` · ${plural(options.total)}` : ""}
          </span>
        </button>
        <button
          type="button"
          className="method-card"
          aria-label={METHOD_LABEL.weighted_random}
          aria-describedby={ids.weighted}
          aria-disabled={busy || undefined}
          onClick={() => {
            if (!busy) onRoll({ mode: "weighted_random" });
          }}
        >
          <span className="method-title">{METHOD_LABEL.weighted_random}</span>
          <span id={ids.weighted} className="method-hint">
            Only albums tagged Listen ASAP
            {options ? ` · ${plural(options.listenAsap)}` : ""}
          </span>
        </button>
      </div>

      {guidedOpen && (
        <div id={ids.panel} className="panel section next-up-guided">
          <BubbleGroup
            label="Decade"
            bubbles={decades}
            selected={selection.years}
            onChange={(years) => {
              onSelectionChange({ ...selection, years });
            }}
          />
          <BubbleGroup
            label="Genre"
            bubbles={genres}
            selected={selection.genres}
            onChange={(g) => {
              onSelectionChange({ ...selection, genres: g });
            }}
          />
          <BubbleGroup
            label="Tag"
            bubbles={withSelected(tags, selection.tags, () => "Deleted tag")}
            selected={selection.tags}
            onChange={(t) => {
              onSelectionChange({ ...selection, tags: t });
            }}
          />
          <p className="setting-hint">
            Choose several options in a category to match any of them; categories combine, so an
            album must match each one. Bubble numbers count matches with your other categories’
            choices.
            {options && options.unknownYears > 0
              ? ` ${plural(options.unknownYears)} with an unknown year can match only when Decade is Agnostic.`
              : ""}
          </p>
          <div className="next-up-find">
            <p
              className={matchCount === 0 ? "match-note match-note-error" : "next-up-count"}
              aria-live="polite"
            >
              {matchCount === null
                ? "Counting matches…"
                : matchCount === 0
                  ? "No albums on your Listen List match all of these choices. Remove a choice to widen the search."
                  : `${plural(matchCount)} match`}
            </p>
            <Button
              variant="primary"
              disabledReason={findReason}
              onClick={() => {
                onRoll({ mode: "guided", criteria });
              }}
            >
              Find an album
            </Button>
          </div>
        </div>
      )}
      {error && (
        <p className="match-note match-note-error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
