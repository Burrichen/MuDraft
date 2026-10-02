/** Aggregate stars may be fractional ("2.75 of 5 stars"); zero stays "0 of 5 stars". */
export function formatStars(stars: number): string {
  return `${String(Math.round(stars * 100) / 100)} of 5 stars`;
}

export function formatDuration(ms: number): string {
  const minutes = Math.round(ms / 60_000);
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return h > 0 ? `${String(h)} h ${String(m)} min` : `${String(m)} min`;
}

export const plural = (n: number, one: string, many = `${one}s`) =>
  `${String(n)} ${n === 1 ? one : many}`;

const MONTHS = new Intl.DateTimeFormat(undefined, { month: "short", year: "numeric" });

/** "2026-08" → "Aug 2026" on the local calendar (no time-zone shift). */
export function formatMonth(key: string): string {
  const [y, m] = key.split("-").map(Number);
  return y && m ? MONTHS.format(new Date(y, m - 1, 1)) : key;
}

export const METHOD_LABEL: Record<string, string> = {
  completely_random: "Completely Random",
  weighted_random: "Weighted Random",
  guided: "Guided Recommendation",
  manual: "Chosen by you",
  all: "All picks",
};
