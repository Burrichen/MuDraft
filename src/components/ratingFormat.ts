/** Ratings are half-star integers 0–10 (matching Rust); null is unrated. Zero is a rating. */
export type HalfStars = number | null;

export function formatRating(value: HalfStars): string {
  if (value === null) return "Unrated";
  const stars = value / 2;
  return `${stars.toFixed(stars % 1 === 0 ? 0 : 1)} of 5 stars`;
}
