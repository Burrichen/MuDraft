import type { EditionCandidate } from "../../services/metadata";

/**
 * Words that describe a physical or digital format rather than an edition. MusicBrainz
 * often puts these in a release's disambiguation ("vinyl", "180g", "digital"), which
 * would otherwise split one edition into several entries.
 */
const FORMAT_WORDS =
  /\b(?:\d+\s*x\s*)?(?:\d+\s*)?(?:lp|cd|cds|vinyl|cassette|tape|digital(?:\s+media)?|download|streaming|stream|hi-?res|\d+\s*-?\s*bit|\d+\s*(?:g|gram|gr)|colou?red|picture\s+disc|black|clear|splatter|limited\s+pressing|pressing|repress|sacd|shm-?cd|blu-?spec(?:\s+cd2?)?|mini-?lp|gatefold|jewel\s*case|digipak|digipack)\b/gi;

/** An edition's label with format words removed, e.g. "Deluxe, 2LP vinyl" → "Deluxe". */
export function editionLabel(disambiguation: string | null): string {
  if (!disambiguation) return "";
  return disambiguation
    .replace(FORMAT_WORDS, " ")
    .replace(/[(),;:/+&·-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function groupKey(e: EditionCandidate): string {
  const label = editionLabel(e.disambiguation)
    .replace(/\b(?:edition|version|release)\b/gi, " ")
    .replace(/\s+/g, " ")
    .trim()
    .toLowerCase();
  return [e.title.trim().toLowerCase(), label, e.trackCount].join("|");
}

const isOfficial = (e: EditionCandidate) => e.status === "Official";
const isDigital = (e: EditionCandidate) => e.formats.some((f) => /digital/i.test(f));
const dateOf = (e: EditionCandidate) => e.date.value ?? "9999";

/**
 * One entry per edition (Standard, Deluxe, Expanded…) instead of one per CD, vinyl,
 * cassette, country or digital release. Releases with the same title, edition label,
 * and track count are the same edition. The entry kept for each is, in order: one
 * `preferred` says to keep (e.g. already in the library), official, digital, earliest.
 * Bootlegs and promos are dropped when an official release exists.
 */
export function collapseEditions<T extends EditionCandidate>(
  editions: readonly T[],
  preferred: (e: T) => boolean = () => false,
): T[] {
  const anyOfficial = editions.some(isOfficial);
  const kept = anyOfficial
    ? editions.filter((e) => e.status === null || isOfficial(e) || preferred(e))
    : editions;
  const rank = (e: T) => [preferred(e) ? 0 : 1, isOfficial(e) ? 0 : 1, isDigital(e) ? 0 : 1];
  const better = (a: T, b: T) => {
    const ra = rank(a);
    const rb = rank(b);
    for (let i = 0; i < ra.length; i++) if (ra[i] !== rb[i]) return (ra[i] ?? 0) < (rb[i] ?? 0);
    return dateOf(a) < dateOf(b);
  };
  const groups = new Map<string, T>();
  for (const e of kept) {
    const key = groupKey(e);
    const current = groups.get(key);
    if (!current || better(e, current)) groups.set(key, e);
  }
  // Earliest edition first; ties keep the provider's order.
  return [...groups.values()].sort((a, b) => dateOf(a).localeCompare(dateOf(b)));
}
