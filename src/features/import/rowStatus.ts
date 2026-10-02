import type { StagedRow } from "../../services/csvImport";

/** Plain-language status for a row. */
export function rowStatus(row: StagedRow): {
  label: string;
  tone: "ok" | "warn" | "bad" | "muted";
} {
  switch (row.readiness) {
    case "error":
      return { label: row.issues.map((i) => i.message).join(" "), tone: "bad" };
    case "skipped":
      return {
        label: row.duplicateOf !== null ? `Duplicate of row ${String(row.duplicateOf)}` : "Skipped",
        tone: "muted",
      };
    case "needs_edition":
      return { label: "Choose an edition", tone: "warn" };
    case "needs_lookup":
      return {
        label:
          row.candidates?.kind === "failed" ? row.candidates.message : "Needs a MusicBrainz lookup",
        tone: "warn",
      };
    case "ready":
      break;
  }
  const d = row.decision;
  if (d.kind === "use_existing") {
    const where = row.existing
      ? `“${row.existing.albumTitle}” (${row.existing.editionName})`
      : "your library";
    switch (d.reason) {
      case "previous_import":
        return { label: `Imported before: ${where}`, tone: "muted" };
      case "same_name":
        return { label: `Possible duplicate of ${where} — check`, tone: "warn" };
      default:
        return { label: `Already in library: ${where}`, tone: "muted" };
    }
  }
  if (d.kind === "match") {
    return { label: d.chosenBy === "user" ? "Matched (your choice)" : "Matched by ID", tone: "ok" };
  }
  const n = row.candidates?.kind === "groups" ? row.candidates.candidates.length : 0;
  return n > 0
    ? { label: `Manual entry · ${String(n)} possible matches to review`, tone: "warn" }
    : { label: "Manual entry", tone: "ok" };
}
