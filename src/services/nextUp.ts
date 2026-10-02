import { callNative, NativeError } from "../transport/native";
import type { ListItem, TagRef } from "./library";
import { notifyLibraryChanged } from "./libraryEvents";

/** Mirrors `src-tauri/src/{domain/picker,library/next_up}.rs`. */
export type Filter<T> = "agnostic" | { any_of: T[] };
export type YearChoice = { decade: number } | "current_year";

export interface GuidedCriteria {
  years?: Filter<YearChoice>;
  genres?: Filter<string>;
  tagIds?: Filter<string>;
}

export type Method =
  | { mode: "completely_random" }
  | { mode: "weighted_random" }
  | { mode: "guided"; criteria: GuidedCriteria };

export type SelectionSource = "completely_random" | "weighted_random" | "guided" | "manual";
export type EmptyReason = "listen_list_empty" | "no_listen_asap" | "no_guided_matches";
export type Eligibility =
  "eligible" | "not_on_listen_list" | "edition_changed" | "no_longer_matches";

export interface Shown {
  attemptId: string;
  sessionId: string;
  albumId: string;
  editionId: string;
  source: SelectionSource;
}

export type RollOutcome =
  | { kind: "picked"; shown: Shown; poolSize: number; remaining: number }
  | { kind: "empty"; reason: EmptyReason }
  | { kind: "exhausted"; poolSize: number };

export interface MatchedBy {
  /** null for a manual pick. */
  method: Method | null;
  years: YearChoice[];
  genres: string[];
  tags: TagRef[];
}

export interface CurrentPick extends Shown {
  status: "shown" | "skipped" | "completed";
  shownAt: string;
  eligibility: Eligibility;
  matched: MatchedBy;
  item: ListItem;
}

export interface Counted<T> {
  value: T;
  count: number;
}

export interface TagOption extends TagRef {
  /** Matches given the other categories' choices. */
  count: number;
  onListenList: number;
  anywhere: number;
}

export interface GuidedOptions {
  decades: Counted<number>[];
  currentYear: Counted<number> | null;
  unknownYears: number;
  genres: { name: string; key: string; count: number }[];
  tags: TagOption[];
  matchCount: number;
  listenAsap: number;
  total: number;
}

export interface NextUpState {
  current: CurrentPick | null;
  session: {
    sessionId: string;
    method: Method;
    poolCycle: number;
    poolSize: number;
    remaining: number;
    exhausted: boolean;
  } | null;
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null;
}

function invalid(what: string): NativeError {
  return new NativeError("invalid_response", `${what} had an unexpected shape`);
}

export async function nextUpState(): Promise<NextUpState> {
  const raw = await callNative("next_up_state");
  if (!isRecord(raw) || !("current" in raw) || !("session" in raw)) throw invalid("Next Up");
  return raw as unknown as NextUpState;
}

/** `requestId` makes a double-clicked reroll roll once. Rolling never logs a listen. */
export async function rollNextUp(requestId: string, method: Method): Promise<RollOutcome> {
  const raw = await callNative("next_up_roll", { requestId, method });
  if (!isRecord(raw) || !["picked", "empty", "exhausted"].includes(String(raw.kind)))
    throw invalid("Next Up roll");
  return raw as unknown as RollOutcome;
}

/** Explicitly starts the pool again after it is exhausted; resolves to the new cycle. */
export async function resetNextUpPool(): Promise<number> {
  const raw = await callNative("next_up_reset_pool");
  if (typeof raw !== "number") throw invalid("Pool reset");
  return raw;
}

export async function chooseNextUp(requestId: string, editionId: string): Promise<Shown> {
  const raw = await callNative("next_up_choose", { requestId, editionId });
  if (!isRecord(raw) || typeof raw.attemptId !== "string") throw invalid("Next Up pick");
  notifyLibraryChanged();
  return raw as unknown as Shown;
}

/** Choices and live counts for `criteria` (default: everything Agnostic). */
export async function nextUpOptions(criteria?: GuidedCriteria): Promise<GuidedOptions> {
  const raw = await callNative("next_up_options", { criteria: criteria ?? null });
  if (!isRecord(raw) || typeof raw.matchCount !== "number" || !Array.isArray(raw.tags))
    throw invalid("Next Up choices");
  return raw as unknown as GuidedOptions;
}

/** Clears the current pick without logging a listen. */
export async function clearNextUp(): Promise<boolean> {
  const raw = await callNative("next_up_clear");
  notifyLibraryChanged();
  return raw === true;
}
