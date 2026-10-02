import { useId } from "react";

/** Labelled completion bar showing the count and percentage as visible text. */
export function ProgressBar({ label, value, max }: { label: string; value: number; max: number }) {
  const labelId = useId();
  const safeMax = Math.max(0, max);
  const safeValue = Math.min(Math.max(0, value), safeMax);
  const percent = safeMax > 0 ? Math.round((safeValue / safeMax) * 100) : 0;
  const text = `${String(safeValue)} of ${String(safeMax)} (${String(percent)}%)`;
  return (
    <div className="progress">
      <div className="progress-header">
        <span id={labelId} className="progress-label">
          {label}
        </span>
        <span className="progress-value" aria-hidden="true">
          {safeValue} / {safeMax} · {percent}%
        </span>
      </div>
      <div
        className="progress-track"
        role="progressbar"
        aria-labelledby={labelId}
        aria-valuemin={0}
        aria-valuemax={safeMax}
        aria-valuenow={safeValue}
        aria-valuetext={text}
      >
        <div className="progress-fill" style={{ width: `${String(percent)}%` }} />
      </div>
    </div>
  );
}
