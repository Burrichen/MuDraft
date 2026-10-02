import { useId } from "react";
import { toggleFilter } from "./filterSelection";
import { Icon, type IconName } from "./Icon";

export interface FilterOption {
  value: string;
  label: string;
  count?: number;
}

export function FilterBubble({
  label,
  pressed,
  onToggle,
  count,
  icon,
  disabledReason,
}: {
  label: string;
  pressed: boolean;
  onToggle: () => void;
  count?: number | undefined;
  icon?: IconName;
  /** Unavailable: stays focusable and shows this reason as visible, linked text. */
  disabledReason?: string | undefined;
}) {
  const reasonId = useId();
  const unavailable = disabledReason !== undefined;
  const bubble = (
    <button
      type="button"
      className="filter-bubble"
      aria-pressed={pressed}
      aria-disabled={unavailable || undefined}
      aria-describedby={unavailable ? reasonId : undefined}
      onClick={() => {
        if (!unavailable) onToggle();
      }}
    >
      {icon && <Icon name={icon} size={16} />}
      <span className="filter-label">{label}</span>
      {count !== undefined && <span className="filter-count">{count}</span>}
    </button>
  );
  if (!unavailable) return bubble;
  return (
    <span className="bubble-with-reason">
      {bubble}
      <span id={reasonId} className="disabled-reason">
        {disabledReason}
      </span>
    </span>
  );
}

export function FilterGroup({
  label,
  options,
  selected,
  onChange,
  exclusive,
}: {
  label: string;
  options: readonly FilterOption[];
  selected: readonly string[];
  onChange: (next: string[]) => void;
  exclusive?: string;
}) {
  const labelId = useId();
  return (
    <div className="filter-group" role="group" aria-labelledby={labelId}>
      <span id={labelId} className="filter-group-label">
        {label}
      </span>
      <div className="filter-bubbles">
        {options.map((o) => (
          <FilterBubble
            key={o.value}
            label={o.label}
            count={o.count}
            pressed={selected.includes(o.value)}
            onToggle={() => {
              onChange(toggleFilter(selected, o.value, exclusive));
            }}
          />
        ))}
      </div>
    </div>
  );
}

export function LayoutToggle({
  value,
  onChange,
}: {
  value: "grid" | "list";
  onChange: (layout: "grid" | "list") => void;
}) {
  const labelId = useId();
  return (
    <div className="filter-group layout-toggle" role="group" aria-labelledby={labelId}>
      <span id={labelId} className="filter-group-label">
        Layout
      </span>
      <div className="filter-bubbles">
        <FilterBubble
          label="Grid"
          icon="grid"
          pressed={value === "grid"}
          onToggle={() => {
            onChange("grid");
          }}
        />
        <FilterBubble
          label="List"
          icon="list"
          pressed={value === "list"}
          onToggle={() => {
            onChange("list");
          }}
        />
      </div>
    </div>
  );
}
