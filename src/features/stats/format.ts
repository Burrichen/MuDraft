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

const SHORT_MONTH = new Intl.DateTimeFormat(undefined, { month: "short" });

const monthKey = (d: Date) =>
  `${String(d.getFullYear())}-${String(d.getMonth() + 1).padStart(2, "0")}`;

/** The 12 calendar months ending with `today`'s, with zero for months without listens. */
export function lastTwelveMonths(
  byMonth: readonly { key: string; count: number }[],
  today: Date = new Date(),
): { key: string; label: string; short: string; count: number }[] {
  const counts = new Map(byMonth.map((m) => [m.key, m.count]));
  return Array.from({ length: 12 }, (_, i) => {
    const d = new Date(today.getFullYear(), today.getMonth() - 11 + i, 1);
    const key = monthKey(d);
    return {
      key,
      label: formatMonth(key),
      short: SHORT_MONTH.format(d),
      count: counts.get(key) ?? 0,
    };
  });
}

/** Every year from the first to the last with listens, zero-filled. */
export function continuousYears(
  byYear: readonly { key: string; count: number }[],
): { key: string; count: number }[] {
  const years = byYear.map((y) => Number(y.key)).filter(Number.isInteger);
  if (years.length === 0) return [];
  const counts = new Map(byYear.map((y) => [Number(y.key), y.count]));
  const first = Math.min(...years);
  const last = Math.max(...years);
  return Array.from({ length: last - first + 1 }, (_, i) => ({
    key: String(first + i),
    count: counts.get(first + i) ?? 0,
  }));
}

/**
 * Album scores (exact means, e.g. 3.75) binned into half-star buckets 0, 0.5 … 5. Each
 * bucket holds scores from its value up to the next half star; 5 holds only perfect scores.
 */
export function halfStarBuckets(
  distribution: readonly { stars: number; albums: number }[],
): { stars: number; albums: number }[] {
  const buckets = Array.from({ length: 11 }, (_, i) => ({ stars: i / 2, albums: 0 }));
  for (const d of distribution) {
    const i = Math.min(10, Math.max(0, Math.floor(d.stars * 2 + 1e-9)));
    const bucket = buckets[i];
    if (bucket) bucket.albums += d.albums;
  }
  return buckets;
}

/** "42 h 10 min" as parts, so the number and unit can be styled apart. */
export function durationParts(ms: number): { value: string; unit: string }[] {
  const minutes = Math.round(ms / 60_000);
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return h > 0
    ? [
        { value: h.toLocaleString(), unit: "h" },
        { value: String(m), unit: "min" },
      ]
    : [{ value: String(m), unit: "min" }];
}

/** A clean axis maximum (1, 2, 2.5, 4, 5, 6, 8 × 10ⁿ) that is a whole number. */
export function niceMax(value: number): number {
  if (value <= 0) return 1;
  const steps = [1, 2, 2.5, 4, 5, 6, 8, 10];
  const p = 10 ** Math.floor(Math.log10(value));
  for (const s of steps) {
    const m = s * p;
    if (m >= value && Number.isInteger(m)) return m;
  }
  return Math.ceil(value);
}
