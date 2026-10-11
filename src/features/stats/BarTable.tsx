import { useId, type ReactNode } from "react";
import { Link } from "react-router";

export interface BarRow {
  key: string;
  label: string;
  value: number;
  /** Visible value text; defaults to the number. */
  display?: string;
  /** Drill-down into the matching albums. */
  href?: string;
}

/**
 * A ranked horizontal bar chart that is also its own text alternative: a captioned table
 * with one row per bar. The bars are decorative (the value is printed at each bar's end);
 * labels never rotate or truncate, so they stay readable at any width or zoom.
 */
export function BarTable({
  caption,
  subtitle,
  valueHeading,
  rows,
  empty,
  note,
  max: fixedMax,
  ranked = false,
}: {
  caption: string;
  subtitle?: ReactNode;
  valueHeading: string;
  rows: BarRow[];
  empty: string;
  note?: ReactNode;
  /** Scale end, e.g. 5 for star ratings; defaults to the largest value. */
  max?: number;
  /** Number the rows 1, 2, 3… */
  ranked?: boolean;
}) {
  const id = useId();
  const max = fixedMax ?? Math.max(0, ...rows.map((r) => r.value));
  return (
    <figure className="chart-card bar-table" aria-labelledby={id}>
      <div className="chart-head">
        <div className="chart-titles">
          <figcaption id={id} className="chart-caption">
            {caption}
          </figcaption>
          {subtitle && <p className="chart-subtitle">{subtitle}</p>}
        </div>
      </div>
      {rows.length === 0 ? (
        <p className="chart-empty">{empty}</p>
      ) : (
        <table>
          <thead className="visually-hidden">
            <tr>
              <th scope="col">{caption}</th>
              <th scope="col">Bar</th>
              <th scope="col">{valueHeading}</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r, i) => (
              <tr key={r.key}>
                <th scope="row" className="bar-label">
                  {ranked && (
                    <span className="bar-rank" aria-hidden="true">
                      {i + 1}
                    </span>
                  )}
                  {r.href ? <Link to={r.href}>{r.label}</Link> : r.label}
                </th>
                <td className="bar-cell" aria-hidden="true">
                  <span className="bar-track">
                    <span
                      className="bar-fill"
                      style={{ width: `${String(max > 0 ? (r.value / max) * 100 : 0)}%` }}
                    />
                  </span>
                </td>
                <td className="bar-value">{r.display ?? r.value}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {note && <p className="chart-note">{note}</p>}
    </figure>
  );
}
