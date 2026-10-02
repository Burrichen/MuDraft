/** Mirrors `PRIMARY_TYPES` / `SECONDARY_TYPES` in `library/discography.rs`. */
export const PRIMARY_TYPES = ["Album", "EP", "Single", "Broadcast", "Other"] as const;
export const SECONDARY_TYPES = [
  "Compilation",
  "Live",
  "Soundtrack",
  "Remix",
  "DJ-mix",
  "Mixtape/Street",
  "Demo",
  "Spokenword",
  "Interview",
  "Audiobook",
  "Audio drama",
  "Field recording",
] as const;

/** Aggregates may be fractional: "3.75 of 5 stars"; zero stays "0 of 5 stars". */
export function formatStars(stars: number): string {
  const rounded = Math.round(stars * 100) / 100;
  return `${String(rounded)} of 5 stars`;
}

export function formatHalfStars(halfStars: number): string {
  return formatStars(halfStars / 2);
}
