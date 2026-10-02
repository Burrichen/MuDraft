import { useId, useState } from "react";
import { Button } from "../../components/Button";
import {
  FIELDS,
  type ColumnMapping,
  type Field,
  type SessionSummary,
} from "../../services/csvImport";

/** Choose which column holds each field, with a preview of the first rows. */
export function MappingStep({
  summary,
  busy,
  onApply,
}: {
  summary: SessionSummary;
  busy: boolean;
  onApply: (mapping: ColumnMapping) => void;
}) {
  const [mapping, setMapping] = useState<ColumnMapping>(
    summary.mapping ?? summary.suggestedMapping,
  );
  const baseId = useId();
  const missing = FIELDS.filter((f) => f.required && mapping[f.field] === null).map((f) => f.label);
  const used = Object.values(mapping).filter((v): v is number => v !== null);
  const clash = used.length !== new Set(used).size;
  const reason = busy
    ? "Applying…"
    : missing.length > 0
      ? `Choose the ${missing.join(" and ")} column${missing.length > 1 ? "s" : ""}.`
      : clash
        ? "Each column can be used for one field only."
        : undefined;

  const set = (field: Field, value: string) => {
    setMapping({ ...mapping, [field]: value === "" ? null : Number(value) });
  };

  return (
    <section className="panel section" aria-labelledby={`${baseId}-title`}>
      <h2 id={`${baseId}-title`} className="section-title">
        Match columns
      </h2>
      <p className="setting-hint">
        Tell MuDraft which column holds each value. Album and Artist are required. Artist names are
        used exactly as written — they are never split on “&” or commas.
      </p>
      <div className="import-mapping">
        {FIELDS.map(({ field, label, required }) => (
          <div key={field} className="match-field">
            <label htmlFor={`${baseId}-${field}`}>
              {label}
              {required ? " (required)" : ""}
            </label>
            <select
              id={`${baseId}-${field}`}
              className="import-select"
              value={mapping[field] ?? ""}
              onChange={(e) => {
                set(field, e.target.value);
              }}
            >
              <option value="">{required ? "Choose a column…" : "Not in this file"}</option>
              {summary.headers.map((h, i) => (
                <option key={`${String(i)}-${h}`} value={i}>
                  {h || `Column ${String(i + 1)}`}
                </option>
              ))}
            </select>
          </div>
        ))}
      </div>

      {summary.sample.length > 0 && (
        <div className="import-table-wrap">
          <table className="import-table">
            <caption className="setting-hint">First rows of {summary.fileName}</caption>
            <thead>
              <tr>
                {summary.headers.map((h, i) => (
                  <th key={`${String(i)}-${h}`} scope="col">
                    {h || `Column ${String(i + 1)}`}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {summary.sample.map((row, r) => (
                <tr key={r}>
                  {summary.headers.map((_, c) => (
                    <td key={c}>{row[c] ?? ""}</td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      <div>
        <Button
          variant="primary"
          disabledReason={reason}
          onClick={() => {
            onApply(mapping);
          }}
        >
          Use these columns
        </Button>
      </div>
    </section>
  );
}
