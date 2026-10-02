import { Link } from "react-router";

/** One overview number. With `href`, the whole card opens the matching albums. */
export function StatCard({
  label,
  value,
  hint,
  href,
}: {
  label: string;
  value: string;
  hint?: string | undefined;
  href?: string | undefined;
}) {
  const body = (
    <>
      <span className="stat-value">{value}</span>
      <span className="stat-label">{label}</span>
      {hint && <span className="stat-hint">{hint}</span>}
    </>
  );
  return (
    <li className="stat-card">
      {href ? (
        <Link className="stat-link" to={href}>
          {body}
        </Link>
      ) : (
        body
      )}
    </li>
  );
}
