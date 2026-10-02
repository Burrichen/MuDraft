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
 * A bar chart that is also its own text alternative: a captioned table with one row per
 * bar. The bars are decorative (the numbers are in the next column); labels never rotate
 * or truncate, so they stay readable at any width or zoom.
 */
export function BarTable({
  caption,
  valueHeading,
  rows,
  empty,
  note,
}: {
  caption: string;
  valueHeading: string;
  rows: BarRow[];
  empty: string;
  note?: ReactNode;
}) {
  const id = useId();
  const max = Math.max(0, ...rows.map((r) => r.value));
  return (
    <figure className="bar-table" aria-labelledby={id}>
      <figcaption id={id} className="bar-table-caption">
        {caption}
      </figcaption>
      {rows.length === 0 ? (
        <p className="setting-hint">{empty}</p>
      ) : (
        <table>
          <thead className="visually-hidden">
            <tr>
              <th scope="col">{caption}</th>
              <th scope="col">{valueHeading}</th>
              <th scope="col">Bar</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <tr key={r.key}>
                <th scope="row" className="bar-label">
                  {r.href ? <Link to={r.href}>{r.label}</Link> : r.label}
                </th>
                <td className="bar-value">{r.display ?? r.value}</td>
                <td className="bar-cell" aria-hidden="true">
                  <span
                    className="bar-fill"
                    style={{ width: `${String(max > 0 ? (r.value / max) * 100 : 0)}%` }}
                  />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {note && <p className="setting-hint">{note}</p>}
    </figure>
  );
}
