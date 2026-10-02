import type { CommitReport } from "../../services/csvImport";

const OUTCOME_LABEL = {
  created: "Created",
  added_edition: "Added edition",
  reused: "Already in library",
  skipped: "Skipped",
  error: "Not imported",
} as const;

export function ImportReport({ report }: { report: CommitReport }) {
  const problems = report.rows.filter((r) => r.outcome === "skipped" || r.outcome === "error");
  const facts: [string, number][] = [
    ["Albums created", report.albumsCreated],
    ["Editions created", report.editionsCreated],
    ["Already in your library", report.reusedExisting],
    ["Added to Listen List", report.addedToListenList],
    ["Already on Listen List", report.alreadyOnListenList],
    ["Skipped", report.skipped],
    ["Rows with problems", report.errors],
    ["Tag links added", report.tagLinksAdded],
  ];
  return (
    <section className="panel section" aria-labelledby="import-report">
      <h2 id="import-report" className="section-title">
        Import complete
      </h2>
      <dl className="facts">
        {facts.map(([label, value]) => (
          <div key={label} className="import-fact">
            <dt>{label}</dt>
            <dd>{value}</dd>
          </div>
        ))}
      </dl>
      {report.tagsCreated.length > 0 && <p>New tags: {report.tagsCreated.join(", ")}</p>}
      {report.tagsSkipped.length > 0 && (
        <p className="setting-hint">Not created: {report.tagsSkipped.join(", ")}</p>
      )}
      {problems.length > 0 && (
        <div className="import-table-wrap">
          <table className="import-table">
            <caption className="setting-hint">Rows that were not imported</caption>
            <thead>
              <tr>
                <th scope="col">Row</th>
                <th scope="col">Result</th>
                <th scope="col">Why</th>
              </tr>
            </thead>
            <tbody>
              {problems.map((r) => (
                <tr key={r.rowNumber}>
                  <td>{r.rowNumber}</td>
                  <td>{OUTCOME_LABEL[r.outcome]}</td>
                  <td>{r.detail ?? "Skipped during review"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}
