// This test reads tokens.css from disk; only it opts into Node types, app code stays Node-free.
/// <reference types="node" />
import { readFileSync } from "node:fs";
import { resolve as resolvePath } from "node:path";

// Reads the real token file (Vitest stubs CSS imports) so palette edits cannot silently break contrast.
const css = readFileSync(resolvePath(process.cwd(), "src/theme/tokens.css"), "utf8");
const tokens = new Map(
  [...css.matchAll(/--([\w-]+):\s*([^;]+);/g)].map((m) => [m[1] ?? "", (m[2] ?? "").trim()]),
);

function resolve(name: string): string {
  const raw = tokens.get(name);
  if (!raw) throw new Error(`missing token --${name}`);
  const ref = /^var\(--([\w-]+)\)$/.exec(raw);
  return ref?.[1] ? resolve(ref[1]) : raw;
}

function luminance(hex: string): number {
  const m = /^#([0-9a-f]{6})$/i.exec(hex);
  if (!m?.[1]) throw new Error(`not a solid hex colour: ${hex}`);
  const n = parseInt(m[1], 16);
  const [r, g, b] = [(n >> 16) & 255, (n >> 8) & 255, n & 255].map((c) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  }) as [number, number, number];
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

export function contrast(fg: string, bg: string): number {
  const [a, b] = [luminance(resolve(fg)), luminance(resolve(bg))].sort((x, y) => y - x) as [
    number,
    number,
  ];
  return (a + 0.05) / (b + 0.05);
}

// [foreground, background, minimum] — 4.5 for text, 3 for UI graphics and focus indicators.
const PAIRS: [string, string, number][] = [
  ["text", "bg", 4.5],
  ["text", "surface-hover", 4.5],
  ["text-muted", "bg", 4.5],
  ["text-muted", "surface", 4.5],
  ["text-muted", "surface-raised", 4.5],
  ["text-muted", "surface-hover", 4.5],
  ["text-subtle", "bg", 4.5],
  ["text-subtle", "surface", 4.5],
  ["accent-text", "bg", 4.5],
  ["accent-text", "surface", 4.5],
  ["accent-text", "surface-raised", 4.5],
  ["accent-text", "surface-hover", 4.5],
  ["text-subtle", "surface-raised", 4.5],
  ["on-accent", "accent", 4.5],
  ["on-accent", "accent-hover", 4.5],
  ["focus-ring", "bg", 3],
  ["focus-ring", "surface-hover", 3],
  ["focus-ring", "surface-raised", 3],
  ["accent-indicator", "surface-hover", 3],
  ["accent-indicator", "surface-raised", 3],
  ["accent", "bg", 3],
  ["danger", "bg", 4.5],
  ["warning", "bg", 4.5],
  ["star", "bg", 3],
  ["star-empty", "bg", 3],
  ["border-strong", "bg", 3],
  ["border-strong", "surface", 3],
];

// Grey application, blue accents: large surfaces stay neutral (R, G, B within a few steps).
const NEUTRAL = ["bg", "bg-sidebar", "surface", "surface-raised", "surface-hover", "border"];

describe("palette contrast (WCAG 2.2)", () => {
  it.each(PAIRS)("--%s on --%s ≥ %s:1", (fg, bg, min) => {
    expect(contrast(fg, bg)).toBeGreaterThanOrEqual(min);
  });
});

describe("neutral surfaces", () => {
  it.each(NEUTRAL)("--%s is grey, not tinted blue", (name) => {
    const hex = resolve(name);
    const n = parseInt(hex.slice(1), 16);
    const [r, g, b] = [(n >> 16) & 255, (n >> 8) & 255, n & 255];
    expect(Math.max(r, g, b) - Math.min(r, g, b)).toBeLessThanOrEqual(8);
  });
});
