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
export type EmptyReason =
  | "listen_list_empty"
  | "no_listen_asap"
  | "no_guided_matches"
  /** Every matching album is already another slot's pick. */
  | "all_in_next_up";
export type Eligibility =
  "eligible" | "not_on_listen_list" | "edition_changed" | "no_longer_matches";

export interface Shown {
  /** Which Next Up slot (1 to `maxSlots`) holds the pick. */
  slot: number;
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
  /** False once the user has deleted the built-in Listen ASAP tag. */
  listenAsapTag: boolean;
  total: number;
}

/** A slot's picking method and its no-repeat pool. */
export interface PoolView {
  sessionId: string;
  method: Method;
  poolCycle: number;
  poolSize: number;
  /** Matching albums not yet shown this round (other slots' picks excluded). */
  remaining: number;
  exhausted: boolean;
}

/** One Next Up slot: a pick, a picking method to roll with, or both. */
export interface NextUpSlot {
  slot: number;
  current: CurrentPick | null;
  session: PoolView | null;
}

export interface NextUpState {
  /** Slots in use, in order; free slots are omitted. */
  slots: NextUpSlot[];
  maxSlots: number;
  /** Where a new pick goes; null when every slot is in use. */
  nextFreeSlot: number | null;
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null;
}

function invalid(what: string): NativeError {
  return new NativeError("invalid_response", `${what} had an unexpected shape`);
}

export async function nextUpState(): Promise<NextUpState> {
  const raw = await callNative("next_up_state");
  if (!isRecord(raw) || !Array.isArray(raw.slots) || typeof raw.maxSlots !== "number")
    throw invalid("Next Up");
  return raw as unknown as NextUpState;
}

/**
 * Roll (or reroll) one slot. `requestId` makes a double-clicked reroll roll once. Rolling
 * never logs a listen and never picks another slot's album.
 */
export async function rollNextUp(
  requestId: string,
  slot: number,
  method: Method,
): Promise<RollOutcome> {
  const raw = await callNative("next_up_roll", { requestId, slot, method });
  if (!isRecord(raw) || !["picked", "empty", "exhausted"].includes(String(raw.kind)))
    throw invalid("Next Up roll");
  return raw as unknown as RollOutcome;
}

/** Explicitly starts a slot's pool again after it is exhausted; resolves to the new cycle. */
export async function resetNextUpPool(slot: number): Promise<number> {
  const raw = await callNative("next_up_reset_pool", { slot });
  if (typeof raw !== "number") throw invalid("Pool reset");
  return raw;
}

/** Picks by hand into `slot` (replacing its pick), or else into the lowest free slot. */
export async function chooseNextUp(
  requestId: string,
  editionId: string,
  slot?: number,
): Promise<Shown> {
  const raw = await callNative("next_up_choose", { requestId, editionId, slot: slot ?? null });
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

/** Removes a slot (its pick and picking method) without logging a listen. */
export async function clearNextUp(slot: number): Promise<boolean> {
  const raw = await callNative("next_up_clear", { slot });
  notifyLibraryChanged();
  return raw === true;
}
