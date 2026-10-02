import type {
  CurrentPick,
  EmptyReason,
  Filter,
  GuidedCriteria,
  MatchedBy,
  Method,
  YearChoice,
} from "../../services/nextUp";

export const AGNOSTIC = "agnostic";
export const CURRENT_YEAR = "current";

/** Bubble values per category; `[AGNOSTIC]` means the category is bypassed. */
export interface Selection {
  years: string[];
  genres: string[];
  tags: string[];
}

export const ALL_AGNOSTIC: Selection = { years: [AGNOSTIC], genres: [AGNOSTIC], tags: [AGNOSTIC] };

export const decadeValue = (decade: number) => `d${String(decade)}`;
export const decadeLabel = (decade: number) => `${String(decade)}s`;

function filter<T>(values: string[], map: (v: string) => T): Filter<T> {
  return values.includes(AGNOSTIC) || values.length === 0
    ? "agnostic"
    : { any_of: values.map(map) };
}

export function toCriteria(s: Selection): GuidedCriteria {
  return {
    years: filter<YearChoice>(s.years, (v) =>
      v === CURRENT_YEAR ? "current_year" : { decade: Number(v.slice(1)) },
    ),
    genres: filter(s.genres, (v) => v),
    tagIds: filter(s.tags, (v) => v),
  };
}

function values<T>(f: Filter<T> | undefined, map: (v: T) => string): string[] {
  return !f || f === "agnostic" ? [AGNOSTIC] : f.any_of.map(map);
}

/** Restore bubbles from a stored (canonical) method, e.g. after Change Choices. */
export function fromMethod(method: Method | null | undefined): Selection {
  if (method?.mode !== "guided") return ALL_AGNOSTIC;
  const c = method.criteria;
  return {
    years: values(c.years, (y) => (y === "current_year" ? CURRENT_YEAR : decadeValue(y.decade))),
    genres: values(c.genres, (g) => g),
    tags: values(c.tagIds, (t) => t),
  };
}

export const METHOD_LABEL: Record<Method["mode"], string> = {
  guided: "Guided Recommendation",
  completely_random: "Completely Random",
  weighted_random: "Weighted Random",
};

export function emptyMessage(reason: EmptyReason): string {
  switch (reason) {
    case "listen_list_empty":
      return "Your Listen List is empty, so there’s nothing to pick from. Add albums to it first.";
    case "no_listen_asap":
      return "No albums on your Listen List are tagged Listen ASAP. Weighted Random only picks from those — tag some albums, or use Completely Random.";
    case "no_guided_matches":
      return "No albums on your Listen List match all of these choices. Remove a choice to widen the search.";
  }
}

/** One sentence explaining why the current album was picked. */
export function explainMatch(matched: MatchedBy, year: number | null): string {
  const method = matched.method;
  if (!method) return "You chose this album yourself.";
  if (method.mode === "completely_random") return "Picked at random from your whole Listen List.";
  if (method.mode === "weighted_random")
    return "Tagged Listen ASAP, and picked at random from only those albums.";
  const c = method.criteria;
  const part = (label: string, f: Filter<unknown> | undefined, hits: string[]) =>
    `${label}: ${!f || f === "agnostic" ? "any" : hits.join(" or ")}`;
  const years = matched.years.map((y) =>
    y === "current_year" ? `this year (${String(year)})` : decadeLabel(y.decade),
  );
  return `Matches your choices — ${[
    part("Decade", c.years, years),
    part("Genre", c.genres, matched.genres),
    part(
      "Tag",
      c.tagIds,
      matched.tags.map((t) => t.name),
    ),
  ].join(" · ")}.`;
}

export function eligibilityMessage(pick: CurrentPick): string | null {
  switch (pick.eligibility) {
    case "eligible":
      return null;
    case "not_on_listen_list":
      return "This album is no longer on your Listen List. It stays your Next Up until you change it.";
    case "edition_changed":
      return "Your Listen List now has a different edition of this album.";
    case "no_longer_matches":
      return "This album no longer matches the choices that picked it (its tags, genres, or year changed).";
  }
}
