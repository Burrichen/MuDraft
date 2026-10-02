/**
 * Toggle `value` in a multi-select. When `exclusive` is given (e.g. "Agnostic"), it is
 * mutually exclusive with every other option and is what an empty selection falls back to.
 */
export function toggleFilter(
  selected: readonly string[],
  value: string,
  exclusive?: string,
): string[] {
  if (exclusive !== undefined && value === exclusive) return [exclusive];
  const rest = selected.filter((v) => v !== exclusive);
  const next = rest.includes(value) ? rest.filter((v) => v !== value) : [...rest, value];
  if (exclusive !== undefined && next.length === 0) return [exclusive];
  return next;
}
