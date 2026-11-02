import { useId, useState, type CSSProperties, type ReactNode } from "react";
import { Link } from "react-router";
import { niceMax } from "./format";

/**
 * Hand-built HTML charts. Every chart has a table twin: hidden from sight (but read by
 * screen readers) until the viewer asks for it with the "Table" toggle, so no value is
 * only reachable by hovering.
 */

export interface ChartRow {
  key: string;
  label: string;
  /** Shorter label under a column, e.g. "Aug" for "Aug 2026". */
  axisLabel?: string;
  value: number;
  /** Tooltip and table text; defaults to the number. */
  display?: string;
  /** Drill-down into the matching albums. */
  href?: string;
}

function ticksFor(max: number): number[] {
  return max >= 2 && Number.isInteger(max / 2) ? [0, max / 2, max] : [0, max];
}

/** Figure with caption, optional subtitle, and a chart/table toggle. */
export function ChartFrame({
  caption,
  subtitle,
  chart,
  table,
  note,
  empty,
  className,
}: {
  caption: string;
  subtitle?: ReactNode;
  /** Visual chart; omitted when there is nothing to plot. */
  chart: ReactNode;
  /** The same data as a table. With `asTable`, links may be shown in it. */
  table: (asTable: boolean) => ReactNode;
  note?: ReactNode;
  /** Shown instead of chart and toggle when set. */
  empty?: string | undefined;
  className?: string;
}) {
  const id = useId();
  const [asTable, setAsTable] = useState(false);
  return (
    <figure className={["chart-card", className].filter(Boolean).join(" ")} aria-labelledby={id}>
      <div className="chart-head">
        <div className="chart-titles">
          <figcaption id={id} className="chart-caption">
            {caption}
          </figcaption>
          {subtitle && <p className="chart-subtitle">{subtitle}</p>}
        </div>
        {!empty && (
          <button
            type="button"
            className="chart-toggle"
            aria-pressed={asTable}
            onClick={() => {
              setAsTable(!asTable);
            }}
          >
            Table
          </button>
        )}
      </div>
      {empty ? (
        <p className="chart-empty">{empty}</p>
      ) : (
        <>
          {!asTable && chart}
          <div className={asTable ? "chart-table" : "visually-hidden"}>{table(asTable)}</div>
        </>
      )}
      {note && <p className="chart-note">{note}</p>}
    </figure>
  );
}

function RowsTable({
  caption,
  valueHeading,
  rows,
  links,
}: {
  caption: string;
  valueHeading: string;
  rows: ChartRow[];
  links: boolean;
}) {
  return (
    <table className="stat-table">
      <thead>
        <tr>
          <th scope="col">{caption}</th>
          <th scope="col">{valueHeading}</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <tr key={r.key}>
            <th scope="row">{links && r.href ? <Link to={r.href}>{r.label}</Link> : r.label}</th>
            <td>{r.display ?? r.value}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/**
 * Vertical columns for ordered data (months, years, decades, rating buckets). Hovering a
 * column shows its value; the tallest is labelled directly. Axis labels are drill-down
 * links when rows have `href`.
 */
export function ColumnChart({
  caption,
  subtitle,
  valueHeading,
  rows,
  empty,
  note,
}: {
  caption: string;
  subtitle?: ReactNode;
  valueHeading: string;
  rows: ChartRow[];
  empty: string;
  note?: ReactNode;
}) {
  const top = Math.max(0, ...rows.map((r) => r.value));
  const max = niceMax(top);
  const ticks = ticksFor(max);
  // Thin out axis labels so they never collide.
  const every = rows.length > 16 ? Math.ceil(rows.length / 8) : 1;
  const peak = rows.findIndex((r) => r.value === top && top > 0);
  return (
    <ChartFrame
      caption={caption}
      subtitle={subtitle}
      note={note}
      empty={rows.length === 0 || top === 0 ? empty : undefined}
      chart={
        <div className="col-chart" style={{ "--cols": rows.length } as CSSProperties}>
          <div className="col-ticks" aria-hidden="true">
            {ticks.map((t) => (
              <span key={t} className="col-tick" style={{ bottom: `${String((t / max) * 100)}%` }}>
                <span className="col-tick-label">{t}</span>
              </span>
            ))}
          </div>
          <div className="col-plot" aria-hidden="true">
            {rows.map((r, i) => {
              const h = (r.value / max) * 100;
              const edge = i === 0 ? "start" : i === rows.length - 1 ? "end" : undefined;
              return (
                <div key={r.key} className="col" data-edge={edge}>
                  {r.value > 0 && <span className="col-bar" style={{ height: `${String(h)}%` }} />}
                  {i === peak && (
                    <span className="col-cap" style={{ bottom: `${String(h)}%` }}>
                      {r.value}
                    </span>
                  )}
                  <span className="col-tip" style={{ bottom: `${String(h)}%` }}>
                    <span className="col-tip-label">{r.label}</span>
                    {r.display ?? `${String(r.value)} ${valueHeading.toLowerCase()}`}
                  </span>
                </div>
              );
            })}
          </div>
          <div className="col-axis">
            {rows.map((r, i) => {
              const text = r.axisLabel ?? r.label;
              const shown = i % every === 0 || i === rows.length - 1;
              return (
                <span key={r.key} className="col-axis-label" data-hidden={!shown || undefined}>
                  {shown && (r.href ? <Link to={r.href}>{text}</Link> : text)}
                </span>
              );
            })}
          </div>
        </div>
      }
      table={(asTable) => (
        <RowsTable caption={caption} valueHeading={valueHeading} rows={rows} links={asTable} />
      )}
    />
  );
}

/** Five stars filled to `stars` (0–5), decorative: pair with the number in text. */
export function StarMeter({ stars }: { stars: number }) {
  return (
    <span className="star-meter" aria-hidden="true">
      <span className="star-meter-fill" style={{ width: `${String((stars / 5) * 100)}%` }}>
        ★★★★★
      </span>
      ★★★★★
    </span>
  );
}
